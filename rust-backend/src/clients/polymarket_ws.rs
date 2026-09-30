//! Polymarket US markets WebSocket (`wss://api.polymarket.us/v1/ws/markets`).
//!
//! Push-based replacement for waiting on the ~3s REST poll when detecting
//! arbitrage. Subscribes to `SUBSCRIPTION_TYPE_MARKET_DATA_LITE`, whose
//! `longQuote`/`shortQuote` are the same per-side quotes the gateway REST
//! feed exposes as `marketSides[].quote` (verified live). These are
//! detection prices only - execution depth still comes exclusively from
//! `/v1/markets/{slug}/book` at order time.
//!
//! Auth is the same Ed25519 scheme as REST: sign `{timestamp}GET{path}`.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message,
};
use tracing::{debug, info, warn};

use super::polymarket_auth;

const WS_URL: &str = "wss://api.polymarket.us/v1/ws/markets";
const WS_PATH: &str = "/v1/ws/markets";
/// Documented cap: 100 markets per subscription.
const MAX_SLUGS_PER_SUBSCRIPTION: usize = 100;
/// The server sends heartbeats; silence beyond this means a dead socket.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(30);
const SLUG_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A per-side quote update for one market slug.
#[derive(Debug, Clone, PartialEq)]
pub struct PolyWsQuote {
    pub slug: String,
    pub long_quote: f64,
    pub short_quote: f64,
}

fn price(value: &Value) -> Option<f64> {
    let v = value.get("value").unwrap_or(value);
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// Parse a raw WS text frame. Returns `None` for heartbeats, acks, full
/// order-book messages, closed/expired markets, or anything missing a
/// quote - the caller then simply leaves the previous quote in place.
pub fn parse_lite_message(text: &str) -> Option<PolyWsQuote> {
    let root: Value = serde_json::from_str(text).ok()?;
    let lite = root.get("marketDataLite")?;
    if lite.get("state").and_then(Value::as_str) != Some("MARKET_STATE_OPEN") {
        return None;
    }
    let long_quote = price(lite.get("longQuote")?)?;
    let short_quote = price(lite.get("shortQuote")?)?;
    // Mirror the REST parser's validity window.
    if !(0.01..=0.99).contains(&long_quote) || !(0.01..=0.99).contains(&short_quote) {
        return None;
    }
    Some(PolyWsQuote {
        slug: lite.get("marketSlug")?.as_str()?.to_string(),
        long_quote,
        short_quote,
    })
}

fn subscribe_messages(slugs: &[String], next_id: &mut u64) -> Vec<Message> {
    slugs
        .chunks(MAX_SLUGS_PER_SUBSCRIPTION)
        .map(|chunk| {
            *next_id += 1;
            Message::Text(
                json!({"subscribe": {
                    "requestId": format!("lite-{}", next_id),
                    "subscriptionType": "SUBSCRIPTION_TYPE_MARKET_DATA_LITE",
                    "marketSlugs": chunk,
                }})
                .to_string(),
            )
        })
        .collect()
}

/// One connection attempt. Returns when the socket dies or goes silent.
async fn run_once<S, Q>(
    key_id: &str,
    secret_key: &str,
    slugs: &S,
    on_quote: &Q,
) -> Result<()>
where
    S: Fn() -> Vec<String>,
    Q: Fn(PolyWsQuote),
{
    let headers = polymarket_auth::sign_request(key_id, secret_key, "GET", WS_PATH)?;
    let mut request = WS_URL.into_client_request()?;
    let h = request.headers_mut();
    h.insert("X-PM-Access-Key", headers.access_key.parse()?);
    h.insert("X-PM-Timestamp", headers.timestamp.parse()?);
    h.insert("X-PM-Signature", headers.signature.parse()?);

    let (stream, _) = connect_async(request)
        .await
        .context("Failed to connect to the Polymarket US WebSocket")?;
    let (mut write, mut read) = stream.split();

    let mut subscribed: HashSet<String> = HashSet::new();
    let mut next_id = 0u64;
    let mut refresh = tokio::time::interval(SLUG_REFRESH_INTERVAL);

    loop {
        tokio::select! {
            _ = refresh.tick() => {
                // Subscribe to newly matched slugs. There is no documented
                // unsubscribe; slugs that drop out stay subscribed until the
                // next reconnect, which is harmless (their updates are
                // ignored once no matched market carries the slug).
                let new: Vec<String> = slugs()
                    .into_iter()
                    .filter(|s| !subscribed.contains(s))
                    .collect();
                if !new.is_empty() {
                    for msg in subscribe_messages(&new, &mut next_id) {
                        write.send(msg).await.context("WS subscribe send failed")?;
                    }
                    info!("📡 Polymarket WS subscribed to {} new markets ({} total)",
                        new.len(), subscribed.len() + new.len());
                    subscribed.extend(new);
                }
            }
            frame = tokio::time::timeout(SILENCE_TIMEOUT, read.next()) => {
                let frame = frame.context("Polymarket WS silent for 30s")?;
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(quote) = parse_lite_message(&text) {
                            on_quote(quote);
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        anyhow::bail!("Polymarket WS closed by server: {:?}", frame);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e).context("Polymarket WS read error"),
                    None => anyhow::bail!("Polymarket WS stream ended"),
                }
            }
        }
    }
}

