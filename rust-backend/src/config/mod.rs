//! Configuration management module
//!
//! Handles loading and parsing of TOML configuration files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;

/// HTTP client behavior shared by the exchange REST clients.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HttpClientConfig {
    /// Per-request timeout.
    pub timeout_secs: u64,
    /// Total attempts for a retryable request (1 = no retry). Order
    /// placement never retries regardless of this value.
    pub max_attempts: u32,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 10,
            max_attempts: 3,
        }
    }
}

impl HttpClientConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    fn validate(&self, section: &str) -> Result<(), String> {
        if self.timeout_secs == 0 || self.max_attempts == 0 {
            return Err(format!("[{section}.http] timeout_secs and max_attempts must be positive"));
        }
        Ok(())
    }
}

/// Kalshi API configuration
#[derive(Debug, Clone, Deserialize)]
pub struct KalshiConfig {
    pub api_key: String,
    pub api_secret: String,
    #[serde(default = "default_kalshi_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub http: HttpClientConfig,
}

fn default_kalshi_base_url() -> String {
    "https://external-api.kalshi.com/trade-api/v2".to_string()
}

/// Polymarket US API configuration.
///
/// `base_url` is the public gateway (unauthenticated: events, sports, market
/// book/detail). `api_base_url` is the authenticated API (orders, account,
/// portfolio), signed with Ed25519 per request in `clients/polymarket/mod.rs`.
/// Credentials are preferably supplied via `POLYMARKET_KEY_ID` /
/// `POLYMARKET_SECRET_KEY` env vars, falling back to `key_id`/`secret_key`
/// below.
#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    /// Public gateway base URL
    #[serde(default = "default_poly_base_url")]
    pub base_url: String,

    /// Authenticated API base URL
    #[serde(default = "default_poly_api_base_url")]
    pub api_base_url: String,

    /// Polymarket US API key ID (UUID). Prefer the POLYMARKET_KEY_ID env var.
    #[serde(default)]
    pub key_id: String,

    /// Base64-encoded Ed25519 secret key. Prefer the POLYMARKET_SECRET_KEY env var.
    #[serde(default)]
    pub secret_key: String,

    #[serde(default)]
    pub http: HttpClientConfig,

    #[serde(default)]
    pub cache: PolymarketCacheConfig,

    #[serde(default)]
    pub ws: PolymarketWsConfig,
}

/// How long account reads are served from memory before re-fetching.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PolymarketCacheConfig {
    pub balance_ttl_secs: u64,
    pub positions_ttl_secs: u64,
    /// minimumTradeQty / orderPriceMinTickSize are static per-market validation
    /// constraints, not live pricing data - cache them well past a single trading
    /// session so we don't add a second live API call (and more rate-limit risk)
    /// to every depth check and order attempt.
    pub market_constraints_ttl_secs: u64,
}

impl Default for PolymarketCacheConfig {
    fn default() -> Self {
        Self {
            balance_ttl_secs: 30,
            positions_ttl_secs: 30,
            market_constraints_ttl_secs: 3600,
        }
    }
}

impl PolymarketCacheConfig {
    pub fn balance_ttl(&self) -> Duration {
        Duration::from_secs(self.balance_ttl_secs)
    }
    pub fn positions_ttl(&self) -> Duration {
        Duration::from_secs(self.positions_ttl_secs)
    }
    pub fn market_constraints_ttl(&self) -> Duration {
        Duration::from_secs(self.market_constraints_ttl_secs)
    }
}

/// Polymarket market-data WebSocket timing.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct PolymarketWsConfig {
    /// Reconnect when no frame arrives for this long. The server sends
    /// heartbeats; silence beyond this means a dead socket.
    pub silence_timeout_secs: u64,
    /// How often the subscribed slug set is re-read for hot-adds.
    pub slug_refresh_secs: u64,
    /// Ceiling of the exponential reconnect backoff.
    pub max_backoff_secs: u64,
}

impl Default for PolymarketWsConfig {
    fn default() -> Self {
        Self {
            silence_timeout_secs: 30,
            slug_refresh_secs: 5,
            max_backoff_secs: 30,
        }
    }
}

