//! Read-only diagnostic: prints Kalshi orders (any status) whose id contains
//! an argument. Only GET calls; cannot place orders.
//! Run with: cargo run --release --example probe_kalshi_orders -- <ORDER_ID> ...

use polytaoli::clients::kalshi::KalshiClient;
use polytaoli::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let kalshi = KalshiClient::new(config.kalshi.clone())?;
    let orders = kalshi.get_orders(None).await?;
    let list = orders["orders"].as_array().cloned().unwrap_or_default();
    println!("TOTAL ORDERS: {}", list.len());
    for id in std::env::args().skip(1) {
        for order in list.iter().filter(|o| o.to_string().contains(&id)) {
            println!("ORDER {id}: {order}");
        }
    }
    Ok(())
}
