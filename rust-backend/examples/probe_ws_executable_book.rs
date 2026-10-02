//! Read-only end-to-end check of the execution book path: stream the real
//! Polymarket US markets WebSocket into a `WebSocketManager` exactly as the
//! app does, then repeatedly compare `fresh_polymarket_book` (what
//! execution now sizes from) with an origin-fresh REST book for the given
//! slugs. Matching best levels confirm the WS book is current. Places no
//! orders. Run:
//!   cargo run --example probe_ws_executable_book -- <slug> [<slug> ...]

use std::sync::Arc;
use std::time::Duration;

use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_ws::{run_markets_stream, PolyWsEvent};
use polytaoli::clients::PolymarketClient;
use polytaoli::config::Config;
use polytaoli::services::websocket_manager::POLYMARKET_WS_MAX_SILENCE;
use polytaoli::services::{ArbitrageStorage, PerformanceMetrics, WebSocketManager};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let slugs: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!slugs.is_empty(), "pass at least one market slug");
    let config = Config::from_file("config.toml")?;
    let (key_id, secret) = resolve_credentials(&config.polymarket)?;
    let client = PolymarketClient::new(config.polymarket)?;
    let manager = Arc::new(WebSocketManager::new(
        1.0,
        10.0,
        1.0,
        Arc::new(ArbitrageStorage::new(":memory:")?),
        Arc::new(PerformanceMetrics::new()),
        Duration::from_secs(10),
    ));

    let ws_manager = manager.clone();
    let ws_slugs = slugs.clone();
    tokio::spawn(run_markets_stream(
        key_id,
        secret,
        move || ws_slugs.clone(),
        move |event| match event {
            PolyWsEvent::Book(book) => ws_manager.update_polymarket_ws_book(book),
            PolyWsEvent::Connected => ws_manager.on_polymarket_ws_connected(),
            PolyWsEvent::Heartbeat => ws_manager.note_polymarket_ws_frame(),
            PolyWsEvent::Disconnected => ws_manager.on_polymarket_ws_disconnected(),
            PolyWsEvent::Quote(_) => {}
        },
    ));

    let best = |levels: &[polytaoli::clients::PolymarketBookLevel]| levels.first().map(|l| l.price);
    let (mut agree, mut differ, mut missing) = (0, 0, 0);
    for round in 0..8 {
        tokio::time::sleep(Duration::from_secs(if round == 0 { 3 } else { 4 })).await;
        for slug in &slugs {
            let ws = manager.fresh_polymarket_book(slug, POLYMARKET_WS_MAX_SILENCE);
            let rest = client.get_market_book(slug).await;
            match (&ws, &rest) {
                (Some(ws), Ok(rest)) => {
                    let same = best(&ws.bids) == best(&rest.bids)
                        && best(&ws.offers) == best(&rest.offers);
                    if same { agree += 1 } else { differ += 1 }
                    println!(
                        "{slug:<40} ws bid/offer {:?}/{:?} (state {}) | origin rest {:?}/{:?} | {}",
                        best(&ws.bids),
                        best(&ws.offers),
                        ws.state,
                        best(&rest.bids),
                        best(&rest.offers),
                        if same { "match" } else { "DIFFERENT (moved between reads?)" }
                    );
                }
                (None, _) => {
                    missing += 1;
                    println!("{slug:<40} no fresh WS book (execution would use REST)");
                }
                (_, Err(error)) => println!("{slug:<40} REST error: {error:#}"),
            }
        }
    }
    println!("summary: {agree} matching, {differ} different, {missing} without a fresh WS book");
    Ok(())
}
