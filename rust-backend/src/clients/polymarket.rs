//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from the Polymarket US API
//! - Order placement via Python order service

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::config::PolymarketConfig;
use crate::core::{competitor_event_name, normalize_competitor_name, normalize_team_name};
use crate::models::{PolymarketEvent, PolymarketMarket, PolymarketPositionSide};

// Polymarket US sport codes for the supported tennis match-winner feeds.
// Series IDs are resolved from /v1/sports at runtime so they are not baked in.
const TENNIS_SPORT_CODES: &[&str] = &["atp", "wta", "itfm", "itfw", "itfme", "itfwo", "atpcq"];

fn extract_balance_from_snapshot(snapshot: &Value) -> Option<f64> {
    let balances = snapshot.get("balances")?;
    let mut total = 0.0;

    match balances {
        Value::Array(entries) => {
            for entry in entries {
                if let Some(value) = extract_balance_value(entry) {
                    total += value;
                }
            }
        }
        Value::Object(_) => {
            if let Some(value) = extract_balance_value(balances) {
                total += value;
            }
        }
        _ => {}
    }

    if total > 0.0 {
        Some(total)
    } else {
        None
    }
}

fn extract_balance_value(entry: &Value) -> Option<f64> {
    if let Some(obj) = entry.as_object() {
        for key in ["available", "balance", "amount", "value", "usdc_balance"] {
            if let Some(value) = obj.get(key) {
                if let Some(number) = value.as_f64() {
                    return Some(number);
                }
                if let Some(string) = value.as_str() {
                    return string.parse::<f64>().ok();
                }
            }
        }
    }

    None
}

// ==================== Python Order Service Types ====================

/// Limit order request to Python service
#[derive(Debug, Serialize)]
struct LimitOrderRequest {
    market_slug: String,
    position_side: String,
    side: String,
    price: f64,
    size: f64,
    order_type: Option<String>,
}

/// Cancel order request to Python service
#[derive(Debug, Serialize)]
struct CancelOrderRequest {
    order_id: String,
}

/// Order response from Python service
#[derive(Debug, Deserialize)]
struct OrderResponse {
    success: bool,
    order_id: Option<String>,
    error: Option<String>,
    data: Option<Value>,
}

/// Position data for aggregation (internal use)
#[allow(dead_code)]
struct PositionData {
    asset_id: String,
    market: String,
    outcome: String,
    size: f64,
    total_cost: f64,
    trade_count: u32,
}

/// Polymarket API client
#[derive(Clone)]
pub struct PolymarketClient {
    pub config: PolymarketConfig,
    http: Client,
}

