//! Kalshi orders and portfolio: V2 IOC order payloads, fill parsing, orders, positions, fills, settlements.

use super::*;

/// A parsed immediate-or-cancel order acknowledgement. A submitted order is
/// only considered filled when Kalshi returned an explicit positive fill count.
#[derive(Debug, Clone)]
pub struct KalshiOrderResult {
    pub accepted: bool,
    pub filled_contracts: i32,
    pub fill_quantity_valid: bool,
    /// Kalshi reported a fractional fill. `filled_contracts` is the whole part,
    /// floored; the remainder is unhedgeable dust the caller must record.
    pub fractional_fill: bool,
    pub order_id: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub average_fill_price: Option<f64>,
}

pub(super) fn parse_kalshi_order_result(response: &Value, outcome: &str) -> KalshiOrderResult {
    let order = response.get("order").unwrap_or(response);
    let order_id = order
        .get("order_id")
        .or_else(|| order.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let status = order
        .get("status")
        .or_else(|| order.get("state"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let reported_fill = [
        "fill_count_fp",
        "fill_count",
        "filled_count",
        "filled_contracts",
    ]
    .iter()
    .find_map(|field| order.get(*field).and_then(value_to_f64));
    let fill_quantity_valid = reported_fill
        .map(|value| value.is_finite() && value >= 0.0 && value.floor() <= i32::MAX as f64)
        .unwrap_or(true);
    let fractional_fill =
        fill_quantity_valid && reported_fill.is_some_and(|value| value.fract() != 0.0);
    let filled_contracts = reported_fill
        .filter(|_| fill_quantity_valid)
        .map(|value| value.floor() as i32)
        .unwrap_or(0);
    let average_fill_price = ["average_fill_price", "average_price", "avg_price"]
        .iter()
        .find_map(|field| order.get(*field).and_then(value_to_price))
        .and_then(|price| match outcome {
            "yes" => Some(price),
            "no" => Some(1.0 - price),
            _ => None,
        });
    let error = response
        .get("error")
        .or_else(|| response.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            (!fill_quantity_valid)
                .then(|| "Kalshi returned an invalid fill quantity".to_string())
        })
        .or_else(|| {
            fractional_fill.then(|| {
                format!(
                    "Kalshi filled a fractional quantity ({}); the fraction is unhedged",
                    reported_fill.unwrap_or_default()
                )
            })
        })
        .or_else(|| {
            (filled_contracts == 0).then(|| {
                format!(
                    "Kalshi IOC order has no explicit fill (status: {})",
                    status.as_deref().unwrap_or("unknown")
                )
            })
        });
    KalshiOrderResult {
        accepted: order_id.is_some() || status.is_some(),
        filled_contracts,
        fill_quantity_valid,
        fractional_fill,
        order_id,
        status,
        error,
        average_fill_price,
    }
}

pub(super) fn value_to_f64(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

pub(super) fn value_to_price(value: &Value) -> Option<f64> {
    let value = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (value.is_finite() && (0.0..1.0).contains(&value)).then_some(value)
}

pub(super) fn v2_order_book_side_and_price(
    action: &str,
    outcome: &str,
    price: i32,
) -> Result<(&'static str, i32)> {
    if !(1..=99).contains(&price) {
        bail!("Kalshi order price must be between 1 and 99 cents");
    }

    match (action, outcome) {
        ("buy", "yes") => Ok(("bid", price)),
        ("sell", "yes") => Ok(("ask", price)),
        ("buy", "no") => Ok(("ask", 100 - price)),
        ("sell", "no") => Ok(("bid", 100 - price)),
        _ => bail!("Kalshi order action must be buy or sell and outcome must be yes or no"),
    }
}

pub(super) fn v2_order_payload(
    ticker: &str,
    book_side: &str,
    count: i32,
    yes_price_cents: i32,
) -> Result<Value> {
    v2_order_payload_with_id(
        ticker,
        book_side,
        count,
        yes_price_cents,
        &uuid::Uuid::new_v4().to_string(),
    )
}

pub(super) fn v2_order_payload_with_id(
    ticker: &str,
    book_side: &str,
    count: i32,
    yes_price_cents: i32,
    client_order_id: &str,
) -> Result<Value> {
    if ticker.trim().is_empty()
        || client_order_id.trim().is_empty()
        || count <= 0
        || !matches!(book_side, "bid" | "ask")
        || !(1..=99).contains(&yes_price_cents)
    {
        bail!("Invalid Kalshi V2 order parameters");
    }

    Ok(json!({
        "ticker": ticker,
        "client_order_id": client_order_id,
        "side": book_side,
        "count": format!("{count}.00"),
        "price": format!("{:.4}", yes_price_cents as f64 / 100.0),
        "time_in_force": "immediate_or_cancel",
        "self_trade_prevention_type": "taker_at_cross",
        "cancel_order_on_pause": true,
    }))
}

impl KalshiClient {
    /// Place a market order - fill immediately at best available prices
    pub async fn place_order(
        &self,
        ticker: &str,
        action: &str,
        outcome: &str,
        count: i32,
        price: i32,
    ) -> Result<Value> {
        let (book_side, yes_price_cents) = v2_order_book_side_and_price(action, outcome, price)?;
        let body = v2_order_payload(ticker, book_side, count, yes_price_cents)?;

        self.post("/portfolio/events/orders", &body).await
    }

    pub async fn place_order_with_client_order_id(
        &self,
        ticker: &str,
        action: &str,
        outcome: &str,
        count: i32,
        price: i32,
        client_order_id: &str,
    ) -> Result<Value> {
        let (book_side, yes_price_cents) = v2_order_book_side_and_price(action, outcome, price)?;
        let body =
            v2_order_payload_with_id(ticker, book_side, count, yes_price_cents, client_order_id)?;
        self.post("/portfolio/events/orders", &body).await
    }

    /// Submit an IOC order and parse only explicit fill data from the
    /// acknowledgement. Unknown acknowledgement shapes fail closed.
    pub async fn submit_order(
        &self,
        ticker: &str,
        action: &str,
        outcome: &str,
        count: i32,
        price: i32,
    ) -> Result<KalshiOrderResult> {
        let response = self
            .place_order(ticker, action, outcome, count, price)
            .await?;
        Ok(parse_kalshi_order_result(&response, outcome))
    }

    pub async fn submit_order_with_client_order_id(
        &self,
        ticker: &str,
        action: &str,
        outcome: &str,
        count: i32,
        price: i32,
        client_order_id: &str,
    ) -> Result<KalshiOrderResult> {
        let response = self
            .place_order_with_client_order_id(
                ticker,
                action,
                outcome,
                count,
                price,
                client_order_id,
            )
            .await?;
        Ok(parse_kalshi_order_result(&response, outcome))
    }

    /// Get orders with optional status filter
    pub async fn get_orders(&self, status: Option<&str>) -> Result<Value> {
        let path = if let Some(s) = status {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("status", s);
            format!("/portfolio/orders?{}", query.finish())
        } else {
            "/portfolio/orders".to_string()
        };

        self.get(&path).await
    }

    /// Kalshi returns `client_order_id` on each order but does not expose it
    /// as a server-side filter. Page the documented time-window query and
    /// match the exact id locally. Absence is not proof of non-submission when
    /// the scan fails or the order has moved to historical storage.
    pub async fn get_order_by_client_order_id(
        &self,
        client_order_id: &str,
        min_timestamp: i64,
    ) -> Result<Option<Value>> {
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let path = {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                query.append_pair("min_ts", &min_timestamp.to_string());
                query.append_pair("limit", "1000");
                if let Some(value) = cursor.as_deref() {
                    query.append_pair("cursor", value);
                }
                format!("/portfolio/orders?{}", query.finish())
            };
            let response = self.get(&path).await?;
            let orders = response["orders"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid Kalshi orders response"))?;
            if let Some(order) = orders
                .iter()
                .find(|order| order["client_order_id"].as_str() == Some(client_order_id))
            {
                return Ok(Some(order.clone()));
            }
            let next = response["cursor"].as_str().unwrap_or_default();
            if next.is_empty() || orders.is_empty() {
                return Ok(None);
            }
            cursor = Some(next.to_string());
        }
        anyhow::bail!("Kalshi order lookup exceeded pagination safety limit")
    }

    /// Get positions
    pub async fn get_positions(&self) -> Result<Value> {
        self.get("/portfolio/positions").await
    }

    /// Fills (executions) for one ticker, newest first. Read-only; the exact
    /// price, count, side and fee of every fill, unlike the order echo.
    pub async fn get_fills(&self, ticker: &str) -> Result<Value> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("ticker", ticker);
        query.append_pair("limit", "200");
        self.get(&format!("/portfolio/fills?{}", query.finish())).await
    }

    /// Settlements (payout per resolved market). Read-only.
    pub async fn get_settlements(&self) -> Result<Value> {
        self.get("/portfolio/settlements?limit=200").await
    }

    /// Cancel an order
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let timestamp = Self::get_timestamp_ms();
        let path = format!("/portfolio/events/orders/{}", order_id);
        let sign_path = Self::signing_path(&path);
        let signature = self.sign_request(timestamp, "DELETE", &sign_path);

        let url = format!("{}{}", self.config.base_url, path);

        let response = self
            .http
            .delete(&url)
            .header("KALSHI-ACCESS-KEY", &self.config.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &signature)
            .header("KALSHI-ACCESS-TIMESTAMP", timestamp.to_string())
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("Kalshi API error {}: {}", status, body);
        }

        serde_json::from_str(&body).with_context(|| format!("Failed to parse response: {}", body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_treat_ioc_acknowledgement_as_fill() {
        let result = parse_kalshi_order_result(
            &json!({
                "order": {"order_id": "order-1", "status": "canceled", "fill_count": "0.00"}
            }),
            "yes",
        );
        assert!(result.accepted);
        assert_eq!(result.filled_contracts, 0);
        assert!(result.error.unwrap().contains("no explicit fill"));
    }

    #[test]
    fn parses_current_fixed_point_ioc_fill_count() {
        let result = parse_kalshi_order_result(
            &json!({
                "order": {
                    "order_id": "order-1",
                    "status": "executed",
                    "fill_count_fp": "2.00"
                }
            }),
            "yes",
        );

        assert_eq!(result.filled_contracts, 2);
        assert!(result.fill_quantity_valid);
    }

    #[test]
    fn a_fractional_kalshi_fill_reports_the_whole_part_and_flags_the_fraction() {
        let result = parse_kalshi_order_result(
            &json!({"order": {"order_id": "o", "status": "executed", "fill_count_fp": "2.50"}}),
            "yes",
        );
        assert_eq!(result.filled_contracts, 2);
        assert!(result.fill_quantity_valid);
        assert!(result.fractional_fill);
        assert!(result.error.unwrap().contains("2.5"));
    }

    #[test]
    fn converts_no_order_fill_price_from_the_yes_book() {
        let result = parse_kalshi_order_result(
            &json!({
                "order": {
                    "order_id": "order-1",
                    "fill_count_fp": "1.00",
                    "avg_price": "0.6700"
                }
            }),
            "no",
        );

        assert!(
            (result.average_fill_price.unwrap() - 0.33).abs() < f64::EPSILON,
            "expected a 33-cent NO fill price"
        );
    }

    #[test]
    fn converts_no_side_orders_to_v2_yes_book_orders() {
        assert_eq!(
            v2_order_book_side_and_price("buy", "no", 44).unwrap(),
            ("ask", 56)
        );
        assert_eq!(
            v2_order_book_side_and_price("sell", "no", 44).unwrap(),
            ("bid", 56)
        );

        let payload = v2_order_payload("KXATPMATCH-26AUG30DEJPAS-DEJ", "ask", 3, 56).unwrap();
        assert_eq!(payload["side"], "ask");
        assert_eq!(payload["count"], "3.00");
        assert_eq!(payload["price"], "0.5600");
        assert_eq!(payload["time_in_force"], "immediate_or_cancel");
        assert_eq!(payload["self_trade_prevention_type"], "taker_at_cross");
        assert!(payload.get("exchange_index").is_none());
        assert!(payload["client_order_id"].as_str().is_some());
    }
}
