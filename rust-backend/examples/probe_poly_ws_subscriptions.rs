//! Read-only probe: does the Polymarket US markets WebSocket keep accepting
//! subscriptions after many hot-adds on one connection? Subscribes sports
//! slugs in many small batches (like the bot's 5s hot-add loop), then the
//! given slugs last, and prints every frame that is not market data plus a
//! per-slug count of data frames for the last slugs.
//! Run: cargo run --example probe_poly_ws_subscriptions -- <batches> <slug> [<slug>...]
use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_auth;
use polytaoli::clients::PolymarketClient;
use polytaoli::config::Config;
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let batches: usize = args.next().ok_or_else(|| anyhow::anyhow!("batches"))?.parse()?;
    let last: Vec<String> = args.collect();
    let config = Config::from_file("config.toml")?;
    let (key_id, secret) = resolve_credentials(&config.polymarket)?;
    let client = PolymarketClient::new(config.polymarket)?;
    let (_, markets) = client.get_supported_events_and_markets().await?;
    let mut sports: Vec<String> = markets.iter().map(|m| m.market_slug.clone()).collect();
    sports.sort();
    sports.dedup();
    sports.retain(|s| !last.contains(s));

    let headers = polymarket_auth::sign_request(&key_id, &secret, "GET", "/v1/ws/markets")?;
    let mut request = "wss://api.polymarket.us/v1/ws/markets".into_client_request()?;
    let h = request.headers_mut();
    h.insert("X-PM-Access-Key", headers.access_key.parse()?);
    h.insert("X-PM-Timestamp", headers.timestamp.parse()?);
    h.insert("X-PM-Signature", headers.signature.parse()?);
    let (stream, _) = connect_async(request).await?;
    let (mut write, mut read) = stream.split();

    let per_batch = (sports.len() / batches.max(1)).max(1);
    let mut groups: Vec<Vec<String>> = sports.chunks(per_batch).take(batches).map(|c| c.to_vec()).collect();
    groups.push(last.clone());
    let mut id = 0;
    let mut subs = 0;
    for group in &groups {
        for kind in ["SUBSCRIPTION_TYPE_MARKET_DATA_LITE", "SUBSCRIPTION_TYPE_MARKET_DATA"] {
            id += 1;
            subs += 1;
            write
                .send(Message::Text(
                    json!({"subscribe": {"requestId": format!("r{id}"), "subscriptionType": kind, "marketSlugs": group}})
                        .to_string(),
                ))
                .await?;
        }
    }
    println!("sent {subs} subscriptions, {} slugs total; last group {:?}", groups.iter().map(Vec::len).sum::<usize>(), last);

    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut other = 0;
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, read.next()).await {
        let Ok(Message::Text(text)) = frame else { continue };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let lite = v.get("marketDataLite").and_then(|d| d.get("marketSlug")).and_then(Value::as_str);
        let book = v.get("marketData").and_then(|d| d.get("marketSlug")).and_then(Value::as_str);
        match (lite, book) {
            (Some(s), _) => counts.entry(s.to_string()).or_default().0 += 1,
            (_, Some(s)) => counts.entry(s.to_string()).or_default().1 += 1,
            _ => {
                other += 1;
                if other <= 15 {
                    println!("OTHER FRAME: {}", text.chars().take(300).collect::<String>());
                }
            }
        }
    }
    println!("non-data frames: {other}; slugs with data: {}", counts.len());
    for slug in &last {
        println!("{slug}: lite/book frames {:?}", counts.get(slug));
    }
    Ok(())
}
