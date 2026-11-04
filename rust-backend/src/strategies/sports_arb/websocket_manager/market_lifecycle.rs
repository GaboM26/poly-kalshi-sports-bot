//! Market lifecycle management
//!
//! Handles ended market detection and cleanup.

use chrono::Utc;
use tracing::{debug, info};

use super::{
    WebSocketManager, ENDED_DETECTION_DURATION_MINS, EXTREME_PRICE_THRESHOLD_KALSHI_HIGH,
    EXTREME_PRICE_THRESHOLD_KALSHI_LOW, EXTREME_PRICE_THRESHOLD_POLY_HIGH,
    EXTREME_PRICE_THRESHOLD_POLY_LOW,
};
use crate::core::EventMatcher;

impl WebSocketManager {
    /// Check if a market shows extreme prices indicating the game has ended
    pub fn check_market_ended(&self, idx: usize) -> bool {
        let markets = self.matched_markets.read();
        if idx >= markets.len() {
            return false;
        }
        let mm = &markets[idx];
        let market_key = mm.market_key();

        if self.confirmed_ended_markets.read().contains(&market_key) {
            return true;
        }

        let (k_yes_ask, k_no_ask) =
            match self.fresh_kalshi_rest_prices(mm, std::time::Instant::now()) {
                Some(prices) => prices,
                None => return false,
            };

        let (p_yes, p_no) = match self.fresh_poly_rest_prices(mm, std::time::Instant::now()) {
            Some(prices) => prices,
            None => return false,
        };

        drop(markets);

        let kalshi_extreme = self.is_kalshi_price_extreme(k_yes_ask, k_no_ask);
        let poly_extreme = self.is_poly_price_extreme(p_yes, p_no);

        if kalshi_extreme && poly_extreme {
            let now = Utc::now();
            let mut detection = self.ended_market_detection.write();

            if let Some(first_detected) = detection.get(&market_key) {
                let duration = now.signed_duration_since(*first_detected);
                if duration.num_minutes() >= ENDED_DETECTION_DURATION_MINS {
                    drop(detection);
                    self.confirmed_ended_markets
                        .write()
                        .insert(market_key.clone());
                    info!(
                        "🏁 比赛已结束: {} (极端价格持续 {} 分钟)",
                        market_key,
                        duration.num_minutes()
                    );
                    return true;
                }
            } else {
                detection.insert(market_key.clone(), now);
                debug!(
                    "⏱️ 检测到极端价格: {} (Kalshi: {:.0}¢/{:.0}¢, Poly: {:.0}¢/{:.0}¢)",
                    market_key,
                    k_yes_ask * 100.0,
                    k_no_ask * 100.0,
                    p_yes * 100.0,
                    p_no * 100.0
                );
            }
        } else {
            let mut detection = self.ended_market_detection.write();
            if detection.remove(&market_key).is_some() {
                debug!("🔄 极端价格恢复正常: {}", market_key);
            }
        }

        false
    }

    /// Check if Kalshi prices are extreme (99/2 or 2/99)
    pub(crate) fn is_kalshi_price_extreme(&self, yes_ask: f64, no_ask: f64) -> bool {
        let pattern1 = yes_ask >= EXTREME_PRICE_THRESHOLD_KALSHI_HIGH
            && no_ask <= EXTREME_PRICE_THRESHOLD_KALSHI_LOW;
        let pattern2 = yes_ask <= EXTREME_PRICE_THRESHOLD_KALSHI_LOW
            && no_ask >= EXTREME_PRICE_THRESHOLD_KALSHI_HIGH;

        pattern1 || pattern2
    }

    /// Check if Polymarket prices are extreme (100/0 or 0/100)
    pub(crate) fn is_poly_price_extreme(&self, yes_price: f64, no_price: f64) -> bool {
        let pattern1 = yes_price >= EXTREME_PRICE_THRESHOLD_POLY_HIGH
            && no_price <= EXTREME_PRICE_THRESHOLD_POLY_LOW;
        let pattern2 = yes_price <= EXTREME_PRICE_THRESHOLD_POLY_LOW
            && no_price >= EXTREME_PRICE_THRESHOLD_POLY_HIGH;

        pattern1 || pattern2
    }

