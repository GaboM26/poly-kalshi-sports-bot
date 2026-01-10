//! WebSocket Manager
//!
//! Manages real-time WebSocket connections to both Kalshi and Polymarket,
//! handles price updates, and triggers arbitrage calculations.
//!
//! Key logic:
//! - Each MatchedMarket needs data from 3 sources:
//!   1. Kalshi ticker (for yes_ask, no_ask)
//!   2. Own Poly token (for poly_yes_price)
//!   3. Opponent Poly token (for poly_no_price)
//! - Price update triggers recalculation for affected MatchedMarkets

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
    ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket, MatchedMarketFrontend,
    Platform, PriceUpdate, ScanStats, SystemStats,
};
use crate::services::storage::{ArbitrageStorage, AutoTradeState};
use crate::services::metrics::{PerformanceMetrics, Operation};

/// Extreme price threshold for Kalshi (99¢ = 0.99)
const EXTREME_PRICE_THRESHOLD_KALSHI_HIGH: f64 = 0.99;
/// Extreme price threshold for Kalshi low side (2¢ = 0.02)
const EXTREME_PRICE_THRESHOLD_KALSHI_LOW: f64 = 0.02;
/// Extreme price threshold for Polymarket high (100¢ = 1.00)
const EXTREME_PRICE_THRESHOLD_POLY_HIGH: f64 = 1.00;
/// Extreme price threshold for Polymarket low (0¢ = 0.00)
const EXTREME_PRICE_THRESHOLD_POLY_LOW: f64 = 0.00;
/// Duration in minutes for extreme price to be considered ended
const ENDED_DETECTION_DURATION_MINS: i64 = 20;

