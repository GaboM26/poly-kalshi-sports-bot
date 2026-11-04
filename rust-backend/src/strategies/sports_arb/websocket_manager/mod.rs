//! WebSocket Manager
//!
//! Manages real-time WebSocket connections to both Kalshi and Polymarket,
//! handles price updates, and triggers arbitrage calculations.
//!
//! This module is split into:
//! - mod.rs: the manager struct, construction and wiring
//! - markets.rs: matched-market registry
//! - quotes.rs: REST quote caches and freshness
//! - polymarket_feed.rs: Polymarket WebSocket books, epochs and liveness
//! - detection.rs: price updates, calculation and opportunity notification
//! - frontend.rs: stats, coverage and market list views
//! - opportunity_tracker.rs: Opportunity tracking logic
//! - auto_trade.rs: Auto-trade state management
//! - market_lifecycle.rs: Ended market detection and cleanup

mod auto_trade;
mod detection;
mod frontend;
mod market_lifecycle;
mod markets;
mod opportunity_tracker;
mod polymarket_feed;
mod quotes;

pub use frontend::*;
pub use polymarket_feed::*;
pub(crate) use quotes::*;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::{debug, info};

use crate::clients::polymarket::PolymarketMarketBook;
use crate::clients::{KalshiClient, KalshiMarketQuote, PolymarketClient};
use crate::core::{ArbitrageCalculator, EventMatcher};
use crate::models::{
    ArbitrageOpportunity, ArbitrageTrackingRecord, MatchedMarket, MatchedMarketFrontend, Platform,
    PolymarketMarket, PriceUpdate, ScanStats, SystemStats,
};
use crate::services::metrics::{Operation, PerformanceMetrics};
use crate::services::storage::ArbitrageStorage;
use crate::config::SportsArbConfig;

/// WebSocket manager for real-time price updates
pub struct WebSocketManager {
    /// Matched markets to monitor
    pub(crate) matched_markets: Arc<RwLock<Vec<MatchedMarket>>>,
    /// Strategy settings (`[sports_arb]`), shared with `ArbitrageService`.
    pub(crate) settings: Arc<SportsArbConfig>,
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
    /// Polymarket client for real CLOB depth lookups when tracking starts
    pub(crate) polymarket_client: Option<PolymarketClient>,
    /// Tracking threshold for high-profit opportunities (percentage)
    pub(crate) tracking_threshold: crate::core::calculator::LiveF64,
    /// Set of opportunity IDs that have been auto-traded (to prevent duplicates)
    pub(crate) auto_traded_opportunities: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Extreme price detection: market_key -> first_detected_time
    pub(crate) ended_market_detection: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// Set of market keys that have been confirmed as ended
    pub(crate) confirmed_ended_markets: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Consecutive Polymarket REST refresh cycles in which a still-matched
    /// market's polymarket_market_id was absent from the fresh
    /// active=true&closed=false feed. Debounced (see
    /// `ended_markets.polymarket_missing_cycles`) so one transient gap in that feed
    /// doesn't remove a market that is still genuinely open.
    pub(crate) poly_missing_streak: Arc<RwLock<HashMap<String, u32>>>,
    /// market_id -> when the newest WebSocket quote for it arrived, so a
    /// REST snapshot fetched *before* that moment can't overwrite it with
    /// older prices.
    pub(crate) poly_ws_quote_at: Arc<RwLock<HashMap<String, Instant>>>,
    /// market_id -> connection epoch its newest WebSocket quote arrived on.
    /// While that connection is alive, its quote is current and a REST
    /// snapshot never overwrites it (see `update_polymarket_rest_quotes_at`).
    pub(crate) poly_ws_quote_epoch: Arc<RwLock<HashMap<String, u64>>>,
    /// Polymarket WebSocket connection state, mirroring how
    /// `KalshiClient::get_fresh_orderbook` vouches for Kalshi books: the
    /// epoch is bumped on every (re)connect, so anything stored under an
    /// older epoch is not trusted until the new connection re-sends it.
    pub(crate) poly_ws_epoch: Arc<AtomicU64>,
    pub(crate) poly_ws_connected: Arc<AtomicBool>,
    pub(crate) poly_ws_last_frame_at: Arc<RwLock<Option<Instant>>>,
    /// ticker -> when the newest Kalshi WebSocket-derived quote arrived;
    /// same purpose as `poly_ws_quote_at`.
    pub(crate) kalshi_ws_quote_at: Arc<RwLock<HashMap<String, Instant>>>,
    /// slug -> latest full Polymarket book pushed over the WebSocket, tagged
    /// with its connection epoch. Display and tracking read it as is;
    /// execution only through `fresh_polymarket_book`.
    pub(crate) poly_live_books: Arc<RwLock<HashMap<String, LivePolymarketBook>>>,
    /// Cumulative WebSocket message counters, read by the periodic stats
    /// line (`ws_stats`) to show whether each stream is actually delivering.
    poly_ws_quote_msgs: Arc<AtomicU64>,
    poly_ws_book_msgs: Arc<AtomicU64>,
    kalshi_ws_msgs: Arc<AtomicU64>,
    /// Set of recorded skip reasons: "market_key:simplified_reason" -> prevent duplicate skip records
    pub(crate) recorded_skip_reasons: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Set of market keys excluded from auto-trade (user-defined)
    pub(crate) excluded_markets: Arc<RwLock<std::collections::HashSet<String>>>,
    /// market_key -> last time a one-shot Polymarket depth snapshot was
    /// fetched for it, so a market flapping in and out of tracking doesn't
    /// re-hit Polymarket's real book endpoint on every restart.
    pub(crate) poly_depth_snapshot_cooldown: Arc<RwLock<HashMap<String, Instant>>>,
    /// Set once at startup when Discord notifications are enabled.
    discord_client: std::sync::OnceLock<Arc<crate::services::DiscordClient>>,
    /// Auto-trade queue: market keys waiting to be executed
    pub auto_trade_queue: Arc<RwLock<VecDeque<String>>>,
    /// Flag indicating if an auto-trade is currently being executed
    pub is_auto_trading: Arc<AtomicBool>,
}

