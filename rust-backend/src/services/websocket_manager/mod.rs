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

mod auto_trade;
mod market_lifecycle;
mod opportunity_tracker;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{debug, info};

use crate::clients::{KalshiClient, KalshiMarketQuote};
use crate::core::{ArbitrageCalculator, EventMatcher};
use crate::models::{
    ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket, MatchedMarketFrontend, Platform,
    PolymarketMarket, PriceUpdate, ScanStats, SystemStats,
};
use crate::services::metrics::{Operation, PerformanceMetrics};
use crate::services::storage::ArbitrageStorage;

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

/// WebSocket manager for real-time price updates
pub struct WebSocketManager {
    /// Matched markets to monitor
    pub(crate) matched_markets: Arc<RwLock<Vec<MatchedMarket>>>,
    /// Market lookup: subscription_id -> indices into matched_markets
    pub(crate) market_lookup: Arc<RwLock<HashMap<String, Vec<usize>>>>,
    /// Kalshi WebSocket price cache: market_id -> (yes_bid, yes_ask, no_bid, no_ask).
    /// This cache is retained for executable order-book pricing only.
    pub(crate) kalshi_ws_prices: Arc<RwLock<HashMap<String, (f64, f64, f64, f64)>>>,
    /// Authoritative Kalshi REST quote cache used for calculations and UI.
    pub(crate) kalshi_rest_quotes: Arc<RwLock<HashMap<String, KalshiRestQuote>>>,
    /// Polymarket US REST quotes: market_id -> current two-outcome quote.
    pub(crate) poly_rest_quotes: Arc<RwLock<HashMap<String, PolymarketRestQuote>>>,
    /// How long a REST quote remains usable for calculations.
    pub(crate) poly_quote_freshness: Duration,
    /// How long an authoritative Kalshi REST quote remains usable.
    pub(crate) kalshi_quote_freshness: Duration,
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
    /// The authoritative Kalshi REST quote feed was successfully refreshed.
    pub(crate) kalshi_rest_connected: Arc<RwLock<bool>>,
    /// The Polymarket US REST feed was successfully refreshed.
    pub(crate) polymarket_rest_connected: Arc<RwLock<bool>>,
    /// Update counters
    pub(crate) kalshi_update_count: Arc<RwLock<u64>>,
    pub(crate) polymarket_update_count: Arc<RwLock<u64>>,
    pub(crate) calculation_count: Arc<RwLock<u64>>,
    /// Last update timestamps (for latency calculation)
    pub(crate) kalshi_ws_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    pub(crate) kalshi_rest_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    pub(crate) polymarket_last_update_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    /// Monotonic timestamp for REST source freshness checks.
    pub(crate) kalshi_rest_last_success: Arc<RwLock<Option<Instant>>>,
    pub(crate) polymarket_rest_last_success: Arc<RwLock<Option<Instant>>>,
    /// Performance metrics
    pub(crate) metrics: Arc<PerformanceMetrics>,
    /// Kalshi client for orderbook depth queries
    pub(crate) kalshi_client: Option<KalshiClient>,
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
    /// Auto-trade queue: market keys waiting to be executed
    pub auto_trade_queue: Arc<RwLock<VecDeque<String>>>,
    /// Flag indicating if an auto-trade is currently being executed
    pub is_auto_trading: Arc<AtomicBool>,
}

