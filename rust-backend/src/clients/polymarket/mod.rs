//! Polymarket platform client
//!
//! Handles Polymarket API interactions including:
//! - Market data retrieval from the Polymarket US gateway
//! - Ed25519-signed order placement directly against the Polymarket US API

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info, warn};

use crate::config::PolymarketConfig;
use crate::core::{competitor_event_name, normalize_competitor_name, normalize_team_name};
use crate::models::{PolymarketEvent, PolymarketMarket, PolymarketPositionSide};

pub mod auth;
pub mod ws;
mod book;
mod orders;
mod events;

pub use book::*;
pub use orders::*;

/// Polymarket API client
#[derive(Clone)]
pub struct PolymarketClient {
    pub config: PolymarketConfig,
    http: Client,
    key_id: String,
    secret_key: String,
    balance_cache: std::sync::Arc<AsyncMutex<Option<(Instant, f64)>>>,
    positions_cache: std::sync::Arc<AsyncMutex<Option<(Instant, Value)>>>,
    market_constraints_cache:
        std::sync::Arc<AsyncMutex<HashMap<String, (Instant, Option<f64>, Option<f64>)>>>,
}

/// Read Polymarket US API credentials, env vars first, falling back to
/// config file values.
pub fn resolve_credentials(config: &PolymarketConfig) -> Result<(String, String)> {
    let key_id = std::env::var("POLYMARKET_KEY_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| (!config.key_id.trim().is_empty()).then(|| config.key_id.clone()));
    let secret_key = std::env::var("POLYMARKET_SECRET_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| (!config.secret_key.trim().is_empty()).then(|| config.secret_key.clone()));
    match (key_id, secret_key) {
        (Some(key_id), Some(secret_key)) => Ok((key_id, secret_key)),
        _ => anyhow::bail!(
            "Polymarket US credentials are required. Set POLYMARKET_KEY_ID and \
             POLYMARKET_SECRET_KEY, or configure polymarket.key_id and polymarket.secret_key."
        ),
    }
}

/// Convert an HTTP error response into a concise message, keeping upstream
/// HTML/JSON error pages out of application logs.
fn concise_api_error(status: StatusCode, body_text: &str) -> String {
    let message = serde_json::from_str::<Value>(body_text)
        .ok()
        .and_then(|body| {
            body.get("message")
                .or_else(|| body.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body_text.to_string());
    if message.contains("1015") || message.to_lowercase().contains("rate limited") {
        return "Polymarket US is rate limiting this IP. Retry after the temporary restriction expires."
            .to_string();
    }
    let mut truncated = message;
    truncated.truncate(500);
    format!("Polymarket US API error {}: {}", status, truncated)
}

impl PolymarketClient {
    /// Create a new Polymarket client
    pub fn new(config: PolymarketConfig) -> Result<Self> {
        let (key_id, secret_key) = resolve_credentials(&config)?;
        let http = Client::builder()
            .timeout(config.http.timeout())
            .build()
            .context("Failed to build Polymarket HTTP client")?;

        info!(
            "✅ Initialized Polymarket US client with API key ID {}...",
            &key_id[..key_id.len().min(8)]
        );

        Ok(Self {
            config,
            http,
            key_id,
            secret_key,
            balance_cache: std::sync::Arc::new(AsyncMutex::new(None)),
            positions_cache: std::sync::Arc::new(AsyncMutex::new(None)),
            market_constraints_cache: std::sync::Arc::new(AsyncMutex::new(HashMap::new())),
        })
    }

    /// Send a request to the Polymarket US API, signing it (Ed25519) when
    /// `signed_path` is set. Retries only on HTTP 429 - per
    /// docs.polymarket.us/api-reference/rate-limits, stop immediately, wait
    /// at least 1 second, then retry with exponential backoff (1s/2s/4s, 3
    /// attempts total). A 429 means the request was rejected before
    /// reaching the matching engine, so retrying an order submission on a
    /// 429 cannot cause a duplicate order - unlike a transport-level
    /// failure (timeout/reset), which is never retried here for the same
    /// reason `clients/kalshi/mod.rs::post()` never retries those blindly. The
    /// timestamp and signature are regenerated on every attempt.
    async fn request_json(
        &self,
        method: Method,
        url: &str,
        signed_path: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value> {
        self.request_json_with_headers(method, url, signed_path, body)
            .await
            .map(|(json, _)| json)
    }

    /// `request_json`, also returning the response headers (the market book
    /// fetch needs them to tell an origin response from a CDN-cached one).
    async fn request_json_with_headers(
        &self,
        method: Method,
        url: &str,
        signed_path: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(Value, HeaderMap)> {
        let mut attempt = 0u32;
        loop {
            let mut builder = self.http.request(method.clone(), url);
            if let Some(path) = signed_path {
                let headers =
                    auth::sign_request(&self.key_id, &self.secret_key, method.as_str(), path)?;
                builder = builder
                    .header("X-PM-Access-Key", headers.access_key)
                    .header("X-PM-Timestamp", headers.timestamp)
                    .header("X-PM-Signature", headers.signature);
            }
            builder = builder.header("Content-Type", "application/json");
            if let Some(body) = body {
                builder = builder.json(body);
            }

            let response = builder
                .send()
                .await
                .context("Failed to reach Polymarket US")?;
            let status = response.status();

            if status.as_u16() == 429 {
                if attempt + 1 >= self.config.http.max_attempts {
                    anyhow::bail!(
                        "Polymarket US is rate limiting this IP. Retry after the temporary restriction expires."
                    );
                }
                let delay = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "Polymarket US rate limit hit for {} {}, retrying in {:?} (attempt {}/{})",
                    method,
                    url,
                    delay,
                    attempt + 1,
                    self.config.http.max_attempts
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }

            let headers = response.headers().clone();
            let text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!(concise_api_error(status, &text));
            }
            if text.trim().is_empty() {
                return Ok((json!({}), headers));
            }
            let json = serde_json::from_str(&text)
                .with_context(|| format!("Failed to parse Polymarket US response: {}", text))?;
            return Ok((json, headers));
        }
    }

    /// Get account balance from the Polymarket US API, cached briefly to
    /// keep repeated balance checks from adding rate-limit pressure.
    pub async fn get_balance(&self) -> Result<f64> {
        let mut cache = self.balance_cache.lock().await;
        if let Some((cached_at, balance)) = cache.as_ref() {
            if cached_at.elapsed() < self.config.cache.balance_ttl() {
                return Ok(*balance);
            }
        }

        let url = format!("{}/v1/account/balances", self.config.api_base_url);
        let response = self
            .request_json(Method::GET, &url, Some("/v1/account/balances"), None)
            .await
            .context("Failed to fetch Polymarket US balances")?;
        let balances = response
            .get("balances")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("Polymarket US balances response did not contain a balance list")
            })?;
        let total: f64 = balances
            .iter()
            .filter_map(|balance| amount_as_f64(&balance["buyingPower"]))
            .sum();

        *cache = Some((Instant::now(), total));
        Ok(total)
    }
}
