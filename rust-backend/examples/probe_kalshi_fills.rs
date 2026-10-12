//! Read-only diagnostic: prints Kalshi fills for each ticker given on the
//! command line, then settlements. Only GET calls; cannot place orders.
//! Run with: cargo run --release --example probe_kalshi_fills -- <TICKER> ...

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    for ticker in std::env::args().skip(1) {
        let fills = kalshi.get_fills(&ticker).await?;
        println!("FILLS {ticker}: {}", serde_json::to_string(&fills["fills"])?);
    }
    let settlements = kalshi.get_settlements().await?;
    println!("SETTLEMENTS: {}", serde_json::to_string(&settlements["settlements"])?);
    Ok(())
}
