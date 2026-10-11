//! Exchange taker fees, the single source of truth for every margin in the
//! bot (detection in `core::calculator`, sizing in `services::paired_execution`,
//! stored estimates in `services::storage`). Both legs of an arbitrage are
//! taker (IOC) orders, so every trade pays both fees; a margin that omits
//! either one is overstated.
//!
//! Both exchanges charge `rate x contracts x price x (1 - price)`, which is
//! symmetric around 50c (so the same figure applies whether a leg is priced
//! as YES or as NO) and rounded to a whole cent:
//!
//! - **Kalshi**: rate 0.07, rounded **up** to the next cent.
//! - **Polymarket US**: taker rate 0.0695, rounded to the **nearest** cent
//!   with banker's rounding (round half to even) -
//!   <https://docs.polymarket.us/fees.md>. An order's fee is the banker's
//!   rounding of the *cumulative exact* fee across all of its fills, not a
//!   sum of per-fill roundings, so `contracts` here is the whole order. The
//!   rule was checked against the published 99-row fee schedule and against
//!   every real fill on the account since 2026-09-17 (see the tests); fills
//!   before ~2026-09-11 followed an older, lower schedule.
//!
//! Maker rebates (Polymarket: -0.0125) are deliberately ignored: every order
//! this bot sends is an immediate-or-cancel taker order.

/// Kalshi taker fee coefficient.
pub const KALSHI_TAKER_FEE_RATE: f64 = 0.07;

/// Polymarket US taker fee coefficient (docs.polymarket.us/fees.md, "Θ").
pub const POLYMARKET_TAKER_FEE_RATE: f64 = 0.0695;

fn is_valid_fill(contracts: f64, price: f64) -> bool {
    contracts > 0.0 && price > 0.0 && price < 1.0
}

/// Kalshi taker fee in dollars for `contracts` at `price` (dollars):
/// `ceil_to_cent(0.07 x C x P x (1 - P))`. Zero for an invalid fill.
pub fn kalshi_taker_fee(contracts: f64, price: f64) -> f64 {
    if !is_valid_fill(contracts, price) {
        return 0.0;
    }
    let raw_fee = KALSHI_TAKER_FEE_RATE * contracts * price * (1.0 - price);
    (raw_fee * 100.0).ceil() / 100.0
}

/// Polymarket US taker fee in dollars for an order of `contracts` filled at
/// `price` (the price of the side being bought; the formula is symmetric, so
/// the YES-book price gives the same fee). Zero for an invalid fill.
pub fn polymarket_taker_fee(contracts: f64, price: f64) -> f64 {
    if !is_valid_fill(contracts, price) {
        return 0.0;
    }
    round_to_cent_half_even(POLYMARKET_TAKER_FEE_RATE * contracts * price * (1.0 - price))
}