impl WebSocketManager {
    /// Enable Discord alerts for newly tracked opportunities (set once).
    pub fn set_discord_client(&self, client: Arc<crate::services::DiscordClient>) {
        let _ = self.discord_client.set(client);
    }

    /// Create a new WebSocket manager
    pub fn new(
        min_profit_margin: f64,
        default_bet_amount: f64,
        tracking_threshold: f64,
        storage: Arc<ArbitrageStorage>,
        metrics: Arc<PerformanceMetrics>,
        rest_quote_freshness: Duration,
        settings: Arc<SportsArbConfig>,
    ) -> Self {
        let (opportunity_tx, _) = broadcast::channel(100);
        let (scan_stats_tx, _) = broadcast::channel(100);

        Self {
            matched_markets: Arc::new(RwLock::new(Vec::new())),
            settings,
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
            polymarket_client: None,
            tracking_threshold: crate::core::calculator::LiveF64::new(tracking_threshold),
            auto_traded_opportunities: Arc::new(RwLock::new(std::collections::HashSet::new())),
            ended_market_detection: Arc::new(RwLock::new(HashMap::new())),
            confirmed_ended_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
            poly_missing_streak: Arc::new(RwLock::new(HashMap::new())),
            poly_ws_quote_at: Arc::new(RwLock::new(HashMap::new())),
            poly_ws_quote_epoch: Arc::new(RwLock::new(HashMap::new())),
            poly_ws_epoch: Arc::new(AtomicU64::new(0)),
            poly_ws_connected: Arc::new(AtomicBool::new(false)),
            poly_ws_last_frame_at: Arc::new(RwLock::new(None)),
            kalshi_ws_quote_at: Arc::new(RwLock::new(HashMap::new())),
            poly_live_books: Arc::new(RwLock::new(HashMap::new())),
            poly_ws_quote_msgs: Arc::new(AtomicU64::new(0)),
            poly_ws_book_msgs: Arc::new(AtomicU64::new(0)),
            kalshi_ws_msgs: Arc::new(AtomicU64::new(0)),
            recorded_skip_reasons: Arc::new(RwLock::new(std::collections::HashSet::new())),
            excluded_markets: Arc::new(RwLock::new(std::collections::HashSet::new())),
            poly_depth_snapshot_cooldown: Arc::new(RwLock::new(HashMap::new())),
            discord_client: std::sync::OnceLock::new(),
            auto_trade_queue: Arc::new(RwLock::new(VecDeque::new())),
            is_auto_trading: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the Kalshi client used for executable-depth checks.
    pub fn set_kalshi_client(&mut self, kalshi: KalshiClient) {
        self.kalshi_client = Some(kalshi);
    }

    /// Set the Polymarket client used for real CLOB depth lookups when a
    /// tracked opportunity starts. Reporting only — the auto-trade path
    /// fetches its own fresh book immediately before submitting.
    pub fn set_polymarket_client(&mut self, polymarket: PolymarketClient) {
        self.polymarket_client = Some(polymarket);
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

    /// Get a reference to storage
    pub fn get_storage(&self) -> Arc<ArbitrageStorage> {
        self.storage.clone()
    }
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
                live: false,
                period: None,
            },
            poly_yes_price: 0.5,
            poly_no_price: 0.5,
            confidence: 1.0,
        }
    }

