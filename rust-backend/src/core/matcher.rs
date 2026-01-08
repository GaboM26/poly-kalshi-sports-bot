//! Event and Market Matcher
//!
//! Matching logic optimization:
//! - Event matching: Use team abbreviation + game date to match events on both platforms
//! - Market matching: 2:1 matching (two Kalshi markets correspond to one Poly market)
//!
//! Key points:
//! - Kalshi: One event has 2 markets (one for each team)
//! - Polymarket: One event has 1 market (contains prices for both teams)
//! - When matching, determine Poly price perspective based on team name

use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::models::{
    KalshiEvent, KalshiMarket, MatchedEvent, MatchedMarket, PolymarketEvent, PolymarketMarket,
};

/// Event matcher - two-stage matching
pub struct EventMatcher {
    /// Time tolerance in hours for matching
    #[allow(dead_code)]
    time_tolerance_hours: i64,
}

/// Subscription info returned by get_subscription_info
pub struct SubscriptionInfo {
    /// Kalshi tickers to subscribe
    pub kalshi_tickers: Vec<String>,
    /// Polymarket token IDs to subscribe
    pub polymarket_token_ids: Vec<String>,
    /// Lookup map: subscription_id -> Vec<MatchedMarket index>
    pub market_lookup: HashMap<String, Vec<usize>>,
}

impl EventMatcher {
    /// Create a new event matcher
    pub fn new(time_tolerance_hours: i64) -> Self {
        Self {
            time_tolerance_hours,
        }
    }

    /// Execute two-stage matching
    pub fn match_events_and_markets(
        &self,
        kalshi_events: &[KalshiEvent],
        _kalshi_markets: &[KalshiMarket],
        polymarket_events: &[PolymarketEvent],
        _polymarket_markets: &[PolymarketMarket],
    ) -> (Vec<MatchedEvent>, Vec<MatchedMarket>) {
        info!("============================================================");
        info!("🔍 开始两阶段匹配 (非拆分版本)");
        info!(
            "   Kalshi: {} 个事件",
            kalshi_events.len()
        );
        info!(
            "   Polymarket: {} 个事件",
            polymarket_events.len()
        );
        info!("============================================================");

        // Stage 1: Event matching
        let matched_events = self.match_events(kalshi_events, polymarket_events);
        info!(
            "📊 阶段 1 完成: 匹配了 {} 个事件",
            matched_events.len()
        );

        // Stage 2: Market matching (2:1)
        let matched_markets = self.match_markets(&matched_events);
        info!(
            "📊 阶段 2 完成: 匹配了 {} 个市场对",
            matched_markets.len()
        );

        (matched_events, matched_markets)
    }

