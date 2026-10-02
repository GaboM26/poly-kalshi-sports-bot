//! Unified position view: per-leg economics and hedge pairing.
//!
//! Pure functions only (no I/O) so the pairing rules can be unit-tested.
//! Pairing matches the full tuple `(kalshi_market_id, kalshi_side,
//! polymarket_market_id, polymarket_side)`, never a market id alone: the
//! same market pair can appear on opposite sides across different trades.
//! Two sources, in priority order:
//!
//! 1. This bot's own trade history, using only the most recent eligible
//!    history row per held Kalshi leg.
//! 2. The matcher's market mapping (`hedge_pairs_for_market`): a Kalshi YES
//!    on a competitor is offset by Polymarket's position on the *opponent*,
//!    a Kalshi NO by Polymarket's position on the competitor. This is what
//!    recognises a hedge placed by hand - on the order form or on either
//!    exchange's own site - which leaves no bot history row.
//!
//! Hedged means "opposite outcomes of the same game on the two exchanges",
//! so legs pointing the same way (both pay if the same competitor wins) are
//! never paired by either source.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use crate::models::MatchedMarket;
use crate::services::storage::{AutoTradeRecord, HedgePair};

/// History statuses that represent a trade which actually opened a hedge.
const PAIRED_STATUSES: [&str; 3] = ["executed", "neutralized", "partial_paired"];

