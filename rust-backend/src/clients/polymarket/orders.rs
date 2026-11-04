//! Polymarket US orders and portfolio: YES-book pricing, IOC submission, fill parsing and confirmation, positions.

use super::*;

/// Result returned by the official SDK execution envelope, normalized by the
/// local Python service. A submission without a positive fill is not success.
#[derive(Debug, Clone)]
pub struct PolymarketOrderResult {
    pub accepted: bool,
    pub filled_contracts: i32,
    pub fill_quantity_valid: bool,
    /// The exchange filled a fractional quantity (e.g. 8.85 on a partial IOC
    /// fill of a fractional book level). `filled_contracts` is then the whole
    /// part, floored; the remainder is unhedgeable dust the caller must record.
    pub fractional_fill: bool,
    pub order_id: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub latency_ms: Option<i64>,
    pub average_fill_price: Option<f64>,
    /// Taker commission Polymarket actually collected on this order
    /// (`order.commissionNotionalTotalCollected`), in dollars. Only present
    /// on a fill; used to check the fee model in `core::fees` against the
    /// exchange's own number.
    pub commission_collected: Option<f64>,
}

/// Position data for aggregation (internal use)
#[allow(dead_code)]
pub(super) struct PositionData {
    asset_id: String,
    market: String,
    outcome: String,
    size: f64,
    total_cost: f64,
    trade_count: u32,
}

/// Convert the SDK's market-keyed position map into a frontend-safe list.
pub(super) fn normalize_positions(response: &Value) -> Result<Value> {
    let positions = response.get("positions");
    if let Some(Value::Array(items)) = positions {
        return Ok(Value::Array(items.clone()));
    }
    let Some(Value::Object(map)) = positions else {
        anyhow::bail!("Polymarket US positions response did not contain a position list or map");
    };

    let mut normalized = Vec::with_capacity(map.len());
    for (market_slug, position) in map {
        let Some(position) = position.as_object() else {
            continue;
        };
        let metadata = position.get("marketMetadata").and_then(Value::as_object);
        let title = metadata
            .and_then(|m| m.get("title"))
            .and_then(Value::as_str)
            .unwrap_or(market_slug);
        normalized.push(json!({
            "id": market_slug,
            "asset": market_slug,
            "conditionId": market_slug,
            "title": title,
            "size": position.get("netPosition").cloned().unwrap_or_else(|| json!("0")),
            "value": position.get("cashValue").and_then(amount_as_f64),
            // Total cost basis; with `cashValue` it yields avg entry, mark
            // and unrealized P&L without any extra API call. Verified on
            // live positions: `cost` = `baseCost` + `fees`, i.e. it already
            // includes the taker fees paid on the position.
            "cost": position.get("cost").and_then(amount_as_f64),
            "base_cost": position.get("baseCost").and_then(amount_as_f64),
            // Fees paid on this position, as reported by the exchange
            // (null/absent -> unknown, never a fake zero).
            "fees": position.get("fees").and_then(amount_as_f64),
            "pnl": position.get("realized").and_then(amount_as_f64),
            "outcome": metadata.and_then(|m| m.get("outcome")).cloned().unwrap_or(Value::Null),
        }));
    }
    Ok(Value::Array(normalized))
}

/// Polymarket US quotes every order price, and every fill's `avgPx`, on the
/// YES (long) book: "buy NO at $0.40" is "sell YES at $0.60"
/// (docs.polymarket.us/concepts/orders). Converts a price for
/// `position_side` to that YES-book price - and, since the conversion is its
/// own inverse, a YES-book price back to the `position_side` price.
pub fn yes_book_price(position_side: PolymarketPositionSide, price: f64) -> f64 {
    match position_side {
        PolymarketPositionSide::Long => price,
        PolymarketPositionSide::Short => 1.0 - price,
    }
}

