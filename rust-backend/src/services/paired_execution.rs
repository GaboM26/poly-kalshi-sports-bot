//! Shared paired-order execution core.
//!
//! Both the auto-trade executor and the manual "Execute" endpoint place a
//! Kalshi leg and a Polymarket leg for the same tracked competitor and must
//! obey the same invariants (durable `submitting` record before either leg
//! fires, bounded neutralization on unequal fills, halt-on-persistence-
//! failure). Keeping this in one module means both callers share the exact
//! same safety-critical code path instead of two copies drifting apart.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::{error, info, warn};

use crate::clients::polymarket::yes_book_price;
use crate::clients::{
    KalshiOrderResult, PolymarketBookLevel, PolymarketMarketBook, PolymarketOrderResult,
};
use crate::core::fees::{kalshi_taker_fee, polymarket_taker_fee};
use crate::models::PolymarketPositionSide;
use crate::services::websocket_manager::POLYMARKET_WS_MAX_SILENCE;
use crate::services::{ArbitrageService, AutoTradeExecutionRecord, TelegramClient};

/// For Kalshi: a connection-liveness backstop, checked against the last
/// WebSocket message received on *any* subscribed ticker (see
/// `KalshiClient::get_fresh_orderbook`) - not "this ticker changed
/// recently," which Kalshi only signals on a real book change and would
/// wrongly reject an accurate but quiet market. This just needs to be loose
/// enough to never fire during normal operation and tight enough to catch a
/// genuinely hung connection promptly. The actual per-book correctness
/// check is the connection epoch, not this duration.
/// Polymarket books come from `executable_polymarket_book` instead.
pub const EXECUTABLE_BOOK_MAX_AGE: Duration = Duration::from_secs(30);

/// Where an executable Polymarket book came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolymarketBookSource {
    /// The live WebSocket book, vouched for by `fresh_polymarket_book`.
    WebSocket,
    /// One origin (non-CDN-cached) REST fetch, used when the WebSocket has
    /// nothing current for the market.
    Rest,
}

impl PolymarketBookSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PolymarketBookSource::WebSocket => "websocket",
            PolymarketBookSource::Rest => "rest",
        }
    }
}

/// The Polymarket book an order may be sized, priced, or neutralized
/// against: the live WebSocket book when that stream is provably current
/// (no network round trip), otherwise one REST fetch that bypasses the
/// gateway's 30s CDN cache and refuses a cached response. Either way the
/// market must be `MARKET_STATE_OPEN`, and the book carries the market's
/// order constraints (a book without a known minimum trade quantity is
/// refused by sizing, never treated as "no minimum").
pub async fn executable_polymarket_book(
    service: &ArbitrageService,
    market_slug: &str,
) -> Result<(PolymarketMarketBook, PolymarketBookSource), String> {
    let (mut book, source) = match service
        .ws_manager
        .fresh_polymarket_book(market_slug, POLYMARKET_WS_MAX_SILENCE)
    {
        Some(book) => (book, PolymarketBookSource::WebSocket),
        None => {
            let book = service
                .polymarket_client
                .get_market_book(market_slug)
                .await
                .map_err(|error| format!("{error:#}"))?;
            (book, PolymarketBookSource::Rest)
        }
    };
    if book.state != "MARKET_STATE_OPEN" {
        return Err(format!(
            "Polymarket US market is not open for trading (state: {})",
            book.state
        ));
    }
    if source == PolymarketBookSource::WebSocket {
        // Cached for an hour per market; the REST path already filled these.
        match service.polymarket_client.market_constraints(market_slug).await {
            Ok((minimum_trade_qty, price_tick_size)) => {
                book.minimum_trade_qty = minimum_trade_qty;
                book.price_tick_size = price_tick_size;
            }
            Err(error) => warn!(
                "Polymarket US market constraint lookup failed for {}: {:#}",
                market_slug, error
            ),
        }
    }
    Ok((book, source))
}

/// A conservative floor on Polymarket leg notional (contracts * price), kept
/// as an extra safety margin. Kalshi has no equivalent floor, so this only
/// bounds the Polymarket leg's size search. (For the record: the long-running
/// `ORD_REJECT_REASON_EXCHANGE_OPTION` "rejections" were never about size -
/// that is the reject-reason enum's zero value, serialized on every
/// execution; those orders were IOCs expiring unfilled against a book that
/// was up to 30s stale. See CLAUDE.md, 2026-10-01.)
pub const POLYMARKET_MIN_NOTIONAL_USD: f64 = 1.0;

#[derive(Debug)]
pub struct RecoveryResult {
    pub filled_contracts: i32,
    pub order_id: Option<String>,
    pub error: Option<String>,
}

pub fn failed_poly_order(error: String) -> PolymarketOrderResult {
    PolymarketOrderResult {
        accepted: false,
        filled_contracts: 0,
        fill_quantity_valid: true,
        order_id: None,
        status: None,
        error: Some(error),
        latency_ms: None,
        average_fill_price: None,
        commission_collected: None,
    }
}

pub fn failed_kalshi_order(error: String) -> KalshiOrderResult {
    KalshiOrderResult {
        accepted: false,
        filled_contracts: 0,
        fill_quantity_valid: true,
        order_id: None,
        status: None,
        error: Some(error),
        average_fill_price: None,
    }
}

pub fn can_submit_kalshi_after_polymarket(polymarket_filled_contracts: i32) -> bool {
    polymarket_filled_contracts > 0
}

pub fn kalshi_buy_levels(book: &crate::clients::kalshi::OrderBook, side: &str) -> Vec<(i32, i32)> {
    let mut levels = match side {
        "yes" => book
            .no
            .iter()
            .rev()
            .map(|(price, qty)| (100 - price, *qty))
            .collect(),
        "no" => book
            .yes
            .iter()
            .rev()
            .map(|(price, qty)| (100 - price, *qty))
            .collect(),
        _ => Vec::new(),
    };
    levels.sort_by_key(|(price, _)| *price);
    levels
}