impl PolymarketWsConfig {
    pub fn silence_timeout(&self) -> Duration {
        Duration::from_secs(self.silence_timeout_secs)
    }
    pub fn slug_refresh(&self) -> Duration {
        Duration::from_secs(self.slug_refresh_secs)
    }
    pub fn max_backoff(&self) -> Duration {
        Duration::from_secs(self.max_backoff_secs)
    }

    fn validate(&self) -> Result<(), String> {
        if self.silence_timeout_secs == 0 || self.slug_refresh_secs == 0 || self.max_backoff_secs == 0 {
            return Err("[polymarket.ws] values must be positive".into());
        }
        Ok(())
    }
}

fn default_poly_base_url() -> String {
    "https://gateway.polymarket.us".to_string()
}

fn default_poly_api_base_url() -> String {
    "https://api.polymarket.us".to_string()
}

/// Application settings
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsConfig {
    /// Refresh interval in seconds (REST quote polling clamps to at least 3 seconds)
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    /// Minimum profit margin percentage
    #[serde(default = "default_min_profit_margin")]
    pub min_profit_margin: f64,
    /// Default bet amount in USD
    #[serde(default = "default_bet_amount")]
    pub default_bet_amount: f64,
    /// Tracking threshold for high-profit opportunities (percentage)
    #[serde(default = "default_tracking_threshold")]
    pub tracking_threshold: f64,
}

fn default_refresh_interval() -> u64 {
    3
}

fn default_min_profit_margin() -> f64 {
    1.0
}

fn default_bet_amount() -> f64 {
    10.0 // Testing phase: reduced from 100.0 to 10.0
}

fn default_tracking_threshold() -> f64 {
    1.0 // Start tracking when profit >= 1%
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            min_profit_margin: default_min_profit_margin(),
            default_bet_amount: default_bet_amount(),
            tracking_threshold: default_tracking_threshold(),
        }
    }
}

/// Authentication configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default = "default_password")]
    pub password: String,
    #[serde(default = "default_secret_key")]
    pub secret_key: String,
    #[serde(default = "default_token_expire_hours")]
    pub token_expire_hours: u64,
}

fn default_username() -> String {
    "admin".to_string()
}

fn default_password() -> String {
    "admin123".to_string()
}

fn default_secret_key() -> String {
    "your-secret-key-change-this-in-production".to_string()
}

fn default_token_expire_hours() -> u64 {
    24
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            username: default_username(),
            password: default_password(),
            secret_key: default_secret_key(),
            token_expire_hours: default_token_expire_hours(),
        }
    }
}

/// Discord webhook notification configuration
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordConfig {
    /// Whether Discord notifications are enabled
    #[serde(default)]
    pub enabled: bool,
    /// Incoming webhook URL (https://discord.com/api/webhooks/...)
    #[serde(default)]
    pub webhook_url: String,
    /// Separate webhook for executed-trade results. Empty: trades go to
    /// `webhook_url` with the opportunity alerts.
    #[serde(default)]
    pub trades_webhook_url: String,
    /// Minimum profit margin (%) for an opportunity alert
    #[serde(default = "default_opportunity_min_margin")]
    pub opportunity_min_margin: f64,
    /// At most one opportunity alert per market per this many seconds.
    #[serde(default = "default_opportunity_cooldown_secs")]
    pub opportunity_cooldown_secs: u64,
}

fn default_opportunity_cooldown_secs() -> u64 {
    60
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: String::new(),
            trades_webhook_url: String::new(),
            opportunity_min_margin: default_opportunity_min_margin(),
            opportunity_cooldown_secs: default_opportunity_cooldown_secs(),
        }
    }
}

fn default_opportunity_min_margin() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MentionMode {
    Off,
    Paper,
    Live,
}