/// WebSocket manager for real-time price updates
pub struct WebSocketManager {
    /// Matched markets to monitor
    matched_markets: Arc<RwLock<Vec<MatchedMarket>>>,
    /// Market lookup: subscription_id -> indices into matched_markets
    market_lookup: Arc<RwLock<HashMap<String, Vec<usize>>>>,
    /// Kalshi prices cache: market_id -> (yes_bid, yes_ask, no_bid, no_ask)
    kalshi_prices: Arc<RwLock<HashMap<String, (f64, f64, f64, f64)>>>,
    /// Polymarket token prices cache: token_id -> ask_price
    poly_token_prices: Arc<RwLock<HashMap<String, f64>>>,
    /// Arbitrage calculator
    calculator: ArbitrageCalculator,
    /// Storage for tracking
    storage: Arc<ArbitrageStorage>,
    /// Active opportunity tracking
    active_tracking: Arc<RwLock<HashMap<String, ArbitrageTrackingRecord>>>,
    /// Current opportunities
    opportunities: Arc<RwLock<Vec<ArbitrageOpportunity>>>,
    /// Opportunity broadcast channel
    opportunity_tx: broadcast::Sender<ArbitrageOpportunity>,
    /// Scan stats broadcast channel
    scan_stats_tx: broadcast::Sender<ScanStats>,
    /// Connection status
    kalshi_connected: Arc<RwLock<bool>>,
    polymarket_connected: Arc<RwLock<bool>>,
    /// Update counters
    kalshi_update_count: Arc<RwLock<u64>>,
    polymarket_update_count: Arc<RwLock<u64>>,
    calculation_count: Arc<RwLock<u64>>,
    /// Last update timestamps (for latency calculation)
    kalshi_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    polymarket_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    /// Performance metrics
    metrics: Arc<PerformanceMetrics>,
    /// Kalshi client for orderbook depth queries
    kalshi_client: Option<KalshiClient>,
    /// Polymarket client for orderbook depth queries
    polymarket_client: Option<PolymarketClient>,
    /// Tracking threshold for high-profit opportunities (percentage)
    tracking_threshold: f64,
    /// Set of opportunity IDs that have been auto-traded (to prevent duplicates)
    auto_traded_opportunities: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Extreme price detection: market_key -> first_detected_time
    /// Used to track when a market first showed extreme prices (game ended)
    ended_market_detection: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// Set of market keys that have been confirmed as ended
    confirmed_ended_markets: Arc<RwLock<std::collections::HashSet<String>>>,
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
        }
    }

    /// Set clients for orderbook depth queries
    pub fn set_clients(&mut self, kalshi: KalshiClient, polymarket: PolymarketClient) {
        self.kalshi_client = Some(kalshi);
        self.polymarket_client = Some(polymarket);
    }

    /// Get Polymarket best ask depth and size for a token
    /// Returns (depth_usd, size) where depth_usd = price * size
    fn get_poly_ask_depth_and_size(&self, token_id: &str) -> (f64, f64) {
        if let Some(client) = &self.polymarket_client {
            if let Some(book) = client.get_orderbook(token_id) {
                // 获取 best ask 的 (price, size)
                if let Some((price, size)) = book.best_ask() {
                    return (price * size, size);
                }
            }
        }
        (0.0, 0.0)
    }

    /// Get Kalshi best ask depth for a market and side (real depth in contracts)
    fn get_kalshi_ask_depth(&self, ticker: &str, side: &str) -> i32 {
        if let Some(client) = &self.kalshi_client {
            if let Some(book) = client.get_orderbook(ticker) {
                // yes_ask 对应 no 的 best_bid，no_ask 对应 yes 的 best_bid
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
    ///
    /// This method updates internal state with new matched markets.
    /// The actual WebSocket subscription is handled by the caller through
    /// the subscription channels.
    ///
    /// Returns the number of markets added
    pub fn add_matched_markets(&self, new_markets: Vec<MatchedMarket>, new_lookup: std::collections::HashMap<String, Vec<usize>>) -> usize {
        if new_markets.is_empty() {
            return 0;
        }

        let old_count = self.matched_markets.read().len();
        
        // Add new markets
        {
            let mut markets = self.matched_markets.write();
            let offset = markets.len();
            markets.extend(new_markets.clone());
            
            // Merge lookup with offset adjustment
            let mut lookup = self.market_lookup.write();
            for (key, indices) in new_lookup {
                let adjusted_indices: Vec<usize> = indices.iter().map(|&i| i + offset).collect();
                lookup.entry(key).or_default().extend(adjusted_indices);
            }
        }

        let new_count = self.matched_markets.read().len();
        let added = new_count - old_count;
        
        info!(
            "📊 市场数据已更新: {} → {} 个配对市场 (+{})",
            old_count, new_count, added
        );

        added
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
        
        // Update last update time for latency calculation
        *self.kalshi_last_update_time.write() = Some(Utc::now());

        // Mark connected
        if !*self.kalshi_connected.read() {
            *self.kalshi_connected.write() = true;
            info!("✅ [Kalshi] 开始接收实时价格数据");
        }

        // Update price cache
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

            // Find affected matched markets and recalculate
            let lookup = self.market_lookup.read();
            if let Some(indices) = lookup.get(&update.market_id) {
                debug!("[Kalshi] 影响 {} 个匹配市场", indices.len());
                for &idx in indices {
                    self.calculate_and_notify(idx);
                }
            }
        }
        
        // Record timing
        self.metrics.record(Operation::KalshiWsProcess, start.elapsed());
    }

    /// Handle Polymarket price update
    ///
    /// Important: Each MatchedMarket has two related tokens:
    /// - Own token: Ask price = poly_yes_price (buy this team wins)
    /// - Opponent token: Ask price = poly_no_price (buy opponent wins = this team loses)
    fn on_polymarket_price_update(&self, update: PriceUpdate) {
        let start = Instant::now();
        
        *self.polymarket_update_count.write() += 1;
        
        // Update last update time for latency calculation
        *self.polymarket_last_update_time.write() = Some(Utc::now());

        // Mark connected
        if !*self.polymarket_connected.read() {
            *self.polymarket_connected.write() = true;
            info!("✅ [Polymarket] 开始接收实时价格数据");
        }

        // Update token price cache (using Ask price for buying)
        // 只使用 Ask 价格，不 fallback 到 Bid - 套利买入必须用 Ask
        // 如果没有 Ask 价格，保持之前的缓存值不变
        if let Some(price) = update.yes_ask {
            debug!(
                "[Polymarket] 价格更新: {} - Price: {:.4}",
                update.market_id, price
            );
            
            self.poly_token_prices
                .write()
                .insert(update.market_id.clone(), price);

            // Find affected matched markets
            let lookup = self.market_lookup.read();
            if let Some(indices) = lookup.get(&update.market_id) {
                let markets = self.matched_markets.read();
                let mut markets_to_update = Vec::new();

                for &idx in indices {
                    if idx < markets.len() {
                        let mm = &markets[idx];
                        let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);

                        // Update the correct price field
                        let is_own = Some(update.market_id.as_str()) == own_token;
                        markets_to_update.push((idx, is_own, price));
                    }
                }

                drop(markets);
                drop(lookup);

                // Apply updates and recalculate
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
        
        // Record timing
        self.metrics.record(Operation::PolyWsProcess, start.elapsed());
    }

    /// Check if a matched market has complete data
    fn is_market_ready(&self, idx: usize) -> bool {
        let markets = self.matched_markets.read();
        if idx >= markets.len() {
            return false;
        }
        let mm = &markets[idx];

        // Check Kalshi prices
        let has_kalshi = self.kalshi_prices.read().contains_key(&mm.kalshi_market.market_id);

        // Check both Poly tokens
        let poly_prices = self.poly_token_prices.read();
        let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
        let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
        let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

        let has_own_poly = own_token.map_or(false, |t| poly_prices.contains_key(t));
        let has_opponent_poly = opponent_token.map_or(false, |t| poly_prices.contains_key(t));

        has_kalshi && has_own_poly && has_opponent_poly
    }

    /// Calculate arbitrage and notify subscribers
    fn calculate_and_notify(&self, idx: usize) {
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

        // Get Kalshi Ask prices
        let k_prices = self.kalshi_prices.read();
        let (_, k_yes_ask, _, k_no_ask) = match k_prices.get(&mm.kalshi_market.market_id) {
            Some(p) => *p,
            None => return,
        };

        // Get Polymarket prices (already updated in MatchedMarket)
        let p_yes = mm.poly_yes_price;
        let p_no = mm.poly_no_price;

        drop(markets);

        // Calculate arbitrage
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

        // Get depth info for this opportunity (save values before dropping)
        // 获取自己和对手的 token，用于根据策略方向查询正确的深度
        let poly_own_token = mm.polymarket_market.get_token_for_team(&mm.team_name).map(|s| s.to_string());
        let poly_opponent_token = mm.polymarket_market.get_opponent(&mm.team_name)
            .and_then(|opp_name| mm.polymarket_market.get_token_for_team(opp_name))
            .map(|s| s.to_string());
        let kalshi_ticker = mm.kalshi_market.market_id.clone();

        drop(markets);

        if let Some(mut opp) = opportunity {
            // Get depth info (10 USD for poly, 10 contracts for kalshi)
            // 根据 polymarket_side 选择正确的 token 查询深度
            // "yes" = 买自己队伍的 token, "no" = 买对手队伍的 token
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

            // Broadcast opportunity
            let _ = self.opportunity_tx.send(opp.clone());

            // Track high-profit opportunities
            if opp.profit_margin >= self.tracking_threshold {
                self.track_opportunity(&opp);
            }

            // Update opportunities list
            self.update_opportunities(opp);
        } else {
            // Check if tracking should end
            let markets = self.matched_markets.read();
            if idx < markets.len() {
                let mm = &markets[idx];
                let key = format!("{}_{}", mm.event_name, mm.team_name);
                drop(markets);
                self.maybe_end_tracking(&key);
            }
        }
        
        // Record timing
        self.metrics.record(Operation::ArbitrageCalc, start.elapsed());
    }

    /// Track a high-profit opportunity
    fn track_opportunity(&self, opp: &ArbitrageOpportunity) {
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
                duration_ms: 0,  // Will be calculated on track_end
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
    fn maybe_end_tracking(&self, key: &str) {
        let mut tracking = self.active_tracking.write();

        if let Some(record) = tracking.remove(key) {
            info!(
                "📉 跟踪结束: {} {} - 最高 {:.2}%",
                record.event_name, record.team_name, record.max_profit_margin
            );
            self.storage.track_end(key);
        }
    }

    /// Update opportunities list
    fn update_opportunities(&self, opp: ArbitrageOpportunity) {
        let mut opps = self.opportunities.write();

        // Find and update or insert
        let key = format!("{}_{}", opp.event_name, opp.team_name);
        if let Some(pos) = opps.iter().position(|o| {
            format!("{}_{}", o.event_name, o.team_name) == key
        }) {
            opps[pos] = opp;
        } else {
            opps.push(opp);
        }

        // Sort by profit margin
        opps.sort_by(|a, b| {
            b.profit_margin
                .partial_cmp(&a.profit_margin)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Keep top opportunities
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

        // Sort by profit margin
        opportunities.sort_by(|a, b| {
            b.profit_margin
                .partial_cmp(&a.profit_margin)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        *self.opportunities.write() = opportunities.clone();
        
        // Record timing
        self.metrics.record(Operation::FullScan, start.elapsed());
        
        opportunities
    }

    /// Get system statistics
    pub fn get_stats(&self) -> SystemStats {
        let storage_stats = self.storage.get_stats();

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
        
        // Calculate latency (milliseconds)
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

        // Get recent completed from storage
        let recent_completed = self.storage.get_history(20).unwrap_or_default();

        serde_json::json!({
            "active_count": active.len(),
            "completed_count": recent_completed.len(),
            "active": active_records,
            "recent_completed": recent_completed,
        })
    }

    /// Get connection status
    pub fn is_kalshi_connected(&self) -> bool {
        *self.kalshi_connected.read()
    }

    /// Get connection status
    pub fn is_polymarket_connected(&self) -> bool {
        *self.polymarket_connected.read()
    }

    // ==================== Auto-Trade Methods ====================

    /// Get current auto-trade state
    pub fn get_auto_trade_state(&self) -> AutoTradeState {
        self.storage.get_auto_trade_state().unwrap_or_default()
    }

    /// Enable auto-trade
    pub fn enable_auto_trade(&self) -> anyhow::Result<()> {
        self.storage.set_auto_trade_enabled(true)?;
        info!("🤖 自动下单已开启");
        Ok(())
    }

    /// Disable auto-trade
    pub fn disable_auto_trade(&self) -> anyhow::Result<()> {
        self.storage.set_auto_trade_enabled(false)?;
        info!("🛑 自动下单已关闭");
        Ok(())
    }

    /// Reset trade count
    pub fn reset_trade_count(&self) -> anyhow::Result<()> {
        self.storage.reset_trade_count()?;
        // Clear auto-traded set
        self.auto_traded_opportunities.write().clear();
        info!("🔄 下单次数已重置");
        Ok(())
    }

    /// Update auto-trade settings
    pub fn update_auto_trade_settings(
        &self,
        max_amount: Option<f64>,
        min_duration_ms: Option<i64>,
        max_trade_count: Option<i32>,
    ) -> anyhow::Result<()> {
        self.storage.update_auto_trade_settings(max_amount, min_duration_ms, max_trade_count)?;
        Ok(())
    }

    /// Check if an opportunity is eligible for auto-trade
    /// Returns (eligible, reason)
    pub fn check_auto_trade_eligibility(&self, key: &str, duration_ms: i64) -> (bool, String) {
        let state = self.get_auto_trade_state();

        // Check if auto-trade is enabled
        if !state.enabled {
            return (false, "自动下单未开启".to_string());
        }

        // Check trade count limit
        if state.trade_count >= state.max_trade_count {
            return (false, format!("已达到最大下单次数 ({}/{})", state.trade_count, state.max_trade_count));
        }

        // Check duration threshold
        if duration_ms < state.min_duration_ms {
            return (false, format!("持续时间不足 ({}ms < {}ms)", duration_ms, state.min_duration_ms));
        }

        // Check if already auto-traded
        if self.auto_traded_opportunities.read().contains(key) {
            return (false, "该机会已自动下单".to_string());
        }

        (true, format!("可以下单 ({}/{})", state.trade_count + 1, state.max_trade_count))
    }

    /// Mark an opportunity as auto-traded
    pub fn mark_as_auto_traded(&self, key: &str) {
        self.auto_traded_opportunities.write().insert(key.to_string());
    }

    /// Increment trade count after successful auto-trade
    pub fn increment_trade_count(&self) -> anyhow::Result<i32> {
        self.storage.increment_trade_count()
    }

    /// Get active tracking records for auto-trade checking
    pub fn get_active_tracking_for_auto_trade(&self) -> Vec<(String, ArbitrageTrackingRecord, i64)> {
        let now = Utc::now();
        let tracking = self.active_tracking.read();
        
        tracking
            .iter()
            .map(|(key, record)| {
                // Calculate current duration
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

    /// Get a reference to storage
    pub fn get_storage(&self) -> Arc<ArbitrageStorage> {
        self.storage.clone()
    }

    /// Get Polymarket token ID for a specific team based on side
    /// 
    /// # Arguments
    /// * `event_name` - Event name (e.g., "MEM-LAL")
    /// * `team_name` - Team name (e.g., "MEM")
    /// * `side` - "yes" for team wins, "no" for team loses (opponent wins)
    /// 
    /// # Returns
    /// Token ID if found, None otherwise
    pub fn get_poly_token_for_side(&self, event_name: &str, team_name: &str, side: &str) -> Option<String> {
        let markets = self.matched_markets.read();
        
        markets.iter()
            .find(|mm| mm.event_name == event_name && mm.team_name == team_name)
            .and_then(|mm| {
                if side == "yes" {
                    // Buy team wins -> use team's own token
                    mm.polymarket_market.get_token_for_team(team_name)
                        .map(|s| s.to_string())
                } else {
                    // Buy team loses (opponent wins) -> use opponent's token
                    mm.polymarket_market.get_opponent(team_name)
                        .and_then(|opponent| mm.polymarket_market.get_token_for_team(opponent))
                        .map(|s| s.to_string())
                }
            })
    }

    /// Validate orderbook depth and price before auto-trade execution
    /// 
    /// Retrieves current orderbook data from local cache and validates:
    /// 1. Kalshi depth is sufficient for required contracts
    /// 2. Polymarket depth is sufficient for required amount
    /// 3. Current prices still support arbitrage (sum < 1)
    /// 
    /// # Arguments
    /// * `kalshi_ticker` - Kalshi market ticker
    /// * `kalshi_side` - "yes" or "no" for Kalshi
    /// * `poly_token` - Polymarket token ID
    /// * `required_contracts` - Number of contracts to trade
    /// 
    /// # Returns
    /// (valid, kalshi_depth, poly_depth, kalshi_price, poly_price, reason)
    pub fn validate_auto_trade_depth(
        &self,
        kalshi_ticker: &str,
        kalshi_side: &str,
        poly_token: &str,
        required_contracts: i32,
    ) -> (bool, i32, f64, f64, f64, String) {
        // Get Kalshi orderbook from local cache
        let kalshi_book = match &self.kalshi_client {
            Some(client) => client.get_orderbook(kalshi_ticker),
            None => {
                return (false, 0, 0.0, 0.0, 0.0, "Kalshi client 未初始化".to_string());
            }
        };

        // Get Polymarket orderbook from local cache
        let poly_book = match &self.polymarket_client {
            Some(client) => client.get_orderbook(poly_token),
            None => {
                return (false, 0, 0.0, 0.0, 0.0, "Polymarket client 未初始化".to_string());
            }
        };

        // Validate Kalshi orderbook exists
        let kalshi_book = match kalshi_book {
            Some(book) => book,
            None => {
                return (false, 0, 0.0, 0.0, 0.0, format!("Kalshi 订单簿不存在: {}", kalshi_ticker));
            }
        };

        // Validate Polymarket orderbook exists
        let poly_book = match poly_book {
            Some(book) => book,
            None => {
                return (false, 0, 0.0, 0.0, 0.0, format!("Polymarket 订单簿不存在: {}", poly_token));
            }
        };

        // Get Kalshi depth for the specified side
        let kalshi_depth = kalshi_book.ask_depth_for_side(kalshi_side, required_contracts);

        // Get Kalshi current ask price from local price cache
        let kalshi_price = {
            let prices = self.kalshi_prices.read();
            match prices.get(kalshi_ticker) {
                Some((_, yes_ask, _, no_ask)) => {
                    if kalshi_side == "yes" { *yes_ask } else { *no_ask }
                }
                None => {
                    return (false, kalshi_depth, 0.0, 0.0, 0.0, 
                        format!("Kalshi 价格缓存不存在: {}", kalshi_ticker));
                }
            }
        };

        // Get Polymarket best ask price and calculate required amount
        let (poly_price, poly_size) = match poly_book.best_ask() {
            Some((price, size)) => (price, size),
            None => {
                return (false, kalshi_depth, 0.0, kalshi_price, 0.0, 
                    "Polymarket 无可用 ask".to_string());
            }
        };

        // Calculate required Polymarket amount (contracts * price)
        let required_poly_amount = required_contracts as f64 * poly_price;
        
        // Calculate available Polymarket depth (price * size at best ask)
        let poly_depth = poly_price * poly_size;

        // Check Kalshi depth
        if kalshi_depth < required_contracts {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("Kalshi 深度不足: 需要 {} 合约, 可用 {} 合约", 
                    required_contracts, kalshi_depth));
        }

        // Check Polymarket depth
        if poly_depth < required_poly_amount {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("Polymarket 深度不足: 需要 ${:.2}, 可用 ${:.2}", 
                    required_poly_amount, poly_depth));
        }

        // Check arbitrage condition still valid
        let price_sum = kalshi_price + poly_price;
        if price_sum >= 1.0 {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("套利条件已消失: K={:.4} + P={:.4} = {:.4} >= 1", 
                    kalshi_price, poly_price, price_sum));
        }

        // All checks passed
        (true, kalshi_depth, poly_depth, kalshi_price, poly_price, 
            format!("验证通过: K深度={}, P深度=${:.2}, 价格和={:.4}", 
                kalshi_depth, poly_depth, price_sum))
    }

    // ==================== Ended Market Detection Methods ====================

    /// Check if a market shows extreme prices indicating the game has ended
    /// 
    /// Extreme prices are:
    /// - Kalshi: Yes >= 99¢ and No <= 2¢ (or reversed)
    /// - Polymarket: Yes >= 100¢ and No <= 0¢ (or reversed)
    /// 
    /// Returns true if market has shown extreme prices for >= 20 minutes
    pub fn check_market_ended(&self, idx: usize) -> bool {
        let markets = self.matched_markets.read();
        if idx >= markets.len() {
            return false;
        }
        let mm = &markets[idx];
        let market_key = format!("{}_{}", mm.event_name, mm.team_name);
        
        // Check if already confirmed as ended
        if self.confirmed_ended_markets.read().contains(&market_key) {
            return true;
        }

        // Get Kalshi prices
        let kalshi_prices = self.kalshi_prices.read();
        let (k_yes_ask, k_no_ask) = match kalshi_prices.get(&mm.kalshi_market.market_id) {
            Some((_, ya, _, na)) => (*ya, *na),
            None => return false, // No price data yet
        };

        // Get Polymarket prices
        let p_yes = mm.poly_yes_price;
        let p_no = mm.poly_no_price;

        drop(kalshi_prices);
        drop(markets);

        // Check if prices are extreme
        let kalshi_extreme = self.is_kalshi_price_extreme(k_yes_ask, k_no_ask);
        let poly_extreme = self.is_poly_price_extreme(p_yes, p_no);

        if kalshi_extreme && poly_extreme {
            // Both platforms show extreme prices - check/update detection time
            let now = Utc::now();
            let mut detection = self.ended_market_detection.write();
            
            if let Some(first_detected) = detection.get(&market_key) {
                // Check if 20 minutes have passed
                let duration = now.signed_duration_since(*first_detected);
                if duration.num_minutes() >= ENDED_DETECTION_DURATION_MINS {
                    // Confirmed ended!
                    drop(detection);
                    self.confirmed_ended_markets.write().insert(market_key.clone());
                    info!(
                        "🏁 比赛已结束: {} (极端价格持续 {} 分钟)",
                        market_key, duration.num_minutes()
                    );
                    return true;
                }
            } else {
                // First time detecting extreme prices
                detection.insert(market_key.clone(), now);
                debug!(
                    "⏱️ 检测到极端价格: {} (Kalshi: {:.0}¢/{:.0}¢, Poly: {:.0}¢/{:.0}¢)",
                    market_key, k_yes_ask * 100.0, k_no_ask * 100.0, p_yes * 100.0, p_no * 100.0
                );
            }
        } else {
            // Prices are not extreme anymore - clear detection
            let mut detection = self.ended_market_detection.write();
            if detection.remove(&market_key).is_some() {
                debug!("🔄 极端价格恢复正常: {}", market_key);
            }
        }

        false
    }

    /// Check if Kalshi prices are extreme (99/2 or 2/99)
    fn is_kalshi_price_extreme(&self, yes_ask: f64, no_ask: f64) -> bool {
        // Pattern 1: Yes is very high (99+), No is very low (2-)
        let pattern1 = yes_ask >= EXTREME_PRICE_THRESHOLD_KALSHI_HIGH 
            && no_ask <= EXTREME_PRICE_THRESHOLD_KALSHI_LOW;
        // Pattern 2: Yes is very low (2-), No is very high (99+)
        let pattern2 = yes_ask <= EXTREME_PRICE_THRESHOLD_KALSHI_LOW 
            && no_ask >= EXTREME_PRICE_THRESHOLD_KALSHI_HIGH;
        
        pattern1 || pattern2
    }

    /// Check if Polymarket prices are extreme (100/0 or 0/100)
    fn is_poly_price_extreme(&self, yes_price: f64, no_price: f64) -> bool {
        // Pattern 1: Yes is 100%, No is 0%
        let pattern1 = yes_price >= EXTREME_PRICE_THRESHOLD_POLY_HIGH 
            && no_price <= EXTREME_PRICE_THRESHOLD_POLY_LOW;
        // Pattern 2: Yes is 0%, No is 100%
        let pattern2 = yes_price <= EXTREME_PRICE_THRESHOLD_POLY_LOW 
            && no_price >= EXTREME_PRICE_THRESHOLD_POLY_HIGH;
        
        pattern1 || pattern2
    }

    /// Check if a market key is confirmed as ended
    pub fn is_market_ended(&self, market_key: &str) -> bool {
        self.confirmed_ended_markets.read().contains(market_key)
    }

    /// Remove ended markets and return subscription IDs to unsubscribe
    /// 
    /// This method:
    /// 1. Checks all markets for extreme prices
    /// 2. Identifies markets that have been ended for 20+ minutes
    /// 3. Removes them from internal caches
    /// 4. Returns the Kalshi tickers and Polymarket token IDs to unsubscribe
    /// 
    /// Returns: (kalshi_tickers_to_unsub, poly_tokens_to_unsub)
    pub fn remove_ended_markets(&self) -> (Vec<String>, Vec<String>) {
        let mut kalshi_to_unsub = Vec::new();
        let mut poly_to_unsub = Vec::new();
        let mut indices_to_remove = Vec::new();
        let mut market_keys_to_remove = Vec::new();

        // First pass: identify ended markets
        {
            let markets = self.matched_markets.read();
            for (idx, mm) in markets.iter().enumerate() {
                let market_key = format!("{}_{}", mm.event_name, mm.team_name);
                
                // Check if this market has ended
                if self.check_market_ended(idx) {
                    indices_to_remove.push(idx);
                    market_keys_to_remove.push(market_key.clone());
                    
                    // Collect subscription IDs to unsubscribe
                    kalshi_to_unsub.push(mm.kalshi_market.market_id.clone());
                    
                    // Get both Polymarket tokens (own and opponent)
                    if let Some(token) = mm.polymarket_market.get_token_for_team(&mm.team_name) {
                        poly_to_unsub.push(token.to_string());
                    }
                    if let Some(opponent) = mm.polymarket_market.get_opponent(&mm.team_name) {
                        if let Some(token) = mm.polymarket_market.get_token_for_team(opponent) {
                            poly_to_unsub.push(token.to_string());
                        }
                    }
                }
            }
        }

        if indices_to_remove.is_empty() {
            return (kalshi_to_unsub, poly_to_unsub);
        }

        info!(
            "🧹 清理已结束的市场: {} 个 (Kalshi: {} 个, Poly: {} 个 token)",
            indices_to_remove.len(),
            kalshi_to_unsub.len(),
            poly_to_unsub.len()
        );

        // Remove from matched_markets (in reverse order to maintain indices)
        {
            let mut markets = self.matched_markets.write();
            for &idx in indices_to_remove.iter().rev() {
                if idx < markets.len() {
                    let removed = markets.remove(idx);
                    info!("   移除: {} - {}", removed.event_name, removed.team_name);
                }
            }
        }

        // Clean up market_lookup
        {
            let mut lookup = self.market_lookup.write();
            // Remove entries for unsubscribed tickers/tokens
            for ticker in &kalshi_to_unsub {
                lookup.remove(ticker);
            }
            for token in &poly_to_unsub {
                lookup.remove(token);
            }
            
            // Rebuild indices for remaining markets (since we removed some)
            // This is necessary because indices shifted after removal
            let markets = self.matched_markets.read();
            let matcher = EventMatcher::new(24);
            let new_sub_info = matcher.get_subscription_info(&markets);
            drop(markets);
            *lookup = new_sub_info.market_lookup;
        }

        // Clean up price caches
        {
            let mut kalshi_prices = self.kalshi_prices.write();
            for ticker in &kalshi_to_unsub {
                kalshi_prices.remove(ticker);
            }
        }
        {
            let mut poly_prices = self.poly_token_prices.write();
            for token in &poly_to_unsub {
                poly_prices.remove(token);
            }
        }

        // Clean up opportunities list
        {
            let mut opps = self.opportunities.write();
            opps.retain(|o| {
                let key = format!("{}_{}", o.event_name, o.team_name);
                !market_keys_to_remove.contains(&key)
            });
        }

        // Clean up active tracking
        {
            let mut tracking = self.active_tracking.write();
            for key in &market_keys_to_remove {
                if tracking.remove(key).is_some() {
                    // End tracking in storage
                    self.storage.track_end(key);
                }
            }
        }

        // Clean up detection cache (already in confirmed_ended_markets)
        {
            let mut detection = self.ended_market_detection.write();
            for key in &market_keys_to_remove {
                detection.remove(key);
            }
        }

        // Deduplicate the unsubscribe lists
        kalshi_to_unsub.sort();
        kalshi_to_unsub.dedup();
        poly_to_unsub.sort();
        poly_to_unsub.dedup();

        info!(
            "✅ 清理完成，剩余 {} 个活跃市场",
            self.matched_markets.read().len()
        );

        (kalshi_to_unsub, poly_to_unsub)
    }

    /// Get count of markets currently being monitored for ending
    pub fn get_ending_detection_count(&self) -> usize {
        self.ended_market_detection.read().len()
    }

    /// Get count of confirmed ended markets
    pub fn get_confirmed_ended_count(&self) -> usize {
        self.confirmed_ended_markets.read().len()
    }

    /// Get matched markets formatted for frontend
    /// This is the key function that converts internal MatchedMarket to frontend format
    /// Filters out markets that have ended (confirmed ended or showing extreme prices)
    pub fn get_matched_markets_for_frontend(&self) -> Vec<MatchedMarketFrontend> {
        let markets = self.matched_markets.read();
        let kalshi_prices = self.kalshi_prices.read();
        let poly_prices = self.poly_token_prices.read();
        let opportunities = self.opportunities.read();
        let confirmed_ended = self.confirmed_ended_markets.read();

        // Build opportunity lookup map
        let opp_map: HashMap<String, &ArbitrageOpportunity> = opportunities
            .iter()
            .map(|o| (format!("{}_{}", o.event_name, o.team_name), o))
            .collect();

        markets
            .iter()
            .filter_map(|mm| {
                let key = format!("{}_{}", mm.event_name, mm.team_name);

                // Skip if confirmed ended
                if confirmed_ended.contains(&key) {
                    return None;
                }

                // Check Kalshi ready status and get prices
                let k_prices = kalshi_prices.get(&mm.kalshi_market.market_id);
                let kalshi_ready = k_prices.is_some();
                let (k_yes, k_no) = if let Some((_, ya, _, na)) = k_prices {
                    (*ya, *na)
                } else {
                    (mm.kalshi_market.yes_price, mm.kalshi_market.no_price)
                };

                // Check Polymarket ready status
                let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
                let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
                let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

                let has_own = own_token.map_or(false, |t| poly_prices.contains_key(t));
                let has_opp = opponent_token.map_or(false, |t| poly_prices.contains_key(t));
                let poly_ready = has_own && has_opp;

                // Get Polymarket prices
                let p_yes = mm.poly_yes_price;
                let p_no = mm.poly_no_price;

                // Filter out markets showing extreme prices (game likely ended/started)
                // Even if not yet confirmed (20 min), don't show to users
                if kalshi_ready && poly_ready {
                    let kalshi_extreme = self.is_kalshi_price_extreme(k_yes, k_no);
                    let poly_extreme = self.is_poly_price_extreme(p_yes, p_no);
                    if kalshi_extreme && poly_extreme {
                        return None;
                    }
                }

                // Get opportunity info if exists
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

                // Format end_time
                let end_time = mm
                    .kalshi_market
                    .start_time
                    .map(|t| t.to_rfc3339());

                Some(MatchedMarketFrontend {
                    event_name: mm.event_name.clone(),
                    team_name: mm.team_name.clone(),
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

/// Data coverage statistics (extended for frontend compatibility)
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
