//! Arbitrage Calculator
//!
//! Arbitrage principle:
//! - If two platforms price the same team's win differently, there may be arbitrage opportunity
//! - Arbitrage condition: sum of prices on both platforms < 1 (implied probability sum < 100%)
//!
//! Price semantics (unified to Yes/No perspective):
//! - Kalshi MEM Yes: Buy MEM wins
//! - Kalshi MEM No: Buy MEM doesn't win
//! - Poly MEM Yes: Buy MEM wins (= prices[0] if MEM is team_a)
//! - Poly MEM No: Buy MEM doesn't win (= prices[1] = LAL wins)
//!
//! Trading fees (both legs are taker orders, so BOTH exchanges charge; see
//! `core::fees` for the formulas, rounding rules and their sources):
//! - Kalshi: round_up_to_cent(0.07 × C × P × (1-P))
//! - Polymarket US: round_half_even_to_cent(0.0695 × C × P × (1-P))
//! - P = contract price (dollars), e.g., 50 cents = 0.5
//! - C = number of contracts traded
//!
//! Omitting the Polymarket fee overstated every margin by ~1.7 points near
//! 50¢ (it is ~$0.17 on 10 contracts at 50¢), turning a displayed 2.25%
//! "opportunity" into a real ~0.5%.

use chrono::{NaiveDate, Utc};
use tracing::debug;

use crate::core::fees::{kalshi_taker_fee, polymarket_taker_fee};
use crate::models::{ArbitrageOpportunity, KalshiMarket, PolymarketMarket};

/// Arbitrage calculator using Fixed Contract Strategy
pub struct ArbitrageCalculator {
    /// Minimum profit margin to report
    min_profit_margin: f64,
    /// Fixed contract count for both platforms (e.g., 10 contracts)
    fixed_contract_count: f64,
}

impl ArbitrageCalculator {
    /// Create a new arbitrage calculator
    ///
    /// # Arguments
    /// * `min_profit_margin` - Minimum profit margin percentage to report (e.g., 0.1 = 0.1%)
    /// * `fixed_contract_count` - Fixed number of contracts to buy on each platform (e.g., 10)
    /// Minimum net margin (percent) an opportunity must clear to be reported.
    pub fn min_profit_margin(&self) -> f64 {
        self.min_profit_margin
    }

    pub fn new(min_profit_margin: f64, fixed_contract_count: f64) -> Self {
        Self {
            min_profit_margin,
            fixed_contract_count,
        }
    }

    /// Calculate arbitrage opportunity for a single matched market pair
    ///
    /// All prices are unified to the same team's perspective:
    /// - kalshi_yes_price: Price to buy "this team wins" on Kalshi
    /// - kalshi_no_price: Price to buy "this team doesn't win" on Kalshi
    /// - polymarket_yes_price: Price to buy "this team wins" on Poly
    /// - polymarket_no_price: Price to buy "this team doesn't win" on Poly
    ///
    /// Returns the better of the two hedges only if it clears the configured
    /// minimum margin (after both exchanges' fees).
    pub fn calculate_single(
        &self,
        event_name: &str,
        team_name: &str,
        kalshi_market: &KalshiMarket,
        kalshi_yes_price: f64,
        kalshi_no_price: f64,
        polymarket_market: &PolymarketMarket,
        polymarket_yes_price: f64,
        polymarket_no_price: f64,
    ) -> Option<ArbitrageOpportunity> {
        self.best_strategy(
            event_name,
            team_name,
            kalshi_market,
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_market,
            polymarket_yes_price,
            polymarket_no_price,
            true,
        )
    }