impl Default for MentionMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MentionConfig {
    pub mode: MentionMode,
    pub signal_url: String,
    pub poll_interval_secs: u64,
    pub max_signal_age_secs: u64,
    pub bankroll_usd: f64,
    pub kelly_fraction: f64,
    pub min_net_edge: f64,
    pub max_spread: f64,
    pub min_book_depth_contracts: i32,
    pub max_close_minutes: i64,
    pub allow_hold_to_settlement: bool,
    pub pre_event_only: bool,
    pub max_market_risk_pct: f64,
    pub max_event_risk_pct: f64,
    pub max_day_risk_pct: f64,
    pub max_total_open_risk_pct: f64,
    pub max_open_positions: i32,
    pub daily_loss_breaker_pct: f64,
    pub live_gate_min_days: i64,
    pub live_gate_max_age_days: i64,
    /// A Kalshi WebSocket book older than this is not used for a decision.
    pub book_max_age_secs: u64,
    /// Repeat an unchanged rejection reason in the decision log at most this often.
    pub rejection_log_interval_secs: u64,
    /// HTTP timeout for the local signal service.
    pub signal_timeout_secs: u64,
}

impl Default for MentionConfig {
    fn default() -> Self {
        Self {
            mode: MentionMode::Off,
            signal_url: "http://127.0.0.1:8765".to_string(),
            poll_interval_secs: 10,
            max_signal_age_secs: 60,
            bankroll_usd: 1000.0,
            kelly_fraction: 0.25,
            min_net_edge: 0.03,
            max_spread: 0.10,
            min_book_depth_contracts: 1,
            max_close_minutes: 15,
            allow_hold_to_settlement: false,
            pre_event_only: true,
            max_market_risk_pct: 0.01,
            max_event_risk_pct: 0.02,
            max_day_risk_pct: 0.03,
            max_total_open_risk_pct: 0.05,
            max_open_positions: 5,
            daily_loss_breaker_pct: 0.03,
            live_gate_min_days: 60,
            live_gate_max_age_days: 14,
            book_max_age_secs: 30,
            rejection_log_interval_secs: 15 * 60,
            signal_timeout_secs: 5,
        }
    }
}

impl MentionConfig {
    pub fn book_max_age(&self) -> Duration {
        Duration::from_secs(self.book_max_age_secs)
    }
    pub fn rejection_log_interval(&self) -> Duration {
        Duration::from_secs(self.rejection_log_interval_secs)
    }
    pub fn signal_timeout(&self) -> Duration {
        Duration::from_secs(self.signal_timeout_secs)
    }

    pub fn validate(&self) -> Result<(), String> {
        let finite_positive = [self.bankroll_usd, self.min_net_edge]
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0);
        if !finite_positive {
            return Err("mention bankroll and min_net_edge must be finite and positive".into());
        }
        if !self.kelly_fraction.is_finite() || !(0.0..=1.0).contains(&self.kelly_fraction) {
            return Err("mention kelly_fraction must be finite and in [0,1]".into());
        }
        if !self.max_spread.is_finite() || !(0.0..1.0).contains(&self.max_spread) {
            return Err("mention max_spread must be finite and in (0,1)".into());
        }
        let risk_caps = [
            self.max_market_risk_pct,
            self.max_event_risk_pct,
            self.max_day_risk_pct,
            self.max_total_open_risk_pct,
            self.daily_loss_breaker_pct,
        ];
        if risk_caps
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value > 1.0)
        {
            return Err("mention risk percentages must be finite and in (0,1]".into());
        }
        if self.max_signal_age_secs == 0
            || self.poll_interval_secs == 0
            || self.min_book_depth_contracts <= 0
            || self.max_close_minutes < 0
            || self.max_open_positions <= 0
            || self.live_gate_min_days <= 0
            || self.live_gate_max_age_days <= 0
            || self.book_max_age_secs == 0
            || self.rejection_log_interval_secs == 0
            || self.signal_timeout_secs == 0
        {
            return Err(
                "mention ages, counts, depth, and gate requirements must be positive".into(),
            );
        }
        Ok(())
    }
}

