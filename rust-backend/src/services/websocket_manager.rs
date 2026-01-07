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

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::core::{ArbitrageCalculator, EventMatcher};
use crate::models::{
    ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket, Platform, PriceUpdate,
    SystemStats,
};
use crate::services::storage::ArbitrageStorage;

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
    /// Connection status
    kalshi_connected: Arc<RwLock<bool>>,
    polymarket_connected: Arc<RwLock<bool>>,
    /// Update counters
    kalshi_update_count: Arc<RwLock<u64>>,
    polymarket_update_count: Arc<RwLock<u64>>,
    calculation_count: Arc<RwLock<u64>>,
}

impl WebSocketManager {
    /// Create a new WebSocket manager
    pub fn new(
        min_profit_margin: f64,
        default_bet_amount: f64,
        storage: Arc<ArbitrageStorage>,
    ) -> Self {
        let (opportunity_tx, _) = broadcast::channel(100);

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
            kalshi_connected: Arc::new(RwLock::new(false)),
            polymarket_connected: Arc::new(RwLock::new(false)),
            kalshi_update_count: Arc::new(RwLock::new(0)),
            polymarket_update_count: Arc::new(RwLock::new(0)),
            calculation_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Subscribe to opportunity updates
    pub fn subscribe(&self) -> broadcast::Receiver<ArbitrageOpportunity> {
        self.opportunity_tx.subscribe()
    }

    /// Set matched markets and build lookup tables
    pub fn set_matched_markets(&self, markets: Vec<MatchedMarket>) {
        let matcher = EventMatcher::new(24);
        let sub_info = matcher.get_subscription_info(&markets);

        *self.matched_markets.write() = markets;
        *self.market_lookup.write() = sub_info.market_lookup;

        info!(
            "WebSocket manager configured with {} matched markets",
            self.matched_markets.read().len()
        );
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
        *self.kalshi_update_count.write() += 1;

        // Mark connected
        if !*self.kalshi_connected.read() {
            *self.kalshi_connected.write() = true;
            info!("✅ [Kalshi] Started receiving real-time price data");
        }

        // Update price cache
        if let (Some(yb), Some(ya), Some(nb), Some(na)) =
            (update.yes_bid, update.yes_ask, update.no_bid, update.no_ask)
        {
            self.kalshi_prices
                .write()
                .insert(update.market_id.clone(), (yb, ya, nb, na));

            // Find affected matched markets and recalculate
            let lookup = self.market_lookup.read();
            if let Some(indices) = lookup.get(&update.market_id) {
                for &idx in indices {
                    self.calculate_and_notify(idx);
                }
            }
        }
    }

    /// Handle Polymarket price update
    ///
    /// Important: Each MatchedMarket has two related tokens:
    /// - Own token: Ask price = poly_yes_price (buy this team wins)
    /// - Opponent token: Ask price = poly_no_price (buy opponent wins = this team loses)
    fn on_polymarket_price_update(&self, update: PriceUpdate) {
        *self.polymarket_update_count.write() += 1;

        // Mark connected
        if !*self.polymarket_connected.read() {
            *self.polymarket_connected.write() = true;
            info!("✅ [Polymarket] Started receiving real-time price data");
        }

        // Update token price cache (using Ask price for buying)
        let yes_price = update.yes_ask.or(update.yes_bid);
        if let Some(price) = yes_price {
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
                        if Some(update.market_id.as_str()) == own_token {
                            markets_to_update.push((idx, true, price)); // own token
                        } else {
                            markets_to_update.push((idx, false, price)); // opponent token
                        }
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
            return;
        }

        *self.calculation_count.write() += 1;

        let markets = self.matched_markets.read();
        let mm = &markets[idx];

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
                "📈 Tracking started: {} {} - {:.2}%",
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
                "📉 Tracking ended: {} {} - max {:.2}%",
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

        let mut kalshi_coverage = 0;
        let mut poly_coverage = 0;
        let mut full_coverage = 0;

        for mm in markets.iter() {
            let has_kalshi = kalshi_prices.contains_key(&mm.kalshi_market.market_id);

            let own_token = mm.polymarket_market.get_token_for_team(&mm.team_name);
            let opponent = mm.polymarket_market.get_opponent(&mm.team_name);
            let opponent_token = opponent.and_then(|o| mm.polymarket_market.get_token_for_team(o));

            let has_own = own_token.map_or(false, |t| poly_prices.contains_key(t));
            let has_opp = opponent_token.map_or(false, |t| poly_prices.contains_key(t));

            if has_kalshi {
                kalshi_coverage += 1;
            }
            if has_own && has_opp {
                poly_coverage += 1;
            }
            if has_kalshi && has_own && has_opp {
                full_coverage += 1;
            }
        }

        DataCoverage {
            total_markets: markets.len(),
            kalshi_coverage,
            poly_coverage,
            full_coverage,
        }
    }
}

/// Data coverage statistics
#[derive(Debug, Clone)]
pub struct DataCoverage {
    pub total_markets: usize,
    pub kalshi_coverage: usize,
    pub poly_coverage: usize,
    pub full_coverage: usize,
}