    /// The better of the two hedges at the given prices WITHOUT the minimum-
    /// margin gate: contracts, both fees and the net margin are reported even
    /// when the margin is negative. For display only (the Details tab) - it is
    /// never an opportunity and nothing may trade on it.
    pub fn preview_single(
        &self,
        event_name: &str,
        team_name: &str,
        kalshi_market: &KalshiMarket,
        kalshi_yes_price: f64,
        kalshi_no_price: f64,
        polymarket_market: &PolymarketMarket,
        polymarket_yes_price: f64,
        polymarket_no_price: f64,
    ) -> Option<ArbitrageOpportunity> {
        self.best_strategy(
            event_name,
            team_name,
            kalshi_market,
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_market,
            polymarket_yes_price,
            polymarket_no_price,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn best_strategy(
        &self,
        event_name: &str,
        team_name: &str,
        kalshi_market: &KalshiMarket,
        kalshi_yes_price: f64,
        kalshi_no_price: f64,
        polymarket_market: &PolymarketMarket,
        polymarket_yes_price: f64,
        polymarket_no_price: f64,
        gated: bool,
    ) -> Option<ArbitrageOpportunity> {
        // Extract game_date from kalshi_market start_time
        let game_date = kalshi_market.start_time.map(|t| t.date_naive());
        // Validate prices
        if !self.validate_prices(
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_yes_price,
            polymarket_no_price,
        ) {
            return None;
        }

        // Strategy 1: Kalshi Yes + Polymarket No
        //   Buy this team wins on Kalshi, buy this team doesn't win on Poly (opponent wins)
        // Strategy 2: Kalshi No + Polymarket Yes
        //   Buy this team doesn't win on Kalshi, buy this team wins on Poly
        let strategies = [
            (kalshi_yes_price, "yes", polymarket_no_price, "no"),
            (kalshi_no_price, "no", polymarket_yes_price, "yes"),
        ];

        let mut best_opportunity: Option<ArbitrageOpportunity> = None;
        for (kalshi_price, kalshi_side, polymarket_price, polymarket_side) in strategies {
            let opportunity = if gated {
                self.calculate_strategy(
                    event_name,
                    team_name,
                    game_date,
                    kalshi_market,
                    kalshi_price,
                    kalshi_side,
                    kalshi_yes_price,
                    kalshi_no_price,
                    polymarket_market,
                    polymarket_price,
                    polymarket_side,
                    polymarket_yes_price,
                    polymarket_no_price,
                )
            } else {
                Some(self.evaluate_strategy(
                    event_name,
                    team_name,
                    game_date,
                    kalshi_market,
                    kalshi_price,
                    kalshi_side,
                    kalshi_yes_price,
                    kalshi_no_price,
                    polymarket_market,
                    polymarket_price,
                    polymarket_side,
                    polymarket_yes_price,
                    polymarket_no_price,
                ))
            };
            if let Some(opportunity) = opportunity {
                if best_opportunity
                    .as_ref()
                    .map_or(true, |best| opportunity.profit_margin > best.profit_margin)
                {
                    best_opportunity = Some(opportunity);
                }
            }
        }

        best_opportunity
    }

    /// Validate price validity
    fn validate_prices(&self, k_yes: f64, k_no: f64, p_yes: f64, p_no: f64) -> bool {
        for price in [k_yes, k_no, p_yes, p_no] {
            if price <= 0.01 || price >= 0.99 {
                return false;
            }
        }
        true
    }

    /// One strategy's arbitrage, gated: `None` unless prices sum below $1 and
    /// the net margin (after both fees) reaches the configured minimum.
    #[allow(clippy::too_many_arguments)]
    fn calculate_strategy(
        &self,
        event_name: &str,
        team_name: &str,
        game_date: Option<NaiveDate>,
        kalshi_market: &KalshiMarket,
        kalshi_price: f64,
        kalshi_side: &str,
        kalshi_yes_price: f64,
        kalshi_no_price: f64,
        polymarket_market: &PolymarketMarket,
        polymarket_price: f64,
        polymarket_side: &str,
        polymarket_yes_price: f64,
        polymarket_no_price: f64,
    ) -> Option<ArbitrageOpportunity> {
        // Basic arbitrage condition: price sum < 1
        let implied_prob_sum = kalshi_price + polymarket_price;
        if implied_prob_sum >= 1.0 {
            return None;
        }

        let opportunity = self.evaluate_strategy(
            event_name,
            team_name,
            game_date,
            kalshi_market,
            kalshi_price,
            kalshi_side,
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_market,
            polymarket_price,
            polymarket_side,
            polymarket_yes_price,
            polymarket_no_price,
        );

        // Check if minimum profit margin is met (after fees)
        if opportunity.profit_margin < self.min_profit_margin {
            return None;
        }

        debug!(
            "💰 Arbitrage (Fixed Contract): {} - {}",
            event_name, team_name
        );
        debug!(
            "   Kalshi {}: {:.2}¢, Poly {}: {:.2}¢, Sum: {:.4}",
            kalshi_side,
            kalshi_price * 100.0,
            polymarket_side,
            polymarket_price * 100.0,
            implied_prob_sum
        );
        debug!(
            "   Fixed contracts: {:.0}, Kalshi fee: ${:.2}, Polymarket fee: ${:.2}",
            opportunity.kalshi_contracts, opportunity.kalshi_fee, opportunity.polymarket_fee
        );
        debug!(
            "   Total cost: ${:.2}, Return: ${:.0}, Profit: ${:.2} ({:.2}%)",
            opportunity.total_bet,
            opportunity.kalshi_contracts,
            opportunity.expected_profit,
            opportunity.profit_margin
        );

        Some(opportunity)
    }

    /// The economics of one strategy using the Fixed Contract Strategy, with
    /// no gating (the caller decides whether it is an opportunity).
    ///
    /// Fixed Contract Strategy:
    /// - Buy same number of contracts on both platforms (default 10)
    /// - Guaranteed return = N × $1 (whichever side wins)
    /// - Total cost = N × kalshi_price + N × poly_price + kalshi_fee + polymarket_fee
    /// - Profit = N - total_cost
    #[allow(clippy::too_many_arguments)]
    fn evaluate_strategy(
        &self,
        event_name: &str,
        team_name: &str,
        game_date: Option<NaiveDate>,
        kalshi_market: &KalshiMarket,
        kalshi_price: f64,
        kalshi_side: &str,
        kalshi_yes_price: f64,
        kalshi_no_price: f64,
        polymarket_market: &PolymarketMarket,
        polymarket_price: f64,
        polymarket_side: &str,
        polymarket_yes_price: f64,
        polymarket_no_price: f64,
    ) -> ArbitrageOpportunity {
        // Fixed Contract Strategy: buy same number of contracts on both platforms
        let fixed_contracts = self.fixed_contract_count;

        // Calculate actual costs
        let kalshi_bet = fixed_contracts * kalshi_price;
        let polymarket_bet = fixed_contracts * polymarket_price;

        // Both legs are taker orders and both exchanges charge for them.
        let kalshi_fee = kalshi_taker_fee(fixed_contracts, kalshi_price);
        let polymarket_fee = polymarket_taker_fee(fixed_contracts, polymarket_price);

        // Total investment including both fees
        let total_bet = kalshi_bet + kalshi_fee + polymarket_bet + polymarket_fee;

        // Guaranteed return: N contracts × $1 = $N (whichever side wins)
        let guaranteed_return = fixed_contracts;

        // Calculate profit
        let gross_profit = guaranteed_return - (kalshi_bet + polymarket_bet);
        let expected_profit = guaranteed_return - total_bet;

        // Calculate profit margin (after fees)
        let profit_margin = if total_bet > 0.0 {
            (expected_profit / total_bet) * 100.0
        } else {
            0.0
        };

        ArbitrageOpportunity {
            event_name: event_name.to_string(),
            team_name: team_name.to_string(),
            game_date,
            kalshi_market_id: kalshi_market.market_id.clone(),
            kalshi_price,
            kalshi_side: kalshi_side.to_string(),
            kalshi_bet,
            kalshi_yes_price,
            kalshi_no_price,
            kalshi_contracts: fixed_contracts,
            kalshi_fee,
            polymarket_market_id: polymarket_market.market_id.clone(),
            polymarket_price,
            polymarket_side: polymarket_side.to_string(),
            polymarket_bet,
            polymarket_fee,
            polymarket_yes_price,
            polymarket_no_price,
            total_bet,
            profit_margin,
            expected_profit,
            gross_profit,
            timestamp: Utc::now(),
            start_time: kalshi_market.start_time,
            poly_ask_depth: 0.0, // Will be set by WebSocketManager
            poly_ask_size: 0.0,  // Will be set by WebSocketManager
            kalshi_ask_depth: 0, // Will be set by WebSocketManager
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PolymarketPositionSide;

    fn kalshi_market() -> KalshiMarket {
        KalshiMarket {
            market_id: "KXITFMATCH-26OCT01SHABYN-SHA".to_string(),
            event_id: "KXITFMATCH-26OCT01SHABYN".to_string(),
            event_name: "ARYAN SHAH VS EVAN BYNOE".to_string(),
            team_name: "ARYAN SHAH".to_string(),
            opponent_name: "EVAN BYNOE".to_string(),
            yes_price: 0.5,
            no_price: 0.5,
            start_time: None,
            volume: None,
            liquidity: None,
        }
    }

    fn polymarket_market() -> PolymarketMarket {
        PolymarketMarket {
            market_id: "1".to_string(),
            market_slug: "aec-itfme-arysha-evabyn-2026-10-01".to_string(),
            event_name: "ARYAN SHAH VS EVAN BYNOE".to_string(),
            team_a: "ARYAN SHAH".to_string(),
            team_b: "EVAN BYNOE".to_string(),
            price_a: 0.5,
            price_b: 0.5,
            team_a_position: PolymarketPositionSide::Long,
            team_b_position: PolymarketPositionSide::Short,
            start_time: None,
            volume: None,
            live: false,
            period: None,
        }
    }

    /// Best opportunity for ARYAN SHAH given the four side prices.
    fn opportunity(
        calc: &ArbitrageCalculator,
        kalshi_yes: f64,
        kalshi_no: f64,
        poly_yes: f64,
        poly_no: f64,
    ) -> Option<ArbitrageOpportunity> {
        calc.calculate_single(
            "ARYAN SHAH VS EVAN BYNOE",
            "ARYAN SHAH",
            &kalshi_market(),
            kalshi_yes,
            kalshi_no,
            &polymarket_market(),
            poly_yes,
            poly_no,
        )
    }

    #[test]
    fn the_polymarket_taker_fee_is_part_of_every_margin() {
        // The 2026-10-01 Shah/Bynoe opportunity: Kalshi NO at 46c + Polymarket
        // LONG at 50c, 10 contracts, which the UI showed as 2.25%.
        let calc = ArbitrageCalculator::new(0.1, 10.0);
        let opp = opportunity(&calc, 0.54, 0.46, 0.50, 0.50).expect("still profitable");
        assert_eq!(opp.kalshi_side, "no");
        assert_eq!(opp.polymarket_side, "yes");

        // Kalshi: ceil(0.07 * 10 * 0.46 * 0.54) = 0.18; Polymarket:
        // 0.0695 * 10 * 0.5 * 0.5 = 0.17375 -> 0.17.
        assert!((opp.kalshi_fee - 0.18).abs() < 1e-9);
        assert!((opp.polymarket_fee - 0.17).abs() < 1e-9);
        assert!((opp.total_bet - (4.6 + 0.18 + 5.0 + 0.17)).abs() < 1e-9);
        assert!((opp.expected_profit - (10.0 - 9.95)).abs() < 1e-9);
        // 2.25% with the Kalshi fee only, 0.50% once Polymarket's is counted.
        let kalshi_only: f64 = (10.0 - 9.78) / 9.78 * 100.0;
        assert!((kalshi_only - 2.25).abs() < 0.005);
        assert!((opp.profit_margin - 0.5025).abs() < 0.001, "{}", opp.profit_margin);
        // Gross profit is, by definition, before fees.
        assert!((opp.gross_profit - 0.4).abs() < 1e-9);
    }

    #[test]
    fn an_edge_smaller_than_both_fees_is_no_longer_an_opportunity() {
        // Under the default 1% minimum the same trade used to clear it at 2.25%.
        let calc = ArbitrageCalculator::new(1.0, 10.0);
        assert!(opportunity(&calc, 0.54, 0.46, 0.50, 0.50).is_none());
        // A genuinely wide edge still is one, with both fees charged.
        let opp = opportunity(&calc, 0.60, 0.40, 0.50, 0.50).expect("10c gross edge clears 1%");
        assert!(opp.polymarket_fee > 0.0 && opp.kalshi_fee > 0.0);
        assert!(
            (opp.total_bet - (opp.kalshi_bet + opp.polymarket_bet + opp.kalshi_fee + opp.polymarket_fee))
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn the_fee_is_charged_on_the_price_actually_paid_for_the_polymarket_leg() {
        // Strategy 1 buys Polymarket "no" (the opponent) at 0.40; the
        // formula is symmetric, so this must equal the fee at 0.60.
        let calc = ArbitrageCalculator::new(0.0, 100.0);
        let opp = opportunity(&calc, 0.30, 0.70, 0.60, 0.40).expect("profitable");
        assert_eq!(opp.polymarket_side, "no");
        assert!((opp.polymarket_price - 0.40).abs() < 1e-9);
        assert_eq!(opp.polymarket_fee, polymarket_taker_fee(100.0, 0.60));
    }

    fn preview(
        calc: &ArbitrageCalculator,
        kalshi_yes: f64,
        kalshi_no: f64,
        poly_yes: f64,
        poly_no: f64,
    ) -> Option<ArbitrageOpportunity> {
        calc.preview_single(
            "ARYAN SHAH VS EVAN BYNOE",
            "ARYAN SHAH",
            &kalshi_market(),
            kalshi_yes,
            kalshi_no,
            &polymarket_market(),
            poly_yes,
            poly_no,
        )
    }

    #[test]
    fn the_preview_reports_both_fees_even_when_nothing_clears_the_minimum() {
        // The 2026-10-01 trade again, under the default 1% minimum: not an
        // opportunity, but the details page still gets its real economics.
        let calc = ArbitrageCalculator::new(1.0, 10.0);
        assert!(opportunity(&calc, 0.54, 0.46, 0.50, 0.50).is_none());
        let preview = preview(&calc, 0.54, 0.46, 0.50, 0.50).expect("preview ignores the gate");
        assert_eq!((preview.kalshi_side.as_str(), preview.polymarket_side.as_str()), ("no", "yes"));
        assert!((preview.kalshi_fee - 0.18).abs() < 1e-9);
        assert!((preview.polymarket_fee - 0.17).abs() < 1e-9);
        assert!((preview.expected_profit - 0.05).abs() < 1e-9);
        assert!((preview.profit_margin - 0.5025).abs() < 0.001);
    }

    #[test]
    fn the_preview_shows_a_loss_as_a_loss() {
        // Both hedges cost more than $1 once spreads and fees are paid.
        let calc = ArbitrageCalculator::new(1.0, 10.0);
        assert!(opportunity(&calc, 0.55, 0.46, 0.52, 0.49).is_none());
        let preview = preview(&calc, 0.55, 0.46, 0.52, 0.49).expect("preview");
        // The better hedge is Kalshi NO 46c + Polymarket YES 52c = 0.98 a
        // contract: +$0.20 gross, but 0.18 + 0.17 of fees leaves -$0.15.
        assert_eq!((preview.kalshi_side.as_str(), preview.polymarket_side.as_str()), ("no", "yes"));
        assert!((preview.gross_profit - 0.20).abs() < 1e-9);
        assert!((preview.expected_profit - (-0.15)).abs() < 1e-9, "{}", preview.expected_profit);
        assert!(preview.profit_margin < 0.0);
    }

    #[test]
    fn the_preview_never_disagrees_with_a_real_opportunity() {
        let calc = ArbitrageCalculator::new(1.0, 10.0);
        let real = opportunity(&calc, 0.60, 0.40, 0.50, 0.50).expect("opportunity");
        let shown = preview(&calc, 0.60, 0.40, 0.50, 0.50).expect("preview");
        assert_eq!(shown.kalshi_side, real.kalshi_side);
        assert_eq!(shown.polymarket_side, real.polymarket_side);
        assert_eq!(shown.kalshi_fee, real.kalshi_fee);
        assert_eq!(shown.polymarket_fee, real.polymarket_fee);
        assert_eq!(shown.total_bet, real.total_bet);
        assert_eq!(shown.profit_margin, real.profit_margin);
    }

    #[test]
    fn the_preview_still_rejects_unusable_prices() {
        let calc = ArbitrageCalculator::new(1.0, 10.0);
        assert!(preview(&calc, 0.01, 0.99, 0.5, 0.5).is_none());
        assert!(preview(&calc, 0.5, 0.5, 0.0, 1.0).is_none());
    }

    #[test]
    fn test_validate_prices() {
        let calc = ArbitrageCalculator::new(1.0, 100.0);

        // Valid prices
        assert!(calc.validate_prices(0.5, 0.5, 0.5, 0.5));
        assert!(calc.validate_prices(0.02, 0.98, 0.5, 0.5));

        // Invalid prices
        assert!(!calc.validate_prices(0.01, 0.5, 0.5, 0.5)); // too low
        assert!(!calc.validate_prices(0.5, 0.99, 0.5, 0.5)); // too high
        assert!(!calc.validate_prices(0.5, 0.5, 0.0, 0.5)); // zero
    }
}