/// One Kalshi series the sports-arb strategy scans.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SeriesConfig {
    /// Kalshi series ticker, the part of a market ticker before the first `-`.
    pub ticker: String,
    /// Parser for the series' events; one of `clients::kalshi::PARSEABLE_CATEGORIES`.
    pub category: String,
    /// Sport shown in the UI.
    pub label: String,
    /// Whether auto-trade may execute on this series. Detection always runs.
    #[serde(default)]
    pub auto_trade: bool,
}

impl SeriesConfig {
    fn new(ticker: &str, category: &str, label: &str, auto_trade: bool) -> Self {
        Self {
            ticker: ticker.into(),
            category: category.into(),
            label: label.into(),
            auto_trade,
        }
    }
}

/// How a matched market is detected as ended (`[sports_arb.ended_markets]`).
/// Exchange status (Kalshi `closed`/`settled`) ends a market immediately;
/// these rules catch the rest.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EndedMarketConfig {
    /// A Kalshi ask at or above this, with the other side at or below
    /// `kalshi_extreme_low`, counts as a decided market.
    pub kalshi_extreme_high: f64,
    pub kalshi_extreme_low: f64,
    /// Same for Polymarket prices.
    pub poly_extreme_high: f64,
    pub poly_extreme_low: f64,
    /// Both exchanges must stay extreme this long before the market ends.
    pub extreme_duration_mins: i64,
    /// Consecutive Polymarket REST refresh cycles a matched market must be
    /// absent from the active=true&closed=false feed before treating that
    /// absence as confirmed closure, so a single transient gap in the feed
    /// doesn't remove a market that is still genuinely open.
    pub polymarket_missing_cycles: u32,
    /// How often ended markets are removed from the caches.
    pub cleanup_interval_secs: u64,
}

impl Default for EndedMarketConfig {
    fn default() -> Self {
        Self {
            kalshi_extreme_high: 0.99,
            kalshi_extreme_low: 0.02,
            poly_extreme_high: 1.00,
            poly_extreme_low: 0.00,
            extreme_duration_mins: 20,
            polymarket_missing_cycles: 2,
            cleanup_interval_secs: 60,
        }
    }
}

/// Cross-exchange sports arbitrage (`[sports_arb]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SportsArbConfig {
    /// Kalshi's explicitly supported head-to-head match-winner series. Keep this
    /// allowlist narrow: it intentionally excludes sets, games, totals, props,
    /// futures, and every table-tennis series.
    ///
    /// NHL and MLB are detected and displayed only (Kalshi settles a game not
    /// played within 48h at a fair price, Polymarket US waits longer), so
    /// their opportunities never trade until settlement equivalence is
    /// decided per sport.
    pub series: Vec<SeriesConfig>,
    /// Market scans run on wall-clock multiples of this interval...
    pub market_scan_interval_secs: u64,
    /// ...plus each of these offsets. Measured 2026-10-26: both exchanges list
    /// a new 15-minute crypto window 15-30s after it opens (Kalshi flaps
    /// between old and new until ~30s, and at 17:45:41 still returned no ETH
    /// window), so a second scan retries.
    pub market_scan_offsets_secs: Vec<u64>,
    /// Kalshi and Polymarket events match only when their start times are
    /// within this many hours.
    pub match_window_hours: i64,
    /// For Kalshi: a connection-liveness backstop, checked against the last
    /// WebSocket message received on *any* subscribed ticker (see
    /// `KalshiClient::get_fresh_orderbook`) - not "this ticker changed
    /// recently," which Kalshi only signals on a real book change and would
    /// wrongly reject an accurate but quiet market. This just needs to be loose
    /// enough to never fire during normal operation and tight enough to catch a
    /// genuinely hung connection promptly. The actual per-book correctness
    /// check is the connection epoch, not this duration.
    /// Polymarket books come from `executable_polymarket_book` instead.
    pub executable_book_max_age_secs: u64,
    /// The Polymarket WebSocket counts as alive only if some frame (a book,
    /// quote, or heartbeat on any market) arrived this recently. Books and
    /// quotes it delivered are trusted only while it is alive. The server sends
    /// a heartbeat every 10s on an otherwise quiet connection (measured with
    /// `examples/probe_poly_book_freshness.rs`), so 15s tolerates jitter
    /// without flapping while still dropping a hung connection quickly.
    pub polymarket_ws_max_silence_secs: u64,
    /// IOC limits are placed this many cents beyond the worst sized level,
    /// and margin is sized at those limits.
    pub limit_cushion_cents: i32,
    /// A market flapping in and out of tracking (profit margin oscillating near
    /// the threshold) must not re-hit Polymarket's real book endpoint on every
    /// restart — that is what triggered a Cloudflare IP rate limit (error 1015)
    /// that then starved out the endpoint the real execution path depends on.
    /// One live snapshot per market within this window is enough for the
    /// Advanced Search "depth at tracking start" display.
    pub poly_depth_snapshot_cooldown_secs: u64,
    /// Delay before the first periodic scan, so the WebSockets connect first.
    pub initial_scan_delay_secs: u64,
    /// How often tracked opportunities are checked for the auto-trade queue.
    pub auto_trade_queue_check_ms: u64,
    /// Pause after each auto-trade execution before the next one starts.
    pub auto_trade_pause_ms: u64,
    pub ended_markets: EndedMarketConfig,
}

