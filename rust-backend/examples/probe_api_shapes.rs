//! Read-only: record the JSON *shape* (keys and value types, never values)
//! of every GET endpoint and every WebSocket message type the running
//! backend serves, so a refactor can prove it kept the frontend contract.
//! Sends only GET requests and one WebSocket connect. Places no orders.
//!
//! Run (backend must be up on 127.0.0.1:8000):
//!   cargo run --example probe_api_shapes > before.json
//!   ...refactor, restart...
//!   cargo run --example probe_api_shapes > after.json
//!   cargo run --example probe_api_shapes -- compare before.json after.json
//!
//! `compare` exits non-zero on a real contract change: a key added or
//! removed, or a value type changed. "No data seen" (an empty array, a
//! field that was only ever null, a missing WS message type) is a wildcard,
//! because it depends on what the markets were doing, not on the code.
//!
//! Optional: PROBE_BASE (default http://127.0.0.1:8000), PROBE_WS_SECS (default 15).

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Every GET route the frontend can call. Paths with parameters use a
/// harmless example value. Keep in sync with `api/mod.rs`.
const GET_PATHS: &[&str] = &[
    "/api/health",
    "/api/stats",
    "/api/data-coverage",
    "/api/opportunities",
    "/api/matched-markets",
    "/api/arbitrage-history",
    "/api/balance/kalshi",
    "/api/balance/polymarket",
    "/api/account-balance",
    "/api/orders/kalshi",
    "/api/orders/polymarket",
    "/api/positions/kalshi",
    "/api/positions/polymarket",
    "/api/positions/unified",
    "/api/tracking",
    "/api/history/search",
    "/api/history/statistics",
    "/api/orderbook/depth",
    "/api/auto-trade/status",
    "/api/auto-trade/pnl",
    "/api/auto-trade/queue",
    "/api/auto-trade/history",
    "/api/auto-trade/excluded",
    "/api/mention/status",
    "/api/mention/signals",
    "/api/mention/history",
    "/api/mention/decisions",
    "/api/settings",
];

/// Type signature of a JSON value. Arrays merge the shapes of all their
/// elements, so the result does not depend on how many rows exist; an empty
/// array is `[]` and matches nothing more specific.
fn shape(value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => {
            let merged = items.iter().map(shape).reduce(merge);
            json!([merged.unwrap_or(Value::Null)])
        }
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().map(|(k, v)| (k.clone(), shape(v))).collect();
            Value::Object(sorted.into_iter().collect::<Map<_, _>>())
        }
    }
}

/// Union of two shapes: objects merge key by key; differing scalars become
/// a sorted "a|b" alternative (e.g. "null|number" for an optional field).
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Object(mut x), Value::Object(y)) => {
            for (k, v) in y {
                let merged = match x.remove(&k) {
                    Some(existing) => merge(existing, v),
                    None => v,
                };
                x.insert(k, merged);
            }
            let sorted: BTreeMap<_, _> = x.into_iter().collect();
            Value::Object(sorted.into_iter().collect())
        }
        (Value::Array(x), Value::Array(y)) => {
            let inner = match (x.into_iter().next(), y.into_iter().next()) {
                (Some(Value::Null), Some(v)) | (Some(v), Some(Value::Null)) | (Some(v), None) | (None, Some(v)) => v,
                (Some(p), Some(q)) => merge(p, q),
                (None, None) => Value::Null,
            };
            json!([inner])
        }
        (x, y) if x == y => x,
        (x, y) => {
            let mut parts: Vec<String> = [x, y]
                .iter()
                .flat_map(|v| match v {
                    Value::String(s) => s.split('|').map(str::to_string).collect::<Vec<_>>(),
                    other => vec![other.to_string()],
                })
                .collect();
            parts.sort();
            parts.dedup();
            json!(parts.join("|"))
        }
    }
}

/// Non-null alternatives of a scalar shape ("null|number" -> {"number"}).
fn scalar_types(s: &str) -> Vec<&str> {
    s.split('|').filter(|t| *t != "null").collect()
}

fn compare(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Null, _) | (_, Value::Null) => {}
        (Value::Number(x), Value::Number(y)) => {
            if x != y {
                out.push(format!("{path}: {x} -> {y}"));
            }
        }
        (Value::String(x), Value::String(y)) => {
            let (tx, ty) = (scalar_types(x), scalar_types(y));
            if !tx.is_empty() && !ty.is_empty() && tx != ty {
                out.push(format!("{path}: type {x} -> {y}"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if let (Some(p), Some(q)) = (x.first(), y.first()) {
                compare(&format!("{path}[]"), p, q, out);
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            // A WS message type only appears when something triggers it.
            let presence_matters = path != ".ws";
            for (k, v) in x {
                match y.get(k) {
                    Some(w) => compare(&format!("{path}.{k}"), v, w, out),
                    None if presence_matters => out.push(format!("{path}.{k}: removed")),
                    None => {}
                }
            }
            for k in y.keys().filter(|k| presence_matters && !x.contains_key(*k)) {
                out.push(format!("{path}.{k}: added"));
            }
        }
        (Value::String(x), _) | (_, Value::String(x)) if x == "null" => {}
        (x, y) => out.push(format!("{path}: shape {} -> {}", kind(x), kind(y))),
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        _ => "scalar",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("compare") {
        let read = |p: &String| -> anyhow::Result<Value> { Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?) };
        let (before, after) = (read(&args[2])?, read(&args[3])?);
        let mut diffs = Vec::new();
        compare("", &before, &after, &mut diffs);
        for d in &diffs {
            println!("{d}");
        }
        println!("{} contract differences", diffs.len());
        std::process::exit(if diffs.is_empty() { 0 } else { 1 });
    }
    let base = std::env::var("PROBE_BASE").unwrap_or_else(|_| "http://127.0.0.1:8000".into());
    let ws_secs: u64 = std::env::var("PROBE_WS_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(15);
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;

    let mut http_shapes = BTreeMap::new();
    for path in GET_PATHS {
        let entry = match http.get(format!("{base}{path}")).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.json::<Value>().await {
                    Ok(body) => json!({ "status": status, "shape": shape(&body) }),
                    Err(_) => json!({ "status": status, "shape": "non-json" }),
                }
            }
            Err(e) => json!({ "error": e.to_string() }),
        };
        http_shapes.insert(path.to_string(), entry);
    }

    // WebSocket: merge the shape of every message seen per `type`.
    let ws_url = format!("{}/ws", base.replacen("http", "ws", 1));
    let mut ws_shapes: BTreeMap<String, Value> = BTreeMap::new();
    let (mut stream, _) = connect_async(ws_url.as_str()).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(ws_secs);
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, stream.next()).await {
        if let Message::Text(text) = msg {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                let kind = value["type"].as_str().unwrap_or("<no type>").to_string();
                let s = shape(&value);
                let merged = match ws_shapes.remove(&kind) {
                    Some(existing) => merge(existing, s),
                    None => s,
                };
                ws_shapes.insert(kind, merged);
            }
        }
    }

    let out = json!({ "http": http_shapes, "ws": ws_shapes });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
