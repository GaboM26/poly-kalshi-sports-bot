//! Detection: price updates trigger the arbitrage calculation and opportunity notifications.

use super::*;

impl WebSocketManager {
    /// Handle incoming price update
    pub fn on_price_update(&self, update: PriceUpdate) {
        if update.platform == Platform::Kalshi {
            self.on_kalshi_price_update(update);
        }
    }

    /// Handle Kalshi price update
    pub(super) fn on_kalshi_price_update(&self, update: PriceUpdate) {
        let start = Instant::now();

        *self.kalshi_update_count.write() += 1;
        *self.kalshi_ws_last_update_time.write() = Some(Utc::now());

        if !*self.kalshi_connected.read() {
            *self.kalshi_connected.write() = true;
            info!("✅ [Kalshi] 开始接收实时价格数据");
        }

        if let (Some(yb), Some(ya), Some(nb), Some(na)) =
            (update.yes_bid, update.yes_ask, update.no_bid, update.no_ask)
        {
            debug!(
                "[Kalshi] 价格更新: {} - Yes: {:.2}/{:.2}, No: {:.2}/{:.2}",
                update.market_id, yb, ya, nb, na
            );

            self.kalshi_ws_prices
                .write()
                .insert(update.market_id.clone(), (yb, ya, nb, na));
        }

        if let (Some(ya), Some(na)) = (update.yes_ask, update.no_ask) {
            self.kalshi_ws_msgs.fetch_add(1, Ordering::Relaxed);
            self.apply_kalshi_ws_quote(&update.market_id, ya, na);
        }

        self.metrics
            .record(Operation::KalshiWsProcess, start.elapsed());
    }

