//! Read-only: print Kalshi's raw balance response, including the per-shard
//! `balance_breakdown`. Places no orders.
//! Run: cargo run --release --example probe_kalshi_balance

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    println!("{}", serde_json::to_string_pretty(&kalshi.get_balance_details().await?)?);
    Ok(())
}