    /// Stage 1: Event matching
    fn match_events(
        &self,
        kalshi_events: &[KalshiEvent],
        polymarket_events: &[PolymarketEvent],
    ) -> Vec<MatchedEvent> {
        info!("----------------------------------------");
        info!("🎯 阶段 1: 事件匹配");
        info!("----------------------------------------");

        let mut matched_events = Vec::new();
        let mut used_poly_ids = std::collections::HashSet::new();

        // Build Polymarket event index: event_name -> [events]
        let mut poly_index: HashMap<String, Vec<&PolymarketEvent>> = HashMap::new();
        for event in polymarket_events {
            let key = event.name.to_uppercase();
            poly_index.entry(key).or_default().push(event);
        }

        // Find match for each Kalshi event
        for k_event in kalshi_events {
            let k_name = k_event.name.to_uppercase();
            let k_date = k_event.start_time.map(|t| t.date_naive());

            // Also check reversed name (e.g., MEM-LAL vs LAL-MEM)
            let k_name_reversed = {
                let parts: Vec<&str> = k_name.split('-').collect();
                if parts.len() == 2 {
                    Some(format!("{}-{}", parts[1], parts[0]))
                } else {
                    None
                }
            };

            let mut best_match: Option<&PolymarketEvent> = None;
            let mut best_confidence = 0.0_f64;

            // Look for exact match or reversed match
            for (name_to_check, is_reversed) in
                [(Some(&k_name), false), (k_name_reversed.as_ref(), true)]
            {
                let Some(name) = name_to_check else {
                    continue;
                };

                let candidates = poly_index.get(name).map(|v| v.as_slice()).unwrap_or(&[]);

                for p_event in candidates {
                    if used_poly_ids.contains(&p_event.event_id) {
                        continue;
                    }

                    let p_date = p_event.start_time.map(|t| t.date_naive());

                    // Validate date
                    let confidence = match (k_date, p_date) {
                        (Some(kd), Some(pd)) => {
                            if kd != pd {
                                debug!(
                                    "   ❌ 日期不匹配: {} ({}) vs {} ({})",
                                    k_event.name, kd, p_event.name, pd
                                );
                                continue;
                            }
                            if is_reversed {
                                0.95
                            } else {
                                1.0
                            }
                        }
                        _ => {
                            warn!(
                                "   ⚠️ 缺少日期: {} ({:?}) vs {} ({:?})",
                                k_event.name, k_date, p_event.name, p_date
                            );
                            if is_reversed {
                                0.65
                            } else {
                                0.7
                            }
                        }
                    };

                    if confidence > best_confidence {
                        best_confidence = confidence;
                        best_match = Some(p_event);
                    }
                }
            }

            if let Some(p_event) = best_match {
                if best_confidence >= 0.7 {
                    let matched = MatchedEvent {
                        event_name: k_event.name.clone(),
                        kalshi_event: Some(k_event.clone()),
                        polymarket_event: Some(p_event.clone()),
                        confidence: best_confidence,
                    };
                    matched_events.push(matched);
                    used_poly_ids.insert(p_event.event_id.clone());

                    info!(
                        "   ✅ 匹配: {} <-> {} (置信度: {:.2})",
                        k_event.name, p_event.name, best_confidence
                    );
                }
            } else {
                warn!("   ❌ 未找到匹配: {}", k_event.name);
            }
        }

        matched_events
    }

    /// Stage 2: Market matching (2:1)
    ///
    /// For each matched event:
    /// - Kalshi has 2 markets (one for each team)
    /// - Polymarket has 1 market (contains both teams)
    /// - Create 2 MatchedMarkets, each corresponding to one Kalshi market
    fn match_markets(&self, matched_events: &[MatchedEvent]) -> Vec<MatchedMarket> {
        info!("----------------------------------------");
        info!("🎯 阶段 2: 市场匹配 (2:1)");
        info!("----------------------------------------");

        let mut matched_markets = Vec::new();

        for matched_event in matched_events {
            let Some(k_event) = &matched_event.kalshi_event else {
                continue;
            };
            let Some(p_event) = &matched_event.polymarket_event else {
                continue;
            };
            let Some(poly_market) = &p_event.market else {
                continue;
            };

            // For each Kalshi market, create a MatchedMarket
            for k_market in &k_event.markets {
                let team_name = k_market.team_name.to_uppercase();

                // Get Poly market prices for this team
                let (poly_yes, poly_no) = match poly_market.get_price_for_team(&team_name) {
                    Ok(prices) => prices,
                    Err(e) => {
                        warn!("   ⚠️ {}", e);
                        continue;
                    }
                };

                let matched = MatchedMarket {
                    event_name: matched_event.event_name.clone(),
                    team_name: team_name.clone(),
                    kalshi_market: k_market.clone(),
                    polymarket_market: poly_market.clone(),
                    poly_yes_price: poly_yes,
                    poly_no_price: poly_no,
                    confidence: matched_event.confidence,
                };
                matched_markets.push(matched);

                info!(
                    "   ✅ 市场匹配: {} - {}",
                    matched_event.event_name, team_name
                );
                debug!(
                    "      Kalshi: Yes={:.2}, No={:.2}",
                    k_market.yes_price, k_market.no_price
                );
                debug!("      Poly:   Yes={:.2}, No={:.2}", poly_yes, poly_no);
            }
        }

        matched_markets
    }