    /// Apply asks derived from the live Kalshi order book (best ask on one
    /// side is the complement of the best bid on the other) and re-run
    /// detection immediately when the best prices changed. Price-only: this
    /// never touches the depth cache execution relies on. Returns the number
    /// of matched markets whose prices changed.
    pub fn apply_kalshi_ws_quote(&self, ticker: &str, yes_ask: f64, no_ask: f64) -> usize {
        let received_at = Instant::now();
        let changed = {
            let mut cache = self.kalshi_rest_quotes.write();
            let changed = cache.get(ticker).map_or(true, |q| {
                (q.yes_ask - yes_ask).abs() > 1e-9 || (q.no_ask - no_ask).abs() > 1e-9
            });
            // Any book message proves the feed is live, so re-stamp even
            // when the best prices didn't move (deeper-level deltas).
            cache.insert(
                ticker.to_string(),
                KalshiRestQuote { yes_ask, no_ask, received_at },
            );
            changed
        };
        self.kalshi_ws_quote_at
            .write()
            .insert(ticker.to_string(), received_at);
        if !changed {
            return 0;
        }

        let mut updated_indices = Vec::new();
        {
            let mut matched_markets = self.matched_markets.write();
            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                if matched.kalshi_market.market_id == ticker {
                    matched.kalshi_market.yes_price = yes_ask;
                    matched.kalshi_market.no_price = no_ask;
                    updated_indices.push(idx);
                }
            }
        }
        for idx in &updated_indices {
            self.calculate_and_notify(*idx);
        }
        updated_indices.len()
    }

    /// Check if a matched market has complete data
    pub(crate) fn is_market_ready(&self, idx: usize) -> bool {
        let markets = self.matched_markets.read();
        if idx >= markets.len() {
            return false;
        }
        let mm = &markets[idx];

        let has_fresh_kalshi_quote = self.fresh_kalshi_rest_prices(mm, Instant::now()).is_some();
        let has_fresh_poly_quote = self.fresh_poly_rest_prices(mm, Instant::now()).is_some();

        has_fresh_kalshi_quote && has_fresh_poly_quote
    }

    /// Calculate arbitrage and notify subscribers
    pub(crate) fn calculate_and_notify(&self, idx: usize) {
        if !self.is_market_ready(idx) {
            debug!("[计算] 市场 {} 数据未就绪", idx);
            return;
        }

        let start = Instant::now();
        *self.calculation_count.write() += 1;

        let markets = self.matched_markets.read();
        let mm = &markets[idx];

        debug!("[计算] 开始计算套利: {} - {}", mm.event_name, mm.team_name);

        let (k_yes_ask, k_no_ask) = match self.fresh_kalshi_rest_prices(mm, Instant::now()) {
            Some(prices) => prices,
            None => return,
        };

        let (p_yes, p_no) = match self.fresh_poly_rest_prices(mm, Instant::now()) {
            Some(prices) => prices,
            None => return,
        };

        drop(markets);

        let markets = self.matched_markets.read();
        let mm = &markets[idx];

        let opportunity = self.calculator.calculate_single(
            &mm.event_name,
            &mm.team_name,
            &mm.kalshi_market,
            k_yes_ask,
            k_no_ask,
            &mm.polymarket_market,
            p_yes,
            p_no,
        );

        let kalshi_ticker = mm.kalshi_market.market_id.clone();
        // The Polymarket leg buys the competitor opposite the Kalshi side:
        // Kalshi YES pairs with the opponent, Kalshi NO with the tracked
        // team. The native side comes from the competitor mapping only.
        let poly_market = mm.polymarket_market.clone();
        let team_name = mm.team_name.clone();

        drop(markets);

        if let Some(mut opp) = opportunity {
            let poly_competitor = if opp.kalshi_side == "yes" {
                poly_market.get_opponent(&team_name).map(str::to_string)
            } else {
                Some(team_name.clone())
            };
            let poly_execution = poly_competitor
                .as_deref()
                .and_then(|c| poly_market.us_execution_for_competitor(c));

            // Depth comes from the live WebSocket book when one has arrived
            // (display/tracking only - never order sizing); zero otherwise,
            // rather than manufacturing depth from display quotes.
            let (poly_usd, poly_size) = poly_execution
                .as_ref()
                .and_then(|exec| {
                    self.polymarket_live_book(&exec.market_slug)
                        .map(|book| book.buy_depth(exec.position_side, opp.polymarket_price))
                })
                .unwrap_or((0.0, 0.0));
            opp.poly_ask_depth = poly_usd;
            opp.poly_ask_size = poly_size;
            opp.kalshi_ask_depth = self.get_kalshi_ask_depth(&kalshi_ticker, &opp.kalshi_side, opp.kalshi_price);

            let _ = self.opportunity_tx.send(opp.clone());

            if opp.profit_margin >= self.tracking_threshold.get() {
                self.track_opportunity(&opp, poly_execution);
            }

            self.update_opportunities(opp);
        } else {
            let markets = self.matched_markets.read();
            if idx < markets.len() {
                let mm = &markets[idx];
                let key = mm.market_key();
                drop(markets);
                self.maybe_end_tracking(&key);
            }
        }

        self.metrics
            .record(Operation::ArbitrageCalc, start.elapsed());
    }

    /// Update opportunities list
    pub(crate) fn update_opportunities(&self, opp: ArbitrageOpportunity) {
        let mut opps = self.opportunities.write();

        let key = opp.market_key();
        if let Some(pos) = opps.iter().position(|o| o.market_key() == key) {
            opps[pos] = opp;
        } else {
            opps.push(opp);
        }

        opps.sort_by(|a, b| {
            b.profit_margin
                .partial_cmp(&a.profit_margin)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        opps.truncate(50);
    }

    /// Get current opportunities
    pub fn get_opportunities(&self) -> Vec<ArbitrageOpportunity> {
        self.opportunities.read().clone()
    }

    /// Calculate all opportunities (for periodic scanning)
    pub fn calculate_all(&self) -> Vec<ArbitrageOpportunity> {
        let start = Instant::now();
        let mut opportunities = Vec::new();
        let len = self.matched_markets.read().len();

        for idx in 0..len {
            if !self.is_market_ready(idx) {
                continue;
            }

            let markets = self.matched_markets.read();
            let mm = &markets[idx];

            let (k_yes_ask, k_no_ask) = match self.fresh_kalshi_rest_prices(mm, Instant::now()) {
                Some(prices) => prices,
                None => continue,
            };

            let (p_yes, p_no) = match self.fresh_poly_rest_prices(mm, Instant::now()) {
                Some(prices) => prices,
                None => continue,
            };

            if let Some(opp) = self.calculator.calculate_single(
                &mm.event_name,
                &mm.team_name,
                &mm.kalshi_market,
                k_yes_ask,
                k_no_ask,
                &mm.polymarket_market,
                p_yes,
                p_no,
            ) {
                opportunities.push(opp);
            }
        }

        opportunities.sort_by(|a, b| {
            b.profit_margin
                .partial_cmp(&a.profit_margin)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        *self.opportunities.write() = opportunities.clone();

        self.metrics.record(Operation::FullScan, start.elapsed());

        opportunities
    }
}
