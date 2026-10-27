//! Read-only: raw Kalshi WebSocket reader. Subscribes the given tickers
//! (one subscribe each, like the bot) and only measures feed lag
//! (exchange `ts_ms` vs receipt) and message rate, with no book handling,
//! to tell a slow client from a slow feed. Optional first arg `--batch`
//! sends one subscribe with all tickers instead. Places no orders.
//! `--hot N S` subscribes the last N tickers S seconds in, printing control
//! frames and distinct tickers updating per window. `--dump N` first prints the first N raw book frames.
//! Run: cargo run --release --example probe_kalshi_ws_raw -- [--dump N] [--hot N S] [--batch] <TICKER> ...

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut tickers: Vec<String> = std::env::args().skip(1).collect();
    let mut dump = 0usize;
    if tickers.first().map(String::as_str) == Some("--dump") {
        dump = tickers[1].parse()?;
        tickers.drain(..2);
    }
    // `--hot N S`: hold back the last N tickers and subscribe them one by
    // one after S seconds, like the bot's hot subscription.
    let mut hot: Vec<String> = Vec::new();
    let mut hot_after = Duration::ZERO;
    if tickers.first().map(String::as_str) == Some("--hot") {
        let n: usize = tickers[1].parse()?;
        hot_after = Duration::from_secs(tickers[2].parse()?);
        tickers.drain(..3);
        hot = tickers.split_off(tickers.len().saturating_sub(n));
    }
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
    // Split of the total: Kalshi-internal (sent - ts) vs transport plus
    // local clock offset (receipt - sent).
    let mut server_lags: Vec<i64> = Vec::new();
    let mut net_lags: Vec<i64> = Vec::new();
    let mut errors = 0;
    let mut next_id = tickers.len() + 1;
    let mut seen_tickers: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut control_frames = 0;
    while start.elapsed() < Duration::from_secs(120) {
        if !hot.is_empty() && start.elapsed() >= hot_after {
            for t in hot.drain(..) {
                println!("HOT SUBSCRIBE {t}");
                write
                    .send(Message::Text(
                        json!({"id": next_id, "cmd": "subscribe", "params": {"channels": ["orderbook_delta"], "market_ticker": t}})
                            .to_string(),
                    ))
                    .await?;
                next_id += 1;
            }
        }
        let Some(frame) = read.next().await else { break };
        let Message::Text(text) = frame? else { continue };
        msgs += 1;
        if text.contains("\"type\":\"orderbook_delta\"") || text.contains("\"type\":\"orderbook_snapshot\"") {
            if let Some(at) = text.find("\"market_ticker\":\"") {
                let rest = &text[at + 17..];
                seen_tickers.insert(rest[..rest.find('"').unwrap_or(0)].to_string());
            }
        } else if start.elapsed() > Duration::from_secs(5) || control_frames < 3 {
            control_frames += 1;
            if control_frames <= 40 {
                println!("CONTROL t+{}s: {}", start.elapsed().as_secs(), text.chars().take(200).collect::<String>());
            }
        }
        if dump > 0 && (text.contains("orderbook_snapshot") || text.contains("orderbook_delta")) {
            dump -= 1;
            println!("FRAME: {}", text.chars().take(400).collect::<String>());
        }
        bytes += text.len() as u64;
        if text.contains("\"error\"") && errors < 5 {
            errors += 1;
            println!("ERROR FRAME: {}", text.chars().take(200).collect::<String>());
        }
        let field = |name: &str| {
            let at = text.find(name)?;
            text[at + name.len()..].split(|c: char| !c.is_ascii_digit()).next()?.parse::<i64>().ok()
        };
        if let (Some(ts), Some(sent)) = (field("\"ts_ms\":"), field("\"sending_ts_ms\":")) {
            let now = chrono::Utc::now().timestamp_millis();
            lags.push(now - ts);
            server_lags.push(sent - ts);
            net_lags.push(now - sent);
        }
        if window.elapsed() >= Duration::from_secs(15) {
            let pct = |v: &mut Vec<i64>, q: f64| {
                v.sort_unstable();
                v.get(((v.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(0)
            };
            println!(
                "t+{:>3}s {} msgs ({:.0}/s, {:.0} KB/s) lag p50 {}ms p95 {}ms max {}ms | kalshi-side p50 {}ms p95 {}ms | net+clock p50 {}ms p95 {}ms",
                start.elapsed().as_secs(), msgs, msgs as f64 / 15.0, bytes as f64 / 15.0 / 1024.0,
                pct(&mut lags, 0.5), pct(&mut lags, 0.95), pct(&mut lags, 1.0),
                pct(&mut server_lags, 0.5), pct(&mut server_lags, 0.95),
                pct(&mut net_lags, 0.5), pct(&mut net_lags, 0.95)
            );
            println!("        distinct tickers with book frames: {}", seen_tickers.len());
            seen_tickers.clear();
            lags.clear();
            server_lags.clear();
            net_lags.clear();
            msgs = 0;
            bytes = 0;
            window = Instant::now();
        }
    }
    Ok(())
}
