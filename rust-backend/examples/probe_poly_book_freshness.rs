//! Read-only diagnostic: compare how fresh each Polymarket US book source is
//! for the given (ideally live, in-play) market slugs:
//!   WS     - authenticated markets WebSocket full-book stream
//!   CACHED - the gateway REST book exactly as `get_market_book` fetches it
//!   BUSTED - the same URL with a unique query string (forces a CDN miss)
//! Prints best bid/offer per source with the CDN cache headers, so a stale
//! source shows up as a frozen price next to a moving one. WS connection
//! events (connect/heartbeat/disconnect) are printed too, which is how the
//! heartbeat cadence behind `POLYMARKET_WS_MAX_SILENCE` was measured.
//! Places no orders.
//! Run: cargo run --example probe_poly_book_freshness -- <slug> [<slug> ...]
//! Env: PROBE_SECONDS (default 45), PROBE_NO_REST=1 to watch the WS only.

use std::time::{Duration, Instant};

use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_ws::{run_markets_stream, PolyWsEvent};
use polytaoli::config::Config;
use serde_json::Value;

fn best(levels: &Value) -> String {
    levels
        .as_array()
        .and_then(|levels| levels.first())
        .and_then(|level| level["px"]["value"].as_str())
        .unwrap_or("-")
        .to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let slugs: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!slugs.is_empty(), "pass at least one market slug");
    let config = Config::from_file("config.toml")?;
    let (key_id, secret) = resolve_credentials(&config.polymarket)?;
    let base = config.polymarket.base_url.trim_end_matches('/').to_string();
    let started = Instant::now();

    let ws_slugs = slugs.clone();
    let ws = tokio::spawn(run_markets_stream(
        key_id,
        secret,
        move || ws_slugs.clone(),
        move |event| match event {
            PolyWsEvent::Book(book) => println!(
                "{:6.1}s WS     {:<38} bid={:?} offer={:?} transactTime={:?}",
                started.elapsed().as_secs_f64(),
                book.market_slug,
                book.bids.first().map(|l| l.price),
                book.offers.first().map(|l| l.price),
                book.transact_time
            ),
            PolyWsEvent::Quote(_) => {}
            other => println!("{:6.1}s WS     {:?}", started.elapsed().as_secs_f64(), other),
        },
    ));

    let http = reqwest::Client::new();
    let seconds = std::env::var("PROBE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45);
    let rest_polling = std::env::var("PROBE_NO_REST").is_err();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    if !rest_polling {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
    }
    while Instant::now() < deadline {
        for slug in &slugs {
            for (label, url) in [
                ("CACHED", format!("{base}/v1/markets/{slug}/book")),
                (
                    "BUSTED",
                    format!("{base}/v1/markets/{slug}/book?cb={}", started.elapsed().as_nanos()),
                ),
            ] {
                let response = http.get(&url).send().await?;
                let cache = response
                    .headers()
                    .get("cf-cache-status")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string();
                let age = response
                    .headers()
                    .get("age")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string();
                let body: Value = response.json().await?;
                let data = &body["marketData"];
                println!(
                    "{:6.1}s {label} {:<38} bid={} offer={} cf={cache} age={age} transactTime={}",
                    started.elapsed().as_secs_f64(),
                    slug,
                    best(&data["bids"]),
                    best(&data["offers"]),
                    data["transactTime"].as_str().unwrap_or("-")
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    ws.abort();
    Ok(())
}