impl WebSocketManager {
    /// Create a new WebSocket manager
    pub fn new(
        min_profit_margin: f64,
        default_bet_amount: f64,
        tracking_threshold: f64,
        storage: Arc<ArbitrageStorage>,
        metrics: Arc<PerformanceMetrics>,
        rest_quote_freshness: Duration,
    ) -> Self {
        let (opportunity_tx, _) = broadcast::channel(100);
        let (scan_stats_tx, _) = broadcast::channel(100);

        Self {
            matched_markets: Arc::new(RwLock::new(Vec::new())),
            market_lookup: Arc::new(RwLock::new(HashMap::new())),
            kalshi_ws_prices: Arc::new(RwLock::new(HashMap::new())),
            kalshi_rest_quotes: Arc::new(RwLock::new(HashMap::new())),
            poly_rest_quotes: Arc::new(RwLock::new(HashMap::new())),
            poly_quote_freshness: rest_quote_freshness.max(Duration::from_secs(1)),
            kalshi_quote_freshness: rest_quote_freshness.max(Duration::from_secs(1)),
            calculator: ArbitrageCalculator::new(min_profit_margin, default_bet_amount),
            storage,
            active_tracking: Arc::new(RwLock::new(HashMap::new())),
            opportunities: Arc::new(RwLock::new(Vec::new())),
            opportunity_tx,
            scan_stats_tx,
            kalshi_connected: Arc::new(RwLock::new(false)),
            kalshi_rest_connected: Arc::new(RwLock::new(false)),
            polymarket_rest_connected: Arc::new(RwLock::new(false)),
            kalshi_update_count: Arc::new(RwLock::new(0)),
            polymarket_update_count: Arc::new(RwLock::new(0)),
            calculation_count: Arc::new(RwLock::new(0)),
            kalshi_ws_last_update_time: Arc::new(RwLock::new(None)),
            kalshi_rest_last_update_time: Arc::new(RwLock::new(None)),
            polymarket_last_update_time: Arc::new(RwLock::new(None)),
            kalshi_rest_last_success: Arc::new(RwLock::new(None)),
            polymarket_rest_last_success: Arc::new(RwLock::new(None)),
            metrics,
            kalshi_client: None,
            tracking_threshold,
            auto_traded_opportunities: Arc::new(RwLock::new(std::collections::HashSet::new())),
            ended_market_detection: Arc::new(RwLock::new(HashMap::new())),
            confirmed_ended_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
            recorded_skip_reasons: Arc::new(RwLock::new(std::collections::HashSet::new())),
            excluded_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
            auto_trade_queue: Arc::new(RwLock::new(VecDeque::new())),
            is_auto_trading: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the Kalshi client used for executable-depth checks.
    pub fn set_kalshi_client(&mut self, kalshi: KalshiClient) {
        self.kalshi_client = Some(kalshi);
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
        self.kalshi_rest_quotes.write().clear();
        *self.kalshi_rest_connected.write() = false;
        *self.kalshi_rest_last_success.write() = None;
        self.poly_rest_quotes.write().clear();
        *self.polymarket_rest_connected.write() = false;
        *self.polymarket_rest_last_success.write() = None;

        info!(
            "WebSocket 管理器已配置 {} 个匹配的市场",
            self.matched_markets.read().len()
        );
    }

    /// Return the currently matched Kalshi tickers for the REST quote poller.
    pub fn get_kalshi_quote_tickers(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.matched_markets
            .read()
            .iter()
            .filter_map(|market| {
                let ticker = &market.kalshi_market.market_id;
                seen.insert(ticker.clone()).then(|| ticker.clone())
            })
            .collect()
    }

    /// Apply authoritative Kalshi REST asks to the currently matched markets.
    ///
    /// This price-only cache must never populate or relax WebSocket order-book
    /// depth checks used by manual or automatic trading.
    pub fn update_kalshi_rest_quotes(&self, quotes: &[KalshiMarketQuote]) -> usize {
        let quotes_by_ticker: HashMap<&str, &KalshiMarketQuote> = quotes
            .iter()
            .map(|quote| (quote.market_id.as_str(), quote))
            .collect();
        let received_at = Instant::now();
        let mut updated_indices = Vec::new();
        let mut quotes_to_cache = HashMap::new();

        {
            let mut matched_markets = self.matched_markets.write();

            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                let Some(quote) = quotes_by_ticker.get(matched.kalshi_market.market_id.as_str())
                else {
                    continue;
                };

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

        if !quotes_to_cache.is_empty() {
            self.kalshi_rest_quotes.write().extend(quotes_to_cache);
        }

        *self.kalshi_rest_connected.write() = true;
        *self.kalshi_rest_last_success.write() = Some(received_at);
        *self.kalshi_rest_last_update_time.write() = Some(Utc::now());

        for idx in &updated_indices {
            self.calculate_and_notify(*idx);
        }

        updated_indices.len()
    }

    /// Apply current Polymarket US gateway quotes to already matched markets.
    ///
    /// The caller fetches the same supported-sports feed used for discovery.
    /// Only price fields for known matched market IDs are updated; this never
    /// changes subscriptions, token mappings, or order-book state.
    pub fn update_polymarket_rest_quotes(&self, markets: &[PolymarketMarket]) -> usize {
        let quotes_by_market: HashMap<&str, &PolymarketMarket> = markets
            .iter()
            .map(|market| (market.market_id.as_str(), market))
            .collect();
        let received_at = Instant::now();
        let mut updated_indices = Vec::new();
        let mut quotes_to_cache = HashMap::new();

        {
            let mut matched_markets = self.matched_markets.write();

            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                let Some(quote) =
                    quotes_by_market.get(matched.polymarket_market.market_id.as_str())
                else {
                    continue;
                };
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

        updated_indices.len()
    }

    fn is_poly_rest_quote_fresh_at(&self, quote: &PolymarketRestQuote, now: Instant) -> bool {
        now.saturating_duration_since(quote.received_at) <= self.poly_quote_freshness
    }

    fn is_kalshi_rest_quote_fresh_at(&self, quote: &KalshiRestQuote, now: Instant) -> bool {
        now.saturating_duration_since(quote.received_at) <= self.kalshi_quote_freshness
    }

    fn fresh_kalshi_rest_prices(
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

    fn fresh_poly_rest_prices(
        &self,
        matched_market: &MatchedMarket,
        now: Instant,
    ) -> Option<(f64, f64)> {
        let quote = self
            .poly_rest_quotes
            .read()
            .get(&matched_market.polymarket_market.market_id)
            .cloned()?;
        if !self.is_poly_rest_quote_fresh_at(&quote, now) {
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

    fn is_kalshi_rest_available(&self, now: Instant) -> bool {
        *self.kalshi_rest_connected.read()
            && self
                .kalshi_rest_last_success
                .read()
                .is_some_and(|last_success| {
                    now.saturating_duration_since(last_success) <= self.kalshi_quote_freshness
                })
    }

    fn is_polymarket_rest_available(&self, now: Instant) -> bool {
        *self.polymarket_rest_connected.read()
            && self
                .polymarket_rest_last_success
                .read()
                .is_some_and(|last_success| {
                    now.saturating_duration_since(last_success) <= self.poly_quote_freshness
                })
    }

    /// Merge newly discovered markets and rebuild the Kalshi subscription lookup.
    ///
    /// Polymarket US market-side IDs are not subscriptions. Fresh native slug
    /// and long/short mappings replace the previous market atomically.
    pub fn add_matched_markets(
        &self,
        new_markets: Vec<MatchedMarket>,
        _new_lookup: std::collections::HashMap<String, Vec<usize>>,
    ) -> usize {
        if new_markets.is_empty() {
            return 0;
        }

        let mut added = 0;
        {
            let mut markets = self.matched_markets.write();
            for new_market in new_markets {
                if let Some(existing) = markets
                    .iter_mut()
                    .find(|existing| existing.market_key() == new_market.market_key())
                {
                    *existing = new_market;
                } else {
                    markets.push(new_market);
                    added += 1;
                }
            }
            let matcher = EventMatcher::new(24);
            *self.market_lookup.write() = matcher.get_subscription_info(&markets).market_lookup;
        }

        if added > 0 {
            info!("📊 Added {} matched markets", added);
        }
        added
    }

    /// Return current Kalshi tickers for WebSocket subscriptions.
    pub fn get_kalshi_subscription_ids(&self) -> Vec<String> {
        let markets = self.matched_markets.read();
        EventMatcher::new(24)
            .get_subscription_info(&markets)
            .kalshi_tickers
    }

    /// Handle incoming price update
    pub fn on_price_update(&self, update: PriceUpdate) {
        if update.platform == Platform::Kalshi {
            self.on_kalshi_price_update(update);
        }
    }

    /// Handle Kalshi price update
    fn on_kalshi_price_update(&self, update: PriceUpdate) {
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

        self.metrics
            .record(Operation::KalshiWsProcess, start.elapsed());
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

        drop(markets);

        if let Some(mut opp) = opportunity {
            // Gateway quotes do not include an executable size. Keep these
            // fields at zero rather than manufacturing CLOB depth.
            opp.poly_ask_depth = 0.0;
            opp.poly_ask_size = 0.0;
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
            polymarket_connected: self.is_polymarket_rest_available(now_instant),
            kalshi_source: "rest_polling",
            polymarket_source: "rest_polling",
            kalshi_latency_ms,
            polymarket_latency_ms,
        }
    }

    /// Get a reference to storage
    pub fn get_storage(&self) -> Arc<ArbitrageStorage> {
        self.storage.clone()
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
                    arbitrage_type,
                ) = if let Some(opp) = opportunity {
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

                let end_time = mm.kalshi_market.start_time.map(|t| t.to_rfc3339());

                Some(MatchedMarketFrontend {
                    event_name: mm.event_name.clone(),
                    team_name: mm.team_name.clone(),
                    game_date: mm.game_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    kalshi_market_id: mm.kalshi_market.market_id.clone(),
                    polymarket_market_id: mm.polymarket_market.market_id.clone(),
                    polymarket_market_slug: own_execution.market_slug,
                    polymarket_team_position_side: own_execution.position_side,
                    polymarket_opponent_name: opponent.to_string(),
                    polymarket_opponent_position_side: opponent_execution.position_side,
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
    /// Kalshi market data source; current prices come from REST polling.
    pub kalshi_source: &'static str,
    /// Polymarket market data source; currently `rest_polling`.
    pub polymarket_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymarket_latency_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::models::KalshiMarket;
    use crate::services::ArbitrageStorage;

    fn matched_market() -> MatchedMarket {
        MatchedMarket {
            event_name: "LAL-MEM".to_string(),
            team_name: "LAL".to_string(),
            game_date: None,
            kalshi_market: KalshiMarket {
                market_id: "KXLAL".to_string(),
                event_id: "event".to_string(),
                event_name: "LAL-MEM".to_string(),
                team_name: "LAL".to_string(),
                opponent_name: "MEM".to_string(),
                yes_price: 0.5,
                no_price: 0.5,
                start_time: None,
                volume: None,
                liquidity: None,
            },
            polymarket_market: PolymarketMarket {
                market_id: "poly-market".to_string(),
                market_slug: "lal-mem-2026-08-17".to_string(),
                event_name: "LAL-MEM".to_string(),
                team_a: "LAL".to_string(),
                team_b: "MEM".to_string(),
                price_a: 0.5,
                price_b: 0.5,
                team_a_position: crate::models::PolymarketPositionSide::Long,
                team_b_position: crate::models::PolymarketPositionSide::Short,
                start_time: None,
                volume: None,
            },
            poly_yes_price: 0.5,
            poly_no_price: 0.5,
            confidence: 1.0,
        }
    }

    #[tokio::test]
    async fn kalshi_rest_asks_override_websocket_prices_and_expire() {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
        );
        manager.set_matched_markets(vec![matched_market()]);
        manager
            .kalshi_ws_prices
            .write()
            .insert("KXLAL".to_string(), (0.22, 0.23, 0.76, 0.77));
        assert_eq!(
            manager.update_kalshi_rest_quotes(&[KalshiMarketQuote {
                market_id: "KXLAL".to_string(),
                yes_ask: 0.26,
                no_ask: 0.75,
            }]),
            1
        );

        let fresh_market = PolymarketMarket {
            price_a: 0.42,
            price_b: 0.58,
            ..matched_market().polymarket_market
        };
        assert_eq!(manager.update_polymarket_rest_quotes(&[fresh_market]), 1);
        assert!(manager.is_market_ready(0));
        let frontend_market = manager
            .get_matched_markets_for_frontend()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(frontend_market.kalshi_yes_price, 0.26);
        assert_eq!(frontend_market.kalshi_no_price, 0.75);
        assert!(frontend_market.kalshi_ready);
        let opportunity = manager.calculate_all().into_iter().next().unwrap();
        assert_eq!(opportunity.kalshi_price, 0.26);
        assert_eq!(opportunity.kalshi_side, "yes");
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.42);
        assert_eq!(manager.matched_markets.read()[0].poly_no_price, 0.58);

        manager
            .kalshi_rest_quotes
            .write()
            .get_mut("KXLAL")
            .unwrap()
            .received_at = Instant::now() - Duration::from_secs(11);
        assert!(!manager.is_market_ready(0));
        assert!(
            !manager
                .get_matched_markets_for_frontend()
                .into_iter()
                .next()
                .unwrap()
                .kalshi_ready
        );
    }
}
