//! WebSocket Manager
//!
//! Manages real-time WebSocket connections to both Kalshi and Polymarket,
//! handles price updates, and triggers arbitrage calculations.
//!
//! This module is split into:
//! - mod.rs: Core WebSocket manager struct and price handling
//! - opportunity_tracker.rs: Opportunity tracking logic
//! - auto_trade.rs: Auto-trade state management
//! - market_lifecycle.rs: Ended market detection and cleanup

mod opportunity_tracker;
mod auto_trade;
mod market_lifecycle;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{debug, info};

use crate::clients::{KalshiClient, PolymarketClient};
use crate::core::{ArbitrageCalculator, EventMatcher};
use crate::models::{
    generate_market_key, ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket,
    MatchedMarketFrontend, Platform, PriceUpdate, ScanStats, SystemStats,
};
use crate::services::storage::{ArbitrageStorage, AutoTradeState};
use crate::services::metrics::{PerformanceMetrics, Operation};

/// Extreme price threshold for Kalshi (99¢ = 0.99)
pub(crate) const EXTREME_PRICE_THRESHOLD_KALSHI_HIGH: f64 = 0.99;
/// Extreme price threshold for Kalshi low side (2¢ = 0.02)
pub(crate) const EXTREME_PRICE_THRESHOLD_KALSHI_LOW: f64 = 0.02;
/// Extreme price threshold for Polymarket high (100¢ = 1.00)
pub(crate) const EXTREME_PRICE_THRESHOLD_POLY_HIGH: f64 = 1.00;
/// Extreme price threshold for Polymarket low (0¢ = 0.00)
pub(crate) const EXTREME_PRICE_THRESHOLD_POLY_LOW: f64 = 0.00;
/// Duration in minutes for extreme price to be considered ended
pub(crate) const ENDED_DETECTION_DURATION_MINS: i64 = 20;

