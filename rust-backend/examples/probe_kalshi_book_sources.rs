//! Read-only: compares, for the given Kalshi tickers, the WebSocket book,
//! the REST order book, and the market-quote endpoint. Places no orders.
//! Run: cargo run --release --example probe_kalshi_book_sources -- <TICKER> ...

use std::time::Duration;

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tickers: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!tickers.is_empty(), "pass at least one ticker");
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    let ws = kalshi.clone();
    let ws_tickers = tickers.clone();
    tokio::spawn(async move {
        if let Err(e) = ws.connect_websocket(ws_tickers, tx).await {
            eprintln!("ws ended: {e:#}");
        }
    });
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let top = |v: &[(i32, i32)]| {
        v.iter().rev().take(3).map(|(p, q)| format!("{p}x{q}")).collect::<Vec<_>>().join(" ")
    };
    for round in 0..6 {
        tokio::time::sleep(Duration::from_secs(if round == 0 { 6 } else { 5 })).await;
        println!("--- round {round}");
        for t in &tickers {
            let ws = kalshi.get_orderbook(t);
            let rest = kalshi.get_rest_orderbook(t).await;
            let quote = kalshi.get_market_quotes(std::slice::from_ref(t)).await;
            println!("{t}");
            match ws {
                Some(b) => println!("  WS   yes_bids[{}] no_bids[{}]", top(&b.yes), top(&b.no)),
                None => println!("  WS   none"),
            }
            match rest {
                Ok(b) => println!("  REST yes_bids[{}] no_bids[{}]", top(&b.yes), top(&b.no)),
                Err(e) => println!("  REST err {e:#}"),
            }
            match quote {
                Ok(q) => println!("  QUOTE {:?}", q),
                Err(e) => println!("  QUOTE err {e:#}"),
            }
        }
    }
    Ok(())
}