impl Default for SportsArbConfig {
    fn default() -> Self {
        Self {
            series: vec![
                SeriesConfig::new("KXNBAGAME", "NBA", "NBA", true),
                SeriesConfig::new("KXATPMATCH", "TENNIS", "ATP", true),
                SeriesConfig::new("KXWTAMATCH", "TENNIS", "WTA", true),
                SeriesConfig::new("KXITFMATCH", "TENNIS", "ITF M", true),
                SeriesConfig::new("KXITFWMATCH", "TENNIS", "ITF W", true),
                SeriesConfig::new("KXATPCHALLENGERMATCH", "TENNIS", "ATP Challenger", true),
                SeriesConfig::new("KXNHLGAME", "NHL", "NHL", false),
                SeriesConfig::new("KXMLBGAME", "MLB", "MLB", false),
                SeriesConfig::new("KXBTC15M", "CRYPTO15", "BTC 15m", true),
                SeriesConfig::new("KXETH15M", "CRYPTO15", "ETH 15m", true),
                SeriesConfig::new("KXSOL15M", "CRYPTO15", "SOL 15m", true),
            ],
            market_scan_interval_secs: 300,
            market_scan_offsets_secs: vec![40, 100],
            match_window_hours: 24,
            executable_book_max_age_secs: 30,
            polymarket_ws_max_silence_secs: 15,
            limit_cushion_cents: 1,
            poly_depth_snapshot_cooldown_secs: 600,
            initial_scan_delay_secs: 30,
            auto_trade_queue_check_ms: 200,
            auto_trade_pause_ms: 1000,
            ended_markets: EndedMarketConfig::default(),
        }
    }
}

impl SportsArbConfig {
    /// `(ticker, category)` of every scanned series.
    pub fn kalshi_series(&self) -> Vec<(&str, &str)> {
        self.series
            .iter()
            .map(|s| (s.ticker.as_str(), s.category.as_str()))
            .collect()
    }

    fn series_for(&self, kalshi_ticker: &str) -> Option<&SeriesConfig> {
        let (series, _) = kalshi_ticker.split_once('-')?;
        self.series.iter().find(|s| s.ticker == series)
    }

    /// Whether auto-trade may execute on this Kalshi market ticker.
    pub fn auto_trade_enabled_for(&self, kalshi_ticker: &str) -> bool {
        self.series_for(kalshi_ticker).is_some_and(|s| s.auto_trade)
    }

    /// Human-readable sport for a Kalshi market ticker (e.g.
    /// `KXATPCHALLENGERMATCH-...` -> "ATP Challenger"). Unknown series return
    /// an empty string so the UI shows a blank, not a guess.
    pub fn sport_label(&self, kalshi_ticker: &str) -> &str {
        self.series_for(kalshi_ticker).map_or("", |s| s.label.as_str())
    }

    pub fn executable_book_max_age(&self) -> Duration {
        Duration::from_secs(self.executable_book_max_age_secs)
    }

