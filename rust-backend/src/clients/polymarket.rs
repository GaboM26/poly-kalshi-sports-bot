//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from Gamma API
//! - WebSocket price subscription
//! - Order placement via CLOB client

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::clob::{ApiCreds, ClobClient, MarketOrderArgs, Side, SignatureType};
use crate::config::PolymarketConfig;
use crate::models::{Platform, PolymarketEvent, PolymarketMarket, PriceUpdate};

const POLY_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

/// Polymarket API client
#[derive(Clone)]
pub struct PolymarketClient {
    pub config: PolymarketConfig,
    http: Client,
    /// CLOB client for order operations
    clob: Option<Arc<ClobClient>>,
}

impl PolymarketClient {
    /// Create a new Polymarket client
    pub fn new(config: PolymarketConfig) -> Self {
        Self {
            config,
            http: Client::new(),
            clob: None,
        }
    }

    /// Initialize CLOB client with API credentials
    pub async fn init_clob(&mut self) -> Result<()> {
        if self.config.private_key.is_empty() {
            return Ok(()); // No private key, skip CLOB initialization
        }

        let funder = if self.config.wallet_address.is_empty() {
            None
        } else {
            Some(self.config.wallet_address.as_str())
        };

        let sig_type = match self.config.signature_type {
            1 => SignatureType::PolyProxy,
            2 => SignatureType::PolyGnosisSafe,
            _ => SignatureType::Eoa,
        };

        // Create L1 client first
        let mut clob = ClobClient::with_l1_auth(
            &self.config.clob_url,
            137, // Polygon mainnet
            &self.config.private_key,
            Some(sig_type),
            funder,
        )?;

        // Try to derive or create API credentials
        if self.config.api_key.is_empty() {
            info!("Deriving Polymarket API credentials...");
            match clob.create_or_derive_api_creds(Some(0)).await {
                Ok(creds) => {
                    info!("Successfully derived API credentials");
                    clob.set_api_creds(creds);
                }
                Err(e) => {
                    warn!("Failed to derive API credentials: {}. Order placement disabled.", e);
                }
            }
        } else {
            // Use configured credentials
            clob.set_api_creds(ApiCreds {
                api_key: self.config.api_key.clone(),
                api_secret: self.config.api_secret.clone(),
                api_passphrase: self.config.api_passphrase.clone(),
            });
        }

        self.clob = Some(Arc::new(clob));
        Ok(())
    }

