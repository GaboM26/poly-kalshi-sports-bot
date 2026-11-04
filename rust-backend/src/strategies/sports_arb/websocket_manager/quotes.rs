//! REST quote caches for both exchanges, their freshness, and executable Kalshi depth.

use super::*;

/// A Polymarket US quote received from the gateway REST feed.
///
/// This intentionally contains prices only. The gateway's market-side
/// identifiers are not CLOB asset IDs and must never be treated as such.
#[derive(Debug, Clone)]
pub(crate) struct PolymarketRestQuote {
    pub price_a: f64,
    pub price_b: f64,
    pub received_at: Instant,
}

/// An authoritative Kalshi REST quote for calculation and UI display.
///
/// Order-book quantities remain exclusively in the Kalshi WebSocket cache.
#[derive(Debug, Clone)]
pub(crate) struct KalshiRestQuote {
    pub yes_ask: f64,
    pub no_ask: f64,
    pub received_at: Instant,
}

impl WebSocketManager {
    /// Kalshi contracts available at `price` (dollars, the quoted ask) or
    /// better for a market and side: the same measure Polymarket depth uses.
    pub(crate) fn get_kalshi_ask_depth(&self, ticker: &str, side: &str, price: f64) -> i32 {
        let Some(book) = self
            .kalshi_client
            .as_ref()
            .and_then(|client| client.get_orderbook(ticker))
        else {
            return 0;
        };
        let limit_cents = (price * 100.0).round() as i32;
        crate::strategies::sports_arb::execution::kalshi_buy_levels(&book, &side.to_lowercase())
            .iter()
            .filter(|(cents, _)| *cents <= limit_cents)
            .fold(0_i32, |total, (_, qty)| total.saturating_add(*qty))
    }

    /// Apply authoritative Kalshi REST asks to the currently matched markets.
    ///
    /// This price-only cache must never populate or relax WebSocket order-book
    /// depth checks used by manual or automatic trading.
    pub fn update_kalshi_rest_quotes(&self, quotes: &[KalshiMarketQuote]) -> usize {
        self.update_kalshi_rest_quotes_at(quotes, Instant::now())
    }

    /// Same as `update_kalshi_rest_quotes`, for a snapshot whose request
    /// started at `fetched_at`. A ticker that received a WebSocket-derived
    /// quote after that instant keeps the newer WebSocket prices; the REST
    /// cycle still applies closed/settled status and refreshes freshness.
    pub fn update_kalshi_rest_quotes_at(
        &self,
        quotes: &[KalshiMarketQuote],
        fetched_at: Instant,
    ) -> usize {
        let ws_quote_at = self.kalshi_ws_quote_at.read().clone();
        let quotes_by_ticker: HashMap<&str, &KalshiMarketQuote> = quotes
            .iter()
            .map(|quote| (quote.market_id.as_str(), quote))
            .collect();
        let received_at = Instant::now();
        let mut updated_indices = Vec::new();
        let mut kept_ws = 0usize;
        let mut quotes_to_cache = HashMap::new();
        let mut newly_ended_keys = Vec::new();

        {
            let mut matched_markets = self.matched_markets.write();

            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                let Some(quote) = quotes_by_ticker.get(matched.kalshi_market.market_id.as_str())
                else {
                    continue;
                };

                // Kalshi's own market status is authoritative and doesn't
                // flicker - trust "closed"/"settled" immediately rather than
                // waiting on the extreme-price heuristic in
                // market_lifecycle.rs, which can miss a market that closes
                // while its price is nowhere near an extreme value (e.g. the
                // market simply closed for trading at 84c, not 99c/2c).
                if matches!(quote.status.as_str(), "closed" | "settled") {
                    newly_ended_keys.push(matched.market_key());
                    continue;
                }

                // Kalshi serves REST market data from a server-side cache
                // (values step every ~8-10s), so a REST snapshot fetched
                // after a WS quote can still hold older prices and would
                // overwrite them, producing a phantom opportunity. While a
                // book from the live, sequence-checked connection backs this
                // ticker, the WebSocket owns its prices.
                let ws_owns_prices = ws_quote_at.contains_key(&matched.kalshi_market.market_id)
                    && self.kalshi_client.as_ref().is_some_and(|client| {
                        client
                            .get_fresh_orderbook(
                                &matched.kalshi_market.market_id,
                                self.settings.executable_book_max_age(),
                            )
                            .is_some()
                    });
                if ws_owns_prices
                    || ws_quote_at
                        .get(&matched.kalshi_market.market_id)
                        .is_some_and(|at| *at > fetched_at)
                {
                    // Keep the WebSocket prices; only re-stamp.
                    kept_ws += 1;
                    quotes_to_cache
                        .entry(matched.kalshi_market.market_id.clone())
                        .or_insert_with(|| KalshiRestQuote {
                            yes_ask: matched.kalshi_market.yes_price,
                            no_ask: matched.kalshi_market.no_price,
                            received_at,
                        });
                    continue;
                }

                // The REST ask is the buy cost shown in the UI and used by
                // arbitrage calculations; bids are intentionally ignored.
                matched.kalshi_market.yes_price = quote.yes_ask;
                matched.kalshi_market.no_price = quote.no_ask;
                updated_indices.push(idx);
                quotes_to_cache
                    .entry(matched.kalshi_market.market_id.clone())
                    .or_insert_with(|| KalshiRestQuote {
                        yes_ask: quote.yes_ask,
                        no_ask: quote.no_ask,
                        received_at,
                    });
            }
        }

