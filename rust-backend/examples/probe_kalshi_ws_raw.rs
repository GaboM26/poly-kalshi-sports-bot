//! Read-only: raw Kalshi WebSocket reader. Subscribes the given tickers
//! (one subscribe each, like the bot) and only measures feed lag
//! (exchange `ts_ms` vs receipt) and message rate, with no book handling,
//! to tell a slow client from a slow feed. Optional first arg `--batch`
//! sends one subscribe with all tickers instead. Places no orders.
//! Run: cargo run --release --example probe_kalshi_ws_raw -- [--batch] <TICKER> ...

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut tickers: Vec<String> = std::env::args().skip(1).collect();
    let batch = tickers.first().map(String::as_str) == Some("--batch");
    if batch {
        tickers.remove(0);
    }
    anyhow::ensure!(!tickers.is_empty(), "pass at least one ticker");
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    let (stream, _) = connect_async(kalshi.websocket_request()?).await?;
    let (mut write, mut read) = stream.split();
    if batch {
        write
            .send(Message::Text(
                json!({"id": 1, "cmd": "subscribe", "params": {"channels": ["orderbook_delta"], "market_tickers": tickers}})
                    .to_string(),
            ))
            .await?;
    } else {
        for (i, t) in tickers.iter().enumerate() {
            write
                .send(Message::Text(
                    json!({"id": i + 1, "cmd": "subscribe", "params": {"channels": ["orderbook_delta"], "market_ticker": t}})
                        .to_string(),
                ))
                .await?;
        }
    }
    let start = Instant::now();
    let mut window = Instant::now();
    let (mut msgs, mut bytes) = (0u64, 0u64);
    let mut lags: Vec<i64> = Vec::new();
    let mut errors = 0;
    while start.elapsed() < Duration::from_secs(120) {
        let Some(frame) = read.next().await else { break };
        let Message::Text(text) = frame? else { continue };
        msgs += 1;
        bytes += text.len() as u64;
        if text.contains("\"error\"") && errors < 5 {
            errors += 1;
            println!("ERROR FRAME: {}", text.chars().take(200).collect::<String>());
        }
        if let Some(at) = text.find("\"ts_ms\":") {
            if let Some(ts) = text[at + 8..].split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<i64>().ok()) {
                lags.push(chrono::Utc::now().timestamp_millis() - ts);
            }
        }
        if window.elapsed() >= Duration::from_secs(15) {
            lags.sort_unstable();
            let pick = |q: f64| lags.get(((lags.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(0);
            println!(
                "t+{:>3}s {} msgs ({:.0}/s, {:.0} KB/s) lag p50 {}ms p95 {}ms max {}ms",
                start.elapsed().as_secs(), msgs, msgs as f64 / 15.0, bytes as f64 / 15.0 / 1024.0,
                pick(0.5), pick(0.95), pick(1.0)
            );
            lags.clear();
            msgs = 0;
            bytes = 0;
            window = Instant::now();
        }
    }
    Ok(())
}