impl PolymarketClient {
    /// Create a new Polymarket client
    pub fn new(config: PolymarketConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    /// Initialize client (check if Python order service is available)
    pub async fn init_order_service(&mut self) -> Result<()> {
        // Check if Python order service is running
        let health_url = format!("{}/health", self.config.order_service_url);
        match self.http.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    "✅ Polymarket Python order service connected: {}",
                    self.config.order_service_url
                );
            }
            Ok(resp) => {
                warn!(
                    "⚠️ Polymarket Python order service returned a non-success status: {}",
                    resp.status()
                );
            }
            Err(e) => {
                warn!("⚠️ Polymarket Python order service is not running: {}. Order placement will be unavailable.", e);
            }
        }
        Ok(())
    }

    /// Get account balance via Python service using the newer account/portfolio/orders snapshot format.
    pub async fn get_balance(&self) -> Result<f64> {
        let base_url = self.config.order_service_url.trim_end_matches('/');
        let url = format!("{}/account/snapshot", base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to reach the Python order service")?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to get balance: HTTP {}", resp.status());
        }

        let data: Value = resp.json().await?;
        if data
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let balance = data
                .get("balance")
                .and_then(|v| v.as_f64())
                .or_else(|| {
                    data.get("snapshot")
                        .and_then(|snapshot| extract_balance_from_snapshot(snapshot))
                })
                .unwrap_or(0.0);
            Ok(balance)
        } else {
            let error = data
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Failed to get balance: {}", error)
        }
    }

    /// Get all supported NBA and tennis match-winner events and markets.
    pub async fn get_supported_events_and_markets(
        &self,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let sports = self.get_sports().await?;
        let mut events = Vec::new();
        let mut markets = Vec::new();
        let mut fetched_series = HashSet::new();

        let nba_sport = sports
            .iter()
            .find(|sport| {
                let name = sport_code(sport).unwrap_or_default().to_ascii_uppercase();
                name.contains("NBA") && !name.contains("WNBA")
            })
            .ok_or_else(|| anyhow::anyhow!("NBA league not found"))?;
        let nba_series =
            sport_series_id(nba_sport).ok_or_else(|| anyhow::anyhow!("NBA series_id not found"))?;
        self.append_series_events_and_markets(
            &nba_series,
            "NBA",
            &mut fetched_series,
            &mut events,
            &mut markets,
        )
        .await?;

        for sport_code_name in TENNIS_SPORT_CODES {
            let Some(sport) = sports
                .iter()
                .find(|sport| sport_has_code(sport, sport_code_name))
            else {
                warn!(
                    "Polymarket US tennis sport code {} is unavailable; skipping it",
                    sport_code_name
                );
                continue;
            };

            let Some(series_id) = sport_series_id(sport) else {
                warn!(
                    "Polymarket US tennis sport code {} has no series ID; skipping it",
                    sport_code_name
                );
                continue;
            };

            self.append_series_events_and_markets(
                &series_id,
                "TENNIS",
                &mut fetched_series,
                &mut events,
                &mut markets,
            )
            .await?;
        }

        info!(
            "✅ Polymarket supported sports: {} events, {} markets",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Fetch only the legacy NBA feed for callers that do not use the
    /// supported-sports scan.
    #[allow(dead_code)]
    pub async fn get_nba_events_and_markets(
        &self,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let sports = self.get_sports().await?;
        let nba_sport = sports
            .iter()
            .find(|sport| {
                let name = sport_code(sport).unwrap_or_default().to_ascii_uppercase();
                name.contains("NBA") && !name.contains("WNBA")
            })
            .ok_or_else(|| anyhow::anyhow!("NBA league not found"))?;
        let nba_series =
            sport_series_id(nba_sport).ok_or_else(|| anyhow::anyhow!("NBA series_id not found"))?;
        self.get_series_events_and_markets(&nba_series, "NBA").await
    }

    async fn get_sports(&self) -> Result<Vec<Value>> {
        let sports_url = format!("{}/v1/sports", self.config.base_url);
        let response = self.http.get(&sports_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to get sports leagues: {}", response.status());
        }

        response
            .json::<Value>()
            .await?
            .get("sports")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Polymarket US sports response is missing sports"))
    }

    async fn append_series_events_and_markets(
        &self,
        series_id: &str,
        category: &str,
        fetched_series: &mut HashSet<String>,
        events: &mut Vec<PolymarketEvent>,
        markets: &mut Vec<PolymarketMarket>,
    ) -> Result<()> {
        if !fetched_series.insert(series_id.to_string()) {
            return Ok(());
        }

        let (mut series_events, mut series_markets) = self
            .get_series_events_and_markets(series_id, category)
            .await?;
        events.append(&mut series_events);
        markets.append(&mut series_markets);
        Ok(())
    }

    async fn get_series_events_and_markets(
        &self,
        series_id: &str,
        category: &str,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let events_url = format!(
            "{}/v1/events?seriesId={}&active=true&closed=false&limit=100",
            self.config.base_url, series_id
        );
        let response = self.http.get(&events_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to get Polymarket {} events: {}",
                category,
                response.status()
            );
        }

        let api_events: Vec<Value> = response
            .json::<Value>()
            .await?
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Polymarket US events response is missing events"))?;
        info!(
            "📥 Retrieved {} Polymarket {} events",
            api_events.len(),
            category
        );

        let mut events = Vec::new();
        let mut markets = Vec::new();
        for api_event in &api_events {
            let event_title = api_event["title"].as_str().unwrap_or("");
            let event_date = extract_date_from_slug(api_event["slug"].as_str().unwrap_or(""));

            for market_data in api_event["markets"].as_array().into_iter().flatten() {
                let parsed = match category {
                    "NBA" => parse_nba_market(event_title, event_date, market_data),
                    "TENNIS" => parse_tennis_match_winner_market(event_date, market_data),
                    _ => None,
                };
                if let Some((event, market)) = parsed {
                    events.push(event);
                    markets.push(market);
                }
            }
        }

        Ok((events, markets))
    }

    // ========================================================================
    // ORDER PLACEMENT API (via Python service)
    // ========================================================================

    /// Place a Polymarket US limit order for a whole number of contracts.
    pub async fn place_limit_order(
        &self,
        market_slug: &str,
        position_side: PolymarketPositionSide,
        side: &str,
        contracts: i32,
        price: f64,
    ) -> Result<Value> {
        let side = side.to_ascii_lowercase();
        if market_slug.trim().is_empty()
            || contracts <= 0
            || !price.is_finite()
            || !(0.0..1.0).contains(&price)
            || !matches!(side.as_str(), "buy" | "sell")
        {
            anyhow::bail!("Polymarket US limit order requires a slug, positive whole contract count, limit price, and buy or sell action");
        }

        let size = contracts as f64;

        let url = format!("{}/order/limit", self.config.order_service_url);
        let request = LimitOrderRequest {
            market_slug: market_slug.to_string(),
            position_side: position_side.as_str().to_string(),
            side,
            price,
            size,
            order_type: Some("GTC".to_string()),
        };
        let resp = self
            .http
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to call the Python order service")?;
        let response: OrderResponse = resp.json().await?;

        if response.success {
            Ok(response
                .data
                .unwrap_or(json!({"success": true, "order_id": response.order_id})))
        } else {
            anyhow::bail!(
                "Polymarket US market order failed: {}",
                response
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string())
            )
        }
    }

    /// Get open orders via Python service
    pub async fn get_open_orders(&self) -> Result<Value> {
        let url = format!("{}/orders", self.config.order_service_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to get order: HTTP {}", resp.status());
        }

        let data: Value = resp.json().await?;
        if data
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            Ok(data.get("orders").cloned().unwrap_or(json!([])))
        } else {
            let error = data
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Failed to get order: {}", error)
        }
    }

    /// Get positions (placeholder - returns empty for now)
    pub async fn get_positions(&self) -> Result<Value> {
        let url = format!("{}/positions", self.config.order_service_url);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to reach the Python order service")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to get positions: HTTP {}", response.status());
        }

        let data: Value = response.json().await?;
        if data
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            Ok(data.get("positions").cloned().unwrap_or_else(|| json!([])))
        } else {
            let error = data
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Failed to get positions: {}", error)
        }
    }

    /// Cancel an order via Python service
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let url = format!("{}/order/cancel", self.config.order_service_url);
        let request = CancelOrderRequest {
            order_id: order_id.to_string(),
        };

        let resp = self
            .http
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("调用 Python 下单服务失败")?;

        let response: OrderResponse = resp.json().await?;

        if response.success {
            Ok(json!({"success": true, "order_id": order_id}))
        } else {
            let error = response
                .error
                .unwrap_or_else(|| "Unknown error".to_string());
            anyhow::bail!("Failed to cancel order: {}", error)
        }
    }

}