/// `PositionCard::paired_by`: the pair was opened by this bot's auto-trader.
pub const PAIRED_BY_BOT_TRADE: &str = "bot_trade";
/// `PositionCard::paired_by`: the pair was recognised from the matcher's
/// market mapping (placed manually, or outside this bot).
pub const PAIRED_BY_MARKET_MATCH: &str = "market_match";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PositionLeg {
    pub platform: &'static str,
    pub market_id: String,
    pub title: String,
    /// Kalshi: "yes"/"no". Polymarket: "long"/"short".
    pub side: String,
    pub contracts: f64,
    pub entry_price: Option<f64>,
    pub current_price: Option<f64>,
    pub cost: f64,
    pub value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    /// `None` means unknown (Polymarket exposes no fee field), never $0.
    pub fees: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PositionCard {
    /// "paired" or "standalone"
    pub kind: &'static str,
    /// For a paired card, how the two legs were matched
    /// (`PAIRED_BY_BOT_TRADE` / `PAIRED_BY_MARKET_MATCH`); `None` otherwise.
    pub paired_by: Option<&'static str>,
    pub event_name: Option<String>,
    pub legs: Vec<PositionLeg>,
    pub total_cost: f64,
    pub total_value: Option<f64>,
    pub total_pnl: Option<f64>,
    /// Paired legs whose contract counts differ are only partly hedged.
    pub contracts_mismatch: bool,
}

fn num(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

fn leg_economics(contracts: f64, cost: f64, mark: Option<f64>) -> (Option<f64>, Option<f64>, Option<f64>) {
    let entry = (contracts > 0.0).then(|| cost / contracts);
    let value = mark.map(|m| m * contracts);
    let pnl = value.map(|v| v - cost);
    (entry, value, pnl)
}

/// Build a Kalshi leg from a raw `market_positions` entry. `mark` is the
/// per-contract liquidation price of the held side. Returns `None` for a
/// flat or unparseable position.
pub fn kalshi_leg(position: &Value, mark: Option<f64>) -> Option<PositionLeg> {
    let signed = num(position.get("position_fp").or_else(|| position.get("position")))?;
    if signed == 0.0 {
        return None;
    }
    let ticker = position.get("ticker")?.as_str()?.to_string();
    let contracts = signed.abs();
    // Sign of market_exposure_dollars is undocumented for NO positions.
    let cost = num(position.get("market_exposure_dollars").or_else(|| position.get("market_exposure")))?.abs();
    let (entry_price, value, unrealized_pnl) = leg_economics(contracts, cost, mark);
    Some(PositionLeg {
        platform: "kalshi",
        title: position
            .get("event_ticker")
            .and_then(Value::as_str)
            .unwrap_or(&ticker)
            .to_string(),
        market_id: ticker,
        side: if signed > 0.0 { "yes" } else { "no" }.to_string(),
        contracts,
        entry_price,
        current_price: mark,
        cost,
        value,
        unrealized_pnl,
        fees: num(position.get("fees_paid_dollars").or_else(|| position.get("fees_paid"))),
    })
}

/// Build a Polymarket leg from a `normalize_positions` entry.
pub fn polymarket_leg(position: &Value) -> Option<PositionLeg> {
    let signed = num(position.get("size"))?;
    if signed == 0.0 {
        return None;
    }
    let market_id = position.get("id")?.as_str()?.to_string();
    let contracts = signed.abs();
    let cost = num(position.get("cost"))?.abs();
    let value = num(position.get("value")).map(f64::abs);
    let (entry_price, _, _) = leg_economics(contracts, cost, None);
    Some(PositionLeg {
        platform: "polymarket",
        title: position
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&market_id)
            .to_string(),
        market_id,
        side: if signed > 0.0 { "long" } else { "short" }.to_string(),
        contracts,
        entry_price,
        current_price: value.map(|v| v / contracts),
        cost,
        value,
        unrealized_pnl: value.map(|v| v - cost),
        fees: None,
    })
}

fn card(
    kind: &'static str,
    paired_by: Option<&'static str>,
    event_name: Option<String>,
    legs: Vec<PositionLeg>,
) -> PositionCard {
    let total_cost = legs.iter().map(|l| l.cost).sum();
    let total_value = legs.iter().map(|l| l.value).sum::<Option<f64>>();
    let contracts_mismatch = legs.len() == 2 && (legs[0].contracts - legs[1].contracts).abs() > 1e-9;
    PositionCard {
        kind,
        paired_by,
        event_name,
        total_cost,
        total_pnl: total_value.map(|v| v - total_cost),
        total_value,
        contracts_mismatch,
        legs,
    }
}

/// The two hedges the matcher's mapping defines for one matched market.
/// A Kalshi YES on the tracked competitor is offset by Polymarket's
/// *opponent* position, and a Kalshi NO by Polymarket's position on the
/// tracked competitor itself - the same rule the auto-trader uses to pick
/// its Polymarket leg (CLAUDE.md pairing invariant). The native LONG/SHORT
/// side comes from the gateway's per-competitor mapping, never from an
/// outcome index.
pub fn hedge_pairs_for_market(matched: &MatchedMarket) -> Vec<HedgePair> {
    let polymarket = &matched.polymarket_market;
    [
        ("yes", polymarket.get_opponent(&matched.team_name)),
        ("no", Some(matched.team_name.as_str())),
    ]
    .into_iter()
    .filter_map(|(kalshi_side, competitor)| {
        let execution = polymarket.us_execution_for_competitor(competitor?)?;
        Some(HedgePair {
            event_name: matched.event_name.clone(),
            kalshi_market_id: matched.kalshi_market.market_id.clone(),
            kalshi_side: kalshi_side.to_string(),
            polymarket_market_id: execution.market_slug,
            polymarket_side: execution.position_side.as_str().to_string(),
        })
    })
    .collect()
}

/// Pairs recognised through the market mapping that are not remembered yet,
/// so the caller can persist them (see `ArbitrageStorage::remember_hedge_pairs`).
pub fn newly_verified_pairs(cards: &[PositionCard], known: &[HedgePair]) -> Vec<HedgePair> {
    let known: HashSet<(&str, String, &str, String)> = known
        .iter()
        .map(|p| {
            (
                p.kalshi_market_id.as_str(),
                p.kalshi_side.to_ascii_lowercase(),
                p.polymarket_market_id.as_str(),
                p.polymarket_side.to_ascii_lowercase(),
            )
        })
        .collect();
    let mut seen = HashSet::new();
    cards
        .iter()
        .filter(|c| c.paired_by == Some(PAIRED_BY_MARKET_MATCH))
        .filter_map(|c| {
            let [kalshi, polymarket] = c.legs.as_slice() else {
                return None;
            };
            let pair = HedgePair {
                event_name: c.event_name.clone().unwrap_or_default(),
                kalshi_market_id: kalshi.market_id.clone(),
                kalshi_side: kalshi.side.to_ascii_lowercase(),
                polymarket_market_id: polymarket.market_id.clone(),
                polymarket_side: polymarket.side.to_ascii_lowercase(),
            };
            let key = (
                pair.kalshi_market_id.as_str(),
                pair.kalshi_side.clone(),
                pair.polymarket_market_id.as_str(),
                pair.polymarket_side.clone(),
            );
            (!known.contains(&key) && seen.insert(pair.clone())).then_some(pair)
        })
        .collect()
}

/// Group held legs into hedged pairs and standalone leftovers. `history`
/// must be ordered newest first (as `get_auto_trade_history` returns it);
/// `known_pairs` are the matcher-derived and remembered `HedgePair`s. Each
/// leg lands in exactly one card, and the bot's own history claims its legs
/// before the market mapping is consulted for what is left.
pub fn pair_positions(
    kalshi: Vec<PositionLeg>,
    polymarket: Vec<PositionLeg>,
    history: &[AutoTradeRecord],
    known_pairs: &[HedgePair],
) -> Vec<PositionCard> {
    let mut poly: Vec<Option<PositionLeg>> = polymarket.into_iter().map(Some).collect();
    let mut claimed = vec![false; poly.len()];
    // Per Kalshi leg: (index of its Polymarket partner, event name, source).
    let mut matches: Vec<Option<(usize, String, &'static str)>> = vec![None; kalshi.len()];

    let free_poly_leg = |poly: &[Option<PositionLeg>], claimed: &[bool], market_id: &str, side: &str| {
        poly.iter().enumerate().position(|(i, p)| {
            !claimed[i]
                && p.as_ref().is_some_and(|p| {
                    p.market_id == market_id && p.side.eq_ignore_ascii_case(side)
                })
        })
    };

    // Pass 1: the bot's own trade history. Only the most recent eligible
    // row for this exact Kalshi leg counts.
    for (ki, k) in kalshi.iter().enumerate() {
        let Some(row) = history.iter().find(|r| {
            PAIRED_STATUSES.contains(&r.status.as_str())
                && r.kalshi_market_id == k.market_id
                && r.kalshi_side.eq_ignore_ascii_case(&k.side)
        }) else {
            continue;
        };
        if let Some(pi) = free_poly_leg(&poly, &claimed, &row.polymarket_market_id, &row.polymarket_side) {
            claimed[pi] = true;
            matches[ki] = Some((pi, row.event_name.clone(), PAIRED_BY_BOT_TRADE));
        }
    }

    // Pass 2: the market mapping, for whatever the history didn't explain
    // (hedges placed by hand or outside this bot).
    for (ki, k) in kalshi.iter().enumerate() {
        if matches[ki].is_some() {
            continue;
        }
        for pair in known_pairs {
            if pair.kalshi_market_id != k.market_id || !pair.kalshi_side.eq_ignore_ascii_case(&k.side) {
                continue;
            }
            if let Some(pi) =
                free_poly_leg(&poly, &claimed, &pair.polymarket_market_id, &pair.polymarket_side)
            {
                claimed[pi] = true;
                matches[ki] = Some((pi, pair.event_name.clone(), PAIRED_BY_MARKET_MATCH));
                break;
            }
        }
    }

    let mut cards = Vec::new();
    for (k, matched) in kalshi.into_iter().zip(matches) {
        match matched {
            Some((pi, event_name, paired_by)) => {
                let p = poly[pi].take().expect("claimed index is still present");
                cards.push(card("paired", Some(paired_by), Some(event_name), vec![k, p]));
            }
            None => cards.push(card("standalone", None, None, vec![k])),
        }
    }
    cards.extend(
        poly.into_iter()
            .flatten()
            .map(|p| card("standalone", None, None, vec![p])),
    );
    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn leg(platform: &'static str, id: &str, side: &str, contracts: f64, cost: f64) -> PositionLeg {
        PositionLeg {
            platform,
            market_id: id.into(),
            title: id.into(),
            side: side.into(),
            contracts,
            entry_price: Some(cost / contracts),
            current_price: None,
            cost,
            value: None,
            unrealized_pnl: None,
            fees: None,
        }
    }

    fn row(status: &str, k: &str, ks: &str, p: &str, ps: &str) -> AutoTradeRecord {
        AutoTradeRecord {
            id: 0,
            event_name: "Game".into(),
            team_name: "T".into(),
            kalshi_market_id: k.into(),
            polymarket_market_id: p.into(),
            kalshi_side: ks.into(),
            polymarket_side: ps.into(),
            kalshi_contracts: 0,
            kalshi_price: 0.0,
            kalshi_fee: 0.0,
            polymarket_amount: 0.0,
            polymarket_price: 0.0,
            total_amount: 0.0,
            profit_margin: 0.0,
            duration_ms: 0,
            total_duration_ms: 0,
            kalshi_success: true,
            polymarket_success: true,
            kalshi_order_id: None,
            polymarket_order_id: None,
            kalshi_error: None,
            polymarket_error: None,
            kalshi_latency_ms: None,
            poly_latency_ms: None,
            status: status.into(),
            skip_reason: None,
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
            created_at: String::new(),
        }
    }

    #[test]
    fn matching_history_pairs_legs() {
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[row("executed", "K1", "yes", "P1", "short")],
            &[],
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "paired");
        assert_eq!(cards[0].legs.len(), 2);
        assert!((cards[0].total_cost - 4.5).abs() < 1e-9);
        assert!(!cards[0].contracts_mismatch);
    }

    #[test]
    fn unmatched_legs_are_standalone() {
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P9", "short", 5.0, 2.5)],
            &[],
            &[],
        );
        assert_eq!(cards.len(), 2);
        assert!(cards.iter().all(|c| c.kind == "standalone"));
    }

    #[test]
    fn rejected_and_skipped_rows_do_not_pair() {
        for status in ["skipped", "failed", "submitting", "rejected"] {
            let cards = pair_positions(
                vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
                vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
                &[row(status, "K1", "yes", "P1", "short")],
            &[],
        );
            assert_eq!(cards.len(), 2, "{status}");
        }
    }

    #[test]
    fn opposite_side_history_does_not_pair() {
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[row("executed", "K1", "no", "P1", "long")],
            &[],
        );
        assert_eq!(cards.len(), 2);
    }

    #[test]
    fn only_most_recent_matching_row_counts() {
        // Newest row for K1/yes points at a different Polymarket market;
        // the older row that would match P1 must be ignored.
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[
                row("executed", "K1", "yes", "P2", "short"),
                row("executed", "K1", "yes", "P1", "short"),
            ],
            &[],
        );
        assert_eq!(cards.len(), 2);
    }

    #[test]
    fn stale_opposite_row_does_not_shadow_newer_match() {
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[
                row("executed", "K1", "yes", "P1", "short"),
                row("executed", "K1", "no", "P1", "long"),
            ],
            &[],
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "paired");
    }

    #[test]
    fn poly_leg_is_never_in_two_cards() {
        let cards = pair_positions(
            vec![
                leg("kalshi", "K1", "yes", 5.0, 2.0),
                leg("kalshi", "K1", "yes", 3.0, 1.0),
            ],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[row("executed", "K1", "yes", "P1", "short")],
            &[],
        );
        let poly_count = cards
            .iter()
            .flat_map(|c| &c.legs)
            .filter(|l| l.platform == "polymarket")
            .count();
        assert_eq!(poly_count, 1);
        assert_eq!(cards.iter().map(|c| c.legs.len()).sum::<usize>(), 3);
    }

    #[test]
    fn neutralized_status_pairs_and_flags_mismatch() {
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 3.0, 1.5)],
            &[row("neutralized", "K1", "yes", "P1", "short")],
            &[],
        );
        assert_eq!(cards[0].kind, "paired");
        assert!(cards[0].contracts_mismatch);
    }

    // --- Market-mapping pairing (hedges placed outside the bot) ---

    use crate::models::{KalshiMarket, PolymarketMarket, PolymarketPositionSide};

    /// The real ITF match from 2026-10-02: Kalshi has one market per player
    /// (the `-SIN` market is "Digvijay Pratap Singh wins"), Polymarket has
    /// one market whose LONG side is Arda Azkara and SHORT side is Singh.
    fn azk_sin_market(tracked: &str) -> MatchedMarket {
        let (ticker, opponent) = if tracked == "SINGH" {
            ("KXITFMATCH-26OCT02AZKSIN-SIN", "AZKARA")
        } else {
            ("KXITFMATCH-26OCT02AZKSIN-AZK", "SINGH")
        };
        MatchedMarket {
            event_name: "AZKARA VS SINGH".into(),
            team_name: tracked.into(),
            game_date: None,
            kalshi_market: KalshiMarket {
                market_id: ticker.into(),
                event_id: "KXITFMATCH-26OCT02AZKSIN".into(),
                event_name: "AZKARA VS SINGH".into(),
                team_name: tracked.into(),
                opponent_name: opponent.into(),
                yes_price: 0.5,
                no_price: 0.5,
                start_time: None,
                volume: None,
                liquidity: None,
            },
            polymarket_market: PolymarketMarket {
                market_id: "1151900".into(),
                market_slug: "aec-itfme-ardazk-digsin-2026-10-02".into(),
                event_name: "AZKARA VS SINGH".into(),
                team_a: "AZKARA".into(),
                team_b: "SINGH".into(),
                price_a: 0.54,
                price_b: 0.47,
                team_a_position: PolymarketPositionSide::Long,
                team_b_position: PolymarketPositionSide::Short,
                start_time: None,
                volume: None,
                live: false,
                period: None,
            },
            poly_yes_price: 0.5,
            poly_no_price: 0.5,
            confidence: 1.0,
        }
    }

    fn azk_sin_pairs() -> Vec<HedgePair> {
        ["SINGH", "AZKARA"]
            .into_iter()
            .flat_map(|team| hedge_pairs_for_market(&azk_sin_market(team)))
            .collect()
    }

    const SIN_TICKER: &str = "KXITFMATCH-26OCT02AZKSIN-SIN";
    const AZK_TICKER: &str = "KXITFMATCH-26OCT02AZKSIN-AZK";
    const AZK_SIN_SLUG: &str = "aec-itfme-ardazk-digsin-2026-10-02";

    #[test]
    fn market_mapping_defines_the_complementary_hedges() {
        let pairs = hedge_pairs_for_market(&azk_sin_market("SINGH"));
        let find = |side: &str| pairs.iter().find(|p| p.kalshi_side == side).unwrap();
        // Kalshi YES on Singh pays if Singh wins -> hedge by owning the
        // OPPONENT on Polymarket: Azkara is the LONG side.
        assert_eq!(find("yes").polymarket_market_id, AZK_SIN_SLUG);
        assert_eq!(find("yes").polymarket_side, "long");
        // Kalshi NO on Singh pays if Singh loses -> hedge with Singh
        // himself on Polymarket, which is the SHORT side.
        assert_eq!(find("no").polymarket_side, "short");
        assert_eq!(pairs.len(), 2);
        assert!(pairs.iter().all(|p| p.kalshi_market_id == SIN_TICKER));
    }

    #[test]
    fn a_manual_hedge_with_no_history_row_is_paired() {
        // The exact positions held on 2026-10-01: Kalshi NO x2 on the SIN
        // market and Polymarket SHORT x2 on the match, no bot history.
        let cards = pair_positions(
            vec![leg("kalshi", SIN_TICKER, "no", 2.0, 1.02)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 2.0, 0.98)],
            &[],
            &azk_sin_pairs(),
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "paired");
        assert_eq!(cards[0].paired_by, Some(PAIRED_BY_MARKET_MATCH));
        assert_eq!(cards[0].event_name.as_deref(), Some("AZKARA VS SINGH"));
        assert!(!cards[0].contracts_mismatch);
        assert!((cards[0].total_cost - 2.0).abs() < 1e-9);
    }

    #[test]
    fn legs_pointing_the_same_way_are_not_a_hedge() {
        // Kalshi NO on Singh (Azkara wins) + Polymarket LONG (Azkara wins)
        // is double exposure, not a hedge.
        let cards = pair_positions(
            vec![leg("kalshi", SIN_TICKER, "no", 2.0, 1.02)],
            vec![leg("polymarket", AZK_SIN_SLUG, "long", 2.0, 1.10)],
            &[],
            &azk_sin_pairs(),
        );
        assert_eq!(cards.len(), 2);
        assert!(cards.iter().all(|c| c.kind == "standalone" && c.paired_by.is_none()));

        // Likewise Kalshi YES on Singh + Polymarket SHORT (both: Singh wins).
        let cards = pair_positions(
            vec![leg("kalshi", SIN_TICKER, "yes", 2.0, 1.0)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 2.0, 1.0)],
            &[],
            &azk_sin_pairs(),
        );
        assert!(cards.iter().all(|c| c.kind == "standalone"));
    }

    #[test]
    fn the_other_players_kalshi_market_hedges_the_opposite_polymarket_leg() {
        // Kalshi YES on Azkara pays if Azkara wins; the hedge is Singh = SHORT.
        let cards = pair_positions(
            vec![leg("kalshi", AZK_TICKER, "yes", 3.0, 1.6)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 3.0, 1.4)],
            &[],
            &azk_sin_pairs(),
        );
        assert_eq!(cards[0].kind, "paired");
        // ...and the same Polymarket leg does NOT hedge Kalshi NO on Azkara.
        let cards = pair_positions(
            vec![leg("kalshi", AZK_TICKER, "no", 3.0, 1.6)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 3.0, 1.4)],
            &[],
            &azk_sin_pairs(),
        );
        assert!(cards.iter().all(|c| c.kind == "standalone"));
    }

    #[test]
    fn a_mapped_hedge_with_different_sizes_is_paired_but_flagged_partial() {
        let cards = pair_positions(
            vec![leg("kalshi", SIN_TICKER, "no", 5.0, 2.5)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 2.0, 0.98)],
            &[],
            &azk_sin_pairs(),
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "paired");
        assert!(cards[0].contracts_mismatch);
    }

    #[test]
    fn bot_history_takes_priority_over_the_market_mapping() {
        // K1/yes has a history row pairing it with P1; the mapping says P2
        // hedges it. History wins for K1, and P2 stays free for K2.
        let known = vec![
            HedgePair {
                event_name: "MAPPED".into(),
                kalshi_market_id: "K1".into(),
                kalshi_side: "yes".into(),
                polymarket_market_id: "P2".into(),
                polymarket_side: "short".into(),
            },
            HedgePair {
                event_name: "MAPPED2".into(),
                kalshi_market_id: "K2".into(),
                kalshi_side: "yes".into(),
                polymarket_market_id: "P2".into(),
                polymarket_side: "short".into(),
            },
        ];
        let cards = pair_positions(
            vec![
                leg("kalshi", "K1", "yes", 5.0, 2.0),
                leg("kalshi", "K2", "yes", 5.0, 2.0),
            ],
            vec![
                leg("polymarket", "P1", "short", 5.0, 2.5),
                leg("polymarket", "P2", "short", 5.0, 2.5),
            ],
            &[row("executed", "K1", "yes", "P1", "short")],
            &known,
        );
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].paired_by, Some(PAIRED_BY_BOT_TRADE));
        assert_eq!(cards[0].legs[1].market_id, "P1");
        assert_eq!(cards[1].paired_by, Some(PAIRED_BY_MARKET_MATCH));
        assert_eq!(cards[1].legs[1].market_id, "P2");
    }

    #[test]
    fn a_polymarket_leg_is_claimed_by_one_kalshi_leg_only() {
        let known = vec![
            HedgePair {
                event_name: "E".into(),
                kalshi_market_id: "K1".into(),
                kalshi_side: "yes".into(),
                polymarket_market_id: "P1".into(),
                polymarket_side: "short".into(),
            },
            HedgePair {
                event_name: "E".into(),
                kalshi_market_id: "K2".into(),
                kalshi_side: "no".into(),
                polymarket_market_id: "P1".into(),
                polymarket_side: "short".into(),
            },
        ];
        let cards = pair_positions(
            vec![leg("kalshi", "K1", "yes", 5.0, 2.0), leg("kalshi", "K2", "no", 5.0, 2.0)],
            vec![leg("polymarket", "P1", "short", 5.0, 2.5)],
            &[],
            &known,
        );
        assert_eq!(cards.iter().filter(|c| c.kind == "paired").count(), 1);
        assert_eq!(cards.iter().flat_map(|c| &c.legs).count(), 3);
    }

    #[test]
    fn a_remembered_pair_keeps_a_hedge_paired_after_its_market_is_gone() {
        // Once the game closes it drops out of the matched markets, so only
        // the remembered row (loaded from the database) can still pair it.
        let remembered = azk_sin_pairs();
        let cards = pair_positions(
            vec![leg("kalshi", SIN_TICKER, "no", 2.0, 1.02)],
            vec![leg("polymarket", AZK_SIN_SLUG, "short", 2.0, 0.98)],
            &[],
            &remembered,
        );
        assert_eq!(cards[0].kind, "paired");
    }

    #[test]
    fn newly_verified_pairs_reports_only_unremembered_mapped_pairs() {
        let cards = pair_positions(
            vec![
                leg("kalshi", SIN_TICKER, "no", 2.0, 1.02),
                leg("kalshi", "K9", "yes", 5.0, 2.0),
            ],
            vec![
                leg("polymarket", AZK_SIN_SLUG, "short", 2.0, 0.98),
                leg("polymarket", "P9", "short", 5.0, 2.5),
            ],
            &[row("executed", "K9", "yes", "P9", "short")],
            &azk_sin_pairs(),
        );
        // The bot-trade pair needs no remembering; the mapped one does.
        let fresh = newly_verified_pairs(&cards, &[]);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].kalshi_market_id, SIN_TICKER);
        assert_eq!(fresh[0].kalshi_side, "no");
        assert_eq!(fresh[0].polymarket_market_id, AZK_SIN_SLUG);
        assert_eq!(fresh[0].polymarket_side, "short");
        // Already remembered -> nothing to write.
        assert!(newly_verified_pairs(&cards, &fresh).is_empty());
    }

    #[test]
    fn kalshi_no_leg_economics() {
        let raw = json!({
            "ticker": "K1", "event_ticker": "EV",
            "position_fp": "-10.00", "market_exposure_dollars": "-4.20",
            "fees_paid_dollars": "0.15"
        });
        let l = kalshi_leg(&raw, Some(0.50)).unwrap();
        assert_eq!(l.side, "no");
        assert_eq!(l.contracts, 10.0);
        assert!((l.entry_price.unwrap() - 0.42).abs() < 1e-9);
        assert!((l.unrealized_pnl.unwrap() - 0.80).abs() < 1e-9);
        assert_eq!(l.fees, Some(0.15));
    }

    #[test]
    fn polymarket_leg_economics_and_unknown_fee() {
        let raw = json!({"id": "P1", "title": "T", "size": "-4", "cost": 1.6, "value": 2.0});
        let l = polymarket_leg(&raw).unwrap();
        assert_eq!(l.side, "short");
        assert!((l.entry_price.unwrap() - 0.4).abs() < 1e-9);
        assert!((l.current_price.unwrap() - 0.5).abs() < 1e-9);
        assert!((l.unrealized_pnl.unwrap() - 0.4).abs() < 1e-9);
        assert_eq!(l.fees, None);
    }

    #[test]
    fn flat_positions_are_dropped() {
        assert!(kalshi_leg(&json!({"ticker":"K","position_fp":"0","market_exposure_dollars":"0"}), None).is_none());
        assert!(polymarket_leg(&json!({"id":"P","size":"0","cost":0})).is_none());
    }
}