pub fn kalshi_sell_levels(book: &crate::clients::kalshi::OrderBook, side: &str) -> Vec<(i32, i32)> {
    let mut levels = match side {
        "yes" => book
            .yes
            .iter()
            .rev()
            .map(|(price, qty)| (*price, *qty))
            .collect(),
        "no" => book
            .no
            .iter()
            .rev()
            .map(|(price, qty)| (*price, *qty))
            .collect(),
        _ => Vec::new(),
    };
    levels.sort_by(|left, right| right.0.cmp(&left.0));
    levels
}

pub fn polymarket_buy_levels(
    book: &PolymarketMarketBook,
    position_side: PolymarketPositionSide,
) -> Vec<(f64, i32)> {
    let levels = match position_side {
        // Long buys consume native LONG offers in ascending price order.
        PolymarketPositionSide::Long => &book.offers,
        // Short buys consume native LONG bids descending, at complement price.
        PolymarketPositionSide::Short => &book.bids,
    };
    let iter: Box<dyn Iterator<Item = &PolymarketBookLevel>> = match position_side {
        PolymarketPositionSide::Long => Box::new(levels.iter()),
        PolymarketPositionSide::Short => Box::new(levels.iter().rev()),
    };
    let mut result: Vec<_> = iter
        .filter_map(|level| {
            let quantity = level.quantity.floor() as i32;
            (quantity > 0).then_some((
                if position_side == PolymarketPositionSide::Long {
                    level.price
                } else {
                    1.0 - level.price
                },
                quantity,
            ))
        })
        .collect();
    result.sort_by(|left, right| left.0.total_cmp(&right.0));
    result
}

pub fn polymarket_sell_levels(
    book: &PolymarketMarketBook,
    position_side: PolymarketPositionSide,
) -> Vec<(f64, i32)> {
    let levels = match position_side {
        // Selling LONG consumes native LONG bids descending.
        PolymarketPositionSide::Long => &book.bids,
        // Selling SHORT consumes native LONG offers ascending, complemented.
        PolymarketPositionSide::Short => &book.offers,
    };
    let iter: Box<dyn Iterator<Item = &PolymarketBookLevel>> = match position_side {
        PolymarketPositionSide::Long => Box::new(levels.iter()),
        PolymarketPositionSide::Short => Box::new(levels.iter()),
    };
    let mut result: Vec<_> = iter
        .filter_map(|level| {
            let quantity = level.quantity.floor() as i32;
            (quantity > 0).then_some((
                if position_side == PolymarketPositionSide::Long {
                    level.price
                } else {
                    1.0 - level.price
                },
                quantity,
            ))
        })
        .collect();
    result.sort_by(|left, right| right.0.total_cmp(&left.0));
    result
}

pub fn executable_depth<T>(levels: &[(T, i32)]) -> i32 {
    levels.iter().fold(0_i32, |total, (_, quantity)| {
        total.saturating_add(*quantity)
    })
}

pub fn worst_price<T: Copy>(levels: &[(T, i32)], contracts: i32) -> Option<T> {
    let mut remaining = contracts;
    for (price, quantity) in levels {
        if *quantity <= 0 {
            continue;
        }
        remaining -= (*quantity).min(remaining);
        if remaining <= 0 {
            return Some(*price);
        }
    }
    None
}

/// Calculate the number of contracts to trade based on depth and settings
///
/// Returns None if depth is insufficient (below min_contracts)
pub fn calculate_contracts_to_trade(
    kalshi_depth: i32,
    poly_depth_contracts: i32,
    flexible_mode: bool,
    min_contracts: i32,
    max_contracts: i32,
) -> Option<i32> {
    let min_depth = kalshi_depth.min(poly_depth_contracts);

    // Check minimum depth requirement
    if min_depth < min_contracts {
        return None;
    }

    // Fixed mode: always use min_contracts
    if !flexible_mode {
        return Some(min_contracts);
    }

    // Flexible mode logic:
    // - Depth 10-20: use min_contracts (10)
    // - Depth >= 20: use half of the smaller depth
    let contracts = if min_depth < 20 {
        min_contracts
    } else {
        min_depth / 2
    };

    // Apply max_contracts limit
    Some(contracts.min(max_contracts))
}

/// One sized trade: the contract count plus the worst-case prices and the
/// fees of BOTH exchanges that determined it. A named struct (not a tuple)
/// so the two fees - adjacent `f64`s - can never be swapped by a caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizedTrade {
    pub contracts: i32,
    pub kalshi_price_cents: i32,
    pub poly_price: f64,
    pub kalshi_fee: f64,
    pub poly_fee: f64,
    /// Net profit over total cost, in percent, after both exchanges' fees.
    pub profit_margin: f64,
}

