//! Read-only probe: for the given Polymarket US slugs, compare the WebSocket
//! lite quote (what detection uses), the WebSocket full book, and the
//! origin-fresh REST book (what sizing uses).
//! Run: cargo run --example probe_poly_crypto_quotes -- <slug> [<slug>...]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_ws::{run_markets_stream, PolyWsEvent};
use polytaoli::clients::PolymarketClient;
use polytaoli::config::Config;

#[derive(Default, Clone, Debug)]
struct Seen {
    lite_long: Option<f64>,
    lite_short: Option<f64>,
    book_bid: Option<f64>,
    book_offer: Option<f64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let slugs: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!slugs.is_empty(), "pass at least one slug");
    let config = Config::from_file("config.toml")?;
    let (key_id, secret) = resolve_credentials(&config.polymarket)?;
    let client = PolymarketClient::new(config.polymarket.clone())?;
    let seen: Arc<Mutex<HashMap<String, Seen>>> = Arc::default();

    let ws_seen = seen.clone();
    let ws_slugs = slugs.clone();
    tokio::spawn(run_markets_stream(config.polymarket.ws, key_id, secret, move || ws_slugs.clone(), move |event| {
        let mut seen = ws_seen.lock().unwrap();
        match event {
            PolyWsEvent::Quote(q) => {
                let s = seen.entry(q.slug.clone()).or_default();
                s.lite_long = Some(q.long_quote);
                s.lite_short = Some(q.short_quote);
            }
            PolyWsEvent::Book(b) => {
                let s = seen.entry(b.market_slug.clone()).or_default();
                s.book_bid = b.bids.iter().map(|l| l.price).fold(None, |a: Option<f64>, p| Some(a.map_or(p, |a| a.max(p))));
                s.book_offer = b.offers.iter().map(|l| l.price).fold(None, |a: Option<f64>, p| Some(a.map_or(p, |a| a.min(p))));
            }
            _ => {}
        }
    }));

    tokio::time::sleep(Duration::from_secs(5)).await;
    for _ in 0..40 {
        for slug in &slugs {
            let origin = client.get_market_book(slug).await;
            let ws = seen.lock().unwrap().get(slug).cloned().unwrap_or_default();
            let (o_bid, o_offer) = match &origin {
                Ok(b) => (
                    b.bids.iter().map(|l| l.price).fold(None, |a: Option<f64>, p| Some(a.map_or(p, |a| a.max(p)))),
                    b.offers.iter().map(|l| l.price).fold(None, |a: Option<f64>, p| Some(a.map_or(p, |a| a.min(p)))),
                ),
                Err(e) => {
                    println!("{slug}: origin error {e:#}");
                    (None, None)
                }
            };
            println!(
                "{} {:<42} lite long/short {:?}/{:?} (=> bid {:?}) | ws book bid/offer {:?}/{:?} | origin bid/offer {:?}/{:?}",
                chrono::Utc::now().format("%H:%M:%S%.3f"),
                slug,
                ws.lite_long,
                ws.lite_short,
                ws.lite_short.map(|s| ((1.0 - s) * 100.0).round() / 100.0),
                ws.book_bid,
                ws.book_offer,
                o_bid,
                o_offer
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Ok(())
}
