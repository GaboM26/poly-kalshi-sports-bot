//! Read-only views for the API and the frontend: stats, data coverage, the market list.

use super::*;

/// Helper function to capitalize first letter
pub(super) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Data coverage statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct DataCoverage {
    pub total_markets: usize,
    pub kalshi_ready: usize,
    pub polymarket_ready: usize,
    pub both_ready: usize,
    pub kalshi_coverage: String,
    pub polymarket_coverage: String,
    pub full_coverage: String,
    pub kalshi_connected: bool,
    pub polymarket_connected: bool,
    /// Kalshi market data source; current prices come from REST polling.
    pub kalshi_source: &'static str,
    /// Polymarket market data source; currently `rest_polling`.
    pub polymarket_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymarket_latency_ms: Option<i64>,
}

impl WebSocketManager {
    /// Get system statistics
    pub fn get_stats(&self) -> SystemStats {
        SystemStats {
            total_kalshi_events: 0,
            total_kalshi_markets: 0,
            total_polymarket_events: 0,
            total_polymarket_markets: 0,
            matched_events: 0,
            matched_markets: self.matched_markets.read().len(),
            arbitrage_opportunities: self.opportunities.read().len(),
            kalshi_ws_connected: *self.kalshi_connected.read(),
            polymarket_ws_connected: false,
            last_update: Some(Utc::now()),
        }
    }

    /// Get data coverage statistics
    pub fn get_data_coverage(&self) -> DataCoverage {
        let markets = self.matched_markets.read();
        let kalshi_rest_quotes = self.kalshi_rest_quotes.read();
        let poly_rest_quotes = self.poly_rest_quotes.read();
        let now_instant = Instant::now();

        let mut kalshi_ready = 0;
        let mut poly_ready = 0;
        let mut both_ready = 0;

        for mm in markets.iter() {
            let has_kalshi = kalshi_rest_quotes
                .get(&mm.kalshi_market.market_id)
                .is_some_and(|quote| self.is_kalshi_rest_quote_fresh_at(quote, now_instant));

            let has_poly = poly_rest_quotes
                .get(&mm.polymarket_market.market_id)
                .is_some_and(|quote| self.is_poly_rest_quote_fresh_at(quote, now_instant));

            if has_kalshi {
                kalshi_ready += 1;
            }
            if has_poly {
                poly_ready += 1;
            }
            if has_kalshi && has_poly {
                both_ready += 1;
            }
        }

        let total = markets.len();

        let now = Utc::now();
        let kalshi_latency_ms = self
            .kalshi_rest_last_update_time
            .read()
            .map(|last_time| (now - last_time).num_milliseconds());
        let polymarket_latency_ms = self
            .polymarket_last_update_time
            .read()
            .map(|last_time| (now - last_time).num_milliseconds());

        DataCoverage {
            total_markets: total,
            kalshi_ready,
            polymarket_ready: poly_ready,
            both_ready,
            kalshi_coverage: format!("{}/{}", kalshi_ready, total),
            polymarket_coverage: format!("{}/{}", poly_ready, total),
            full_coverage: format!("{}/{}", both_ready, total),
            kalshi_connected: self.is_kalshi_rest_available(now_instant),
            // The gateway poll runs every 30s (CDN max-age); the live
            // WebSocket is what keeps Polymarket prices current.
            polymarket_connected: self.is_polymarket_rest_available(now_instant)
                || self.live_polymarket_ws_epoch(self.settings.polymarket_ws_max_silence()).is_some(),
            kalshi_source: "rest_polling",
            polymarket_source: "rest_polling",
            kalshi_latency_ms,
            polymarket_latency_ms,
        }
    }