/// Largest profitable size within the depth, minimums and amount cap.
/// Profitable means net of BOTH exchanges' taker fees (every order here is
/// an immediate-or-cancel taker order), assuming every contract fills at the
/// worst price on each venue; `max_amount` caps the total cost including
/// both fees.
///
/// `min_profit_margin` is the net margin (percent) the sized trade must still
/// clear on these fresh books. Detection applies the same threshold to a
/// possibly stale quote, so without re-checking here a trade could execute at
/// a margin below the configured minimum. Larger sizes walk deeper into the
/// books and lose margin, so the largest size that still clears it wins.
///
/// `poly_min_trade_qty` is the Polymarket market's own minimum tradeable
/// quantity (docs.polymarket.us/api-reference/orders/overview: "constraints
/// are market-dependent - retrieve minimumTradeQty ... before order
/// submission"); orders below it are rejected by the exchange. `None` means
/// it could not be looked up and must block sizing entirely, not be treated
/// as zero.
pub fn find_profitable_contract_size(
    size_cap: i32,
    min_contracts: i32,
    max_amount: f64,
    min_profit_margin: f64,
    poly_min_trade_qty: Option<f64>,
    kalshi_levels: &[(i32, i32)],
    poly_levels: &[(f64, i32)],
) -> Option<SizedTrade> {
    if !max_amount.is_finite() || max_amount <= 0.0 || !min_profit_margin.is_finite() {
        return None;
    }
    let Some(poly_min_trade_qty) = poly_min_trade_qty else {
        return None;
    };
    let min_contracts = if poly_min_trade_qty.is_finite() && poly_min_trade_qty > 0.0 {
        min_contracts.max(poly_min_trade_qty.ceil() as i32)
    } else {
        min_contracts
    };
    for contracts in (min_contracts..=size_cap).rev() {
        let kalshi_price_cents = worst_price(kalshi_levels, contracts)?;
        let poly_price = worst_price(poly_levels, contracts)?;
        if contracts as f64 * poly_price < POLYMARKET_MIN_NOTIONAL_USD {
            continue;
        }
        let kalshi_price = kalshi_price_cents as f64 / 100.0;
        let kalshi_fee = kalshi_taker_fee(contracts as f64, kalshi_price);
        let poly_fee = polymarket_taker_fee(contracts as f64, poly_price);
        let total_cost =
            contracts as f64 * (kalshi_price + poly_price) + kalshi_fee + poly_fee;
        if total_cost.is_finite() && total_cost <= max_amount && total_cost < contracts as f64 {
            let profit_margin = ((contracts as f64 - total_cost) / total_cost) * 100.0;
            if profit_margin < min_profit_margin {
                continue;
            }
            return Some(SizedTrade {
                contracts,
                kalshi_price_cents,
                poly_price,
                kalshi_fee,
                poly_fee,
                profit_margin,
            });
        }
    }
    None
}

/// Compare the commission Polymarket actually collected for a fill with what
/// the fee model predicted. Returns `(modeled, within_tolerance)`. The
/// tolerance (1 cent or 1.5% of the fee) covers cumulative-vs-per-fill
/// rounding on multi-level fills but would catch a changed fee schedule,
/// which would otherwise silently skew every margin again.
pub fn polymarket_fee_check(contracts: i32, price: f64, collected: f64) -> (f64, bool) {
    let modeled = polymarket_taker_fee(contracts as f64, price);
    let tolerance = (0.015 * modeled).max(0.011);
    (modeled, (collected - modeled).abs() <= tolerance)
}

/// Whether closing at `close_price` stays within `max_loss_cents` per
/// contract of the actual entry price. The closing sell is a taker order and
/// pays `close_fee` (total, dollars), so the bound is checked against the
/// proceeds net of that fee, not the bare price.
pub fn close_within_loss_bound(
    entry_price: f64,
    close_price: f64,
    contracts: i32,
    close_fee: f64,
    max_loss_cents: i32,
) -> bool {
    if contracts <= 0 {
        return false;
    }
    let net_close_price = close_price - close_fee / contracts as f64;
    net_close_price + f64::EPSILON >= entry_price - max_loss_cents as f64 / 100.0
}

/// Polymarket sets `period` to "SUS" while a game is suspended; its book is
/// frozen or unreliable then, so nothing may be auto-traded against it.
pub fn is_game_suspended(period: Option<&str>) -> bool {
    period.is_some_and(|p| p.trim().eq_ignore_ascii_case("SUS"))
}

pub async fn neutralize_polymarket(
    service: &ArbitrageService,
    market_slug: &str,
    position_side: PolymarketPositionSide,
    contracts: i32,
    entry_price: Option<f64>,
    max_loss_cents: i32,
) -> Result<RecoveryResult, String> {
    let entry_price = entry_price.ok_or_else(|| {
        "Polymarket actual entry price was not returned; bounded close cannot be verified"
            .to_string()
    })?;
    let (book, _) = executable_polymarket_book(service, market_slug)
        .await
        .map_err(|error| format!("Unable to get a fresh Polymarket close book: {error}"))?;
    let close_price = worst_price(&polymarket_sell_levels(&book, position_side), contracts)
        .ok_or_else(|| "Fresh Polymarket close book lacks required executable depth".to_string())?;
    let close_fee = polymarket_taker_fee(contracts as f64, close_price);
    if !close_within_loss_bound(entry_price, close_price, contracts, close_fee, max_loss_cents) {
        return Err(format!(
            "Polymarket close price {:.4} (closing fee ${:.2}) exceeds configured {}¢ loss bound from actual entry {:.4}",
            close_price, close_fee, max_loss_cents, entry_price
        ));
    }
    let result = service
        .polymarket_client
        .submit_limit_order(market_slug, position_side, "sell", contracts, close_price)
        .await
        .map_err(|error| format!("Polymarket bounded close submission failed: {error}"))?;
    Ok(RecoveryResult {
        filled_contracts: result.filled_contracts,
        order_id: result.order_id,
        error: result.error,
    })
}

pub async fn neutralize_kalshi(
    service: &ArbitrageService,
    ticker: &str,
    side: &str,
    contracts: i32,
    entry_price: Option<f64>,
    max_loss_cents: i32,
) -> Result<RecoveryResult, String> {
    let entry_price = entry_price.ok_or_else(|| {
        "Kalshi actual entry price was not returned; bounded close cannot be verified".to_string()
    })?;
    let book = service
        .kalshi_client
        .get_fresh_orderbook(ticker, EXECUTABLE_BOOK_MAX_AGE)
        .ok_or_else(|| "Fresh Kalshi websocket close depth is unavailable".to_string())?;
    let close_price_cents = worst_price(&kalshi_sell_levels(&book, side), contracts)
        .ok_or_else(|| "Fresh Kalshi close book lacks required executable depth".to_string())?;
    let close_price = close_price_cents as f64 / 100.0;
    let close_fee = kalshi_taker_fee(contracts as f64, close_price);
    if !close_within_loss_bound(entry_price, close_price, contracts, close_fee, max_loss_cents) {
        return Err(format!(
            "Kalshi close price {:.4} (closing fee ${:.2}) exceeds configured {}¢ loss bound from actual entry {:.4}",
            close_price, close_fee, max_loss_cents, entry_price
        ));
    }
    let result = service
        .kalshi_client
        .submit_order(ticker, "sell", side, contracts, close_price_cents)
        .await
        .map_err(|error| format!("Kalshi bounded close submission failed: {error}"))?;
    Ok(RecoveryResult {
        filled_contracts: result.filled_contracts,
        order_id: result.order_id,
        error: result.error,
    })
}

