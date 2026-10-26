//! Read-only: opens a separate Kalshi WebSocket for the given tickers and
//! prints the client's 30s feed-lag lines, to compare a small connection
//! against the main one. Places no orders.
//! Run: cargo run --release --example probe_kalshi_ws_lag -- <TICKER> ...

use std::time::Duration;

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let tickers: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!tickers.is_empty(), "pass at least one ticker");
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let ws = kalshi.clone();
    tokio::spawn(async move {
        if let Err(e) = ws.connect_websocket(tickers, tx).await {
            eprintln!("ws ended: {e:#}");
        }
    });
    tokio::time::sleep(Duration::from_secs(155)).await;
    Ok(())
}
