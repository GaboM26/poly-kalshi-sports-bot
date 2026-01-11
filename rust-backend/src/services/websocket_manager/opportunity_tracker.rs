//! Opportunity tracking logic
//!
//! Handles tracking of high-profit arbitrage opportunities.

use chrono::Utc;
use tracing::info;

use crate::models::{ArbitrageOpportunity, ArbitrageTrackingRecord};
use super::WebSocketManager;

impl WebSocketManager {
    /// Track a high-profit opportunity
    pub(crate) fn track_opportunity(&self, opp: &ArbitrageOpportunity) {
        let key = format!("{}_{}", opp.event_name, opp.team_name);

        let mut tracking = self.active_tracking.write();

        if let Some(record) = tracking.get_mut(&key) {
            // Update existing tracking
            if opp.profit_margin > record.max_profit_margin {
                record.max_profit_margin = opp.profit_margin;
            }
            record.update_count += 1;
            self.storage.track_update(&key, opp.profit_margin);
        } else {
            // Start new tracking with depth info
            let record = ArbitrageTrackingRecord {
                id: key.clone(),
                event_name: opp.event_name.clone(),
                team_name: opp.team_name.clone(),
                kalshi_market_id: opp.kalshi_market_id.clone(),
                polymarket_market_id: opp.polymarket_market_id.clone(),
                start_time: Utc::now(),
                end_time: None,
                initial_profit_margin: opp.profit_margin,
                max_profit_margin: opp.profit_margin,
                kalshi_side: opp.kalshi_side.clone(),
                polymarket_side: opp.polymarket_side.clone(),
                update_count: 1,
                poly_ask_depth: opp.poly_ask_depth,
                poly_ask_size: opp.poly_ask_size,
                kalshi_ask_depth: opp.kalshi_ask_depth,
                duration_ms: 0,
                kalshi_ask_price: opp.kalshi_price,
                polymarket_ask_price: opp.polymarket_price,
            };

            info!(
                "📈 开始跟踪: {} {} - {:.2}% (poly_depth: ${:.2}, poly_size: {:.0}, kalshi_depth: {})",
                opp.event_name, opp.team_name, opp.profit_margin,
                opp.poly_ask_depth, opp.poly_ask_size, opp.kalshi_ask_depth
            );

            self.storage.track_start(record.clone());
            tracking.insert(key, record);
        }
    }

    /// End tracking if opportunity no longer exists
    pub(crate) fn maybe_end_tracking(&self, key: &str) {
        let mut tracking = self.active_tracking.write();

        if let Some(record) = tracking.remove(key) {
            info!(
                "📉 跟踪结束: {} {} - 最高 {:.2}%",
                record.event_name, record.team_name, record.max_profit_margin
            );
            self.storage.track_end(key);
            drop(tracking);
            self.clear_skip_records_for_market(key);
        }
    }

    /// Get tracking statistics
    pub fn get_tracking_stats(&self) -> serde_json::Value {
        let active = self.active_tracking.read();
        let active_records: Vec<serde_json::Value> = active
            .values()
            .map(|r| {
                serde_json::json!({
                    "event_name": r.event_name,
                    "team_name": r.team_name,
                    "kalshi_market_id": r.kalshi_market_id,
                    "polymarket_market_id": r.polymarket_market_id,
                    "start_time": r.start_time,
                    "max_profit_margin": r.max_profit_margin,
                    "duration_seconds": Utc::now().signed_duration_since(r.start_time).num_seconds(),
                })
            })
            .collect();

        let recent_completed = self.storage.get_history(20).unwrap_or_default();

        serde_json::json!({
            "active_count": active.len(),
            "completed_count": recent_completed.len(),
            "active": active_records,
            "recent_completed": recent_completed,
        })
    }

    /// Get active tracking records for auto-trade checking
    pub fn get_active_tracking_for_auto_trade(&self) -> Vec<(String, ArbitrageTrackingRecord, i64)> {
        let now = Utc::now();
        let tracking = self.active_tracking.read();
        
        tracking
            .iter()
            .map(|(key, record)| {
                let duration_ms = now.signed_duration_since(record.start_time).num_milliseconds();
                (key.clone(), record.clone(), duration_ms)
            })
            .collect()
    }

    /// Get current opportunity by key
    pub fn get_opportunity_by_key(&self, key: &str) -> Option<ArbitrageOpportunity> {
        let opps = self.opportunities.read();
        opps.iter()
            .find(|o| format!("{}_{}", o.event_name, o.team_name) == key)
            .cloned()
    }
}