    fn test_manager() -> WebSocketManager {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
            Arc::new(SportsArbConfig::default()),
        );
        manager.set_matched_markets(vec![matched_market()]);
        manager
    }

    #[tokio::test]
    async fn polymarket_ws_quote_maps_each_competitor_to_its_own_side() {
        let manager = test_manager();
        // Fixture: LAL (team_a) = Long, MEM (team_b) = Short.
        assert_eq!(manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.31, 0.71), 1);
        let m = manager.matched_markets.read()[0].clone();
        assert_eq!(m.polymarket_market.price_a, 0.31);
        assert_eq!(m.polymarket_market.price_b, 0.71);
        // Tracked competitor is LAL: yes = its long quote, no = the other.
        assert_eq!(m.poly_yes_price, 0.31);
        assert_eq!(m.poly_no_price, 0.71);
        assert!(manager.poly_rest_quotes.read().contains_key("poly-market"));
    }

    #[tokio::test]
    async fn polymarket_ws_quote_for_an_unknown_slug_is_ignored() {
        let manager = test_manager();
        assert_eq!(manager.update_polymarket_ws_quote("other-slug", 0.31, 0.71), 0);
        assert!(manager.poly_rest_quotes.read().is_empty());
    }

    #[tokio::test]
    async fn older_rest_snapshot_does_not_overwrite_a_newer_ws_quote() {
        let manager = test_manager();
        let fetch_started = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.31, 0.71);

        let stale_rest = PolymarketMarket {
            price_a: 0.50,
            price_b: 0.50,
            ..matched_market().polymarket_market
        };
        manager.update_polymarket_rest_quotes_at(&[stale_rest.clone()], fetch_started);
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.31);
        let cached = manager.poly_rest_quotes.read().get("poly-market").cloned().unwrap();
        assert_eq!((cached.price_a, cached.price_b), (0.31, 0.71));

        // With no live WebSocket connection, a REST snapshot fetched after
        // the WS quote does win.
        manager.update_polymarket_rest_quotes_at(&[stale_rest], Instant::now());
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.50);
    }

    #[tokio::test]
    async fn a_cached_rest_snapshot_never_overwrites_a_live_ws_quote() {
        let manager = test_manager();
        manager.on_polymarket_ws_connected();
        manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.31, 0.71);

        // Fetched after the WS quote, but the gateway feed is CDN-cached, so
        // its contents can be up to 30s older than the quote.
        let cached_rest = PolymarketMarket {
            price_a: 0.50,
            price_b: 0.50,
            ..matched_market().polymarket_market
        };
        manager.update_polymarket_rest_quotes_at(&[cached_rest.clone()], Instant::now());
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.31);
        let cached = manager.poly_rest_quotes.read().get("poly-market").cloned().unwrap();
        assert_eq!((cached.price_a, cached.price_b), (0.31, 0.71));

        // Once the stream is down, REST is the only source and applies.
        manager.on_polymarket_ws_disconnected();
        manager.update_polymarket_rest_quotes_at(&[cached_rest], Instant::now());
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.50);
    }

    #[tokio::test]
    async fn rest_applies_to_a_market_the_live_ws_has_not_quoted_on_this_connection() {
        let manager = test_manager();
        manager.on_polymarket_ws_connected();
        manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.31, 0.71);
        manager.on_polymarket_ws_disconnected();
        manager.on_polymarket_ws_connected();
        let rest = PolymarketMarket {
            price_a: 0.50,
            price_b: 0.50,
            ..matched_market().polymarket_market
        };
        manager.update_polymarket_rest_quotes_at(&[rest], Instant::now());
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.50);
    }

    fn ws_book(slug: &str, best_offer: f64) -> PolymarketMarketBook {
        use crate::clients::polymarket::PolymarketBookLevel;
        PolymarketMarketBook {
            success: true,
            market_slug: slug.to_string(),
            state: "MARKET_STATE_OPEN".to_string(),
            transact_time: None,
            fetched_at_ms: 0,
            bids: vec![PolymarketBookLevel { price: best_offer - 0.01, quantity: 50.0 }],
            offers: vec![PolymarketBookLevel { price: best_offer, quantity: 50.0 }],
            minimum_trade_qty: None,
            price_tick_size: None,
        }
    }

    #[tokio::test]
    async fn a_ws_book_is_executable_only_from_the_live_connection() {
        let manager = test_manager();
        let slug = "lal-mem-2026-08-17";
        let window = manager.settings.polymarket_ws_max_silence();

        // Arrived before any connection was acknowledged.
        manager.update_polymarket_ws_book(ws_book(slug, 0.50));
        assert!(manager.fresh_polymarket_book(slug, window).is_none());

        manager.on_polymarket_ws_connected();
        assert!(manager.fresh_polymarket_book(slug, window).is_none());
        manager.update_polymarket_ws_book(ws_book(slug, 0.52));
        let fresh = manager.fresh_polymarket_book(slug, window).expect("current book");
        assert_eq!(fresh.offers[0].price, 0.52);
        assert!(manager.fresh_polymarket_book("other-slug", window).is_none());

        manager.on_polymarket_ws_disconnected();
        assert!(manager.fresh_polymarket_book(slug, window).is_none());
        // Display still reads the last book it has.
        assert_eq!(manager.polymarket_live_book(slug).unwrap().offers[0].price, 0.52);

        // After a reconnect the old book stays untrusted until re-sent.
        manager.on_polymarket_ws_connected();
        assert!(manager.fresh_polymarket_book(slug, window).is_none());
        manager.update_polymarket_ws_book(ws_book(slug, 0.55));
        assert_eq!(manager.fresh_polymarket_book(slug, window).unwrap().offers[0].price, 0.55);
    }

    #[tokio::test]
    async fn a_silent_ws_connection_stops_vouching_for_its_books() {
        let manager = test_manager();
        let slug = "lal-mem-2026-08-17";
        let window = manager.settings.polymarket_ws_max_silence();
        manager.on_polymarket_ws_connected();
        manager.update_polymarket_ws_book(ws_book(slug, 0.50));
        assert!(manager.fresh_polymarket_book(slug, window).is_some());

        *manager.poly_ws_last_frame_at.write() =
            Instant::now().checked_sub(window + Duration::from_secs(1));
        assert!(manager.fresh_polymarket_book(slug, window).is_none());

        // A heartbeat alone revives it: a quiet market's book is still exact.
        manager.note_polymarket_ws_frame();
        assert!(manager.fresh_polymarket_book(slug, window).is_some());
    }

    #[tokio::test]
    async fn kalshi_ws_quote_updates_prices_and_only_recalculates_on_change() {
        let manager = test_manager();
        assert_eq!(manager.apply_kalshi_ws_quote("KXLAL", 0.26, 0.75), 1);
        assert_eq!(manager.matched_markets.read()[0].kalshi_market.yes_price, 0.26);
        assert_eq!(manager.matched_markets.read()[0].kalshi_market.no_price, 0.75);
        // Same best prices (a deeper-level delta): no recalculation.
        assert_eq!(manager.apply_kalshi_ws_quote("KXLAL", 0.26, 0.75), 0);
        assert_eq!(manager.apply_kalshi_ws_quote("KXLAL", 0.27, 0.74), 1);
        assert_eq!(manager.apply_kalshi_ws_quote("UNKNOWN", 0.27, 0.74), 0);
    }

    #[tokio::test]
    async fn older_kalshi_rest_snapshot_does_not_overwrite_a_newer_ws_quote() {
        let manager = test_manager();
        let fetch_started = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        manager.apply_kalshi_ws_quote("KXLAL", 0.26, 0.75);
        let rest = KalshiMarketQuote {
            market_id: "KXLAL".to_string(),
            yes_ask: 0.40,
            no_ask: 0.61,
            status: "open".to_string(),
        };
        manager.update_kalshi_rest_quotes_at(&[rest.clone()], fetch_started);
        let cached = manager.kalshi_rest_quotes.read().get("KXLAL").cloned().unwrap();
        assert_eq!((cached.yes_ask, cached.no_ask), (0.26, 0.75));
        assert_eq!(manager.matched_markets.read()[0].kalshi_market.yes_price, 0.26);

        // A snapshot fetched after the WS quote does win (self-heals drift).
        manager.update_kalshi_rest_quotes_at(&[rest], Instant::now());
        assert_eq!(manager.matched_markets.read()[0].kalshi_market.yes_price, 0.40);
    }

    #[tokio::test]
    async fn opportunity_polymarket_depth_uses_the_side_opposite_kalshi() {
        use crate::clients::polymarket::PolymarketBookLevel;
        let manager = test_manager();
        // Fixture: LAL = Long, MEM = Short. Kalshi YES on LAL pairs with
        // Polymarket MEM, i.e. the SHORT side, bought against the bids.
        manager.update_polymarket_ws_book(PolymarketMarketBook {
            success: true,
            market_slug: "lal-mem-2026-08-17".to_string(),
            state: "MARKET_STATE_OPEN".to_string(),
            transact_time: None,
            fetched_at_ms: 0,
            bids: vec![
                PolymarketBookLevel { price: 0.42, quantity: 4.0 },
                PolymarketBookLevel { price: 0.40, quantity: 10.0 },
            ],
            offers: vec![PolymarketBookLevel { price: 0.45, quantity: 99.0 }],
            minimum_trade_qty: None,
            price_tick_size: None,
        });
        manager.apply_kalshi_ws_quote("KXLAL", 0.26, 0.75);
        manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.42, 0.58);
        let opp = manager.get_opportunities().into_iter().next().expect("opportunity");
        assert_eq!(opp.kalshi_side, "yes");
        // Only contracts at the quoted price (1 - 0.42 = 0.58) or better
        // count; the 0.60 level behind it is excluded.
        assert_eq!(opp.poly_ask_size, 4.0);
        assert!((opp.poly_ask_depth - 4.0 * 0.58).abs() < 1e-9);
    }

    #[tokio::test]
    async fn rest_refresh_carries_live_state_even_when_ws_owns_the_prices() {
        let manager = test_manager();
        let fetch_started = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        manager.update_polymarket_ws_quote("lal-mem-2026-08-17", 0.31, 0.71);
        let in_play = PolymarketMarket {
            live: true,
            period: Some("P2".to_string()),
            ..matched_market().polymarket_market
        };
        manager.update_polymarket_rest_quotes_at(&[in_play], fetch_started);
        let front = manager.get_matched_markets_for_frontend();
        assert!(front[0].polymarket_live);
        assert_eq!(front[0].polymarket_period.as_deref(), Some("P2"));
        assert_eq!(manager.matched_markets.read()[0].poly_yes_price, 0.31);
    }

    #[tokio::test]
    async fn polymarket_slugs_are_unique() {
        let manager = test_manager();
        assert_eq!(manager.get_polymarket_slugs(), vec!["lal-mem-2026-08-17".to_string()]);
    }

    /// Manager with the LAL fixture market and fresh quotes on both sides.
    fn manager_with_quotes(kalshi_yes: f64, kalshi_no: f64, poly_a: f64, poly_b: f64) -> WebSocketManager {
        let manager = test_manager();
        assert_eq!(
            manager.update_kalshi_rest_quotes(&[KalshiMarketQuote {
                market_id: "KXLAL".to_string(),
                yes_ask: kalshi_yes,
                no_ask: kalshi_no,
                status: "open".to_string(),
            }]),
            1
        );
        let fresh = PolymarketMarket {
            price_a: poly_a,
            price_b: poly_b,
            ..matched_market().polymarket_market
        };
        assert_eq!(manager.update_polymarket_rest_quotes(&[fresh]), 1);
        assert!(manager.is_market_ready(0));
        manager
    }

    #[tokio::test]
    async fn a_market_without_an_opportunity_still_reports_both_fees_and_a_real_margin() {
        // Min margin is 1.0 (test_manager). Kalshi YES 55c / NO 46c, Poly LAL
        // 52c / MEM 49c: neither hedge clears it (best is -1.48% after fees).
        let manager = manager_with_quotes(0.55, 0.46, 0.52, 0.49);
        assert!(manager.calculate_all().is_empty());
        let market = manager.get_matched_markets_for_frontend().into_iter().next().unwrap();
        assert!(!market.has_opportunity, "a preview is never an opportunity");
        assert_eq!(market.arbitrage_type.as_deref(), Some("KalshiNoPolymarketYes"));
        assert_eq!(market.kalshi_contracts, Some(10.0));
        assert!((market.kalshi_fee.unwrap() - 0.18).abs() < 1e-9);
        assert!((market.polymarket_fee.unwrap() - 0.17).abs() < 1e-9);
        assert!((market.gross_profit.unwrap() - 0.20).abs() < 1e-9);
        assert!((market.expected_profit - (-0.15)).abs() < 1e-9, "{}", market.expected_profit);
        assert!(market.profit_margin < 0.0);
    }

    #[tokio::test]
    async fn an_opportunity_reports_the_same_fees_as_before() {
        // Kalshi YES 26c + Poly MEM 58c is a real edge.
        let manager = manager_with_quotes(0.26, 0.75, 0.42, 0.58);
        let opportunity = manager.calculate_all().into_iter().next().expect("opportunity");
        let market = manager.get_matched_markets_for_frontend().into_iter().next().unwrap();
        assert!(market.has_opportunity);
        assert_eq!(market.kalshi_fee, Some(opportunity.kalshi_fee));
        assert_eq!(market.polymarket_fee, Some(opportunity.polymarket_fee));
        assert_eq!(market.expected_profit, opportunity.expected_profit);
        assert_eq!(market.profit_margin, opportunity.profit_margin);
    }

    #[tokio::test]
    async fn no_preview_is_invented_without_fresh_prices_on_both_sides() {
        // Only Kalshi has a quote: nothing to evaluate, so no fees either.
        let manager = test_manager();
        manager.update_kalshi_rest_quotes(&[KalshiMarketQuote {
            market_id: "KXLAL".to_string(),
            yes_ask: 0.55,
            no_ask: 0.46,
            status: "open".to_string(),
        }]);
        let market = manager.get_matched_markets_for_frontend().into_iter().next().unwrap();
        assert!(market.kalshi_ready && !market.poly_ready);
        assert!(!market.has_opportunity);
        assert_eq!(market.kalshi_fee, None);
        assert_eq!(market.polymarket_fee, None);
        assert_eq!(market.arbitrage_type, None);
        assert_eq!(market.profit_margin, 0.0);
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
            Arc::new(SportsArbConfig::default()),
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
                status: "open".to_string(),
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

    #[tokio::test]
    async fn kalshi_closed_or_settled_status_confirms_ended_immediately() {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
            Arc::new(SportsArbConfig::default()),
        );
        manager.set_matched_markets(vec![matched_market()]);

        // A market status of "closed" must be trusted immediately, even
        // though the price (84c) is nowhere near the extreme-price
        // heuristic's thresholds (99c/2c) - this is the exact bug pattern
        // reported live: a market closes for trading at a normal price.
        let updated = manager.update_kalshi_rest_quotes(&[KalshiMarketQuote {
            market_id: "KXLAL".to_string(),
            yes_ask: 0.84,
            no_ask: 0.16,
            status: "closed".to_string(),
        }]);
        assert_eq!(updated, 0, "a closed market must not be price-updated");
        assert!(manager
            .confirmed_ended_markets
            .read()
            .contains(&matched_market().market_key()));
        // Price fields must remain untouched, not overwritten with the
        // closing quote.
        assert_eq!(manager.matched_markets.read()[0].kalshi_market.yes_price, 0.5);
    }

    #[tokio::test]
    async fn kalshi_open_status_at_a_non_extreme_price_is_not_marked_ended() {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
            Arc::new(SportsArbConfig::default()),
        );
        manager.set_matched_markets(vec![matched_market()]);

        let updated = manager.update_kalshi_rest_quotes(&[KalshiMarketQuote {
            market_id: "KXLAL".to_string(),
            yes_ask: 0.84,
            no_ask: 0.16,
            status: "open".to_string(),
        }]);
        assert_eq!(updated, 1);
        assert!(manager.confirmed_ended_markets.read().is_empty());
    }

    #[tokio::test]
    async fn polymarket_missing_from_active_feed_is_debounced_then_confirmed_ended() {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
            Arc::new(SportsArbConfig::default()),
        );
        manager.set_matched_markets(vec![matched_market()]);
        let key = matched_market().market_key();

        // First absence: below polymarket_missing_cycles, not yet ended.
        manager.update_polymarket_rest_quotes(&[]);
        assert!(!manager.confirmed_ended_markets.read().contains(&key));
        assert_eq!(*manager.poly_missing_streak.read().get(&key).unwrap(), 1);

        // Second consecutive absence reaches the threshold.
        manager.update_polymarket_rest_quotes(&[]);
        assert!(manager.confirmed_ended_markets.read().contains(&key));
        // The streak entry is cleared once confirmed.
        assert!(!manager.poly_missing_streak.read().contains_key(&key));
    }

    #[tokio::test]
    async fn polymarket_reappearing_resets_the_missing_streak() {
        let storage = Arc::new(ArbitrageStorage::new(":memory:").unwrap());
        let manager = WebSocketManager::new(
            1.0,
            10.0,
            1.0,
            storage,
            Arc::new(PerformanceMetrics::new()),
            Duration::from_secs(10),
            Arc::new(SportsArbConfig::default()),
        );
        manager.set_matched_markets(vec![matched_market()]);
        let key = matched_market().market_key();

        manager.update_polymarket_rest_quotes(&[]);
        assert_eq!(*manager.poly_missing_streak.read().get(&key).unwrap(), 1);

        // A single reappearance in the feed must clear the streak, not just
        // decrement it - flicker should never accumulate toward removal.
        manager.update_polymarket_rest_quotes(&[matched_market().polymarket_market]);
        assert!(!manager.poly_missing_streak.read().contains_key(&key));

        manager.update_polymarket_rest_quotes(&[]);
        assert!(!manager.confirmed_ended_markets.read().contains(&key));
        assert_eq!(*manager.poly_missing_streak.read().get(&key).unwrap(), 1);
    }

    #[tokio::test]
    async fn hot_added_markets_are_in_the_reconnect_ticker_list() {
        let manager = test_manager();
        let mut added = matched_market();
        added.team_name = "MEM".to_string();
        added.kalshi_market.market_id = "HOT-ADDED".to_string();
        assert!(!manager
            .get_kalshi_subscription_ids()
            .contains(&"HOT-ADDED".to_string()));
        manager.add_matched_markets(vec![added], HashMap::new());
        assert!(manager
            .get_kalshi_subscription_ids()
            .contains(&"HOT-ADDED".to_string()));
    }
}
