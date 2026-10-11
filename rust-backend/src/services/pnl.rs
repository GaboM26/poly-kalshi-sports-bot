//! P&L report over `auto_trade_history`.
//!
//! Estimates only: the table stores the planned prices (not exchange average
//! fills), the modeled fees, and no close price for neutralization orders.
//! Hedged pairs are therefore valued at their locked-in settlement profit,
//! `paired contracts - cost of both legs - both fees`, and neutralized or
//! exposed rows are reported as counts with a bounded or at-risk amount, never
//! as a precise realized loss.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::services::storage::AutoTradeRecord;

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct PnlBucket {
    pub trades: u32,
    pub contracts: i64,
    /// Cash committed to hedged pairs, fees included.
    pub invested: f64,
    /// Locked-in settlement profit of hedged pairs, net of both fees.
    pub estimated_profit: f64,
    /// Hedged pairs whose Polymarket fee was not recorded (counted as $0).
    pub missing_polymarket_fee: u32,
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct PnlReport {
    pub hedged: PnlBucket,
    pub by_day: BTreeMap<String, PnlBucket>,
    pub by_sport: BTreeMap<String, PnlBucket>,
    /// Unequal fills closed by a bounded offset; exact loss not stored.
    pub neutralized_trades: u32,
    pub neutralized_max_loss: f64,
    /// Unequal fills left open: money still exposed to the game result.
    pub exposed_trades: u32,
    pub exposed_contracts: i64,
    pub exposed_at_risk: f64,
    pub rejected_trades: u32,
    pub skipped_trades: u32,
}

/// Sport label from the Kalshi ticker prefix (e.g. `KXNBAGAME-...`).
fn sport_of(kalshi_market_id: &str) -> String {
    let series = kalshi_market_id.split('-').next().unwrap_or("");
    series.strip_prefix("KX").unwrap_or(series).to_string()
}

fn add(bucket: &mut PnlBucket, contracts: i32, invested: f64, profit: f64, fee_missing: bool) {
    bucket.trades += 1;
    bucket.contracts += i64::from(contracts);
    bucket.invested += invested;
    bucket.estimated_profit += profit;
    bucket.missing_polymarket_fee += u32::from(fee_missing);
}

/// `neutralization_max_loss_cents` bounds the per-contract loss of a
/// neutralization; it is the same setting the executor enforced.
pub fn build_report(records: &[AutoTradeRecord], neutralization_max_loss_cents: f64) -> PnlReport {
    let mut report = PnlReport::default();
    for r in records {
        match r.status.as_str() {
            "executed" => {
                let paired = r.kalshi_filled_contracts.min(r.polymarket_filled_contracts);
                if paired <= 0 {
                    continue;
                }
                let n = f64::from(paired);
                let poly_fee = r.polymarket_fee.unwrap_or(0.0);
                let invested = n * (r.kalshi_price + r.polymarket_price) + r.kalshi_fee + poly_fee;
                let profit = n - invested;
                let missing = r.polymarket_fee.is_none();
                add(&mut report.hedged, paired, invested, profit, missing);
                let day = r.created_at.get(..10).unwrap_or("unknown").to_string();
                add(report.by_day.entry(day).or_default(), paired, invested, profit, missing);
                let sport = sport_of(&r.kalshi_market_id);
                add(report.by_sport.entry(sport).or_default(), paired, invested, profit, missing);
            }
            "neutralized" => {
                report.neutralized_trades += 1;
                report.neutralized_max_loss += f64::from(r.neutralization_filled_contracts)
                    * neutralization_max_loss_cents
                    / 100.0;
            }
            "exposed" => {
                report.exposed_trades += 1;
                report.exposed_contracts += i64::from(r.residual_contracts);
                // Cost of the unhedged leg is its entry price, fee included.
                let entry = match r.residual_leg.as_deref() {
                    Some("kalshi") => r.kalshi_price,
                    _ => r.polymarket_price,
                };
                report.exposed_at_risk += f64::from(r.residual_contracts) * entry;
            }
            "rejected" => report.rejected_trades += 1,
            "skipped" => report.skipped_trades += 1,
            _ => {}
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str) -> AutoTradeRecord {
        serde_json::from_value(serde_json::json!({
            "id": 1, "event_name": "E", "team_name": "T",
            "kalshi_market_id": "KXNBAGAME-26OCT01AAABBB-AAA",
            "polymarket_market_id": "p", "kalshi_side": "yes", "polymarket_side": "short",
            "kalshi_contracts": 10, "kalshi_price": 0.50, "kalshi_fee": 0.18,
            "polymarket_fee": 0.17, "polymarket_amount": 4.4, "polymarket_price": 0.44,
            "total_amount": 9.75, "profit_margin": 3.0, "duration_ms": 0,
            "total_duration_ms": 0, "kalshi_success": true, "polymarket_success": true,
            "kalshi_order_id": null, "polymarket_order_id": null, "kalshi_error": null,
            "polymarket_error": null, "kalshi_latency_ms": null, "poly_latency_ms": null,
            "status": status, "skip_reason": null, "kalshi_filled_contracts": 10,
            "polymarket_filled_contracts": 10, "kalshi_order_status": null,
            "polymarket_order_status": null, "neutralization_leg": null,
            "neutralization_success": null, "neutralization_order_id": null,
            "neutralization_filled_contracts": 0, "neutralization_error": null,
            "residual_leg": null, "residual_contracts": 0,
            "created_at": "2026-09-29T00:43:46+00:00"
        }))
        .unwrap()
    }

    #[test]
    fn a_hedged_pair_earns_settlement_minus_cost_and_both_fees() {
        let report = build_report(&[row("executed")], 5.0);
        // 10 * (1 - 0.50 - 0.44) - 0.18 - 0.17 = 0.25
        assert!((report.hedged.estimated_profit - 0.25).abs() < 1e-9);
        assert!((report.hedged.invested - 9.75).abs() < 1e-9);
        assert_eq!(report.by_sport["NBAGAME"].trades, 1);
        assert_eq!(report.by_day["2026-09-29"].contracts, 10);
    }

    #[test]
    fn only_the_paired_quantity_counts_and_a_missing_fee_is_flagged() {
        let mut r = row("executed");
        r.polymarket_filled_contracts = 4;
        r.polymarket_fee = None;
        let report = build_report(&[r], 5.0);
        assert_eq!(report.hedged.contracts, 4);
        assert_eq!(report.hedged.missing_polymarket_fee, 1);
    }

    #[test]
    fn neutralized_and_exposed_rows_are_not_counted_as_profit() {
        let mut n = row("neutralized");
        n.neutralization_filled_contracts = 2;
        let mut e = row("exposed");
        e.residual_leg = Some("kalshi".into());
        e.residual_contracts = 3;
        let report = build_report(&[n, e, row("rejected"), row("skipped")], 5.0);
        assert_eq!(report.hedged.trades, 0);
        assert!((report.neutralized_max_loss - 0.10).abs() < 1e-9);
        assert_eq!(report.exposed_contracts, 3);
        assert!((report.exposed_at_risk - 1.5).abs() < 1e-9);
        assert_eq!((report.rejected_trades, report.skipped_trades), (1, 1));
    }
}