    /// Get account balance
    pub async fn get_balance(&self) -> Result<f64> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let balance = clob.as_ref().get_balance_allowance().await?;
        let balance_val: f64 = balance.balance.parse().unwrap_or(0.0);
        Ok(balance_val / 1_000_000.0) // USDC has 6 decimals
    }

    /// Get NBA events and markets from Gamma API
    pub async fn get_nba_events_and_markets(
        &self,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let mut events = Vec::new();
        let mut markets = Vec::new();

        // Query Gamma API for NBA markets
        let url = format!(
            "{}/markets?tag=nba&closed=false&limit=100",
            self.config.base_url
        );

        let response = self.http.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("Gamma API error {}: {}", status, body);
        }

        let data: Value = serde_json::from_str(&body)?;
        let market_array = data
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid markets response"))?;

        for market_data in market_array {
            // Parse market data
            let condition_id = market_data["conditionId"]
                .as_str()
                .or_else(|| market_data["condition_id"].as_str())
                .unwrap_or("")
                .to_string();

            if condition_id.is_empty() {
                continue;
            }

            let question = market_data["question"].as_str().unwrap_or("");

            // Parse team names
            let team_names = parse_team_names_poly(question);
            if team_names.is_none() {
                continue;
            }
            let (team_a, team_b) = team_names.unwrap();

            // Standardize event name (alphabetical order)
            let event_name = if team_a < team_b {
                format!("{}-{}", team_a, team_b)
            } else {
                format!("{}-{}", team_b, team_a)
            };

            // Parse outcomes and tokens
            let outcomes = market_data["outcomes"]
                .as_array()
                .or_else(|| market_data["tokens"].as_array());

            let mut token_id_a = None;
            let mut token_id_b = None;
            let mut price_a = 0.5;
            let mut price_b = 0.5;

            if let Some(outcomes_arr) = outcomes {
                for (idx, outcome) in outcomes_arr.iter().enumerate() {
                    let outcome_name = outcome["outcome"]
                        .as_str()
                        .or_else(|| outcome["name"].as_str())
                        .unwrap_or("");
                    let token_id = outcome["token_id"]
                        .as_str()
                        .or_else(|| outcome["tokenId"].as_str())
                        .map(|s| s.to_string());
                    let price = outcome["price"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| outcome["price"].as_f64())
                        .unwrap_or(0.5);

                    // Match outcome to team
                    if outcome_name.to_uppercase().contains(&team_a.to_uppercase()) {
                        token_id_a = token_id;
                        price_a = price;
                    } else if outcome_name.to_uppercase().contains(&team_b.to_uppercase()) {
                        token_id_b = token_id;
                        price_b = price;
                    } else if idx == 0 {
                        // Default: first outcome is team_a (alphabetically first)
                        if team_a < team_b {
                            token_id_a = token_id;
                            price_a = price;
                        } else {
                            token_id_b = token_id;
                            price_b = price;
                        }
                    } else {
                        if team_a < team_b {
                            token_id_b = token_id;
                            price_b = price;
                        } else {
                            token_id_a = token_id;
                            price_a = price;
                        }
                    }
                }
            }

            // Parse start time
            let start_time = market_data["end_date_iso"]
                .as_str()
                .or_else(|| market_data["endDateIso"].as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            let volume = market_data["volume"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| market_data["volume"].as_f64());

            let market = PolymarketMarket {
                market_id: condition_id.clone(),
                event_name: event_name.clone(),
                team_a: if team_a < team_b {
                    team_a.clone()
                } else {
                    team_b.clone()
                },
                team_b: if team_a < team_b {
                    team_b.clone()
                } else {
                    team_a.clone()
                },
                price_a,
                price_b,
                start_time,
                volume,
                token_id_a,
                token_id_b,
            };

            let event = PolymarketEvent {
                event_id: condition_id.clone(),
                name: event_name.clone(),
                team_a: market.team_a.clone(),
                team_b: market.team_b.clone(),
                start_time,
                category: "NBA".to_string(),
                market: Some(market.clone()),
            };

            events.push(event);
            markets.push(market);
        }

        info!(
            "Loaded {} Polymarket events with {} markets",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Place a market order
    pub async fn place_market_order(
        &self,
        token_id: &str,
        side: &str,
        amount: f64,
    ) -> Result<Value> {
        let clob = self
            .clob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized"))?;

        let order_side = if side.to_lowercase() == "buy" {
            Side::Buy
        } else {
            Side::Sell
        };

        let order_args = MarketOrderArgs {
            token_id: token_id.to_string(),
            amount,
            side: order_side,
            fee_rate_bps: None,
            nonce: None,
            slippage: Some(0.005), // 0.5% slippage
        };

        let signed_order = clob.as_ref().create_market_order(&order_args).await?;
        let response = clob
            .as_ref()
            .post_order(&signed_order, crate::clob::OrderType::Fok)
            .await?;

        Ok(serde_json::to_value(response)?)
    }

    /// Connect to WebSocket for real-time price updates
    pub async fn connect_websocket(
        &self,
        token_ids: Vec<String>,
        price_tx: mpsc::Sender<PriceUpdate>,
    ) -> Result<()> {
        info!("Connecting to Polymarket WebSocket...");

        let (ws_stream, _) = connect_async(POLY_WS_URL)
            .await
            .with_context(|| "Failed to connect to Polymarket WebSocket")?;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to markets
        let subscribe_msg = json!({
            "auth": {},
            "type": "subscribe",
            "markets": token_ids,
            "assets_ids": token_ids
        });

        write
            .send(Message::Text(subscribe_msg.to_string()))
            .await?;

        info!("Subscribed to {} Polymarket tokens", token_ids.len());

        // Process messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Some(update) = Self::parse_ws_message(&text) {
                        if price_tx.send(update).await.is_err() {
                            warn!("Price update channel closed");
                            break;
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Polymarket WebSocket closed");
                    break;
                }
                Err(e) => {
                    error!("Polymarket WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Parse WebSocket message
    fn parse_ws_message(text: &str) -> Option<PriceUpdate> {
        let data: Value = serde_json::from_str(text).ok()?;

        // Handle different message formats
        let asset_id = data
            .get("asset_id")
            .or_else(|| data.get("market"))
            .and_then(|v| v.as_str())?
            .to_string();

        // Get price from message
        let price = data
            .get("price")
            .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_f64()))
            .or_else(|| {
                // Try to get from bids/asks
                let best_ask = data
                    .get("asks")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|entry| {
                        entry
                            .get("price")
                            .and_then(|p| p.as_str().and_then(|s| s.parse().ok()).or(p.as_f64()))
                    });

                best_ask
            })?;

        // For Polymarket, we only get the ask price for each token
        Some(PriceUpdate {
            platform: Platform::Polymarket,
            market_id: asset_id,
            yes_bid: None,
            yes_ask: Some(price),
            no_bid: None,
            no_ask: None,
            timestamp: Utc::now(),
        })
    }
}

/// Parse team names from Polymarket question
fn parse_team_names_poly(question: &str) -> Option<(String, String)> {
    // Common patterns:
    // "Will MEM beat LAL?"
    // "MEM vs LAL"
    // "Who will win: MEM or LAL?"

    let separators = [
        " beat ",
        " vs ",
        " vs. ",
        " or ",
        " @ ",
        " at ",
        " - ",
    ];

    let question_lower = question.to_lowercase();

    for sep in separators {
        if let Some(pos) = question_lower.find(sep) {
            // Extract text before and after separator
            let before = &question[..pos];
            let after = &question[pos + sep.len()..];

            // Extract team abbreviations
            let team_a = extract_team_abbr_poly(before)?;
            let team_b = extract_team_abbr_poly(after)?;

            return Some((team_a.to_uppercase(), team_b.to_uppercase()));
        }
    }

    None
}

/// Extract team abbreviation from text
fn extract_team_abbr_poly(text: &str) -> Option<String> {
    // Look for 3-letter uppercase abbreviation
    let words: Vec<&str> = text.split_whitespace().collect();

    // Try to find 3-letter abbreviation
    for word in &words {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.len() == 3 && clean.chars().all(|c| c.is_alphabetic()) {
            return Some(clean.to_uppercase());
        }
    }

    // Fall back to last word
    words.last().map(|w| {
        let clean = w.trim_matches(|c: char| !c.is_alphanumeric());
        clean.to_uppercase()
    })
}
