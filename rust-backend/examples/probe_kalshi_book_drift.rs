//! Read-only: subscribe the given Kalshi tickers on one WebSocket connection
//! (as the bot does), then repeatedly compare each cached WS book's best
//! bids with the REST order book and report how many disagree. Places no
//! orders. Pass `--watch T1,T2` before the tickers to print those books.
//! Run: cargo run --release --example probe_kalshi_book_drift -- [--watch A,B] <TICKER> ...

use std::time::{Duration, Instant};

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut watch: Vec<String> = Vec::new();
    if args.first().map(String::as_str) == Some("--watch") {
        watch = args[1].split(',').map(str::to_string).collect();
        args.drain(..2);
    }
    let tickers = args;
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

    let best = |v: &[(i32, i32)]| v.last().copied();
    let start = Instant::now();
    tokio::time::sleep(Duration::from_secs(10)).await;
    for round in 0..8 {
        let (mut same, mut diff, mut missing) = (0, 0, 0);
        let mut examples = Vec::new();
        for t in &tickers {
            let Some(ws) = kalshi.get_orderbook(t) else {
                missing += 1;
                continue;
            };
            let Ok(rest) = kalshi.get_rest_orderbook(t).await else { continue };
            let w = (best(&ws.yes).map(|l| l.0), best(&ws.no).map(|l| l.0));
            let r = (best(&rest.yes).map(|l| l.0), best(&rest.no).map(|l| l.0));
            if w == r {
                same += 1;
            } else {
                diff += 1;
                if examples.len() < 6 {
                    examples.push(format!("{t} ws {w:?} rest {r:?}"));
                }
            }
            if watch.contains(t) {
                println!("  WATCH {t}\n    WS   yes {:?}\n    WS   no  {:?}\n    REST yes {:?}\n    REST no  {:?}",
                    ws.yes.iter().rev().take(5).collect::<Vec<_>>(), ws.no.iter().rev().take(5).collect::<Vec<_>>(),
                    rest.yes.iter().rev().take(5).collect::<Vec<_>>(), rest.no.iter().rev().take(5).collect::<Vec<_>>());
            }
        }
        println!("round {round} t+{:?}: same {same} diff {diff} no-ws-book {missing}", start.elapsed().as_secs());
        for e in examples {
            println!("  {e}");
        }
    }
    Ok(())
}
