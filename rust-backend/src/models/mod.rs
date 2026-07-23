//! Data models
//!
//! Core concepts:
//! - Kalshi: One event has 2 independent markets, each for one team's Yes/No
//! - Polymarket: One event has 1 market with 2 outcomes (two teams)
//!
//! Matching relationship:
//! - Both Kalshi markets point to the same Poly market
//! - Team name determines which Poly price perspective to use

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::services::metrics::MetricsReport;

/// Generate a unified market key including date for accurate identification
/// Format: EVENT-NAME_YYYY-MM-DD_TEAM-NAME (e.g., PHI-TOR_2026-01-11_PHI)
pub fn generate_market_key(
    event_name: &str,
    game_date: Option<NaiveDate>,
    team_name: &str,
) -> String {
    match game_date {
        Some(date) => format!(
            "{}_{}_{}", 
            event_name.to_uppercase(), 
            date.format("%Y-%m-%d"), 
            team_name.to_uppercase()
        ),
        None => format!(
            "{}_{}", 
            event_name.to_uppercase(), 
            team_name.to_uppercase()
        ),
    }
}

/// Platform enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Kalshi,
    Polymarket,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Kalshi => write!(f, "kalshi"),
            Platform::Polymarket => write!(f, "polymarket"),
        }
    }
}

/// Kalshi market model
///
/// Each Kalshi market is independent, predicting a single team's win/lose
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalshiMarket {
    /// Market ticker (e.g., KXNBAGAME-26JAN04MEMLAL-MEM)
    pub market_id: String,
    /// Event ID this market belongs to
    pub event_id: String,
    /// Standardized event name (e.g., MEM-LAL)
    pub event_name: String,
    /// Team this market predicts (e.g., MEM)
    pub team_name: String,
    /// Opponent team (e.g., LAL)
    pub opponent_name: String,
    /// Yes price (this team wins)
    pub yes_price: f64,
    /// No price (this team doesn't win)
    pub no_price: f64,
    /// Game start time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    /// Trading volume
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    /// Liquidity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity: Option<f64>,
}

/// Polymarket market model
///
/// A Polymarket market contains two outcomes (two teams)
/// Not split, maintains original structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketMarket {
    /// Condition ID
    pub market_id: String,
    /// Standardized event name (e.g., MEM-LAL)
    pub event_name: String,
    /// Team A (alphabetically first)
    pub team_a: String,
    /// Team B (alphabetically second)
    pub team_b: String,
    /// Team A win price
    pub price_a: f64,
    /// Team B win price
    pub price_b: f64,
    /// Game start time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    /// Trading volume
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,

    /// WebSocket subscription info
    /// Team A's token ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_a: Option<String>,
    /// Team B's token ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_b: Option<String>,
}

impl PolymarketMarket {
    /// Get Yes/No prices for a given team
    ///
    /// Returns: (yes_price, no_price) for that team
    pub fn get_price_for_team(&self, team: &str) -> Result<(f64, f64), String> {
        let team_upper = team.to_uppercase();
        if team_upper == self.team_a.to_uppercase() {
            // yes = A wins, no = B wins
            Ok((self.price_a, self.price_b))
        } else if team_upper == self.team_b.to_uppercase() {
            // yes = B wins, no = A wins
            Ok((self.price_b, self.price_a))
        } else {
            Err(format!(
                "Team {} not in market {}",
                team, self.event_name
            ))
        }
    }

    /// Get token ID for a given team
    pub fn get_token_for_team(&self, team: &str) -> Option<&str> {
        let team_upper = team.to_uppercase();
        if team_upper == self.team_a.to_uppercase() {
            self.token_id_a.as_deref()
        } else if team_upper == self.team_b.to_uppercase() {
            self.token_id_b.as_deref()
        } else {
            None
        }
    }

    /// Get opponent team name
    pub fn get_opponent(&self, team: &str) -> Option<&str> {
        let team_upper = team.to_uppercase();
        if team_upper == self.team_a.to_uppercase() {
            Some(&self.team_b)
        } else if team_upper == self.team_b.to_uppercase() {
            Some(&self.team_a)
        } else {
            None
        }
    }
}

/// Kalshi event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalshiEvent {
    pub event_id: String,
    /// Standardized event name (e.g., MEM-LAL)
    pub name: String,
    pub team_a: String,
    pub team_b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default = "default_category")]
    pub category: String,
    /// Markets in this event
    #[serde(default)]
    pub markets: Vec<KalshiMarket>,
}