/// Inputs for one paired (Kalshi + Polymarket) order attempt. All prices and
/// depths must already come from fresh, executable books — never cached
/// display quotes.
pub struct PairedOrderParams {
    pub event_name: String,
    pub team_name: String,
    pub kalshi_market_id: String,
    pub kalshi_side: String,
    pub polymarket_market_slug: String,
    pub polymarket_position_side: PolymarketPositionSide,
    pub contracts: i32,
    pub kalshi_price_cents: i32,
    pub poly_price: f64,
    pub kalshi_fee: f64,
    /// Modeled Polymarket taker fee for this size (see `core::fees`).
    pub poly_fee: f64,
    pub profit_margin: f64,
    pub duration_ms: i64,
    /// `PolymarketMarketBook::fetched_at_ms` of the book `poly_price` was
    /// computed from: when it was fetched (REST) or last pushed (WebSocket).
    /// Diagnostic only.
    pub poly_book_fetched_at_ms: i64,
    /// Which source that book came from. Diagnostic only.
    pub poly_book_source: PolymarketBookSource,
    pub neutralization_max_loss_cents: i32,
    /// When true, no order is submitted to either exchange and nothing is
    /// persisted as a real attempt; the returned outcome carries the
    /// computed sizing/pricing only.
    pub dry_run: bool,
}

pub struct PairedOrderOutcome {
    pub status: String,
    pub execution_record: AutoTradeExecutionRecord,
    pub unsafe_fill_quantity: bool,
    pub residual_contracts: i32,
}

fn dry_run_record(params: &PairedOrderParams) -> AutoTradeExecutionRecord {
    AutoTradeExecutionRecord {
        event_name: params.event_name.clone(),
        team_name: params.team_name.clone(),
        kalshi_market_id: params.kalshi_market_id.clone(),
        polymarket_market_id: params.polymarket_market_slug.clone(),
        kalshi_side: params.kalshi_side.clone(),
        polymarket_side: params.polymarket_position_side.as_str().to_string(),
        contracts: params.contracts,
        kalshi_price: params.kalshi_price_cents as f64 / 100.0,
        kalshi_fee: params.kalshi_fee,
        polymarket_fee: params.poly_fee,
        polymarket_price: params.poly_price,
        profit_margin: params.profit_margin,
        duration_ms: params.duration_ms,
        total_duration_ms: 0,
        kalshi_success: false,
        polymarket_success: false,
        kalshi_order_id: None,
        polymarket_order_id: None,
        kalshi_error: None,
        polymarket_error: None,
        kalshi_latency_ms: None,
        poly_latency_ms: None,
        kalshi_filled_contracts: 0,
        polymarket_filled_contracts: 0,
        kalshi_order_status: None,
        polymarket_order_status: None,
        neutralization_leg: None,
        neutralization_success: None,
        neutralization_order_id: None,
        neutralization_filled_contracts: 0,
        neutralization_error: None,
        residual_leg: None,
        residual_contracts: 0,
        status: "dry_run".to_string(),
    }
}