fn sport_code(sport: &Value) -> Option<&str> {
    ["sport", "code", "slug"]
        .iter()
        .find_map(|field| sport[*field].as_str())
}

fn sport_has_code(sport: &Value, expected_code: &str) -> bool {
    ["sport", "code", "slug"]
        .iter()
        .filter_map(|field| sport[*field].as_str())
        .any(|value| value.eq_ignore_ascii_case(expected_code))
}

fn sport_series_id(sport: &Value) -> Option<String> {
    ["series", "seriesId", "series_id"]
        .iter()
        .find_map(|field| match &sport[*field] {
            Value::String(value) if !value.is_empty() => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

fn parse_nba_market(
    event_title: &str,
    event_date: Option<DateTime<Utc>>,
    market_data: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    let question = market_data["question"].as_str().unwrap_or(event_title);
    if question != event_title {
        return None;
    }

    let (outcomes, sides) =
        parse_binary_outcomes_and_market_sides(market_data, |outcome| {
            if matches!(
                outcome.trim().to_ascii_lowercase().as_str(),
                "yes" | "no" | "over" | "under"
            ) {
                return None;
            }
            let normalized = normalize_team_name(outcome);
            (!normalized.is_empty()).then_some(normalized)
        })?;
    let team_a = outcomes[0].clone();
    let team_b = outcomes[1].clone();
    if team_a == team_b {
        return None;
    }

    let event_name = if team_a < team_b {
        format!("{team_a}-{team_b}")
    } else {
        format!("{team_b}-{team_a}")
    };
    build_competitor_market(
        market_data,
        event_name,
        team_a,
        team_b,
        sides[0],
        sides[1],
        event_date,
        "NBA",
    )
}

fn parse_tennis_match_winner_market(
    event_date: Option<DateTime<Utc>>,
    market_data: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    if market_data["sportsMarketType"].as_str() != Some("tennis_match_winner")
        || market_data["sportsMarketTypeV2"].as_str() != Some("SPORTS_MARKET_TYPE_MONEYLINE")
    {
        return None;
    }

    let (outcomes, sides) =
        parse_binary_outcomes_and_market_sides(market_data, normalize_competitor_name)?;
    let competitor_a = outcomes[0].clone();
    let competitor_b = outcomes[1].clone();
    let event_name = competitor_event_name(&competitor_a, &competitor_b)?;

    build_competitor_market(
        market_data,
        event_name,
        competitor_a,
        competitor_b,
        sides[0],
        sides[1],
        event_date,
        "TENNIS",
    )
}

#[derive(Debug, Clone, Copy)]
struct MarketSide {
    quote: f64,
    position_side: PolymarketPositionSide,
}

/// Parse a binary market and associate every competitor with its own gateway
/// market side. The outcome array supplies competitor names only; it never
/// determines a LONG or SHORT position.
fn parse_binary_outcomes_and_market_sides<F>(
    market_data: &Value,
    normalizer: F,
) -> Option<([String; 2], [MarketSide; 2])>
where
    F: Fn(&str) -> Option<String>,
{
    let outcomes = parse_json_array(&market_data["outcomes"])?
        .iter()
        .map(|value| value.as_str().and_then(|name| normalizer(name)))
        .collect::<Option<Vec<_>>>()?;
    let outcomes: [String; 2] = outcomes.try_into().ok()?;
    if outcomes[0] == outcomes[1] {
        return None;
    }

    let market_sides = market_data["marketSides"].as_array()?;
    let mut outcome_sides: [Option<MarketSide>; 2] = [None, None];
    for market_side in market_sides {
        let description = market_side["description"].as_str()?;
        let Some(competitor) = normalizer(description) else {
            continue;
        };
        let outcome_index = if competitor == outcomes[0] {
            0
        } else if competitor == outcomes[1] {
            1
        } else {
            continue;
        };

        let quote = market_side["quote"]["value"]
            .as_str()
            .and_then(|price| price.parse::<f64>().ok())
            .or_else(|| market_side["quote"]["value"].as_f64())?;
        if !quote.is_finite() || !(0.01..=0.99).contains(&quote) {
            return None;
        }
        let position_side = if market_side["long"].as_bool()? {
            PolymarketPositionSide::Long
        } else {
            PolymarketPositionSide::Short
        };

        // A competitor must map to exactly one side. Continuing would make
        // execution ambiguous and could choose the wrong position.
        if outcome_sides[outcome_index].is_some() {
            return None;
        }
        outcome_sides[outcome_index] = Some(MarketSide {
            quote,
            position_side,
        });
    }

    Some((outcomes, [outcome_sides[0]?, outcome_sides[1]?]))
}

fn parse_json_array(value: &Value) -> Option<Vec<Value>> {
    if let Some(serialized) = value.as_str() {
        serde_json::from_str(serialized).ok()
    } else {
        value.as_array().cloned()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_competitor_market(
    market_data: &Value,
    event_name: String,
    first_competitor: String,
    second_competitor: String,
    first_side: MarketSide,
    second_side: MarketSide,
    start_time: Option<DateTime<Utc>>,
    category: &str,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    if first_competitor == second_competitor {
        return None;
    }

    let market_id = market_data["id"].as_str().unwrap_or("");
    let condition_id = market_data["conditionId"]
        .as_str()
        .or_else(|| market_data["condition_id"].as_str())
        .unwrap_or(market_id)
        .to_string();
    if condition_id.is_empty() {
        return None;
    }

    let market_slug = market_data["slug"].as_str()?.trim();
    if market_slug.is_empty() {
        return None;
    }

    let volume = market_data["volume"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| market_data["volume"].as_f64());

    let (team_a, team_b, side_a, side_b) =
        if first_competitor < second_competitor {
            (
                first_competitor,
                second_competitor,
                first_side,
                second_side,
            )
        } else {
            (
                second_competitor,
                first_competitor,
                second_side,
                first_side,
            )
        };

    let market = PolymarketMarket {
        market_id: condition_id.clone(),
        market_slug: market_slug.to_string(),
        event_name: event_name.clone(),
        team_a: team_a.clone(),
        team_b: team_b.clone(),
        price_a: side_a.quote,
        price_b: side_b.quote,
        team_a_position: side_a.position_side,
        team_b_position: side_b.position_side,
        start_time,
        volume,
    };
    let event = PolymarketEvent {
        event_id: condition_id,
        name: event_name,
        team_a,
        team_b,
        start_time,
        category: category.to_string(),
        market: Some(market.clone()),
    };

    Some((event, market))
}

/// Extract date from slug (e.g., "lakers-vs-grizzlies-2026-01-07" -> 2026-01-07)
fn extract_date_from_slug(slug: &str) -> Option<DateTime<Utc>> {
    let parts: Vec<&str> = slug.split('-').collect();
    if parts.len() >= 3 {
        let year_str = parts[parts.len() - 3];
        let month_str = parts[parts.len() - 2];
        let day_str = parts[parts.len() - 1];

        if let (Ok(year), Ok(month), Ok(day)) = (
            year_str.parse::<i32>(),
            month_str.parse::<u32>(),
            day_str.parse::<u32>(),
        ) {
            use chrono::NaiveDate;
            if let Some(naive_date) = NaiveDate::from_ymd_opt(year, month, day) {
                let naive_datetime = naive_date.and_hms_opt(12, 0, 0)?;
                return Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_tennis_match_winner_outcomes() {
        let market = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-cerundolo-auger-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Juan Manuel Cerundolo\", \"Felix Auger-Aliassime\"]",
            "marketSides": [
                {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}},
                {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}}
            ]
        });

        let (event, parsed_market) = parse_tennis_match_winner_market(None, &market).unwrap();
        assert_eq!(event.name, "FELIX AUGER ALIASSIME VS JUAN MANUEL CERUNDOLO");
        assert_eq!(parsed_market.team_a, "FELIX AUGER ALIASSIME");
        assert_eq!(parsed_market.price_a, 0.56);
        assert_eq!(parsed_market.market_slug, "atp-cerundolo-auger-2026-08-17");
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Felix Auger-Aliassime")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Long
        );
    }

    #[test]
    fn outcome_order_does_not_determine_position_side() {
        let market = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-auger-cerundolo-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Felix Auger-Aliassime\", \"Juan Manuel Cerundolo\"]",
            "marketSides": [
                {"description": "Juan Manuel Cerundolo", "long": true, "quote": {"value": "0.46"}},
                {"description": "Felix Auger-Aliassime", "long": false, "quote": {"value": "0.56"}}
            ]
        });

        let (_, parsed_market) = parse_tennis_match_winner_market(None, &market).unwrap();
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Felix Auger-Aliassime")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Short
        );
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Juan Manuel Cerundolo")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Long
        );
    }

    #[test]
    fn rejects_missing_ambiguous_or_slugless_side_mappings() {
        let base = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-auger-cerundolo-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Felix Auger-Aliassime\", \"Juan Manuel Cerundolo\"]",
            "marketSides": [
                {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}},
                {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}}
            ]
        });

        let mut missing_slug = base.clone();
        missing_slug.as_object_mut().unwrap().remove("slug");
        assert!(parse_tennis_match_winner_market(None, &missing_slug).is_none());

        let mut missing_side = base.clone();
        missing_side["marketSides"] = json!([
            {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}}
        ]);
        assert!(parse_tennis_match_winner_market(None, &missing_side).is_none());

        let mut ambiguous = base;
        ambiguous["marketSides"] = json!([
            {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}},
            {"description": "Felix Auger-Aliassime", "long": false, "quote": {"value": "0.56"}},
            {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}}
        ]);
        assert!(parse_tennis_match_winner_market(None, &ambiguous).is_none());
    }

    #[test]
    fn rejects_non_moneyline_tennis_markets() {
        let market = json!({
            "sportsMarketType": "tennis_tournament_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_FUTURES",
            "outcomes": "[\"Juan Manuel Cerundolo\", \"Felix Auger-Aliassime\"]",
            "outcomePrices": "[\"0.45\", \"0.55\"]"
        });

        assert!(parse_tennis_match_winner_market(None, &market).is_none());
    }
}
