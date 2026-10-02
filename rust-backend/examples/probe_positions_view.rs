//! Read-only: print the raw held legs on both exchanges (market, side,
//! size) exactly as `/api/positions/unified` sees them, so a pairing
//! question ("why does this show as unhedged?") can be answered from real
//! data. Places no orders and does not touch the app's database.
//! Run: cargo run --example probe_positions_view

use polytaoli::clients::{KalshiClient, PolymarketClient};
use polytaoli::config::Config;
use polytaoli::services::positions::{kalshi_leg, polymarket_leg};
use serde_json::Value;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    let poly = PolymarketClient::new(config.polymarket.clone())?;

    println!("== Kalshi legs");
    let raw = kalshi.get_positions().await?;
    for p in raw["market_positions"].as_array().cloned().unwrap_or_default() {
        if let Some(leg) = kalshi_leg(&p, None) {
            println!("  {} {} x{} cost {:.2}", leg.market_id, leg.side, leg.contracts, leg.cost);
        }
    }
    println!("== Polymarket legs");
    if let Value::Array(items) = poly.get_positions().await? {
        for p in items.iter().filter_map(polymarket_leg) {
            println!("  {} {} x{} cost {:.2}", p.market_id, p.side, p.contracts, p.cost);
        }
    }
    Ok(())
}