    pub fn polymarket_ws_max_silence(&self) -> Duration {
        Duration::from_secs(self.polymarket_ws_max_silence_secs)
    }

    pub fn poly_depth_snapshot_cooldown(&self) -> Duration {
        Duration::from_secs(self.poly_depth_snapshot_cooldown_secs)
    }

    fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.series {
            if s.ticker.is_empty() || s.ticker.contains('-') || !seen.insert(s.ticker.as_str()) {
                return Err(format!("[sports_arb] series ticker {:?} is empty, contains '-', or is repeated", s.ticker));
            }
            if !crate::clients::kalshi::PARSEABLE_CATEGORIES.contains(&s.category.as_str()) {
                return Err(format!(
                    "[sports_arb] series {} has unknown category {:?} (expected one of {:?})",
                    s.ticker,
                    s.category,
                    crate::clients::kalshi::PARSEABLE_CATEGORIES
                ));
            }
        }
        if self.market_scan_interval_secs == 0
            || self.market_scan_offsets_secs.is_empty()
            || self.market_scan_offsets_secs.iter().any(|o| *o >= self.market_scan_interval_secs)
        {
            return Err("[sports_arb] market_scan_offsets_secs must be non-empty and each below market_scan_interval_secs".into());
        }
        if self.match_window_hours <= 0
            || self.executable_book_max_age_secs == 0
            || self.polymarket_ws_max_silence_secs == 0
        {
            return Err("[sports_arb] match window, book age and WS silence must be positive".into());
        }
        if self.auto_trade_queue_check_ms == 0
            || self.ended_markets.cleanup_interval_secs == 0
            || self.ended_markets.polymarket_missing_cycles == 0
            || self.ended_markets.extreme_duration_mins <= 0
        {
            return Err("[sports_arb] queue check, cleanup interval, missing cycles and extreme duration must be positive".into());
        }
        // The cushion widens the worst price an IOC may fill at; keep it small.
        if !(0..=10).contains(&self.limit_cushion_cents) {
            return Err("[sports_arb] limit_cushion_cents must be within 0..=10".into());
        }
        Ok(())
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub kalshi: KalshiConfig,
    pub polymarket: PolymarketConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub mention: MentionConfig,
    #[serde(default)]
    pub sports_arb: SportsArbConfig,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Try multiple paths
        let config_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            // Try current directory first
            let current = std::env::current_dir()?.join(path);
            if current.exists() {
                current
            } else {
                // Try parent directory (for running from rust-backend/)
                let parent = std::env::current_dir()?.parent().map(|p| p.join(path));
                if let Some(p) = parent {
                    if p.exists() {
                        p
                    } else {
                        // Fall back to python-backend directory
                        std::env::current_dir()?
                            .parent()
                            .map(|p| p.join("python-backend").join(path))
                            .unwrap_or(current)
                    }
                } else {
                    current
                }
            }
        };

        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;

        let config: Config =
            toml::from_str(&contents).with_context(|| "Failed to parse config file")?;
        config.validate().map_err(|reason| anyhow::anyhow!("Invalid config: {reason}"))?;

        Ok(config)
    }

    /// Reject values that would break a client outright (a zero timeout or
    /// attempt count). `[mention]` is validated separately at startup so an
    /// invalid section disables only that strategy, never paired arbitrage.
    pub fn validate(&self) -> Result<(), String> {
        self.kalshi.http.validate("kalshi")?;
        self.polymarket.http.validate("polymarket")?;
        self.polymarket.ws.validate()?;
        self.sports_arb.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [kalshi]
        api_key = "k"
        api_secret = "s"
        [polymarket]
    "#;

    /// The defaults must equal the constants they replaced, so a config.toml
    /// without the new sections behaves exactly as before.
    #[test]
    fn defaults_match_the_replaced_constants() {
        let config: Config = toml::from_str(MINIMAL).unwrap();
        config.validate().unwrap();
        for http in [&config.kalshi.http, &config.polymarket.http] {
            assert_eq!(http.timeout(), Duration::from_secs(10));
            assert_eq!(http.max_attempts, 3);
        }
        assert_eq!(config.polymarket.cache.balance_ttl(), Duration::from_secs(30));
        assert_eq!(config.polymarket.cache.positions_ttl(), Duration::from_secs(30));
        assert_eq!(config.polymarket.cache.market_constraints_ttl(), Duration::from_secs(3600));
        assert_eq!(config.polymarket.ws.silence_timeout(), Duration::from_secs(30));
        assert_eq!(config.polymarket.ws.slug_refresh(), Duration::from_secs(5));
        assert_eq!(config.polymarket.ws.max_backoff(), Duration::from_secs(30));
        assert_eq!(config.discord.opportunity_cooldown_secs, 60);
        assert_eq!(config.mention.book_max_age(), Duration::from_secs(30));
        assert_eq!(config.mention.rejection_log_interval(), Duration::from_secs(900));
        assert_eq!(config.mention.signal_timeout(), Duration::from_secs(5));
    }

    /// The template must stay loadable and match the defaults it documents.
    #[test]
    fn the_example_config_parses_and_documents_the_defaults() {
        let example: Config = toml::from_str(include_str!("../../config.example.toml")).unwrap();
        example.validate().unwrap();
        example.mention.validate().unwrap();
        let minimal: Config = toml::from_str(MINIMAL).unwrap();
        assert_eq!(format!("{:?}", example.kalshi.http), format!("{:?}", minimal.kalshi.http));
        assert_eq!(format!("{:?}", example.polymarket.http), format!("{:?}", minimal.polymarket.http));
        assert_eq!(format!("{:?}", example.polymarket.cache), format!("{:?}", minimal.polymarket.cache));
        assert_eq!(format!("{:?}", example.polymarket.ws), format!("{:?}", minimal.polymarket.ws));
        assert_eq!(format!("{:?}", example.mention), format!("{:?}", minimal.mention));
        assert_eq!(example.discord.opportunity_cooldown_secs, minimal.discord.opportunity_cooldown_secs);
        assert_eq!(format!("{:?}", example.sports_arb), format!("{:?}", minimal.sports_arb));
    }

    /// Same assertions the Kalshi client carried for the hardcoded lists.
    #[test]
    fn nhl_and_mlb_are_detection_only_crypto_trades() {
        let arb = SportsArbConfig::default();
        assert!(arb.auto_trade_enabled_for("KXNBAGAME-26OCT11LALMEM-LAL"));
        assert!(arb.auto_trade_enabled_for("KXITFWMATCH-26OCT26GLOARL-GLO"));
        assert!(!arb.auto_trade_enabled_for("KXNHLGAME-26OCT27NYINSH-NYI"));
        assert!(!arb.auto_trade_enabled_for("KXMLBGAME-26OCT262000MILTB-TB"));
        assert!(arb.auto_trade_enabled_for("KXBTC15M-26OCT261945-45"));
        assert!(!arb.auto_trade_enabled_for("garbage"));
    }

    #[test]
    fn sport_label_maps_each_supported_series() {
        let arb = SportsArbConfig::default();
        assert_eq!(arb.sport_label("KXNBAGAME-26OCT11LALMEM-LAL"), "NBA");
        assert_eq!(arb.sport_label("KXATPMATCH-26OCT10MEDSVR-MED"), "ATP");
        assert_eq!(arb.sport_label("KXATPCHALLENGERMATCH-26OCT11RIBZEI-RIB"), "ATP Challenger");
        assert_eq!(arb.sport_label("KXWTAMATCH-26OCT11ANNZHE-ZHE"), "WTA");
        assert_eq!(arb.sport_label("KXITFWMATCH-26OCT11SNETHI-THI"), "ITF W");
        assert_eq!(arb.sport_label("KXITFMATCH-26OCT11AZKSIN-SIN"), "ITF M");
        assert_eq!(arb.sport_label("KXSOMETHINGNEW-1"), "");
    }

    #[test]
    fn sports_arb_defaults_match_the_replaced_constants() {
        let arb = SportsArbConfig::default();
        assert_eq!(
            arb.kalshi_series(),
            vec![
                ("KXNBAGAME", "NBA"),
                ("KXATPMATCH", "TENNIS"),
                ("KXWTAMATCH", "TENNIS"),
                ("KXITFMATCH", "TENNIS"),
                ("KXITFWMATCH", "TENNIS"),
                ("KXATPCHALLENGERMATCH", "TENNIS"),
                ("KXNHLGAME", "NHL"),
                ("KXMLBGAME", "MLB"),
                ("KXBTC15M", "CRYPTO15"),
                ("KXETH15M", "CRYPTO15"),
                ("KXSOL15M", "CRYPTO15"),
            ]
        );
        assert_eq!(arb.market_scan_interval_secs, 300);
        assert_eq!(arb.market_scan_offsets_secs, vec![40, 100]);
        assert_eq!(arb.match_window_hours, 24);
        assert_eq!(arb.executable_book_max_age(), Duration::from_secs(30));
        assert_eq!(arb.polymarket_ws_max_silence(), Duration::from_secs(15));
        assert_eq!(arb.limit_cushion_cents, 1);
        assert_eq!(arb.poly_depth_snapshot_cooldown(), Duration::from_secs(600));
        assert_eq!(arb.initial_scan_delay_secs, 30);
        assert_eq!(arb.auto_trade_queue_check_ms, 200);
        assert_eq!(arb.auto_trade_pause_ms, 1000);
        let ended = &arb.ended_markets;
        assert_eq!((ended.kalshi_extreme_high, ended.kalshi_extreme_low), (0.99, 0.02));
        assert_eq!((ended.poly_extreme_high, ended.poly_extreme_low), (1.00, 0.00));
        assert_eq!(ended.extreme_duration_mins, 20);
        assert_eq!(ended.polymarket_missing_cycles, 2);
        assert_eq!(ended.cleanup_interval_secs, 60);
        arb.validate().unwrap();
    }

    #[test]
    fn invalid_sports_arb_settings_are_rejected() {
        for bad in [
            "[sports_arb]\nlimit_cushion_cents = 11",
            "[sports_arb]\nlimit_cushion_cents = -1",
            "[sports_arb]\nmarket_scan_offsets_secs = [300]",
            "[sports_arb]\nexecutable_book_max_age_secs = 0",
            "[[sports_arb.series]]\nticker = \"KXFOO\"\ncategory = \"SOCCER\"\nlabel = \"x\"",
            "[[sports_arb.series]]\nticker = \"KXA\"\ncategory = \"NBA\"\nlabel = \"a\"\n[[sports_arb.series]]\nticker = \"KXA\"\ncategory = \"NBA\"\nlabel = \"b\"",
        ] {
            let config: Config = toml::from_str(&format!("{MINIMAL}\n{bad}")).unwrap();
            assert!(config.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_configured_series_list_replaces_the_default_and_defaults_auto_trade_off() {
        let config: Config = toml::from_str(&format!(
            "{MINIMAL}\n[[sports_arb.series]]\nticker = \"KXNBAGAME\"\ncategory = \"NBA\"\nlabel = \"NBA\""
        ))
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.sports_arb.series.len(), 1);
        assert!(!config.sports_arb.auto_trade_enabled_for("KXNBAGAME-26OCT11LALMEM-LAL"));
    }

    #[test]
    fn a_zero_timeout_or_attempt_count_is_rejected() {
        for bad in [
            "[kalshi.http]\ntimeout_secs = 0",
            "[polymarket.http]\nmax_attempts = 0",
            "[polymarket.ws]\nsilence_timeout_secs = 0",
        ] {
            let config: Config = toml::from_str(&format!("{MINIMAL}\n{bad}")).unwrap();
            assert!(config.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_partial_section_keeps_the_other_defaults() {
        let config: Config =
            toml::from_str(&format!("{MINIMAL}\n[polymarket.cache]\nbalance_ttl_secs = 5")).unwrap();
        assert_eq!(config.polymarket.cache.balance_ttl(), Duration::from_secs(5));
        assert_eq!(config.polymarket.cache.positions_ttl(), Duration::from_secs(30));
    }
}