    /// Get matched markets formatted for frontend
    pub fn get_matched_markets_for_frontend(&self) -> Vec<MatchedMarketFrontend> {
        let markets = self.matched_markets.read();
        let kalshi_rest_quotes = self.kalshi_rest_quotes.read();
        let poly_rest_quotes = self.poly_rest_quotes.read();
        let opportunities = self.opportunities.read();
        let confirmed_ended = self.confirmed_ended_markets.read();
        let now = Instant::now();

        let opp_map: HashMap<String, &ArbitrageOpportunity> =
            opportunities.iter().map(|o| (o.market_key(), o)).collect();

        markets
            .iter()
            .filter_map(|mm| {
                let key = mm.market_key();

                if confirmed_ended.contains(&key) {
                    return None;
                }

                let fresh_kalshi_quote = kalshi_rest_quotes
                    .get(&mm.kalshi_market.market_id)
                    .filter(|quote| self.is_kalshi_rest_quote_fresh_at(quote, now));
                let (kalshi_ready, k_yes, k_no) = match fresh_kalshi_quote {
                    Some(quote) => (true, quote.yes_ask, quote.no_ask),
                    None => (false, mm.kalshi_market.yes_price, mm.kalshi_market.no_price),
                };

                let own_execution = mm
                    .polymarket_market
                    .us_execution_for_competitor(&mm.team_name)?;
                let opponent = mm.polymarket_market.get_opponent(&mm.team_name)?;
                let opponent_execution =
                    mm.polymarket_market.us_execution_for_competitor(opponent)?;
                let fresh_quote = poly_rest_quotes
                    .get(&mm.polymarket_market.market_id)
                    .filter(|quote| self.is_poly_rest_quote_fresh_at(quote, now));
                let (poly_ready, p_yes, p_no) = match fresh_quote {
                    Some(quote)
                        if mm
                            .team_name
                            .eq_ignore_ascii_case(&mm.polymarket_market.team_a) =>
                    {
                        (true, quote.price_a, quote.price_b)
                    }
                    Some(quote)
                        if mm
                            .team_name
                            .eq_ignore_ascii_case(&mm.polymarket_market.team_b) =>
                    {
                        (true, quote.price_b, quote.price_a)
                    }
                    _ => (false, mm.poly_yes_price, mm.poly_no_price),
                };

                if kalshi_ready && poly_ready {
                    let kalshi_extreme = self.is_kalshi_price_extreme(k_yes, k_no);
                    let poly_extreme = self.is_poly_price_extreme(p_yes, p_no);
                    if kalshi_extreme && poly_extreme {
                        return None;
                    }
                }

                let opportunity = opp_map.get(&key);
                let has_opportunity = opportunity.is_some();
                let (
                    profit_margin,
                    expected_profit,
                    gross_profit,
                    kalshi_contracts,
                    kalshi_fee,
                    polymarket_fee,
                    arbitrage_type,
                ) = if let Some(opp) = opportunity {
                    (
                        opp.profit_margin,
                        opp.expected_profit,
                        Some(opp.gross_profit),
                        Some(opp.kalshi_contracts),
                        Some(opp.kalshi_fee),
                        Some(opp.polymarket_fee),
                        Some(format!(
                            "Kalshi{}Polymarket{}",
                            capitalize(&opp.kalshi_side),
                            capitalize(&opp.polymarket_side)
                        )),
                    )
                } else if kalshi_ready && poly_ready {
                    // Not an opportunity at the configured minimum margin, but
                    // the details page should still show what the better hedge
                    // costs and earns at the current prices, net of BOTH
                    // exchanges' fees (a loss shows as a loss), instead of a
                    // placeholder 0.00%. Display only: `has_opportunity` stays
                    // false, and nothing trades on this.
                    match self.calculator.preview_single(
                        &mm.event_name,
                        &mm.team_name,
                        &mm.kalshi_market,
                        k_yes,
                        k_no,
                        &mm.polymarket_market,
                        p_yes,
                        p_no,
                    ) {
                        Some(preview) => (
                            preview.profit_margin,
                            preview.expected_profit,
                            Some(preview.gross_profit),
                            Some(preview.kalshi_contracts),
                            Some(preview.kalshi_fee),
                            Some(preview.polymarket_fee),
                            Some(format!(
                                "Kalshi{}Polymarket{}",
                                capitalize(&preview.kalshi_side),
                                capitalize(&preview.polymarket_side)
                            )),
                        ),
                        None => (0.0, 0.0, None, None, None, None, None),
                    }
                } else {
                    (0.0, 0.0, None, None, None, None, None)
                };

                let end_time = mm.kalshi_market.start_time.map(|t| t.to_rfc3339());

                Some(MatchedMarketFrontend {
                    event_name: mm.event_name.clone(),
                    sport: self.settings.sport_label(&mm.kalshi_market.market_id).to_string(),
                    team_name: mm.team_name.clone(),
                    game_date: mm.game_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    kalshi_market_id: mm.kalshi_market.market_id.clone(),
                    polymarket_market_id: mm.polymarket_market.market_id.clone(),
                    polymarket_market_slug: own_execution.market_slug,
                    polymarket_team_position_side: own_execution.position_side,
                    polymarket_opponent_name: opponent.to_string(),
                    polymarket_opponent_position_side: opponent_execution.position_side,
                    polymarket_live: mm.polymarket_market.live,
                    polymarket_period: mm.polymarket_market.period.clone(),
                    kalshi_yes_price: k_yes,
                    kalshi_no_price: k_no,
                    poly_yes_price: p_yes,
                    poly_no_price: p_no,
                    kalshi_ready,
                    poly_ready,
                    both_ready: kalshi_ready && poly_ready,
                    confidence: mm.confidence,
                    end_time,
                    has_opportunity,
                    profit_margin,
                    expected_profit,
                    gross_profit,
                    kalshi_contracts,
                    kalshi_fee,
                    polymarket_fee,
                    arbitrage_type,
                })
            })
            .collect()
    }
}