        if !newly_ended_keys.is_empty() {
            let mut confirmed = self.confirmed_ended_markets.write();
            for key in &newly_ended_keys {
                if confirmed.insert(key.clone()) {
                    info!("🏁 Kalshi market status confirms closure: {}", key);
                }
            }
        }

        if !quotes_to_cache.is_empty() {
            self.kalshi_rest_quotes.write().extend(quotes_to_cache);
        }

        *self.kalshi_rest_connected.write() = true;
        *self.kalshi_rest_last_success.write() = Some(received_at);
        *self.kalshi_rest_last_update_time.write() = Some(Utc::now());

        for idx in &updated_indices {
            self.calculate_and_notify(*idx);
        }

        updated_indices.len() + kept_ws
    }

    /// Apply current Polymarket US gateway quotes to already matched markets.
    ///
    /// The caller fetches the same supported-sports feed used for discovery.
    /// Only price fields for known matched market IDs are updated; this never
    /// changes subscriptions, token mappings, or order-book state.
    pub fn update_polymarket_rest_quotes(&self, markets: &[PolymarketMarket]) -> usize {
        self.update_polymarket_rest_quotes_at(markets, Instant::now())
    }

    /// Same as `update_polymarket_rest_quotes`, for a snapshot whose request
    /// started at `fetched_at`. The WebSocket keeps ownership of a market's
    /// prices if it quoted that market after `fetched_at`, or - while the
    /// stream is alive - at any point on the current connection: the
    /// gateway feed is Cloudflare-cached for up to 30s, so a snapshot
    /// fetched after a WS quote can still carry far older prices (that is
    /// what produced a 90ms phantom opportunity on 2026-10-01). REST prices
    /// apply only when the WebSocket has nothing current for the market.
    /// The REST cycle still confirms listing and refreshes freshness.
    pub fn update_polymarket_rest_quotes_at(
        &self,
        markets: &[PolymarketMarket],
        fetched_at: Instant,
    ) -> usize {
        let ws_quote_at = self.poly_ws_quote_at.read().clone();
        let ws_quote_epoch = self.poly_ws_quote_epoch.read().clone();
        let live_ws_epoch = self.live_polymarket_ws_epoch(self.settings.polymarket_ws_max_silence());
        let quotes_by_market: HashMap<&str, &PolymarketMarket> = markets
            .iter()
            .map(|market| (market.market_id.as_str(), market))
            .collect();
        let received_at = Instant::now();
        let mut updated_indices = Vec::new();
        let mut kept_ws = 0usize;
        let mut quotes_to_cache = HashMap::new();
        let mut newly_ended_keys = Vec::new();
        let mut present_keys = std::collections::HashSet::new();

        {
            let mut matched_markets = self.matched_markets.write();

            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                let Some(quote) =
                    quotes_by_market.get(matched.polymarket_market.market_id.as_str())
                else {
                    // This scan is the same active=true&closed=false feed
                    // discovery uses, fetched atomically (the caller only
                    // reaches here on a fully successful fetch) - a matched
                    // market missing from it means Polymarket itself no
                    // longer lists the market as active. Debounced via
                    // poly_missing_streak so one transient gap doesn't
                    // remove a market that is still genuinely open.
                    let market_key = matched.market_key();
                    let mut streaks = self.poly_missing_streak.write();
                    let streak = streaks.entry(market_key.clone()).or_insert(0);
                    *streak += 1;
                    if *streak >= self.settings.ended_markets.polymarket_missing_cycles {
                        newly_ended_keys.push(market_key);
                    }
                    continue;
                };
                present_keys.insert(matched.market_key());
                // In-play state only comes from the REST events feed, so
                // take it regardless of which source owns the prices.
                matched.polymarket_market.live = quote.live;
                matched.polymarket_market.period = quote.period.clone();
                let market_id = &matched.polymarket_market.market_id;
                let ws_owns_prices = ws_quote_at.get(market_id).is_some_and(|at| *at > fetched_at)
                    || live_ws_epoch.is_some_and(|epoch| ws_quote_epoch.get(market_id) == Some(&epoch));
                if ws_owns_prices {
                    // Keep the newer WebSocket prices; only re-stamp.
                    kept_ws += 1;
                    quotes_to_cache
                        .entry(matched.polymarket_market.market_id.clone())
                        .or_insert_with(|| PolymarketRestQuote {
                            price_a: matched.polymarket_market.price_a,
                            price_b: matched.polymarket_market.price_b,
                            received_at,
                        });
                    continue;
                }
                let Ok((yes_price, no_price)) = quote.get_price_for_team(&matched.team_name) else {
                    continue;
                };

                // Keep optional, legitimate CLOB token IDs intact. Gateway
                // market-side identifiers are quote data, not CLOB assets.
                matched.polymarket_market.price_a = quote.price_a;
                matched.polymarket_market.price_b = quote.price_b;
                matched.poly_yes_price = yes_price;
                matched.poly_no_price = no_price;
                updated_indices.push(idx);
                quotes_to_cache
                    .entry(matched.polymarket_market.market_id.clone())
                    .or_insert_with(|| PolymarketRestQuote {
                        price_a: quote.price_a,
                        price_b: quote.price_b,
                        received_at,
                    });
            }
        }

        // Clear the miss streak for anything seen this cycle so non-
        // consecutive gaps (flicker) never accumulate toward the threshold.
        {
            let mut streaks = self.poly_missing_streak.write();
            for key in &present_keys {
                streaks.remove(key);
            }
        }

        if !newly_ended_keys.is_empty() {
            let mut confirmed = self.confirmed_ended_markets.write();
            let mut streaks = self.poly_missing_streak.write();
            for key in &newly_ended_keys {
                streaks.remove(key);
                if confirmed.insert(key.clone()) {
                    info!(
                        "🏁 Polymarket no longer lists market as active: {}",
                        key
                    );
                }
            }
        }

        if !quotes_to_cache.is_empty() {
            self.poly_rest_quotes.write().extend(quotes_to_cache);
        }

        *self.polymarket_rest_connected.write() = true;
        *self.polymarket_rest_last_success.write() = Some(received_at);
        *self.polymarket_last_update_time.write() = Some(Utc::now());
        *self.polymarket_update_count.write() += updated_indices.len() as u64;

        for idx in &updated_indices {
            self.calculate_and_notify(*idx);
        }

        updated_indices.len() + kept_ws
    }

    pub(super) fn is_poly_rest_quote_fresh_at(&self, quote: &PolymarketRestQuote, now: Instant) -> bool {
        now.saturating_duration_since(quote.received_at) <= self.poly_quote_freshness
    }

    pub(super) fn is_kalshi_rest_quote_fresh_at(&self, quote: &KalshiRestQuote, now: Instant) -> bool {
        now.saturating_duration_since(quote.received_at) <= self.kalshi_quote_freshness
    }

    pub(super) fn fresh_kalshi_rest_prices(
        &self,
        matched_market: &MatchedMarket,
        now: Instant,
    ) -> Option<(f64, f64)> {
        let quote = self
            .kalshi_rest_quotes
            .read()
            .get(&matched_market.kalshi_market.market_id)
            .cloned()?;
        self.is_kalshi_rest_quote_fresh_at(&quote, now)
            .then_some((quote.yes_ask, quote.no_ask))
    }

    pub(super) fn fresh_poly_rest_prices(
        &self,
        matched_market: &MatchedMarket,
        now: Instant,
    ) -> Option<(f64, f64)> {
        let quote = self
            .poly_rest_quotes
            .read()
            .get(&matched_market.polymarket_market.market_id)
            .cloned()?;
        // A quote the live WebSocket pushed stays exact while that
        // connection is alive (every change is pushed), however quiet the
        // market; only a gateway quote ages out.
        if !self.is_poly_rest_quote_fresh_at(&quote, now)
            && !self.polymarket_price_is_live(&matched_market.polymarket_market.market_id)
        {
            return None;
        }

        if matched_market
            .team_name
            .eq_ignore_ascii_case(&matched_market.polymarket_market.team_a)
        {
            Some((quote.price_a, quote.price_b))
        } else if matched_market
            .team_name
            .eq_ignore_ascii_case(&matched_market.polymarket_market.team_b)
        {
            Some((quote.price_b, quote.price_a))
        } else {
            None
        }
    }

    pub(super) fn is_kalshi_rest_available(&self, now: Instant) -> bool {
        *self.kalshi_rest_connected.read()
            && self
                .kalshi_rest_last_success
                .read()
                .is_some_and(|last_success| {
                    now.saturating_duration_since(last_success) <= self.kalshi_quote_freshness
                })
    }

    pub(super) fn is_polymarket_rest_available(&self, now: Instant) -> bool {
        *self.polymarket_rest_connected.read()
            && self
                .polymarket_rest_last_success
                .read()
                .is_some_and(|last_success| {
                    now.saturating_duration_since(last_success) <= self.poly_quote_freshness
                })
    }
}