    /// Get WebSocket subscription info
    ///
    /// Important: Each MatchedMarket needs to subscribe to two Poly tokens:
    /// - Own token (for poly_yes_price)
    /// - Opponent token (for poly_no_price)
    ///
    /// Because poly_no_price != 1 - poly_yes_ask,
    /// it equals opponent token's ask price
    ///
    /// Returns:
    /// - kalshi_tickers: Kalshi tickers to subscribe
    /// - polymarket_token_ids: Polymarket token IDs to subscribe
    /// - market_lookup: subscription_id -> Vec<MatchedMarket index> mapping
    pub fn get_subscription_info(&self, matched_markets: &[MatchedMarket]) -> SubscriptionInfo {
        let mut kalshi_tickers = Vec::new();
        let mut polymarket_token_ids = Vec::new();
        let mut market_lookup: HashMap<String, Vec<usize>> = HashMap::new();

        let mut seen_kalshi = std::collections::HashSet::new();
        let mut seen_poly = std::collections::HashSet::new();

        for (idx, mm) in matched_markets.iter().enumerate() {
            // Kalshi ticker
            let k_id = &mm.kalshi_market.market_id;
            if !k_id.is_empty() && !seen_kalshi.contains(k_id) {
                kalshi_tickers.push(k_id.clone());
                seen_kalshi.insert(k_id.clone());
            }

            // Add to lookup
            market_lookup.entry(k_id.clone()).or_default().push(idx);

            // Polymarket: Subscribe to both tokens (own and opponent)
            let poly_market = &mm.polymarket_market;

            // Own token (for yes_price)
            let own_token = poly_market.get_token_for_team(&mm.team_name);

            // Opponent token (for no_price)
            let opponent = poly_market.get_opponent(&mm.team_name);
            let opponent_token = opponent.and_then(|o| poly_market.get_token_for_team(o));

            // Subscribe to both tokens
            for p_token in [own_token, opponent_token].into_iter().flatten() {
                if !seen_poly.contains(p_token) {
                    polymarket_token_ids.push(p_token.to_string());
                    seen_poly.insert(p_token.to_string());
                }

                // Both tokens point to the same MatchedMarket
                market_lookup
                    .entry(p_token.to_string())
                    .or_default()
                    .push(idx);
            }
        }

        info!(
            "📡 订阅信息: Kalshi {} 个代码, Polymarket {} 个代币",
            kalshi_tickers.len(),
            polymarket_token_ids.len()
        );

        SubscriptionInfo {
            kalshi_tickers,
            polymarket_token_ids,
            market_lookup,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_poly_market() -> PolymarketMarket {
        PolymarketMarket {
            market_id: "poly-123".to_string(),
            event_name: "LAL-MEM".to_string(),
            team_a: "LAL".to_string(),
            team_b: "MEM".to_string(),
            price_a: 0.45,
            price_b: 0.55,
            start_time: None,
            volume: None,
            token_id_a: Some("token-lal".to_string()),
            token_id_b: Some("token-mem".to_string()),
        }
    }

    #[test]
    fn test_get_price_for_team() {
        let market = create_test_poly_market();

        // LAL is team_a
        let (yes, no) = market.get_price_for_team("LAL").unwrap();
        assert!((yes - 0.45).abs() < 0.001);
        assert!((no - 0.55).abs() < 0.001);

        // MEM is team_b
        let (yes, no) = market.get_price_for_team("MEM").unwrap();
        assert!((yes - 0.55).abs() < 0.001);
        assert!((no - 0.45).abs() < 0.001);

        // Case insensitive
        let (yes, _) = market.get_price_for_team("lal").unwrap();
        assert!((yes - 0.45).abs() < 0.001);

        // Invalid team
        assert!(market.get_price_for_team("BOS").is_err());
    }

    #[test]
    fn test_get_token_for_team() {
        let market = create_test_poly_market();

        assert_eq!(market.get_token_for_team("LAL"), Some("token-lal"));
        assert_eq!(market.get_token_for_team("MEM"), Some("token-mem"));
        assert_eq!(market.get_token_for_team("BOS"), None);
    }
}