/// Dollars rounded to the nearest cent, ties to the even cent. Float noise
/// around an exact tie (17.500000000000004 cents) is treated as a tie.
fn round_to_cent_half_even(dollars: f64) -> f64 {
    let cents = dollars * 100.0;
    let floor = cents.floor();
    let rounded = if (cents - floor - 0.5).abs() < 1e-9 {
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        cents.round()
    };
    rounded / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cents(dollars: f64) -> i64 {
        (dollars * 100.0).round() as i64
    }

    #[test]
    fn kalshi_fee_rounds_up_to_the_cent() {
        // 0.07 * 100 * 0.5 * 0.5 = 1.75 exactly; float noise may push the
        // ceil to 1.76, which is conservative and was always the behaviour.
        let fee = kalshi_taker_fee(100.0, 0.5);
        assert!((1.75..=1.76).contains(&fee), "got {fee}");
        // 0.07 * 50 * 0.3 * 0.7 = 0.735 -> 0.74
        assert!((kalshi_taker_fee(50.0, 0.3) - 0.74).abs() < 1e-9);
        // 0.07 * 10 * 0.46 * 0.54 = 0.17388 -> 0.18 (the 2026-10-01 trade)
        assert!((kalshi_taker_fee(10.0, 0.46) - 0.18).abs() < 1e-9);
    }

    #[test]
    fn invalid_fills_pay_no_fee() {
        for (c, p) in [(0.0, 0.5), (-1.0, 0.5), (10.0, 0.0), (10.0, 1.0), (10.0, 1.2), (10.0, -0.1)] {
            assert_eq!(kalshi_taker_fee(c, p), 0.0, "kalshi {c} @ {p}");
            assert_eq!(polymarket_taker_fee(c, p), 0.0, "polymarket {c} @ {p}");
        }
    }

    /// Every row of Polymarket US's published "Standard Fee Schedule by
    /// Price" (100-lot taker fee, cents), docs.polymarket.us/fees.md.
    const OFFICIAL_SCHEDULE_100_LOT: [(i64, i64); 99] = [
        (1, 7), (2, 14), (3, 20), (4, 27), (5, 33), (6, 39), (7, 45), (8, 51), (9, 57),
        (10, 63), (11, 68), (12, 73), (13, 79), (14, 84), (15, 89), (16, 93), (17, 98), (18, 103),
        (19, 107), (20, 111), (21, 115), (22, 119), (23, 123), (24, 127), (25, 130), (26, 134), (27, 137),
        (28, 140), (29, 143), (30, 146), (31, 149), (32, 151), (33, 154), (34, 156), (35, 158), (36, 160),
        (37, 162), (38, 164), (39, 165), (40, 167), (41, 168), (42, 169), (43, 170), (44, 171), (45, 172),
        (46, 173), (47, 173), (48, 173), (49, 174), (50, 174), (51, 174), (52, 173), (53, 173), (54, 173),
        (55, 172), (56, 171), (57, 170), (58, 169), (59, 168), (60, 167), (61, 165), (62, 164), (63, 162),
        (64, 160), (65, 158), (66, 156), (67, 154), (68, 151), (69, 149), (70, 146), (71, 143), (72, 140),
        (73, 137), (74, 134), (75, 130), (76, 127), (77, 123), (78, 119), (79, 115), (80, 111), (81, 107),
        (82, 103), (83, 98), (84, 93), (85, 89), (86, 84), (87, 79), (88, 73), (89, 68), (90, 63),
        (91, 57), (92, 51), (93, 45), (94, 39), (95, 33), (96, 27), (97, 20), (98, 14), (99, 7),
    ];

    #[test]
    fn matches_the_official_polymarket_fee_schedule() {
        for (price_cents, fee_cents) in OFFICIAL_SCHEDULE_100_LOT {
            let fee = polymarket_taker_fee(100.0, price_cents as f64 / 100.0);
            assert_eq!(cents(fee), fee_cents, "100 contracts @ {price_cents}c -> {fee}");
        }
    }

    #[test]
    fn matches_every_real_fill_since_the_current_schedule_took_effect() {
        // (contracts, average price, fee Polymarket actually collected) for
        // every order on the live account from 2026-09-17 on, straight from
        // GET /v1/portfolio/activities (exact fractional quantities
        // included). Prices are the YES-book prices as reported; the formula
        // is symmetric, so a NO-side order pays the same.
        let fills: [(f64, f64, f64); 26] = [
            (1.0, 0.53, 0.02),
            (1.0, 0.53, 0.02),
            (11.03, 0.10, 0.07), // 3 trades (9.93 + 1 + 0.1), one rounding for the order
            (8.0, 0.59, 0.13),   // 2 trades: per-fill 0.11 + 0.02, but exact 0.1345 -> 0.13
            (1.0, 0.73, 0.01),
            (1.0, 0.59, 0.02),
            (2.63, 0.365, 0.04), // fractional contracts
            (10.0, 0.49, 0.17),
            (10.0, 0.50, 0.17),
            (10.0, 0.50, 0.17),
            (10.0, 0.57, 0.17),
            (10.0, 0.56, 0.17),
            (10.0, 0.55, 0.17),
            (10.0, 0.55, 0.17),
            (59.52, 0.07, 0.27),
            (55.67, 0.95, 0.18),
            (10.0, 0.524, 0.17), // 2 trades: 6 @ 0.52 and 4 @ 0.53
            (10.0, 0.55, 0.17),
            (1.0, 0.14, 0.01),
            (1.0, 0.50, 0.02), // 0.0174 -> up
            (1.0, 0.51, 0.02),
            (1.0, 0.50, 0.02),
            (1.0, 0.04, 0.00), // 0.0027 -> zero
            (1.0, 0.70, 0.01), // 0.0146 -> down
            (1.0, 0.70, 0.01),
            (1.0, 0.70, 0.01),
        ];
        for (contracts, price, collected) in fills {
            let modeled = polymarket_taker_fee(contracts, price);
            assert_eq!(
                cents(modeled),
                cents(collected),
                "{contracts} @ {price}: modeled {modeled}, collected {collected}"
            );
        }
    }

    #[test]
    fn polymarket_fee_is_symmetric_between_yes_and_no_prices() {
        for yes_cents in 1..100 {
            let yes = yes_cents as f64 / 100.0;
            assert_eq!(
                cents(polymarket_taker_fee(37.0, yes)),
                cents(polymarket_taker_fee(37.0, 1.0 - yes)),
                "{yes_cents}c"
            );
        }
    }

    #[test]
    fn rounding_is_bankers_not_half_up() {
        // Exact ties: 0.025 -> 0.02 (even), 0.035 -> 0.04 (even), per the
        // exchange's FAQ. Float noise around the tie must not break it.
        assert_eq!(cents(round_to_cent_half_even(0.025)), 2);
        assert_eq!(cents(round_to_cent_half_even(0.035)), 4);
        assert_eq!(cents(round_to_cent_half_even(0.045)), 4);
        assert_eq!(cents(round_to_cent_half_even(0.015)), 2);
        assert_eq!(cents(round_to_cent_half_even(0.175)), 18);
        assert_eq!(cents(round_to_cent_half_even(0.17499)), 17);
        assert_eq!(cents(round_to_cent_half_even(0.17501)), 18);
        assert_eq!(cents(round_to_cent_half_even(0.0)), 0);
    }

    #[test]
    fn the_polymarket_fee_is_about_a_cent_and_three_quarters_at_fifty_cents() {
        // Docs: max at p = 0.50 is $1.74 per 100 contracts.
        assert!((polymarket_taker_fee(100.0, 0.5) - 1.74).abs() < 1e-9);
        assert!((polymarket_taker_fee(10.0, 0.5) - 0.17).abs() < 1e-9);
    }
}
