//! Throwaway read-only probe: compare Polymarket US WebSocket quotes with
//! the REST gateway feed for real sports markets.
//! Run: cargo run --example probe_polymarket_ws
use std::collections::HashMap;

use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_ws::run_markets_stream;
use polytaoli::clients::PolymarketClient;
use polytaoli::config::Config;
use polytaoli::models::PolymarketPositionSide;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let (key_id, secret) = resolve_credentials(&config.polymarket)?;
    let client = PolymarketClient::new(config.polymarket)?;
    let (_, markets) = client.get_supported_events_and_markets().await?;
    let mut sample: HashMap<String, (f64, f64, PolymarketPositionSide, PolymarketPositionSide)> = HashMap::new();
    for m in markets.iter().take(400) {
        sample.entry(m.market_slug.clone()).or_insert((m.price_a, m.price_b, m.team_a_position, m.team_b_position));
        if sample.len() >= 12 { break; }
    }
    let slugs: Vec<String> = sample.keys().cloned().collect();
    println!("subscribing to {} slugs", slugs.len());
    let seen = std::sync::Mutex::new(std::collections::HashSet::new());
    let sample2 = sample.clone();
    let stream = run_markets_stream(key_id, secret, move || slugs.clone(), move |q| {
        let (pa, pb, sa, sb) = sample2[&q.slug];
        let side = |s: PolymarketPositionSide| if s == PolymarketPositionSide::Long { q.long_quote } else { q.short_quote };
        if seen.lock().unwrap().insert(q.slug.clone()) {
            println!("{:<45} ws a={:.2} b={:.2} | rest a={:.2} b={:.2}", q.slug, side(sa), side(sb), pa, pb);
        }
    });
    let _ = tokio::time::timeout(std::time::Duration::from_secs(25), stream).await;
    Ok(())
}