/// WebSocket manager for real-time price updates
pub struct WebSocketManager {
    /// Matched markets to monitor
    pub(crate) matched_markets: Arc<RwLock<Vec<MatchedMarket>>>,
    /// Market lookup: subscription_id -> indices into matched_markets
    pub(crate) market_lookup: Arc<RwLock<HashMap<String, Vec<usize>>>>,
    /// Kalshi prices cache: market_id -> (yes_bid, yes_ask, no_bid, no_ask)
    pub(crate) kalshi_prices: Arc<RwLock<HashMap<String, (f64, f64, f64, f64)>>>,
    /// Polymarket token prices cache: token_id -> ask_price
    pub(crate) poly_token_prices: Arc<RwLock<HashMap<String, f64>>>,
    /// Arbitrage calculator
    pub(crate) calculator: ArbitrageCalculator,
    /// Storage for tracking
    pub(crate) storage: Arc<ArbitrageStorage>,
    /// Active opportunity tracking
    pub(crate) active_tracking: Arc<RwLock<HashMap<String, ArbitrageTrackingRecord>>>,
    /// Current opportunities
    pub(crate) opportunities: Arc<RwLock<Vec<ArbitrageOpportunity>>>,
    /// Opportunity broadcast channel
    pub(crate) opportunity_tx: broadcast::Sender<ArbitrageOpportunity>,
    /// Scan stats broadcast channel
    pub(crate) scan_stats_tx: broadcast::Sender<ScanStats>,
    /// Connection status
    pub(crate) kalshi_connected: Arc<RwLock<bool>>,
    pub(crate) polymarket_connected: Arc<RwLock<bool>>,
    /// Update counters
    pub(crate) kalshi_update_count: Arc<RwLock<u64>>,
    pub(crate) polymarket_update_count: Arc<RwLock<u64>>,
    pub(crate) calculation_count: Arc<RwLock<u64>>,
    /// Last update timestamps (for latency calculation)
    pub(crate) kalshi_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    pub(crate) polymarket_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    /// Performance metrics
    pub(crate) metrics: Arc<PerformanceMetrics>,
    /// Kalshi client for orderbook depth queries
    pub(crate) kalshi_client: Option<KalshiClient>,
    /// Polymarket client for orderbook depth queries
    pub(crate) polymarket_client: Option<PolymarketClient>,
    /// Tracking threshold for high-profit opportunities (percentage)
    pub(crate) tracking_threshold: f64,
    /// Set of opportunity IDs that have been auto-traded (to prevent duplicates)
    pub(crate) auto_traded_opportunities: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Extreme price detection: market_key -> first_detected_time
    pub(crate) ended_market_detection: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// Set of market keys that have been confirmed as ended
    pub(crate) confirmed_ended_markets: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Set of recorded skip reasons: "market_key:simplified_reason" -> prevent duplicate skip records
    pub(crate) recorded_skip_reasons: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Set of market keys excluded from auto-trade (user-defined)
    pub(crate) excluded_markets: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl WebSocketManager {
    /// Create a new WebSocket manager
    pub fn new(
        min_profit_margin: f64,
        default_bet_amount: f64,
        tracking_threshold: f64,
        storage: Arc<ArbitrageStorage>,
        metrics: Arc<PerformanceMetrics>,
    ) -> Self {
        let (opportunity_tx, _) = broadcast::channel(100);
        let (scan_stats_tx, _) = broadcast::channel(100);

        Self {
            matched_markets: Arc::new(RwLock::new(Vec::new())),
            market_lookup: Arc::new(RwLock::new(HashMap::new())),
            kalshi_prices: Arc::new(RwLock::new(HashMap::new())),
            poly_token_prices: Arc::new(RwLock::new(HashMap::new())),
            calculator: ArbitrageCalculator::new(min_profit_margin, default_bet_amount),
            storage,
            active_tracking: Arc::new(RwLock::new(HashMap::new())),
            opportunities: Arc::new(RwLock::new(Vec::new())),
            opportunity_tx,
            scan_stats_tx,
            kalshi_connected: Arc::new(RwLock::new(false)),
            polymarket_connected: Arc::new(RwLock::new(false)),
            kalshi_update_count: Arc::new(RwLock::new(0)),
            polymarket_update_count: Arc::new(RwLock::new(0)),
            calculation_count: Arc::new(RwLock::new(0)),
            kalshi_last_update_time: Arc::new(RwLock::new(None)),
            polymarket_last_update_time: Arc::new(RwLock::new(None)),
            metrics,
            kalshi_client: None,
            polymarket_client: None,
            tracking_threshold,
            auto_traded_opportunities: Arc::new(RwLock::new(std::collections::HashSet::new())),
            ended_market_detection: Arc::new(RwLock::new(HashMap::new())),
            confirmed_ended_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
            recorded_skip_reasons: Arc::new(RwLock::new(std::collections::HashSet::new())),
            excluded_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Set clients for orderbook depth queries
    pub fn set_clients(&mut self, kalshi: KalshiClient, polymarket: PolymarketClient) {
        self.kalshi_client = Some(kalshi);
        self.polymarket_client = Some(polymarket);
    }

    /// Get Polymarket best ask depth and size for a token
    pub(crate) fn get_poly_ask_depth_and_size(&self, token_id: &str) -> (f64, f64) {
        if let Some(client) = &self.polymarket_client {
            if let Some(book) = client.get_orderbook(token_id) {
                if let Some((price, size)) = book.best_ask() {
                    return (price * size, size);
                }
            }
        }
        (0.0, 0.0)
    }

    /// Get Kalshi best ask depth for a market and side
    pub(crate) fn get_kalshi_ask_depth(&self, ticker: &str, side: &str) -> i32 {
        if let Some(client) = &self.kalshi_client {
            if let Some(book) = client.get_orderbook(ticker) {
                let qty = match side.to_lowercase().as_str() {
                    "yes" => book.no.last().map(|(_, qty)| *qty),
                    "no" => book.yes.last().map(|(_, qty)| *qty),
                    _ => None,
                };
                return qty.unwrap_or(0);
            }
        }
        0
    }
    
    /// Get a reference to the performance metrics
    pub fn get_metrics(&self) -> Arc<PerformanceMetrics> {
        self.metrics.clone()
    }

    /// Subscribe to opportunity updates
    pub fn subscribe(&self) -> broadcast::Receiver<ArbitrageOpportunity> {
        self.opportunity_tx.subscribe()
    }

    /// Subscribe to scan stats updates
    pub fn subscribe_scan_stats(&self) -> broadcast::Receiver<ScanStats> {
        self.scan_stats_tx.subscribe()
    }

    /// Broadcast scan statistics to all subscribers
    pub fn broadcast_scan_stats(&self, stats: ScanStats) {
        let _ = self.scan_stats_tx.send(stats);
    }

    /// Set matched markets and build lookup tables
    pub fn set_matched_markets(&self, markets: Vec<MatchedMarket>) {
        let matcher = EventMatcher::new(24);
        let sub_info = matcher.get_subscription_info(&markets);
        
        *self.matched_markets.write() = markets;
        *self.market_lookup.write() = sub_info.market_lookup;

        info!(
            "WebSocket 管理器已配置 {} 个匹配的市场",
            self.matched_markets.read().len()
        );
    }

    /// Add new subscriptions dynamically (hot subscription)
    /// Also updates token_ids for existing markets if they have changed
    /// Returns (newly_added_count, tokens_to_subscribe, tokens_to_unsubscribe)
    pub fn add_matched_markets(&self, new_markets: Vec<MatchedMarket>, new_lookup: std::collections::HashMap<String, Vec<usize>>) -> usize {
        if new_markets.is_empty() {
            return 0;
        }

        let old_count = self.matched_markets.read().len();
        let mut actually_added = 0;
        let mut tokens_updated = 0;
        
        // Track old tokens that need to be removed from lookup
        let mut old_tokens_to_remove: Vec<(String, usize)> = Vec::new();
        // Track new tokens that need to be added to lookup
        let mut new_tokens_to_add: Vec<(String, usize)> = Vec::new();
        
        {
            let mut markets = self.matched_markets.write();
            
            for new_mm in &new_markets {
                let new_key = new_mm.market_key();
                
                // Check if market already exists
                let existing_idx = markets.iter().position(|m| m.market_key() == new_key);
                
                if let Some(idx) = existing_idx {
                    let existing = &mut markets[idx];
                    
                    // Check if token_ids have changed
                    let new_token_a = new_mm.polymarket_market.token_id_a.as_ref();
                    let old_token_a = existing.polymarket_market.token_id_a.as_ref();
                    let new_token_b = new_mm.polymarket_market.token_id_b.as_ref();
                    let old_token_b = existing.polymarket_market.token_id_b.as_ref();
                    
                    if new_token_a != old_token_a || new_token_b != old_token_b {
                        info!("🔄 Token更新: {} token_a: {:?} -> {:?}, token_b: {:?} -> {:?}", 
                            new_key,
                            old_token_a.map(|s| &s[..8.min(s.len())]),
                            new_token_a.map(|s| &s[..8.min(s.len())]),
                            old_token_b.map(|s| &s[..8.min(s.len())]),
                            new_token_b.map(|s| &s[..8.min(s.len())])
                        );
                        
                        // Track old tokens for removal from lookup
                        if let Some(old_a) = old_token_a {
                            old_tokens_to_remove.push((old_a.clone(), idx));
                        }
                        if let Some(old_b) = old_token_b {
                            old_tokens_to_remove.push((old_b.clone(), idx));
                        }
                        
                        // Track new tokens for addition to lookup
                        if let Some(new_a) = new_token_a {
                            new_tokens_to_add.push((new_a.clone(), idx));
                        }
                        if let Some(new_b) = new_token_b {
                            new_tokens_to_add.push((new_b.clone(), idx));
                        }
                        
                        // Update the market's token_ids
                        existing.polymarket_market.token_id_a = new_mm.polymarket_market.token_id_a.clone();
                        existing.polymarket_market.token_id_b = new_mm.polymarket_market.token_id_b.clone();
                        tokens_updated += 1;
                    }
                } else {
                    // New market, add it
                    markets.push(new_mm.clone());
                    actually_added += 1;
                }
            }
        }
        
        // Update market_lookup for token changes and clear stale price cache
        if !old_tokens_to_remove.is_empty() || !new_tokens_to_add.is_empty() {
            // Collect old tokens for price cache cleanup
            let old_token_ids: Vec<String> = old_tokens_to_remove.iter()
                .map(|(token, _)| token.clone())
                .collect();
            
            let mut lookup = self.market_lookup.write();
            
            // Remove old token mappings
            for (old_token, idx) in old_tokens_to_remove {
                if let Some(indices) = lookup.get_mut(&old_token) {
                    indices.retain(|&i| i != idx);
                    if indices.is_empty() {
                        lookup.remove(&old_token);
                    }
                }
            }
            
            // Add new token mappings
            for (new_token, idx) in new_tokens_to_add {
                lookup.entry(new_token).or_default().push(idx);
            }
            
            // Clear stale prices for old tokens (prevent using outdated data)
            if !old_token_ids.is_empty() {
                let mut prices = self.poly_token_prices.write();
                for old_token in &old_token_ids {
                    if prices.remove(old_token).is_some() {
                        debug!("🗑️ 已清除旧token价格缓存: {}...", &old_token[..8.min(old_token.len())]);
                    }
                }
            }
            
            info!("🔗 市场查找表已更新 (token映射同步)");
        }
        
        // Update lookup with correct indices for new markets
        if actually_added > 0 {
            let mut lookup = self.market_lookup.write();
            let offset = old_count;
            for (key, indices) in new_lookup {
                let adjusted_indices: Vec<usize> = indices.iter().map(|&i| i + offset).collect();
                lookup.entry(key).or_default().extend(adjusted_indices);
            }
        }

        let new_count = self.matched_markets.read().len();
        
        if actually_added > 0 || tokens_updated > 0 {
            info!(
                "📊 市场数据已更新: {} → {} 个配对市场 (新增: {}, Token更新: {})",
                old_count, new_count, actually_added, tokens_updated
            );
        }

        actually_added
    }

    /// Get subscription info for WebSocket connections
    pub fn get_subscription_ids(&self) -> (Vec<String>, Vec<String>) {
        let markets = self.matched_markets.read();
        let matcher = EventMatcher::new(24);
        let sub_info = matcher.get_subscription_info(&markets);

        (sub_info.kalshi_tickers, sub_info.polymarket_token_ids)
    }

    /// Handle incoming price update
    pub fn on_price_update(&self, update: PriceUpdate) {
        match update.platform {
            Platform::Kalshi => self.on_kalshi_price_update(update),
            Platform::Polymarket => self.on_polymarket_price_update(update),
        }
    }

    /// Handle Kalshi price update
    fn on_kalshi_price_update(&self, update: PriceUpdate) {
        let start = Instant::now();
        
        *self.kalshi_update_count.write() += 1;
        *self.kalshi_last_update_time.write() = Some(Utc::now());

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
            
            self.kalshi_prices
                .write()
                .insert(update.market_id.clone(), (yb, ya, nb, na));

            let lookup = self.market_lookup.read();
            if let Some(indices) = lookup.get(&update.market_id) {
                debug!("[Kalshi] 影响 {} 个匹配市场", indices.len());
                for &idx in indices {
                    self.calculate_and_notify(idx);
                }
            }
        }
        
        self.metrics.record(Operation::KalshiWsProcess, start.elapsed());
    }

    /// Handle Polymarket price update
    fn on_polymarket_price_update(&self, update: PriceUpdate) {
        let start = Instant::now();
        
        *self.polymarket_update_count.write() += 1;
        *self.polymarket_last_update_time.write() = Some(Utc::now());

        if !*self.polymarket_connected.read() {
            *self.polymarket_connected.write() = true;
            info!("✅ [Polymarket] 开始接收实时价格数据");
        }

        if let Some(price) = update.yes_ask {
            debug!(
                "[Polymarket] 价格更新: {} - Price: {:.4}",
                update.market_id, price
            );
            
            let is_first_price = !self.poly_token_prices.read().contains_key(&update.market_id);
            
            self.poly_token_prices
                .write()
                .insert(update.market_id.clone(), price);

            let lookup = self.market_lookup.read();
            if let Some(indices) = lookup.get(&update.market_id) {
                let markets = self.matched_markets.read();
                let mut markets_to_update = Vec::new();

                for &idx in indices {
                    if idx < markets.len() {
                        let mm = &markets[idx];
                        let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
                        let is_own = Some(update.market_id.as_str()) == own_token;
                        
                        if is_first_price {
                            let expected_price = if is_own {
                                mm.poly_yes_price
                            } else {
                                mm.poly_no_price
                            };
                            
                            let price_diff = (price - expected_price).abs();
                            let price_tolerance = 0.20;
                            let is_valid = price_diff <= price_tolerance;
                            
                            {
                                use std::io::Write;
                                let debug_log = serde_json::json!({
                                    "timestamp": chrono::Utc::now().to_rfc3339(),
                                    "hypothesisId": "TOKEN_MAPPING_VALIDATION",
                                    "location": "websocket_manager.rs:on_polymarket_price_update",
                                    "message": if is_valid { "✅ Token映射验证通过" } else { "⚠️ Token映射验证异常" },
                                    "data": {
                                        "event_name": &mm.event_name,
                                        "team_name": &mm.team_name,
                                        "token_id": &update.market_id,
                                        "is_own_token": is_own,
                                        "expected_price": expected_price,
                                        "actual_price": price,
                                        "price_diff": price_diff,
                                        "is_valid": is_valid,
                                        "poly_team_a": &mm.polymarket_market.team_a,
                                        "poly_team_b": &mm.polymarket_market.team_b,
                                        "poly_token_id_a": &mm.polymarket_market.token_id_a,
                                        "poly_token_id_b": &mm.polymarket_market.token_id_b,
                                    }
                                });
                                if let Ok(mut file) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&crate::utils::get_debug_log_path())
                                {
                                    let _ = writeln!(file, "{}", debug_log);
                                }
                            }
                            
                            if !is_valid {
                                tracing::warn!(
                                    "⚠️ [Token映射验证] {}-{}: {} token 价格异常 (预期={:.4}, 实际={:.4}, 差={:.4})",
                                    mm.event_name, mm.team_name,
                                    if is_own { "own" } else { "opponent" },
                                    expected_price, price, price_diff
                                );
                            }
                        }
                        
                        markets_to_update.push((idx, is_own, price));
                    }
                }

                drop(markets);
                drop(lookup);

                {
                    let mut markets = self.matched_markets.write();
                    for (idx, is_own, price) in &markets_to_update {
                        if *idx < markets.len() {
                            if *is_own {
                                markets[*idx].poly_yes_price = *price;
                            } else {
                                markets[*idx].poly_no_price = *price;
                            }
                        }
                    }
                }

                for (idx, _, _) in markets_to_update {
                    self.calculate_and_notify(idx);
                }
            }
        }
        
        self.metrics.record(Operation::PolyWsProcess, start.elapsed());
    }

    /// Check if a matched market has complete data
    pub(crate) fn is_market_ready(&self, idx: usize) -> bool {
        let markets = self.matched_markets.read();
        if idx >= markets.len() {
            return false;
        }
        let mm = &markets[idx];

        let has_kalshi = self.kalshi_prices.read().contains_key(&mm.kalshi_market.market_id);

        let poly_prices = self.poly_token_prices.read();
        let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
        let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
        let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

        let has_own_poly = own_token.map_or(false, |t| poly_prices.contains_key(t));
        let has_opponent_poly = opponent_token.map_or(false, |t| poly_prices.contains_key(t));

        has_kalshi && has_own_poly && has_opponent_poly
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
        
        debug!(
            "[计算] 开始计算套利: {} - {}",
            mm.event_name, mm.team_name
        );

        let k_prices = self.kalshi_prices.read();
        let (_, k_yes_ask, _, k_no_ask) = match k_prices.get(&mm.kalshi_market.market_id) {
            Some(p) => *p,
            None => return,
        };

        let p_yes = mm.poly_yes_price;
        let p_no = mm.poly_no_price;

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

        let poly_own_token = mm.polymarket_market.get_token_for_team(&mm.team_name).map(|s| s.to_string());
        let poly_opponent_token = mm.polymarket_market.get_opponent(&mm.team_name)
            .and_then(|opp_name| mm.polymarket_market.get_token_for_team(opp_name))
            .map(|s| s.to_string());
        let kalshi_ticker = mm.kalshi_market.market_id.clone();

        drop(markets);

        if let Some(mut opp) = opportunity {
            let poly_token_for_depth = if opp.polymarket_side == "yes" {
                poly_own_token.as_ref()
            } else {
                poly_opponent_token.as_ref()
            };
            
            if let Some(token_id) = poly_token_for_depth {
                let (depth, size) = self.get_poly_ask_depth_and_size(token_id);
                opp.poly_ask_depth = depth;
                opp.poly_ask_size = size;
            }
            opp.kalshi_ask_depth = self.get_kalshi_ask_depth(&kalshi_ticker, &opp.kalshi_side);

            let _ = self.opportunity_tx.send(opp.clone());

            if opp.profit_margin >= self.tracking_threshold {
                self.track_opportunity(&opp);
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
        
        self.metrics.record(Operation::ArbitrageCalc, start.elapsed());
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

            let k_prices = self.kalshi_prices.read();
            let (_, k_yes_ask, _, k_no_ask) = match k_prices.get(&mm.kalshi_market.market_id) {
                Some(p) => *p,
                None => continue,
            };

            let p_yes = mm.poly_yes_price;
            let p_no = mm.poly_no_price;

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

    /// Get system statistics
    pub fn get_stats(&self) -> SystemStats {
        let _storage_stats = self.storage.get_stats();

        SystemStats {
            total_kalshi_events: 0,
            total_kalshi_markets: 0,
            total_polymarket_events: 0,
            total_polymarket_markets: 0,
            matched_events: 0,
            matched_markets: self.matched_markets.read().len(),
            arbitrage_opportunities: self.opportunities.read().len(),
            kalshi_ws_connected: *self.kalshi_connected.read(),
            polymarket_ws_connected: *self.polymarket_connected.read(),
            last_update: Some(Utc::now()),
        }
    }

    /// Get data coverage statistics
    pub fn get_data_coverage(&self) -> DataCoverage {
        let markets = self.matched_markets.read();
        let kalshi_prices = self.kalshi_prices.read();
        let poly_prices = self.poly_token_prices.read();

        let mut kalshi_ready = 0;
        let mut poly_ready = 0;
        let mut both_ready = 0;

        for mm in markets.iter() {
            let has_kalshi = kalshi_prices.contains_key(&mm.kalshi_market.market_id);

            let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
            let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
            let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

            let has_own = own_token.map_or(false, |t| poly_prices.contains_key(t));
            let has_opp = opponent_token.map_or(false, |t| poly_prices.contains_key(t));
            let has_poly = has_own && has_opp;

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
        let kalshi_latency_ms = self.kalshi_last_update_time.read().map(|last_time| {
            (now - last_time).num_milliseconds()
        });
        let polymarket_latency_ms = self.polymarket_last_update_time.read().map(|last_time| {
            (now - last_time).num_milliseconds()
        });
        
        DataCoverage {
            total_markets: total,
            kalshi_ready,
            polymarket_ready: poly_ready,
            both_ready,
            kalshi_coverage: format!("{}/{}", kalshi_ready, total),
            polymarket_coverage: format!("{}/{}", poly_ready, total),
            full_coverage: format!("{}/{}", both_ready, total),
            kalshi_connected: *self.kalshi_connected.read(),
            polymarket_connected: *self.polymarket_connected.read(),
            kalshi_latency_ms,
            polymarket_latency_ms,
        }
    }

    /// Get connection status
    pub fn is_kalshi_connected(&self) -> bool {
        *self.kalshi_connected.read()
    }

    /// Get connection status
    pub fn is_polymarket_connected(&self) -> bool {
        *self.polymarket_connected.read()
    }

    /// Get a reference to storage
    pub fn get_storage(&self) -> Arc<ArbitrageStorage> {
        self.storage.clone()
    }

    /// Get Polymarket token ID for a specific team based on side
    pub fn get_poly_token_for_side(&self, event_name: &str, team_name: &str, side: &str) -> Option<String> {
        let markets = self.matched_markets.read();
        
        let mm = markets.iter()
            .find(|mm| mm.event_name == event_name && mm.team_name == team_name)?;
        
        // === DETAILED TOKEN MAPPING LOGGING ===
        info!("🔍 [Token映射] 查找 Poly Token:");
        info!("   输入: event={}, team={}, side={}", event_name, team_name, side);
        info!("   Poly市场结构:");
        info!("      team_a: {:?}", mm.polymarket_market.team_a);
        info!("      team_b: {:?}", mm.polymarket_market.team_b);
        let token_a_short = mm.polymarket_market.token_id_a.as_ref()
            .map(|t| format!("{}...", &t[..20.min(t.len())]));
        let token_b_short = mm.polymarket_market.token_id_b.as_ref()
            .map(|t| format!("{}...", &t[..20.min(t.len())]));
        info!("      token_id_a: {:?}", token_a_short);
        info!("      token_id_b: {:?}", token_b_short);
        
        // Determine which token to use based on side
        let selected_token = if side == "yes" {
            // YES side = buy the team to win = use that team's token
            let token = mm.polymarket_market.get_token_for_team(team_name)
                .map(|s| s.to_string());
            info!("   YES侧: 获取 {} 的 token", team_name);
            info!("   结果: {:?}", token.as_ref().map(|t| format!("{}...", &t[..20.min(t.len())])));
            token
        } else {
            // NO side = buy opponent to win = use opponent's token  
            let opponent = mm.polymarket_market.get_opponent(team_name);
            info!("   NO侧: 获取对手方的token");
            info!("      对手: {:?}", opponent);
            let token = opponent
                .and_then(|opp| mm.polymarket_market.get_token_for_team(opp))
                .map(|s| s.to_string());
            info!("      结果: {:?}", token.as_ref().map(|t| format!("{}...", &t[..20.min(t.len())])));
            token
        };
        
        // Write to debug log
        {
            use std::io::Write;
            let debug_log = serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "hypothesisId": "TOKEN_MAPPING",
                "location": "websocket_manager.rs:get_poly_token_for_side",
                "message": "下单时Token选择",
                "data": {
                    "event_name": event_name,
                    "team_name": team_name,
                    "requested_side": side,
                    "poly_team_a": &mm.polymarket_market.team_a,
                    "poly_team_b": &mm.polymarket_market.team_b,
                    "poly_token_id_a": &mm.polymarket_market.token_id_a,
                    "poly_token_id_b": &mm.polymarket_market.token_id_b,
                    "selected_token": &selected_token,
                }
            });
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log")
            {
                let _ = writeln!(file, "{}", debug_log);
            }
        }
        
        selected_token
    }

    /// Get matched markets formatted for frontend
    pub fn get_matched_markets_for_frontend(&self) -> Vec<MatchedMarketFrontend> {
        let markets = self.matched_markets.read();
        let kalshi_prices = self.kalshi_prices.read();
        let poly_prices = self.poly_token_prices.read();
        let opportunities = self.opportunities.read();
        let confirmed_ended = self.confirmed_ended_markets.read();

        let opp_map: HashMap<String, &ArbitrageOpportunity> = opportunities
            .iter()
            .map(|o| (o.market_key(), o))
            .collect();

        markets
            .iter()
            .filter_map(|mm| {
                let key = mm.market_key();

                if confirmed_ended.contains(&key) {
                    return None;
                }

                let k_prices = kalshi_prices.get(&mm.kalshi_market.market_id);
                let kalshi_ready = k_prices.is_some();
                let (k_yes, k_no) = if let Some((_, ya, _, na)) = k_prices {
                    (*ya, *na)
                } else {
                    (mm.kalshi_market.yes_price, mm.kalshi_market.no_price)
                };

                let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
                let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
                let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

                let has_own = own_token.map_or(false, |t| poly_prices.contains_key(t));
                let has_opp = opponent_token.map_or(false, |t| poly_prices.contains_key(t));
                let poly_ready = has_own && has_opp;

                let p_yes = mm.poly_yes_price;
                let p_no = mm.poly_no_price;

                if kalshi_ready && poly_ready {
                    let kalshi_extreme = self.is_kalshi_price_extreme(k_yes, k_no);
                    let poly_extreme = self.is_poly_price_extreme(p_yes, p_no);
                    if kalshi_extreme && poly_extreme {
                        return None;
                    }
                }

                let opportunity = opp_map.get(&key);
                let has_opportunity = opportunity.is_some();
                let (profit_margin, expected_profit, gross_profit, kalshi_contracts, kalshi_fee, arbitrage_type) =
                    if let Some(opp) = opportunity {
                        (
                            opp.profit_margin,
                            opp.expected_profit,
                            Some(opp.gross_profit),
                            Some(opp.kalshi_contracts),
                            Some(opp.kalshi_fee),
                            Some(format!(
                                "Kalshi{}Polymarket{}",
                                capitalize(&opp.kalshi_side),
                                capitalize(&opp.polymarket_side)
                            )),
                        )
                    } else {
                        (0.0, 0.0, None, None, None, None)
                    };

                let end_time = mm
                    .kalshi_market
                    .start_time
                    .map(|t| t.to_rfc3339());

                Some(MatchedMarketFrontend {
                    event_name: mm.event_name.clone(),
                    team_name: mm.team_name.clone(),
                    game_date: mm.game_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    kalshi_market_id: mm.kalshi_market.market_id.clone(),
                    polymarket_market_id: mm.polymarket_market.market_id.clone(),
                    poly_token_id: own_token.map(|s| s.to_string()),
                    poly_opponent_token_id: opponent_token.map(|s| s.to_string()),
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
                    arbitrage_type,
                })
            })
            .collect()
    }
}

/// Helper function to capitalize first letter
fn capitalize(s: &str) -> String {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymarket_latency_ms: Option<i64>,
}
