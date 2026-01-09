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

use crate::core::{ArbitrageCalculator, EventMatcher};
use crate::models::{
    ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket, MatchedMarketFrontend,
    Platform, PriceUpdate, ScanStats, SystemStats,
};
use crate::services::storage::ArbitrageStorage;
use crate::services::metrics::{PerformanceMetrics, Operation};

/// Tracking threshold for high-profit opportunities
const TRACKING_THRESHOLD: f64 = 3.0;

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
}

impl WebSocketManager {
    /// Create a new WebSocket manager
    pub fn new(
        min_profit_margin: f64,
        default_bet_amount: f64,
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
        }
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

        drop(markets);

        if let Some(opp) = opportunity {
            // Broadcast opportunity
            let _ = self.opportunity_tx.send(opp.clone());

            // Track high-profit opportunities
            if opp.profit_margin >= TRACKING_THRESHOLD {
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
            // Start new tracking
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
            };

            info!(
                "📈 开始跟踪: {} {} - {:.2}%",
                opp.event_name, opp.team_name, opp.profit_margin
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

    /// Get matched markets formatted for frontend
    /// This is the key function that converts internal MatchedMarket to frontend format
    pub fn get_matched_markets_for_frontend(&self) -> Vec<MatchedMarketFrontend> {
        let markets = self.matched_markets.read();
        let kalshi_prices = self.kalshi_prices.read();
        let poly_prices = self.poly_token_prices.read();
        let opportunities = self.opportunities.read();

        // Build opportunity lookup map
        let opp_map: HashMap<String, &ArbitrageOpportunity> = opportunities
            .iter()
            .map(|o| (format!("{}_{}", o.event_name, o.team_name), o))
            .collect();

        markets
            .iter()
            .map(|mm| {
                let key = format!("{}_{}", mm.event_name, mm.team_name);

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

                MatchedMarketFrontend {
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
                }
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