fn default_category() -> String {
    "NBA".to_string()
}

impl KalshiEvent {
    /// Get market by team name
    pub fn get_market_by_team(&self, team: &str) -> Option<&KalshiMarket> {
        let team_upper = team.to_uppercase();
        self.markets
            .iter()
            .find(|m| m.team_name.to_uppercase() == team_upper)
    }
}

/// Polymarket event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketEvent {
    pub event_id: String,
    /// Standardized event name (e.g., MEM-LAL)
    pub name: String,
    pub team_a: String,
    pub team_b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default = "default_category")]
    pub category: String,
    /// Single market for this event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<PolymarketMarket>,
}

/// Matched market pair - used for arbitrage calculation
///
/// One Kalshi market corresponds to one Poly market (from a certain perspective)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedMarket {
    pub event_name: String,
    /// Team that Kalshi market predicts
    pub team_name: String,
    /// Game date (for distinguishing same-team games on different dates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_date: Option<NaiveDate>,

    /// Kalshi side
    pub kalshi_market: KalshiMarket,

    /// Polymarket side (not split, references original market)
    pub polymarket_market: PolymarketMarket,

    /// Cached Poly prices (calculated based on team_name)
    /// Yes price for this team
    pub poly_yes_price: f64,
    /// No price for this team (opponent wins)
    pub poly_no_price: f64,

    /// Match confidence
    #[serde(default)]
    pub confidence: f64,
}

impl MatchedMarket {
    /// Update Poly prices based on team_name
    pub fn update_poly_prices(&mut self) {
        if let Ok((yes, no)) = self.polymarket_market.get_price_for_team(&self.team_name) {
            self.poly_yes_price = yes;
            self.poly_no_price = no;
        }
    }

    /// Generate unique market key including date
    pub fn market_key(&self) -> String {
        generate_market_key(&self.event_name, self.game_date, &self.team_name)
    }
}

/// Matched event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedEvent {
    pub event_name: String,
    /// Game date (for distinguishing same-team games on different dates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_event: Option<KalshiEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymarket_event: Option<PolymarketEvent>,
    #[serde(default)]
    pub confidence: f64,
}

/// Price update from WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub platform: Platform,
    /// Kalshi: ticker, Polymarket: token_id
    pub market_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_bid: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_ask: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_bid: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_ask: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

/// Arbitrage opportunity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub event_name: String,
    pub team_name: String,
    /// Game date (for distinguishing same-team games on different dates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_date: Option<NaiveDate>,

    // Kalshi side
    pub kalshi_market_id: String,
    /// Price used in strategy
    pub kalshi_price: f64,
    /// "yes" or "no"
    pub kalshi_side: String,
    pub kalshi_bet: f64,
    /// Display: Yes price
    #[serde(default)]
    pub kalshi_yes_price: f64,
    /// Display: No price
    #[serde(default)]
    pub kalshi_no_price: f64,
    /// Kalshi contract count
    #[serde(default)]
    pub kalshi_contracts: f64,
    /// Kalshi trading fee
    #[serde(default)]
    pub kalshi_fee: f64,

    // Polymarket side
    pub polymarket_market_id: String,
    /// Price used in strategy
    pub polymarket_price: f64,
    /// "yes" or "no"
    pub polymarket_side: String,
    pub polymarket_bet: f64,
    /// Display: Yes price (own ask)
    #[serde(default)]
    pub polymarket_yes_price: f64,
    /// Display: No price (opponent's ask)
    #[serde(default)]
    pub polymarket_no_price: f64,

    // Arbitrage info
    pub total_bet: f64,
    pub profit_margin: f64,
    pub expected_profit: f64,
    /// Gross profit before fees
    #[serde(default)]
    pub gross_profit: f64,
    pub timestamp: DateTime<Utc>,
    /// Game start time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,

    // Depth info for liquidity tracking
    /// Polymarket ask-side depth (USD = price * size)
    #[serde(default)]
    pub poly_ask_depth: f64,
    /// Polymarket ask-side quantity (token count)
    #[serde(default)]
    pub poly_ask_size: f64,
    /// Kalshi ask-side depth (contracts)
    #[serde(default)]
    pub kalshi_ask_depth: i32,
}

impl ArbitrageOpportunity {
    /// Generate unique market key including date
    pub fn market_key(&self) -> String {
        generate_market_key(&self.event_name, self.game_date, &self.team_name)
    }
}