    /// Remove ended markets and return Kalshi subscription IDs to unsubscribe.
    pub fn remove_ended_markets(&self) -> Vec<String> {
        let mut kalshi_to_unsub = Vec::new();
        let mut poly_market_ids_to_remove = Vec::new();
        let mut market_keys_to_remove = Vec::new();

        // Snapshot identifiers under a short read lock. `check_market_ended`
        // takes its own read lock, and a nested read on a parking_lot RwLock
        // deadlocks as soon as a writer is queued between the two reads.
        let snapshot: Vec<(String, String, String)> = self
            .matched_markets
            .read()
            .iter()
            .map(|mm| {
                (
                    mm.market_key(),
                    mm.kalshi_market.market_id.clone(),
                    mm.polymarket_market.market_id.clone(),
                )
            })
            .collect();

        for (idx, (market_key, kalshi_id, poly_id)) in snapshot.into_iter().enumerate() {
            if self.check_market_ended(idx) {
                market_keys_to_remove.push(market_key);
                kalshi_to_unsub.push(kalshi_id);
                poly_market_ids_to_remove.push(poly_id);
            }
        }

        if market_keys_to_remove.is_empty() {
            return kalshi_to_unsub;
        }

        info!(
            "🧹 清理已结束的市场: {} 个 (Kalshi: {} 个)",
            market_keys_to_remove.len(),
            kalshi_to_unsub.len(),
        );

        {
            let mut markets = self.matched_markets.write();
            markets.retain(|mm| {
                let ended = market_keys_to_remove.contains(&mm.market_key());
                if ended {
                    info!("   移除: {} - {}", mm.event_name, mm.team_name);
                }
                !ended
            });
        }

        {
            let mut lookup = self.market_lookup.write();
            for ticker in &kalshi_to_unsub {
                lookup.remove(ticker);
            }
            let markets = self.matched_markets.read();
            let matcher = EventMatcher::new(24);
            let new_sub_info = matcher.get_subscription_info(&markets);
            drop(markets);
            *lookup = new_sub_info.market_lookup;
        }

        {
            let mut kalshi_ws_prices = self.kalshi_ws_prices.write();
            for ticker in &kalshi_to_unsub {
                kalshi_ws_prices.remove(ticker);
            }
        }
        {
            let mut kalshi_quotes = self.kalshi_rest_quotes.write();
            for ticker in &kalshi_to_unsub {
                kalshi_quotes.remove(ticker);
            }
        }
        {
            let mut poly_quotes = self.poly_rest_quotes.write();
            for market_id in &poly_market_ids_to_remove {
                poly_quotes.remove(market_id);
            }
        }

        {
            let mut opps = self.opportunities.write();
            opps.retain(|o| {
                let key = o.market_key();
                !market_keys_to_remove.contains(&key)
            });
        }

        {
            let mut tracking = self.active_tracking.write();
            for key in &market_keys_to_remove {
                if tracking.remove(key).is_some() {
                    self.storage.track_end(key);
                }
            }
        }

        {
            let mut detection = self.ended_market_detection.write();
            for key in &market_keys_to_remove {
                detection.remove(key);
            }
        }

        {
            let mut streaks = self.poly_missing_streak.write();
            for key in &market_keys_to_remove {
                streaks.remove(key);
            }
        }

        kalshi_to_unsub.sort();
        kalshi_to_unsub.dedup();

        info!(
            "✅ 清理完成，剩余 {} 个活跃市场",
            self.matched_markets.read().len()
        );

        kalshi_to_unsub
    }

    /// Get count of markets currently being monitored for ending
    pub fn get_ending_detection_count(&self) -> usize {
        self.ended_market_detection.read().len()
    }

    /// Get count of confirmed ended markets
    pub fn get_confirmed_ended_count(&self) -> usize {
        self.confirmed_ended_markets.read().len()
    }
}