/// Attempt one paired order: persist a durable `submitting` record, place the
/// Polymarket leg, persist its acknowledgement, then (only if Polymarket
/// filled a positive quantity) place the Kalshi leg and persist its
/// acknowledgement, then neutralize any unequal fill within the configured
/// bound. Halts auto-trading and alerts via Telegram if either exchange
/// acknowledgement can't be durably recorded, or if a residual, un-neutralized
/// exposure remains.
pub async fn submit_paired_order(
    service: &ArbitrageService,
    telegram_client: &Arc<TelegramClient>,
    params: PairedOrderParams,
) -> PairedOrderOutcome {
    if params.dry_run {
        return PairedOrderOutcome {
            status: "dry_run".to_string(),
            execution_record: dry_run_record(&params),
            unsafe_fill_quantity: false,
            residual_contracts: 0,
        };
    }

    let started = Instant::now();
    let lifecycle_id = match service.ws_manager.get_storage().save_auto_trade_execution(
        &AutoTradeExecutionRecord {
            event_name: params.event_name.clone(),
            team_name: params.team_name.clone(),
            kalshi_market_id: params.kalshi_market_id.clone(),
            polymarket_market_id: params.polymarket_market_slug.clone(),
            kalshi_side: params.kalshi_side.clone(),
            polymarket_side: params.polymarket_position_side.as_str().to_string(),
            contracts: params.contracts,
            kalshi_price: params.kalshi_price_cents as f64 / 100.0,
            kalshi_fee: params.kalshi_fee,
            polymarket_fee: params.poly_fee,
            polymarket_price: params.poly_price,
            profit_margin: params.profit_margin,
            duration_ms: params.duration_ms,
            total_duration_ms: 0,
            kalshi_success: false,
            polymarket_success: false,
            kalshi_order_id: None,
            polymarket_order_id: None,
            kalshi_error: None,
            polymarket_error: None,
            kalshi_latency_ms: None,
            poly_latency_ms: None,
            kalshi_filled_contracts: 0,
            polymarket_filled_contracts: 0,
            kalshi_order_status: None,
            polymarket_order_status: None,
            neutralization_leg: None,
            neutralization_success: None,
            neutralization_order_id: None,
            neutralization_filled_contracts: 0,
            neutralization_error: None,
            residual_leg: None,
            residual_contracts: 0,
            status: "submitting".to_string(),
        },
    ) {
        Ok(id) => id,
        Err(reason) => {
            error!(
                "Refusing paired order submission because durable intent persistence failed: {:#}",
                reason
            );
            let mut record = dry_run_record(&params);
            record.status = "persistence_failed_before_submission".to_string();
            return PairedOrderOutcome {
                status: "persistence_failed_before_submission".to_string(),
                execution_record: record,
                unsafe_fill_quantity: false,
                residual_contracts: 0,
            };
        }
    };

    // Every input below came from the direct executable books resolved by
    // the caller. Both native orders are IOC/FAK; an acknowledgement with
    // zero fill is failure.
    let poly_started = Instant::now();
    let book_age_at_submit_ms = Utc::now().timestamp_millis() - params.poly_book_fetched_at_ms;
    info!(
        "Polymarket leg submitting: market_slug={} side={} price={:.4} (YES-book {:.4}) contracts={} book_source={} book_age_ms={}",
        params.polymarket_market_slug,
        params.polymarket_position_side.as_str(),
        params.poly_price,
        yes_book_price(params.polymarket_position_side, params.poly_price),
        params.contracts,
        params.poly_book_source.as_str(),
        book_age_at_submit_ms
    );
    let poly_result = match service
        .polymarket_client
        .submit_limit_order(
            &params.polymarket_market_slug,
            params.polymarket_position_side,
            "buy",
            params.contracts,
            params.poly_price,
        )
        .await
    {
        Ok(result) => result,
        Err(reason) => failed_poly_order(reason.to_string()),
    };
    let poly_latency = poly_result
        .latency_ms
        .or(Some(poly_started.elapsed().as_millis() as i64));

    // Ground-truth check on the fee model: Polymarket reports the commission
    // it collected on the order. If that stops matching `core::fees`, every
    // margin is silently wrong again, so say so loudly.
    if poly_result.filled_contracts > 0 {
        if let Some(collected) = poly_result.commission_collected {
            let price = poly_result.average_fill_price.unwrap_or(params.poly_price);
            let (modeled, within_tolerance) =
                polymarket_fee_check(poly_result.filled_contracts, price, collected);
            if within_tolerance {
                info!(
                    "Polymarket fee check: market_slug={} filled={} @ {:.4} modeled=${:.2} collected=${:.2}",
                    params.polymarket_market_slug, poly_result.filled_contracts, price, modeled, collected
                );
            } else {
                warn!(
                    "Polymarket fee model drift: market_slug={} filled={} @ {:.4} modeled=${:.2} but collected=${:.2} - the fee schedule may have changed; update POLYMARKET_TAKER_FEE_RATE in core/fees.rs",
                    params.polymarket_market_slug, poly_result.filled_contracts, price, modeled, collected
                );
            }
        }
    }

    // Diagnostic only, fires after the order already resolved: an
    // independent origin REST fetch (never the CDN cache, never the WS book
    // the price came from) showing where the market actually was. The IOC
    // was priced at the exact worst-case boundary for `contracts` with no
    // slippage buffer, so any movement since the sizing book shows up here.
    if poly_result.filled_contracts <= 0 {
        match service
            .polymarket_client
            .get_market_book(&params.polymarket_market_slug)
            .await
        {
            Ok(post_book) => {
                let post_levels = polymarket_buy_levels(&post_book, params.polymarket_position_side);
                let current_best_price = post_levels.first().map(|(price, _)| *price);
                let market_moved_past_our_price =
                    current_best_price.is_some_and(|price| price > params.poly_price + 1e-9);
                warn!(
                    "Polymarket leg unfilled: post-order origin book check for {} — submitted_price={:.4} book_source={} book_age_ms_at_submit={} current_best_buy_price={:?} market_moved_past_our_price={} book_state={} book_transact_time={:?}",
                    params.polymarket_market_slug,
                    params.poly_price,
                    params.poly_book_source.as_str(),
                    book_age_at_submit_ms,
                    current_best_price,
                    market_moved_past_our_price,
                    post_book.state,
                    post_book.transact_time
                );
            }
            Err(error) => {
                warn!(
                    "Polymarket leg unfilled: post-order origin book check failed for {}: {:#}",
                    params.polymarket_market_slug, error
                );
            }
        }
    }
    if let Err(reason) = service.ws_manager.get_storage().update_auto_trade_leg(
        lifecycle_id,
        "polymarket",
        poly_result.filled_contracts > 0,
        poly_result.filled_contracts,
        poly_result.order_id.as_deref(),
        poly_result.status.as_deref(),
        poly_result.error.as_deref(),
        poly_latency,
    ) {
        error!(
            "Halting auto-trading because Polymarket acknowledgement could not be persisted: {:#}",
            reason
        );
        if let Err(disable_reason) = service.ws_manager.disable_auto_trade() {
            error!(
                "Failed to halt auto-trading after persistence failure: {:#}",
                disable_reason
            );
        }
        let mut record = dry_run_record(&params);
        record.status = "persistence_failed_after_poly_ack".to_string();
        return PairedOrderOutcome {
            status: "persistence_failed_after_poly_ack".to_string(),
            execution_record: record,
            unsafe_fill_quantity: false,
            residual_contracts: 0,
        };
    }

    // Never create Kalshi exposure after a zero-fill Polymarket response.
    // The Polymarket acknowledgement is durable above, so this rejection is
    // also auditable without submitting a second leg.
    let (kalshi_result, kalshi_latency) = if !can_submit_kalshi_after_polymarket(
        poly_result.filled_contracts,
    ) {
        (
            failed_kalshi_order(
                "Kalshi leg was not submitted because Polymarket did not fill a positive contract quantity"
                    .to_string(),
            ),
            None,
        )
    } else {
        let kalshi_started = Instant::now();
        let result = match service
            .kalshi_client
            .submit_order(
                &params.kalshi_market_id,
                "buy",
                &params.kalshi_side,
                params.contracts,
                params.kalshi_price_cents,
            )
            .await
        {
            Ok(result) => result,
            Err(reason) => failed_kalshi_order(reason.to_string()),
        };
        (result, Some(kalshi_started.elapsed().as_millis() as i64))
    };
    if let Err(reason) = service.ws_manager.get_storage().update_auto_trade_leg(
        lifecycle_id,
        "kalshi",
        kalshi_result.filled_contracts > 0,
        kalshi_result.filled_contracts,
        kalshi_result.order_id.as_deref(),
        kalshi_result.status.as_deref(),
        kalshi_result.error.as_deref(),
        kalshi_latency,
    ) {
        error!(
            "Halting auto-trading because Kalshi acknowledgement could not be persisted: {:#}",
            reason
        );
        if let Err(disable_reason) = service.ws_manager.disable_auto_trade() {
            error!(
                "Failed to halt auto-trading after persistence failure: {:#}",
                disable_reason
            );
        }
        let mut record = dry_run_record(&params);
        record.status = "persistence_failed_after_kalshi_ack".to_string();
        return PairedOrderOutcome {
            status: "persistence_failed_after_kalshi_ack".to_string(),
            execution_record: record,
            unsafe_fill_quantity: false,
            residual_contracts: 0,
        };
    }

    let mut neutralization_leg = None;
    let mut neutralization_success = None;
    let mut neutralization_order_id = None;
    let mut neutralization_filled_contracts = 0;
    let mut neutralization_error = None;
    let mut residual_leg = None;
    let mut residual_contracts = 0;
    let unsafe_fill_quantity =
        !poly_result.fill_quantity_valid || !kalshi_result.fill_quantity_valid;

    if unsafe_fill_quantity {
        residual_leg = Some("unknown".to_string());
        neutralization_error = Some(
            "An exchange reported a non-whole fill quantity; exact bounded neutralization is unsafe"
                .to_string(),
        );
    } else if poly_result.filled_contracts != kalshi_result.filled_contracts {
        let (leg, excess) = if poly_result.filled_contracts > kalshi_result.filled_contracts {
            (
                "polymarket",
                poly_result.filled_contracts - kalshi_result.filled_contracts,
            )
        } else {
            (
                "kalshi",
                kalshi_result.filled_contracts - poly_result.filled_contracts,
            )
        };
        neutralization_leg = Some(leg.to_string());
        let recovery = if leg == "polymarket" {
            neutralize_polymarket(
                service,
                &params.polymarket_market_slug,
                params.polymarket_position_side,
                excess,
                poly_result.average_fill_price,
                params.neutralization_max_loss_cents,
            )
            .await
        } else {
            neutralize_kalshi(
                service,
                &params.kalshi_market_id,
                &params.kalshi_side,
                excess,
                kalshi_result.average_fill_price,
                params.neutralization_max_loss_cents,
            )
            .await
        };
        match recovery {
            Ok(recovery) if recovery.filled_contracts >= excess => {
                neutralization_success = Some(true);
                neutralization_order_id = recovery.order_id;
                neutralization_filled_contracts = recovery.filled_contracts;
            }
            Ok(recovery) => {
                neutralization_success = Some(false);
                neutralization_order_id = recovery.order_id;
                neutralization_filled_contracts = recovery.filled_contracts;
                neutralization_error = recovery.error;
                residual_leg = Some(leg.to_string());
                residual_contracts = excess - neutralization_filled_contracts;
            }
            Err(reason) => {
                neutralization_success = Some(false);
                neutralization_error = Some(reason);
                residual_leg = Some(leg.to_string());
                residual_contracts = excess;
            }
        }
    }

    let status = if unsafe_fill_quantity || residual_contracts > 0 {
        "exposed"
    } else if neutralization_leg.is_some() {
        "neutralized"
    } else if poly_result.filled_contracts == params.contracts
        && kalshi_result.filled_contracts == params.contracts
    {
        "executed"
    } else if poly_result.filled_contracts == 0 && kalshi_result.filled_contracts == 0 {
        "rejected"
    } else {
        // Equal partial fills can still be directionally paired, but are
        // recorded explicitly rather than presented as a full execution.
        "partial_paired"
    };
    let execution_record = AutoTradeExecutionRecord {
        event_name: params.event_name.clone(),
        team_name: params.team_name.clone(),
        kalshi_market_id: params.kalshi_market_id.clone(),
        polymarket_market_id: params.polymarket_market_slug.clone(),
        kalshi_side: params.kalshi_side.clone(),
        polymarket_side: params.polymarket_position_side.as_str().to_string(),
        contracts: params.contracts,
        kalshi_price: params.kalshi_price_cents as f64 / 100.0,
        kalshi_fee: params.kalshi_fee,
        polymarket_fee: params.poly_fee,
        polymarket_price: params.poly_price,
        profit_margin: params.profit_margin,
        duration_ms: params.duration_ms,
        total_duration_ms: started.elapsed().as_millis() as i64,
        kalshi_success: kalshi_result.filled_contracts > 0,
        polymarket_success: poly_result.filled_contracts > 0,
        kalshi_order_id: kalshi_result.order_id.clone(),
        polymarket_order_id: poly_result.order_id.clone(),
        kalshi_error: kalshi_result.error.clone(),
        polymarket_error: poly_result.error.clone(),
        kalshi_latency_ms: kalshi_latency,
        poly_latency_ms: poly_latency,
        kalshi_filled_contracts: kalshi_result.filled_contracts,
        polymarket_filled_contracts: poly_result.filled_contracts,
        kalshi_order_status: kalshi_result.status.clone(),
        polymarket_order_status: poly_result.status.clone(),
        neutralization_leg,
        neutralization_success,
        neutralization_order_id,
        neutralization_filled_contracts,
        neutralization_error,
        residual_leg,
        residual_contracts,
        status: status.to_string(),
    };
    if let Err(reason) = service
        .ws_manager
        .get_storage()
        .finish_auto_trade_execution(lifecycle_id, &execution_record)
    {
        error!("Failed to finalize paired execution: {:#}", reason);
    }

    // Every attempt that put real money on either exchange gets an explicit
    // success/outcome log line here, independent of the DB record, so fills
    // can be grepped and cross-checked against the exchanges directly.
    if kalshi_result.filled_contracts > 0 || poly_result.filled_contracts > 0 {
        info!(
            "Paired order {}: {} {} | kalshi order_id={:?} filled={}/{} limit {}c avg_fill={:?} | polymarket order_id={:?} {} filled={}/{} limit {:.4} avg_fill={:?} | modeled fees kalshi=${:.2} polymarket=${:.2} | neutralization={:?} residual={}",
            status,
            params.event_name,
            params.team_name,
            kalshi_result.order_id,
            kalshi_result.filled_contracts,
            params.contracts,
            params.kalshi_price_cents,
            kalshi_result.average_fill_price,
            poly_result.order_id,
            params.polymarket_position_side.as_str(),
            poly_result.filled_contracts,
            params.contracts,
            params.poly_price,
            poly_result.average_fill_price,
            params.kalshi_fee,
            params.poly_fee,
            neutralization_success,
            residual_contracts,
        );
    }

    if unsafe_fill_quantity || residual_contracts > 0 {
        // The recovery bound was unavailable or could not be met. Stop future
        // automated submissions and notify through the configured alert path,
        // regardless of whether this attempt was auto-triggered or manual.
        if let Err(reason) = service.ws_manager.disable_auto_trade() {
            error!(
                "Failed to halt auto-trading after residual exposure: {:#}",
                reason
            );
        }
        telegram_client
            .send_auto_trade_notification(
                &params.event_name,
                &params.team_name,
                params.profit_margin,
                execution_record.kalshi_success,
                execution_record.polymarket_success,
                execution_record.kalshi_error.as_deref(),
                execution_record.polymarket_error.as_deref(),
                params.contracts as f64
                    * (params.kalshi_price_cents as f64 / 100.0 + params.poly_price)
                    + params.kalshi_fee,
                0.0,
            )
            .await;
    }

    PairedOrderOutcome {
        status: status.to_string(),
        execution_record,
        unsafe_fill_quantity,
        residual_contracts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_book() -> PolymarketMarketBook {
        PolymarketMarketBook {
            success: true,
            market_slug: "match-winner".to_string(),
            state: "open".to_string(),
            transact_time: Some("2026-09-11T19:37:00Z".to_string()),
            fetched_at_ms: 1,
            bids: vec![
                PolymarketBookLevel {
                    price: 0.62,
                    quantity: 2.0,
                },
                PolymarketBookLevel {
                    price: 0.60,
                    quantity: 3.0,
                },
            ],
            offers: vec![
                PolymarketBookLevel {
                    price: 0.64,
                    quantity: 2.0,
                },
                PolymarketBookLevel {
                    price: 0.66,
                    quantity: 3.0,
                },
            ],
            minimum_trade_qty: None,
            price_tick_size: None,
        }
    }

    #[test]
    fn derives_short_buy_depth_from_descending_native_bids() {
        let levels = polymarket_buy_levels(&native_book(), PolymarketPositionSide::Short);
        assert_eq!(levels, vec![(0.38, 2), (0.40, 3)]);
        assert_eq!(worst_price(&levels, 3), Some(0.40));
    }

    #[test]
    fn rejects_size_when_worst_case_fee_removes_profit() {
        let kalshi = vec![(50, 10)];
        let poly = vec![(0.50, 10)];
        assert!(find_profitable_contract_size(10, 1, 100.0, 0.0, Some(1.0), &kalshi, &poly).is_none());
    }

    #[test]
    fn uses_worst_level_and_respects_total_amount_limit() {
        let kalshi = vec![(40, 2), (42, 3)];
        let poly = vec![(0.45, 2), (0.47, 3)];
        let result = find_profitable_contract_size(5, 1, 5.0, 0.0, Some(1.0), &kalshi, &poly).unwrap();
        assert_eq!(result.contracts, 5);
        assert_eq!(result.kalshi_price_cents, 42);
        assert_eq!(result.poly_price, 0.47);
    }

    #[test]
    fn sizing_charges_both_exchanges_fees() {
        // 10 contracts, Kalshi 40c, Polymarket 45c:
        //   Kalshi     ceil(0.07 * 10 * 0.40 * 0.60) = 0.17
        //   Polymarket 0.0695 * 10 * 0.45 * 0.55 = 0.1720 -> 0.17
        //   total = 10 * 0.85 + 0.17 + 0.17 = 8.84
        let kalshi = vec![(40, 10)];
        let poly = vec![(0.45, 10)];
        let trade = find_profitable_contract_size(10, 1, 100.0, 0.0, Some(1.0), &kalshi, &poly).unwrap();
        assert_eq!(trade.contracts, 10);
        assert!((trade.kalshi_fee - 0.17).abs() < 1e-9);
        assert!((trade.poly_fee - 0.17).abs() < 1e-9);
        let expected_margin = (10.0 - 8.84) / 8.84 * 100.0;
        assert!((trade.profit_margin - expected_margin).abs() < 1e-9, "{}", trade.profit_margin);
    }

    #[test]
    fn rejects_an_edge_that_only_existed_before_the_polymarket_fee() {
        // Kalshi 48c + Polymarket 50c at 10 contracts: with Kalshi's fee
        // alone this looks profitable (9.80 + 0.18 = 9.98 < 10), but
        // Polymarket's 0.17 makes it a 0.15 loss.
        let kalshi = vec![(48, 10)];
        let poly = vec![(0.50, 10)];
        let kalshi_fee = kalshi_taker_fee(10.0, 0.48);
        assert!(10.0 * (0.48 + 0.50) + kalshi_fee < 10.0, "profitable on Kalshi's fee alone");
        assert!(find_profitable_contract_size(10, 1, 100.0, 0.0, Some(1.0), &kalshi, &poly).is_none());
    }

    #[test]
    fn the_amount_cap_includes_both_fees() {
        let kalshi = vec![(40, 10)];
        let poly = vec![(0.45, 10)];
        // Total cost is 8.84 (see above): a cap between the notional (8.50) and
        // the all-in cost must reject this size and fall back to a smaller one.
        let capped = find_profitable_contract_size(10, 1, 8.60, 0.0, Some(1.0), &kalshi, &poly).unwrap();
        assert!(capped.contracts < 10, "{capped:?}");
        let total = capped.contracts as f64 * (0.40 + 0.45) + capped.kalshi_fee + capped.poly_fee;
        assert!(total <= 8.60 + 1e-9, "{total}");
    }

    #[test]
    fn fee_check_accepts_the_modeled_fee_and_flags_a_changed_schedule() {
        // 10 @ 0.50 -> modeled $0.17 (what Polymarket really collected).
        let (modeled, ok) = polymarket_fee_check(10, 0.50, 0.17);
        assert!((modeled - 0.17).abs() < 1e-9 && ok);
        // One cent either way is rounding, not drift.
        assert!(polymarket_fee_check(10, 0.50, 0.18).1);
        assert!(polymarket_fee_check(10, 0.50, 0.16).1);
        // A 100-lot at 0.50 models $1.74; the old, lower schedule (~0.06)
        // would have collected ~$1.50 and must be flagged.
        assert!(polymarket_fee_check(100, 0.50, 1.74).1);
        assert!(!polymarket_fee_check(100, 0.50, 1.50).1);
        assert!(!polymarket_fee_check(100, 0.50, 2.00).1);
    }

    #[test]
    fn rejects_size_below_polymarket_minimum_notional() {
        // 1 contract at $0.22 is $0.22 notional, below our conservative
        // $1 notional floor (POLYMARKET_MIN_NOTIONAL_USD).
        let kalshi = vec![(63, 10)];
        let poly = vec![(0.22, 10)];
        assert!(find_profitable_contract_size(1, 1, 100.0, 0.0, Some(1.0), &kalshi, &poly).is_none());
    }

    #[test]
    fn accepts_size_at_or_above_polymarket_minimum_notional() {
        let kalshi = vec![(40, 10)];
        let poly = vec![(0.22, 10)];
        // 5 contracts * $0.22 = $1.10, clears the $1 floor.
        let result = find_profitable_contract_size(10, 5, 100.0, 0.0, Some(1.0), &kalshi, &poly).unwrap();
        assert_eq!(result.contracts, 10);
    }

    #[test]
    fn blocks_sizing_when_polymarket_minimum_trade_qty_is_unknown() {
        // The exchange rejects any order below its own per-market minimum
        // (ORD_REJECT_REASON_EXCHANGE_OPTION). If the order service couldn't
        // look that minimum up, we must not guess it's zero.
        let kalshi = vec![(50, 10)];
        let poly = vec![(0.50, 10)];
        assert!(find_profitable_contract_size(10, 1, 100.0, 0.0, None, &kalshi, &poly).is_none());
    }

    #[test]
    fn rejects_size_below_polymarket_minimum_trade_qty() {
        let kalshi = vec![(50, 5)];
        let poly = vec![(0.50, 5)];
        // Only 5 contracts of depth are available, but the market requires
        // at least 8 per order - no size can ever clear both constraints.
        assert!(find_profitable_contract_size(5, 1, 100.0, 0.0, Some(8.0), &kalshi, &poly).is_none());
    }

    #[test]
    fn accepts_size_at_polymarket_minimum_trade_qty() {
        let kalshi = vec![(40, 10)];
        let poly = vec![(0.45, 10)];
        let result =
            find_profitable_contract_size(10, 1, 100.0, 0.0, Some(8.0), &kalshi, &poly).unwrap();
        assert_eq!(result.contracts, 10);
    }

    #[test]
    fn does_not_submit_kalshi_without_a_positive_polymarket_fill() {
        assert!(!can_submit_kalshi_after_polymarket(0));
        assert!(!can_submit_kalshi_after_polymarket(-1));
        assert!(can_submit_kalshi_after_polymarket(1));
    }

    #[test]
    fn a_margin_below_the_configured_minimum_is_not_sized() {
        // Same books as the positive-margin sizing test: margin is positive
        // but far under a 50% floor, so nothing may be sized.
        let kalshi = [(40, 100)];
        let poly = [(0.50, 100)];
        let any = find_profitable_contract_size(10, 1, 100.0, 0.0, Some(1.0), &kalshi, &poly)
            .expect("positive margin sizes with no floor");
        assert!(find_profitable_contract_size(
            10,
            1,
            100.0,
            any.profit_margin + 0.01,
            Some(1.0),
            &kalshi,
            &poly
        )
        .is_none());
        let at_floor =
            find_profitable_contract_size(10, 1, 100.0, any.profit_margin, Some(1.0), &kalshi, &poly)
                .expect("a margin equal to the floor passes");
        assert!(at_floor.profit_margin >= any.profit_margin);
    }

    #[test]
    fn a_non_finite_margin_floor_blocks_sizing() {
        let kalshi = [(40, 100)];
        let poly = [(0.50, 100)];
        assert!(
            find_profitable_contract_size(10, 1, 100.0, f64::NAN, Some(1.0), &kalshi, &poly)
                .is_none()
        );
    }

    #[test]
    fn closing_fee_counts_against_the_loss_bound() {
        // Entry 0.50, 5c bound: bare close at 0.45 passes, but a $0.40 fee
        // on 10 contracts is 4c each, so net 0.41 breaches it.
        assert!(close_within_loss_bound(0.50, 0.45, 10, 0.0, 5));
        assert!(!close_within_loss_bound(0.50, 0.45, 10, 0.40, 5));
        assert!(close_within_loss_bound(0.50, 0.50, 10, 0.40, 5));
        assert!(!close_within_loss_bound(0.50, 0.50, 0, 0.0, 5));
    }

    #[test]
    fn only_suspended_games_block_auto_trade() {
        assert!(is_game_suspended(Some("SUS")));
        assert!(is_game_suspended(Some(" sus ")));
        assert!(!is_game_suspended(Some("P2")));
        assert!(!is_game_suspended(Some("NS")));
        assert!(!is_game_suspended(None));
    }
}