/// System statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemStats {
    pub total_kalshi_events: usize,
    pub total_kalshi_markets: usize,
    pub total_polymarket_events: usize,
    /// After not splitting, equals event count
    pub total_polymarket_markets: usize,
    pub matched_events: usize,
    /// Kalshi market count (each has corresponding Poly perspective)
    pub matched_markets: usize,
    pub arbitrage_opportunities: usize,
    pub kalshi_ws_connected: bool,
    pub polymarket_ws_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<DateTime<Utc>>,
}

/// Arbitrage tracking record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageTrackingRecord {
    pub id: String,
    pub event_name: String,
    pub team_name: String,
    /// Game date (for distinguishing same-team games on different dates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_date: Option<NaiveDate>,
    pub kalshi_market_id: String,
    pub polymarket_market_id: String,
    pub start_time: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
    pub initial_profit_margin: f64,
    pub max_profit_margin: f64,
    pub kalshi_side: String,
    pub polymarket_side: String,
    #[serde(default)]
    pub update_count: u64,
    /// Polymarket ask-side depth (USD = price * size)
    #[serde(default)]
    pub poly_ask_depth: f64,
    /// Polymarket ask-side quantity (token count)
    #[serde(default)]
    pub poly_ask_size: f64,
    /// Kalshi ask-side depth (contracts)
    #[serde(default)]
    pub kalshi_ask_depth: i32,
    /// Duration (milliseconds)
    #[serde(default)]
    pub duration_ms: i64,
    /// Kalshi ask price (on the side used for arbitrage)
    #[serde(default)]
    pub kalshi_ask_price: f64,
    /// Polymarket ask price (on the side used for arbitrage)
    #[serde(default)]
    pub polymarket_ask_price: f64,
}

impl ArbitrageTrackingRecord {
    /// Generate unique market key including date
    pub fn market_key(&self) -> String {
        generate_market_key(&self.event_name, self.game_date, &self.team_name)
    }
}

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Order request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub platform: Platform,
    pub market_id: String,
    pub side: OrderSide,
    /// "yes" or "no"
    pub outcome: String,
    pub amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
}

/// Order response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_price: Option<f64>,
}

/// Matched market data for frontend display
/// Corresponds to frontend's MatchedMarketData type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedMarketFrontend {
    pub event_name: String,
    pub team_name: String,
    /// Game date (for distinguishing same-team games on different dates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_date: Option<String>,
    pub kalshi_market_id: String,
    pub polymarket_market_id: String,
    /// Polymarket token_id (asset_id) for Yes orderbook lookup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poly_token_id: Option<String>,
    /// Polymarket opponent token_id for No orderbook lookup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poly_opponent_token_id: Option<String>,
    pub kalshi_yes_price: f64,
    pub kalshi_no_price: f64,
    pub poly_yes_price: f64,
    pub poly_no_price: f64,
    pub kalshi_ready: bool,
    pub poly_ready: bool,
    pub both_ready: bool,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    // Arbitrage related fields
    pub has_opportunity: bool,
    pub profit_margin: f64,
    pub expected_profit: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gross_profit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_contracts: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalshi_fee: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arbitrage_type: Option<String>,
}

/// Matched markets list message with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedMarketsListData {
    pub markets: Vec<MatchedMarketFrontend>,
    pub count: usize,
    pub opportunities_count: usize,
}

/// Market scan statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    /// Number of scans performed
    pub scan_count: u64,
    /// Number of new markets found in this scan
    pub new_markets_found: usize,
    /// Total matched markets after this scan
    pub total_matched_markets: usize,
    /// Timestamp of this scan
    pub timestamp: DateTime<Utc>,
}

/// WebSocket message types for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    #[serde(rename = "opportunity")]
    Opportunity { data: ArbitrageOpportunity },
    #[serde(rename = "opportunities")]
    Opportunities { data: Vec<ArbitrageOpportunity> },
    #[serde(rename = "stats")]
    Stats { data: SystemStats },
    #[serde(rename = "matched_markets_list")]
    MatchedMarketsList {
        data: Vec<MatchedMarketFrontend>,
        count: usize,
        opportunities_count: usize,
    },
    #[serde(rename = "metrics")]
    Metrics { data: MetricsReport },
    #[serde(rename = "scan_stats")]
    ScanStats { data: ScanStats },
    #[serde(rename = "log")]
    Log { level: String, message: String },
    #[serde(rename = "price_update")]
    PriceUpdate { data: PriceUpdate },
    #[serde(rename = "error")]
    Error { message: String },
}
