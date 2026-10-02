//! Read-only diagnostic: signed GET against the authenticated Polymarket US
//! API for each path given on the command line (e.g. `/v1/order/<id>`).
//! The method is hard-coded to GET, so this can never place or cancel an
//! order. Bulky `marketMetadata` blobs are stripped from the output.
//! Run with: cargo run --example probe_polymarket_get -- /v1/order/<id> ...

use anyhow::Context;
use polytaoli::clients::polymarket::resolve_credentials;
use polytaoli::clients::polymarket_auth::sign_request;
use polytaoli::config::Config;
use serde_json::Value;

fn strip_metadata(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("marketMetadata");
            map.values_mut().for_each(strip_metadata);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_metadata),
        _ => {}
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_file("config.toml")?;
    let (key_id, secret_key) = resolve_credentials(&config.polymarket)?;
    let base = config.polymarket.api_base_url.trim_end_matches('/').to_string();
    let http = reqwest::Client::new();

    for path_and_query in std::env::args().skip(1) {
        // Only the path is signed, never the query string.
        let path = path_and_query.split('?').next().unwrap_or_default();
        let headers = sign_request(&key_id, &secret_key, "GET", path)?;
        let response = http
            .get(format!("{base}{path_and_query}"))
            .header("X-PM-Access-Key", headers.access_key)
            .header("X-PM-Timestamp", headers.timestamp)
            .header("X-PM-Signature", headers.signature)
            .send()
            .await
            .with_context(|| format!("GET {path_and_query} failed"))?;
        let status = response.status();
        let text = response.text().await?;
        println!("== GET {path_and_query} -> {status}");
        match serde_json::from_str::<Value>(&text) {
            Ok(mut json) => {
                strip_metadata(&mut json);
                println!("{}", serde_json::to_string_pretty(&json)?);
            }
            Err(_) => println!("{}", &text[..text.len().min(2000)]),
        }
    }
    Ok(())
}