/// Snap a limit price to the market's tick (0.0001 when unknown, which only
/// strips float noise) and render it as a plain decimal string. `1.0 - 0.56`
/// must go out as "0.44", never "0.43999999999999995" - the exchange
/// silently truncated that to 0.43.
pub fn format_order_price(price: f64, tick_size: Option<f64>) -> Result<String> {
    let tick = tick_size
        .filter(|tick| tick.is_finite() && *tick > 0.0 && *tick < 1.0)
        .unwrap_or(0.0001);
    let decimals = ((-tick.log10()) - 1e-9).ceil().clamp(0.0, 8.0) as usize;
    let snapped = (price / tick).round() * tick;
    if !snapped.is_finite() || snapped <= 0.0 || snapped >= 1.0 {
        anyhow::bail!(
            "Polymarket US limit price {} falls outside (0, 1) once snapped to tick {}",
            price,
            tick
        );
    }
    Ok(format!("{:.*}", decimals, snapped))
}

/// Explain an order that filled nothing. `orderRejectReason` is serialized on
/// *every* execution and its zero value is `ORD_REJECT_REASON_EXCHANGE_OPTION`,
/// so on an accepted order (`EXECUTION_TYPE_NEW`) or an IOC that simply
/// expired it is a default, not a reason - it only means something on an
/// actual rejection. Reporting it unconditionally made every zero-fill IOC
/// look like an exchange rejection.
pub(super) fn unfilled_order_reason(executions: &[Value], order: &Value, request_body: &Value) -> String {
    let execution_type = |execution: &Value| {
        execution
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let state = order
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let rejection = executions
        .iter()
        .find(|execution| execution_type(execution) == "EXECUTION_TYPE_REJECTED")
        .or_else(|| (state == "ORDER_STATE_REJECTED").then(|| executions.last()).flatten());
    if let Some(rejection) = rejection {
        let code = rejection
            .get("orderRejectReason")
            .and_then(Value::as_str)
            .unwrap_or("no reason given");
        return match rejection
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => format!("Polymarket US rejected the order: {} ({})", code, text),
            None => format!("Polymarket US rejected the order: {}", code),
        };
    }

    let expired = state == "ORDER_STATE_EXPIRED"
        || executions
            .iter()
            .any(|execution| execution_type(execution) == "EXECUTION_TYPE_EXPIRED");
    if expired {
        let limit = request_body["price"]["value"].as_str().unwrap_or("?");
        format!(
            "Order expired unfilled: nothing was resting at or better than its limit (YES-book price {}) when it reached the exchange",
            limit
        )
    } else {
        format!("Order received no fill (state: {})", state)
    }
}

