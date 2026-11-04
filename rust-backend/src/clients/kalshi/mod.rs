//! Kalshi platform client
//!
//! Handles Kalshi API interactions including:
//! - RSA-PSS authentication
//! - Market data retrieval
//! - WebSocket order book subscription
//! - Order placement

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::{BlindedSigningKey, Signature};
use rsa::sha2::Sha256;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::config::KalshiConfig;
use crate::core::{competitor_event_name, normalize_competitor_name};
use crate::models::{KalshiEvent, KalshiMarket, Platform, PriceUpdate};

mod book;
mod quotes;
mod orders;
mod ws;
mod events;

pub use book::*;
pub use quotes::*;
pub use orders::*;
pub use ws::*;
pub use events::*;

/// Kalshi API client
#[derive(Clone)]
pub struct KalshiClient {
    pub config: KalshiConfig,
    http: Client,
    signing_key: Arc<BlindedSigningKey<Sha256>>,
    /// Order book cache: market_ticker -> { "yes": [[price, qty], ...], "no": [[price, qty], ...] }
    orderbook_cache: Arc<RwLock<HashMap<String, OrderBook>>>,
    /// Channel sender for dynamic subscriptions/unsubscriptions
    command_tx: Arc<RwLock<Option<mpsc::Sender<KalshiWsCommand>>>>,
    /// Mention tickers re-subscribed on reconnect; empty unless directional is running.
    directional_subscriptions: Arc<RwLock<HashSet<String>>>,
    /// Incremented every time a new WebSocket connection is established.
    /// Kalshi only pushes `orderbook_delta` on real book changes, not on a
    /// heartbeat, so "time since last delta" is the wrong staleness signal
    /// for a quiet market - it would reject an accurate book just because
    /// nothing traded recently. A book is trustworthy iff it was built
    /// entirely under the currently-active, unbroken connection; tagging
    /// each book with the epoch it was last written under (see
    /// `OrderBook::epoch`) makes that checkable without any elapsed-time
    /// guess, and a reconnect still correctly invalidates every book until
    /// fresh snapshots arrive under the new epoch.
    connection_epoch: Arc<AtomicU64>,
    /// Set on every WebSocket message received, regardless of ticker - a
    /// connection-level liveness signal, separate from any one book's
    /// staleness. Backstops the epoch check against a connection that's
    /// gone silent without erroring (e.g. a hung read).
    last_message_at: Arc<RwLock<Option<Instant>>>,
}

impl KalshiClient {
    /// Create a new Kalshi client
    pub fn new(config: KalshiConfig) -> Result<Self> {
        // Parse RSA private key - 支持 PKCS#8 和 PKCS#1 两种格式
        let private_key = RsaPrivateKey::from_pkcs8_pem(&config.api_secret)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(&config.api_secret))
            .with_context(|| {
                "Failed to parse Kalshi RSA private key (tried both PKCS#8 and PKCS#1 formats)"
            })?;
        let signing_key = Arc::new(BlindedSigningKey::<Sha256>::new(private_key));

        let http = Client::builder()
            .timeout(config.http.timeout())
            .build()
            .context("Failed to build Kalshi HTTP client")?;

        Ok(Self {
            config,
            http,
            signing_key,
            orderbook_cache: Arc::new(RwLock::new(HashMap::new())),
            command_tx: Arc::new(RwLock::new(None)),
            directional_subscriptions: Arc::new(RwLock::new(HashSet::new())),
            connection_epoch: Arc::new(AtomicU64::new(0)),
            last_message_at: Arc::new(RwLock::new(None)),
        })
    }

    /// Sign a request using RSA-PSS
    fn sign_request(&self, timestamp: i64, method: &str, path: &str) -> String {
        let message = format!("{}{}{}", timestamp, method, path);
        let mut rng = rand::thread_rng();
        let signature: Signature = self.signing_key.sign_with_rng(&mut rng, message.as_bytes());
        BASE64.encode(signature.to_bytes())
    }

    /// Kalshi signatures cover the API path but never query parameters.
    fn signing_path(path: &str) -> String {
        format!("/trade-api/v2{}", path.split('?').next().unwrap_or(path))
    }

    /// Get current timestamp in milliseconds
    fn get_timestamp_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    /// Make an authenticated GET request. A transport-level failure (a
    /// connection timeout, reset, or DNS error before any response arrives)
    /// is retried with exponential backoff (1s/2s, 3 attempts total) since a
    /// GET can never have caused a side effect on Kalshi's side - unlike
    /// `post()`, which places orders and must never be blindly retried. A
    /// real Kalshi API error (a non-success HTTP status) is not retried.
    async fn get(&self, path: &str) -> Result<Value> {
        let mut attempt = 0u32;
        let response = loop {
            let timestamp = Self::get_timestamp_ms();
            let sign_path = Self::signing_path(path);
            let signature = self.sign_request(timestamp, "GET", &sign_path);
            let url = format!("{}{}", self.config.base_url, path);

            match self
                .http
                .get(&url)
                .header("KALSHI-ACCESS-KEY", &self.config.api_key)
                .header("KALSHI-ACCESS-SIGNATURE", &signature)
                .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
                .send()
                .await
            {
                Ok(response) => break response,
                Err(err) if attempt + 1 < self.config.http.max_attempts => {
                    let delay = Duration::from_secs(2u64.pow(attempt));
                    warn!(
                        "Kalshi GET {} failed ({}), retrying in {:?} (attempt {}/{})",
                        path,
                        err,
                        delay,
                        attempt + 1,
                        self.config.http.max_attempts
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(err) => return Err(err.into()),
            }
        };

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }

    /// Make an authenticated POST request
    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let timestamp = Self::get_timestamp_ms();
        let sign_path = Self::signing_path(path);
        let signature = self.sign_request(timestamp, "POST", &sign_path);

        let url = format!("{}{}", self.config.base_url, path);

        let response = self
            .http
            .post(&url)
            .header("KALSHI-ACCESS-KEY", &self.config.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?;

        let status = response.status();
        let resp_body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, resp_body);
        }

        serde_json::from_str(&resp_body)
            .with_context(|| format!("Failed to parse response: {}", resp_body))
    }

    /// Get account balance
    pub async fn get_balance(&self) -> Result<f64> {
        let response = self.get("/portfolio/balance").await?;
        let balance = response["balance"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Invalid balance response"))?;
        Ok(balance / 100.0) // Convert cents to dollars
    }

    /// Raw balance response, including the per-exchange-shard
    /// `balance_breakdown`. Kalshi keeps a separate balance per shard
    /// (crypto is shard 2, tennis/baseball/basketball shard 3), and an order
    /// on a shard with no funds fails with `insufficient_shard_balance`.
    pub async fn get_balance_details(&self) -> Result<Value> {
        self.get("/portfolio/balance").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_path_excludes_query_parameters() {
        assert_eq!(
            KalshiClient::signing_path("/portfolio/orders?status=open&limit=10"),
            "/trade-api/v2/portfolio/orders"
        );
    }
}