/// Run forever, reconnecting with exponential backoff (1s..30s). `slugs`
/// supplies the current desired subscription set; `on_quote` receives each
/// valid quote.
pub async fn run_markets_stream<S, Q>(key_id: String, secret_key: String, slugs: S, on_quote: Q)
where
    S: Fn() -> Vec<String>,
    Q: Fn(PolyWsQuote),
{
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = std::time::Instant::now();
        match run_once(&key_id, &secret_key, &slugs, &on_quote).await {
            Ok(()) => debug!("Polymarket WS loop returned"),
            Err(e) => warn!("Polymarket WS disconnected: {:#}. Reconnecting in {:?}", e, backoff),
        }
        // A connection that stayed up a while earns a fresh backoff.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: &str = r#"{"requestId":"r0","subscriptionType":"SUBSCRIPTION_TYPE_MARKET_DATA_LITE","marketDataLite":{"marketSlug":"s1","currentPx":{"value":"0.1050","currency":"USD"},"bestAsk":{"value":"0.1100","currency":"USD"},"bestBid":{"value":"0.1000","currency":"USD"},"longQuote":{"value":"0.1100","currency":"USD"},"shortQuote":{"value":"0.9","currency":"USD"},"state":"MARKET_STATE_OPEN"}}"#;

    #[test]
    fn parses_an_open_market_quote() {
        let q = parse_lite_message(OPEN).unwrap();
        assert_eq!(q.slug, "s1");
        assert!((q.long_quote - 0.11).abs() < 1e-9);
        assert!((q.short_quote - 0.9).abs() < 1e-9);
    }

    #[test]
    fn ignores_non_open_markets() {
        let expired = OPEN.replace("MARKET_STATE_OPEN", "MARKET_STATE_EXPIRED");
        assert!(parse_lite_message(&expired).is_none());
    }

    #[test]
    fn ignores_out_of_range_quotes() {
        assert!(parse_lite_message(&OPEN.replace(r#""0.9""#, r#""1.00""#)).is_none());
        assert!(parse_lite_message(&OPEN.replace(r#""0.1100","currency":"USD"},"shortQuote""#, r#""0.00","currency":"USD"},"shortQuote""#)).is_none());
    }

    #[test]
    fn ignores_heartbeats_acks_and_full_book_messages() {
        assert!(parse_lite_message(r#"{"heartbeat":{}}"#).is_none());
        assert!(parse_lite_message(r#"{"marketData":{"marketSlug":"s1","bids":[],"offers":[]}}"#).is_none());
        assert!(parse_lite_message("not json").is_none());
    }

    #[test]
    fn ignores_missing_quotes() {
        let no_quote = OPEN.replace("longQuote", "somethingElse");
        assert!(parse_lite_message(&no_quote).is_none());
    }

    #[test]
    fn chunks_subscriptions_at_100_slugs() {
        let slugs: Vec<String> = (0..250).map(|i| format!("s{i}")).collect();
        let mut id = 0;
        assert_eq!(subscribe_messages(&slugs, &mut id).len(), 3);
        assert_eq!(id, 3);
    }
}