/// Convert the raw `/v1/orders` execution envelope into an explicit order
/// result. Unlike the old Python order-service hop this replaced, a non-fill
/// logs the full outgoing request and raw response at `warn!`, so there is
/// always a record of exactly what was sent.
pub(super) fn parse_polymarket_order_result(
    response: &Value,
    latency_ms: i64,
    request_body: &Value,
    position_side: PolymarketPositionSide,
) -> PolymarketOrderResult {
    let executions = response.get("executions").and_then(Value::as_array);
    let Some(executions) = executions.filter(|executions| !executions.is_empty()) else {
        let upstream_error = ["error", "message", "detail", "orderRejectReason", "text"]
            .iter()
            .find_map(|field| response.get(*field).and_then(Value::as_str))
            .map(str::to_string);
        warn!(
            "Polymarket US order did not return an execution: request={} response={}",
            request_body, response
        );
        return PolymarketOrderResult {
            accepted: false,
            filled_contracts: 0,
            fill_quantity_valid: true,
            fractional_fill: false,
            order_id: None,
            status: None,
            error: Some(upstream_error.unwrap_or_else(|| {
                "Polymarket US did not return an order execution".to_string()
            })),
            latency_ms: Some(latency_ms),
            average_fill_price: None,
            commission_collected: None,
        };
    };

    let execution = executions.last().expect("checked non-empty above");
    let Some(order) = execution.get("order").filter(|order| order.is_object()) else {
        warn!(
            "Polymarket US execution did not include order details: request={} response={}",
            request_body, response
        );
        return PolymarketOrderResult {
            accepted: false,
            filled_contracts: 0,
            fill_quantity_valid: true,
            fractional_fill: false,
            order_id: None,
            status: None,
            error: Some("Polymarket US execution did not include order details".to_string()),
            latency_ms: Some(latency_ms),
            average_fill_price: None,
            commission_collected: None,
        };
    };

    let order_id = order.get("id").and_then(Value::as_str).map(str::to_string);
    let status = order
        .get("state")
        .or_else(|| order.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let filled = amount_as_f64(&order["cumQuantity"]).unwrap_or(0.0);

    if filled > 0.0 {
        // `avgPx` is on the YES book like the order price; report it in
        // the traded side's terms so entry prices and the neutralization
        // loss bound compare like with like.
        let average_fill_price = extract_average_fill_price(response)
            .map(|yes_price| yes_book_price(position_side, yes_price));
        let fill_quantity_valid =
            filled.is_finite() && filled >= 0.0 && filled.floor() <= i32::MAX as f64;
        let fractional_fill = fill_quantity_valid && filled.fract() != 0.0;
        info!(
            "Polymarket US order FILLED: order_id={:?} market={:?} status={:?} filled={} avg_price={:?} latency_ms={}",
            order_id,
            order.get("marketSlug"),
            status,
            filled,
            average_fill_price,
            latency_ms
        );
        return PolymarketOrderResult {
            accepted: true,
            filled_contracts: if fill_quantity_valid { filled.floor() as i32 } else { 0 },
            fill_quantity_valid,
            fractional_fill,
            order_id,
            status,
            error: if !fill_quantity_valid {
                Some("Polymarket US returned an invalid fill quantity".to_string())
            } else if fractional_fill {
                Some(format!(
                    "Polymarket US filled a fractional quantity ({filled}); the fraction is unhedged"
                ))
            } else {
                None
            },
            latency_ms: Some(latency_ms),
            average_fill_price,
            commission_collected: amount_as_f64(&order["commissionNotionalTotalCollected"])
                .filter(|fee| fee.is_finite() && *fee >= 0.0),
        };
    }

    let error = unfilled_order_reason(executions, order, request_body);
    warn!(
        "Polymarket US order not filled: order_id={:?} status={:?} error={} request={} response={}",
        order_id, status, error, request_body, response
    );
    PolymarketOrderResult {
        accepted: order_id.is_some(),
        filled_contracts: 0,
        fill_quantity_valid: true,
        fractional_fill: false,
        order_id,
        status,
        error: Some(error),
        latency_ms: Some(latency_ms),
        average_fill_price: None,
        commission_collected: None,
    }
}

pub(super) fn extract_average_fill_price(response: &Value) -> Option<f64> {
    let executions = response.get("executions")?.as_array()?;
    let order = executions.last()?.get("order")?;
    [
        "avgPx",
        "averageFillPrice",
        "averagePrice",
        "avgPrice",
        "avg_price",
    ]
    .iter()
    .find_map(|field| amount_as_f64(&order[*field]))
    .filter(|price| price.is_finite() && (0.0..1.0).contains(price))
}

/// Whole contracts filled according to a `GET /v1/order/{id}` response.
pub(super) fn filled_from_order_lookup(response: &Value) -> Result<i32> {
    let order = response.get("order").unwrap_or(response);
    let filled = amount_as_f64(&order["cumQuantity"])
        .filter(|filled| filled.is_finite() && *filled >= 0.0)
        .ok_or_else(|| anyhow::anyhow!("Polymarket US order lookup had no cumQuantity"))?;
    // Round up: a fractional fill is still exposure, never report less.
    Ok(filled.ceil() as i32)
}

/// Contracts traded on `market_slug` at or after `since_ms` (epoch millis),
/// from a `GET /v1/portfolio/activities` response.
pub(super) fn filled_from_activities(response: &Value, market_slug: &str, since_ms: i64) -> i32 {
    let total: f64 = response
        .get("activities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|activity| activity.get("trade"))
        .filter(|trade| trade["marketSlug"].as_str() == Some(market_slug))
        .filter(|trade| {
            trade["createTime"]
                .as_str()
                .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                .is_some_and(|time| time.timestamp_millis() >= since_ms)
        })
        .filter_map(|trade| amount_as_f64(&trade["qty"]))
        .sum();
    total.ceil() as i32
}

pub(super) fn amount_as_f64(value: &Value) -> Option<f64> {
    let value = value.get("value").unwrap_or(value);
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

impl PolymarketClient {
    /// Place an immediate-or-cancel Polymarket US limit order for whole contracts.
    pub async fn place_limit_order(
        &self,
        market_slug: &str,
        position_side: PolymarketPositionSide,
        side: &str,
        contracts: i32,
        price: f64,
    ) -> Result<Value> {
        let response = self
            .submit_limit_order(market_slug, position_side, side, contracts, price)
            .await?;
        if response.filled_contracts <= 0 {
            anyhow::bail!(
                "Polymarket US limit order did not fill: {}",
                response
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string())
            );
        }
        Ok(json!({
            "success": true,
            "order_id": response.order_id,
            "status": response.status,
            "filled_contracts": response.filled_contracts,
            "latency_ms": response.latency_ms,
        }))
    }

    /// Submit a FAK order and retain explicit acceptance/fill information for
    /// paired-execution recovery. Transport failures remain errors; exchange
    /// rejections and zero fills are returned for durable recording.
    ///
    /// `price` is the price of the `position_side` being traded (a SHORT
    /// order's price is the NO price), and the returned `average_fill_price`
    /// is in those same terms. On the wire Polymarket US prices every order
    /// on the YES book instead - "buy NO at $0.40" is "sell YES at $0.60"
    /// (docs.polymarket.us/concepts/orders) - so both are converted here.
    /// Sending the NO price unconverted turned a BUY_SHORT limit into a
    /// near-meaningless "sell YES at >= NO price" order.
    pub async fn submit_limit_order(
        &self,
        market_slug: &str,
        position_side: PolymarketPositionSide,
        side: &str,
        contracts: i32,
        price: f64,
    ) -> Result<PolymarketOrderResult> {
        let side = side.to_ascii_lowercase();
        if market_slug.trim().is_empty()
            || contracts <= 0
            || !price.is_finite()
            || !(0.0..1.0).contains(&price)
            || !matches!(side.as_str(), "buy" | "sell")
        {
            anyhow::bail!("Polymarket US limit order requires a slug, positive whole contract count, limit price, and buy or sell action");
        }

        // Cached for an hour and already loaded by sizing on the paired
        // path. Without it the price is still snapped to 0.0001, which
        // removes float noise without moving it.
        let tick_size = match self.market_constraints(market_slug).await {
            Ok((_, tick_size)) => tick_size,
            Err(error) => {
                warn!(
                    "Polymarket US tick size lookup failed for {}; formatting at 0.0001: {:#}",
                    market_slug, error
                );
                None
            }
        };
        let wire_price = format_order_price(yes_book_price(position_side, price), tick_size)?;

        // `quantity` is a JSON integer here because `contracts` is already
        // `i32` - the SDK's own type stubs declare `quantity: int`, and a
        // prior bug sent a JSON float (`5.0`) through the old Python hop,
        // which this path can no longer reproduce.
        let intent = format!(
            "ORDER_INTENT_{}_{}",
            side.to_ascii_uppercase(),
            position_side.as_str().to_ascii_uppercase()
        );
        let body = json!({
            "marketSlug": market_slug,
            "intent": intent,
            "type": "ORDER_TYPE_LIMIT",
            "price": {"value": wire_price, "currency": "USD"},
            "quantity": contracts,
            "tif": "TIME_IN_FORCE_IMMEDIATE_OR_CANCEL",
            "manualOrderIndicator": "MANUAL_ORDER_INDICATOR_AUTOMATIC",
            "synchronousExecution": true,
            "maxBlockTime": "5",
        });

        let started = Instant::now();
        let url = format!("{}/v1/orders", self.config.api_base_url);
        let response = self
            .request_json(Method::POST, &url, Some("/v1/orders"), Some(&body))
            .await
            .context("Failed to submit Polymarket US limit order")?;
        let latency_ms = started.elapsed().as_millis() as i64;

        Ok(parse_polymarket_order_result(
            &response,
            latency_ms,
            &body,
            position_side,
        ))
    }

    /// Get open orders directly from the Polymarket US API.
    pub async fn get_open_orders(&self) -> Result<Value> {
        let url = format!("{}/v1/orders/open", self.config.api_base_url);
        let response = self
            .request_json(Method::GET, &url, Some("/v1/orders/open"), None)
            .await
            .context("Failed to fetch Polymarket US open orders")?;
        Ok(response.get("orders").cloned().unwrap_or_else(|| json!([])))
    }

    /// Get positions directly from the Polymarket US API, cached briefly to
    /// keep repeated lookups from adding rate-limit pressure.
    pub async fn get_positions(&self) -> Result<Value> {
        let mut cache = self.positions_cache.lock().await;
        if let Some((cached_at, positions)) = cache.as_ref() {
            if cached_at.elapsed() < self.config.cache.positions_ttl() {
                return Ok(positions.clone());
            }
        }

        let url = format!("{}/v1/portfolio/positions", self.config.api_base_url);
        let response = self
            .request_json(Method::GET, &url, Some("/v1/portfolio/positions"), None)
            .await
            .context("Failed to fetch Polymarket US positions")?;
        let positions = normalize_positions(&response)?;

        *cache = Some((Instant::now(), positions.clone()));
        Ok(positions)
    }

    /// Ask the exchange how many contracts actually filled, for an order the
    /// submit response reported as unfilled. The response can be wrong or
    /// missing (a fill once landed while the bot recorded none), and a naked
    /// unrecorded position is worse than a missed trade.
    ///
    /// With an order id the order itself is read; without one (no execution
    /// came back) the account's recent trades on this market since
    /// `since_ms` are summed. A failed lookup is an `Err`: the caller must
    /// not treat it as "no fill".
    pub async fn confirm_filled_contracts(
        &self,
        order_id: Option<&str>,
        market_slug: &str,
        since_ms: i64,
    ) -> Result<i32> {
        if let Some(order_id) = order_id {
            let path = format!("/v1/order/{}", order_id);
            let url = format!("{}{}", self.config.api_base_url, path);
            let response = self
                .request_json(Method::GET, &url, Some(&path), None)
                .await
                .context("Failed to look up Polymarket US order to confirm its fill")?;
            return filled_from_order_lookup(&response);
        }
        let path = "/v1/portfolio/activities?limit=50";
        let url = format!("{}{}", self.config.api_base_url, path);
        let response = self
            .request_json(Method::GET, &url, Some("/v1/portfolio/activities"), None)
            .await
            .context("Failed to look up Polymarket US trades to confirm a fill")?;
        Ok(filled_from_activities(&response, market_slug, since_ms))
    }

    /// Cancel an order directly against the Polymarket US API. The
    /// exchange requires the order's `marketSlug` on cancel, so this looks
    /// the order up first.
    pub async fn cancel_order(&self, order_id: &str) -> Result<Value> {
        let retrieve_path = format!("/v1/order/{}", order_id);
        let retrieve_url = format!("{}{}", self.config.api_base_url, retrieve_path);
        let order = self
            .request_json(Method::GET, &retrieve_url, Some(&retrieve_path), None)
            .await
            .context("Failed to look up Polymarket US order before cancelling")?;
        let market_slug = order
            .get("order")
            .and_then(|order| order.get("marketSlug"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("Polymarket US did not return a marketSlug for this order")
            })?;

        let cancel_path = format!("/v1/order/{}/cancel", order_id);
        let cancel_url = format!("{}{}", self.config.api_base_url, cancel_path);
        self.request_json(
            Method::POST,
            &cancel_url,
            Some(&cancel_path),
            Some(&json!({ "marketSlug": market_slug })),
        )
        .await
        .context("Failed to cancel Polymarket US order")?;

        Ok(json!({"success": true, "order_id": order_id}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fractional_partial_fill_reports_the_whole_part_and_flags_the_fraction() {
        let response = json!({"executions": [{"order": {
            "id": "o", "state": "ORDER_STATE_EXPIRED", "cumQuantity": 8.85, "avgPx": {"value": "0.53"}
        }}]});
        let result = parse_polymarket_order_result(
            &response,
            1,
            &json!({}),
            PolymarketPositionSide::Short,
        );
        assert_eq!(result.filled_contracts, 8);
        assert!(result.fill_quantity_valid);
        assert!(result.fractional_fill);
        assert!(result.error.unwrap().contains("8.85"));
    }

    #[test]
    fn an_order_lookup_reports_the_exchange_fill() {
        let filled = json!({"order": {"id": "o", "cumQuantity": "3", "state": "ORDER_STATE_FILLED"}});
        assert_eq!(filled_from_order_lookup(&filled).unwrap(), 3);
        let unfilled = json!({"order": {"id": "o", "cumQuantity": "0"}});
        assert_eq!(filled_from_order_lookup(&unfilled).unwrap(), 0);
        assert!(filled_from_order_lookup(&json!({"order": {}})).is_err());
    }

    #[test]
    fn only_trades_on_this_market_since_submission_count_as_a_fill() {
        let since = chrono::DateTime::parse_from_rfc3339("2026-09-11T20:35:37Z").unwrap().timestamp_millis();
        let response = json!({"activities": [
            {"trade": {"marketSlug": "m", "qty": "1", "createTime": "2026-09-11T20:35:38.142Z"}},
            {"trade": {"marketSlug": "m", "qty": "2", "createTime": "2026-09-11T20:00:00Z"}},
            {"trade": {"marketSlug": "other", "qty": "5", "createTime": "2026-09-11T20:35:38Z"}},
            {"positionResolution": {}}
        ]});
        assert_eq!(filled_from_activities(&response, "m", since), 1);
        assert_eq!(filled_from_activities(&response, "none", since), 0);
    }

    use super::*;

    #[test]
    fn extracts_sdk_average_fill_price() {
        let response = json!({
            "executions": [{
                "order": {
                    "avgPx": {"value": "0.6250", "currency": "USD"}
                }
            }]
        });

        assert_eq!(extract_average_fill_price(&response), Some(0.625));
    }

    #[test]
    fn short_orders_are_priced_on_the_yes_book() {
        // Buying NO at 0.44 is selling YES at 0.56.
        assert!((yes_book_price(PolymarketPositionSide::Short, 0.44) - 0.56).abs() < 1e-9);
        assert_eq!(yes_book_price(PolymarketPositionSide::Long, 0.44), 0.44);
        // The conversion is its own inverse (fill prices come back on the
        // YES book too).
        let round_trip = yes_book_price(
            PolymarketPositionSide::Short,
            yes_book_price(PolymarketPositionSide::Short, 0.37),
        );
        assert!((round_trip - 0.37).abs() < 1e-12);
    }

    #[test]
    fn order_prices_are_snapped_to_tick_without_float_noise() {
        // The exact values sent live on 2026-09-29, which the exchange
        // truncated to 0.43 and 0.67.
        assert_eq!(format_order_price(1.0 - 0.56, Some(0.01)).unwrap(), "0.44");
        assert_eq!(format_order_price(1.0 - 0.32, Some(0.01)).unwrap(), "0.68");
        assert_eq!(format_order_price(0.5, Some(0.01)).unwrap(), "0.50");
        // Sub-cent tick markets exist (e.g. 0.411 / 0.413 books).
        assert_eq!(format_order_price(0.411, Some(0.001)).unwrap(), "0.411");
        // Unknown tick: only noise is removed, the price is not moved.
        assert_eq!(format_order_price(1.0 - 0.56, None).unwrap(), "0.4400");
        assert_eq!(format_order_price(0.4115, None).unwrap(), "0.4115");
        // Nonsense ticks fall back to the same safe resolution.
        assert_eq!(format_order_price(0.44, Some(0.0)).unwrap(), "0.4400");
        // Snapping must never produce a price outside (0, 1).
        assert!(format_order_price(0.996, Some(0.01)).is_err());
        assert!(format_order_price(0.004, Some(0.01)).is_err());
    }

    /// The real response to order CV9F74QA0YGE (2026-10-01): accepted, then
    /// expired unfilled, with the enum default on both executions.
    fn expired_ioc_response() -> Value {
        let execution = |id: &str, kind: &str, leaves: i64| {
            json!({
                "aggressor": false,
                "id": id,
                "order": {
                    "action": "ORDER_ACTION_BUY",
                    "avgPx": {"currency": "USD", "value": "0.0000"},
                    "cumQuantity": 0,
                    "id": "CV9F74QA0YGE",
                    "intent": "ORDER_INTENT_BUY_LONG",
                    "leavesQuantity": leaves,
                    "marketSlug": "aec-itfme-arysha-evabyn-2026-10-01",
                    "price": {"currency": "USD", "value": "0.5"},
                    "quantity": 10,
                    "state": if kind == "EXECUTION_TYPE_NEW" { "ORDER_STATE_NEW" } else { "ORDER_STATE_EXPIRED" }
                },
                "orderRejectReason": "ORD_REJECT_REASON_EXCHANGE_OPTION",
                "text": "",
                "type": kind
            })
        };
        json!({"executions": [
            execution("CV9W79QCJYHR", "EXECUTION_TYPE_NEW", 10),
            execution("CV9W79QCMYHR", "EXECUTION_TYPE_EXPIRED", 0)
        ]})
    }

    fn limit_request(price: &str) -> Value {
        json!({"price": {"value": price, "currency": "USD"}})
    }

    #[test]
    fn an_expired_ioc_is_not_reported_as_an_exchange_rejection() {
        let result = parse_polymarket_order_result(
            &expired_ioc_response(),
            134,
            &limit_request("0.50"),
            PolymarketPositionSide::Long,
        );
        assert_eq!(result.filled_contracts, 0);
        assert!(result.accepted, "the exchange accepted the order");
        assert_eq!(result.order_id.as_deref(), Some("CV9F74QA0YGE"));
        let error = result.error.unwrap();
        assert!(!error.contains("EXCHANGE_OPTION"), "{error}");
        assert!(error.contains("expired unfilled"), "{error}");
        assert!(error.contains("0.50"), "{error}");
    }

    #[test]
    fn a_real_rejection_reports_its_reason_and_text() {
        let response = json!({"executions": [{
            "order": {"id": "X1", "cumQuantity": 0, "state": "ORDER_STATE_REJECTED"},
            "orderRejectReason": "ORD_REJECT_REASON_EXCHANGE_CLOSED",
            "text": "Book is SUSPENDED",
            "type": "EXECUTION_TYPE_REJECTED"
        }]});
        let error = parse_polymarket_order_result(
            &response,
            50,
            &limit_request("0.50"),
            PolymarketPositionSide::Long,
        )
        .error
        .unwrap();
        assert!(error.contains("ORD_REJECT_REASON_EXCHANGE_CLOSED"), "{error}");
        assert!(error.contains("Book is SUSPENDED"), "{error}");
    }

    #[test]
    fn a_short_fill_price_is_converted_off_the_yes_book() {
        // Order CSDJ2WV62YB5 (2026-09-29): BUY_SHORT filled at avgPx 0.49 on
        // the YES book, i.e. NO cost 0.51 (position cost basis confirmed it).
        let response = json!({"executions": [{
            "order": {
                "id": "CSDJ2WV62YB5",
                "cumQuantity": 10,
                "state": "ORDER_STATE_FILLED",
                "avgPx": {"currency": "USD", "value": "0.4900"}
            },
            "type": "EXECUTION_TYPE_FILL"
        }]});
        let short = parse_polymarket_order_result(
            &response,
            199,
            &limit_request("0.56"),
            PolymarketPositionSide::Short,
        );
        assert_eq!(short.filled_contracts, 10);
        assert!((short.average_fill_price.unwrap() - 0.51).abs() < 1e-9);
        assert!(short.error.is_none());

        let long = parse_polymarket_order_result(
            &response,
            199,
            &limit_request("0.49"),
            PolymarketPositionSide::Long,
        );
        assert!((long.average_fill_price.unwrap() - 0.49).abs() < 1e-9);
    }

    #[test]
    fn normalized_positions_carry_the_exchange_reported_fees_and_base_cost() {
        // The real /v1/portfolio/positions record for the 2026-10-02 AZKSIN
        // SHORT x2 (marketMetadata omitted): cost 0.98 = baseCost 0.94 + fees 0.04.
        let response = json!({"positions": {
            "aec-itfme-ardazk-digsin-2026-10-02": {
                "avgPx": {"currency": "USD", "value": "0.4900"},
                "baseCost": {"currency": "USD", "value": "0.9400"},
                "cashValue": {"currency": "USD", "value": "0.9400"},
                "cost": {"currency": "USD", "value": "0.9800"},
                "fees": {"currency": "USD", "value": "0.0400"},
                "netPosition": "-2",
                "realized": {"currency": "USD", "value": "0.0000"},
                "marketMetadata": {"title": "Arda Azkara vs. Digvijay Pratap Singh", "outcome": "Digvijay Pratap Singh"}
            },
            "resolved-position": {
                "baseCost": {"currency": "USD", "value": "1.0000"},
                "cashValue": {"currency": "USD", "value": "0"},
                "cost": {"currency": "USD", "value": "1.0000"},
                "fees": null,
                "netPosition": "1",
                "marketMetadata": {}
            }
        }});
        let positions = normalize_positions(&response).unwrap();
        let positions = positions.as_array().unwrap();
        let find = |id: &str| positions.iter().find(|p| p["id"] == id).unwrap();

        let azksin = find("aec-itfme-ardazk-digsin-2026-10-02");
        assert_eq!(azksin["cost"], 0.98);
        assert_eq!(azksin["base_cost"], 0.94);
        assert_eq!(azksin["fees"], 0.04);
        assert_eq!(azksin["value"], 0.94);

        // A null fee stays null (unknown), never a fake zero.
        assert!(find("resolved-position")["fees"].is_null());
    }

    #[test]
    fn the_collected_commission_is_read_from_a_fill_and_absent_otherwise() {
        // Order CSDJ599XAYBY (2026-09-29): 10 contracts, $0.17 collected.
        let filled = json!({"executions": [{
            "order": {
                "id": "CSDJ599XAYBY",
                "cumQuantity": 10,
                "state": "ORDER_STATE_FILLED",
                "avgPx": {"currency": "USD", "value": "0.5000"},
                "commissionNotionalTotalCollected": {"currency": "USD", "value": "0.1700"}
            },
            "type": "EXECUTION_TYPE_FILL"
        }]});
        let result = parse_polymarket_order_result(
            &filled,
            138,
            &limit_request("0.50"),
            PolymarketPositionSide::Short,
        );
        assert_eq!(result.filled_contracts, 10);
        assert_eq!(result.commission_collected, Some(0.17));

        // An unfilled IOC charges nothing, and reports no commission.
        let unfilled = parse_polymarket_order_result(
            &expired_ioc_response(),
            134,
            &limit_request("0.50"),
            PolymarketPositionSide::Long,
        );
        assert_eq!(unfilled.commission_collected, None);
    }
}
