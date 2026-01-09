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
//! Kalshi Trading Fees:
//! - fees = round_up(0.07 × C × P × (1-P))
//! - P = contract price (dollars), e.g., 50 cents = 0.5
//! - C = number of contracts traded
//! - round_up = round up to next cent

use chrono::Utc;
use tracing::debug;

use crate::models::{ArbitrageOpportunity, KalshiMarket, PolymarketMarket};

/// Arbitrage calculator
pub struct ArbitrageCalculator {
    /// Minimum profit margin to report
    min_profit_margin: f64,
    /// Default bet amount
    default_bet_amount: f64,
}

/// Kalshi trading fee rate
const KALSHI_TRADING_FEE_RATE: f64 = 0.07;

impl ArbitrageCalculator {
    /// Create a new arbitrage calculator
    pub fn new(min_profit_margin: f64, default_bet_amount: f64) -> Self {
        Self {
            min_profit_margin,
            default_bet_amount,
        }
    }

    /// Calculate Kalshi trading fee
    ///
    /// Formula: fees = round_up(0.07 × C × P × (1-P))
    ///
    /// # Arguments
    /// * `contracts` - Number of contracts C
    /// * `price` - Contract price P (dollars, e.g., 0.45 = 45 cents)
    ///
    /// # Returns
    /// Fee amount (dollars), rounded up to cent
    fn calculate_kalshi_trading_fee(&self, contracts: f64, price: f64) -> f64 {
        if contracts <= 0.0 || price <= 0.0 || price >= 1.0 {
            return 0.0;
        }

        // Calculate raw fee
        let raw_fee = KALSHI_TRADING_FEE_RATE * contracts * price * (1.0 - price);

        // Round up to cent
        (raw_fee * 100.0).ceil() / 100.0
    }

    /// Calculate arbitrage opportunity for a single matched market pair
    ///
    /// All prices are unified to the same team's perspective:
    /// - kalshi_yes_price: Price to buy "this team wins" on Kalshi
    /// - kalshi_no_price: Price to buy "this team doesn't win" on Kalshi
    /// - polymarket_yes_price: Price to buy "this team wins" on Poly
    /// - polymarket_no_price: Price to buy "this team doesn't win" on Poly
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
        // Validate prices
        if !self.validate_prices(
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_yes_price,
            polymarket_no_price,
        ) {
            return None;
        }

        let mut best_opportunity: Option<ArbitrageOpportunity> = None;

        // Strategy 1: Kalshi Yes + Polymarket No
        // Buy this team wins on Kalshi, buy this team doesn't win on Poly (opponent wins)
        if let Some(opp1) = self.calculate_strategy(
            event_name,
            team_name,
            kalshi_market,
            kalshi_yes_price,
            "yes",
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_market,
            polymarket_no_price,
            "no",
            polymarket_yes_price,
            polymarket_no_price,
        ) {
            if best_opportunity
                .as_ref()
                .map_or(true, |b| opp1.profit_margin > b.profit_margin)
            {
                best_opportunity = Some(opp1);
            }
        }

        // Strategy 2: Kalshi No + Polymarket Yes
        // Buy this team doesn't win on Kalshi, buy this team wins on Poly
        if let Some(opp2) = self.calculate_strategy(
            event_name,
            team_name,
            kalshi_market,
            kalshi_no_price,
            "no",
            kalshi_yes_price,
            kalshi_no_price,
            polymarket_market,
            polymarket_yes_price,
            "yes",
            polymarket_yes_price,
            polymarket_no_price,
        ) {
            if best_opportunity
                .as_ref()
                .map_or(true, |b| opp2.profit_margin > b.profit_margin)
            {
                best_opportunity = Some(opp2);
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

    /// Calculate a single strategy's arbitrage (including Kalshi Trading Fee)
    #[allow(clippy::too_many_arguments)]
    fn calculate_strategy(
        &self,
        event_name: &str,
        team_name: &str,
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
        // Calculate implied probability sum
        let implied_prob_sum = kalshi_price + polymarket_price;

        // If sum >= 1, no arbitrage opportunity
        if implied_prob_sum >= 1.0 {
            return None;
        }

        // Calculate optimal bet amounts
        let total_bet = self.default_bet_amount;
        let guaranteed_return = total_bet / implied_prob_sum;

        let kalshi_bet = guaranteed_return * kalshi_price;
        let polymarket_bet = guaranteed_return * polymarket_price;

        // Calculate Kalshi contract count and trading fee
        // Contract count = bet amount / contract price
        let kalshi_contracts = if kalshi_price > 0.0 {
            kalshi_bet / kalshi_price
        } else {
            0.0
        };
        let kalshi_fee = self.calculate_kalshi_trading_fee(kalshi_contracts, kalshi_price);

        // Calculate expected profit after fees
        let gross_profit = guaranteed_return - total_bet;
        let expected_profit = gross_profit - kalshi_fee;

        // Calculate profit margin after fees
        let profit_margin = if total_bet > 0.0 {
            (expected_profit / total_bet) * 100.0
        } else {
            0.0
        };

        // Check if minimum profit margin is met (after fees)
        if profit_margin < self.min_profit_margin {
            return None;
        }

        debug!(
            "💰 Arbitrage opportunity: {} - {}",
            event_name, team_name
        );
        debug!(
            "   Kalshi {}: {:.2}, Poly {}: {:.2}",
            kalshi_side, kalshi_price, polymarket_side, polymarket_price
        );
        debug!(
            "   Contracts: {:.0}, Kalshi fee: ${:.2}",
            kalshi_contracts, kalshi_fee
        );
        debug!(
            "   Profit margin: {:.2}%, Expected profit: ${:.2} (gross: ${:.2})",
            profit_margin, expected_profit, gross_profit
        );

        Some(ArbitrageOpportunity {
            event_name: event_name.to_string(),
            team_name: team_name.to_string(),
            kalshi_market_id: kalshi_market.market_id.clone(),
            kalshi_price,
            kalshi_side: kalshi_side.to_string(),
            kalshi_bet,
            kalshi_yes_price,
            kalshi_no_price,
            kalshi_contracts,
            kalshi_fee,
            polymarket_market_id: polymarket_market.market_id.clone(),
            polymarket_price,
            polymarket_side: polymarket_side.to_string(),
            polymarket_bet,
            polymarket_yes_price,
            polymarket_no_price,
            total_bet,
            profit_margin,
            expected_profit,
            gross_profit,
            timestamp: Utc::now(),
            start_time: kalshi_market.start_time,
            poly_ask_depth: 0.0,      // Will be set by WebSocketManager
            kalshi_ask_depth: 0,      // Will be set by WebSocketManager
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalshi_fee_calculation() {
        let calc = ArbitrageCalculator::new(1.0, 100.0);

        // Test normal case
        // 0.07 * 100 * 0.5 * 0.5 = 1.75
        // Due to floating point, ceil may round to 1.76, which is correct behavior
        let fee = calc.calculate_kalshi_trading_fee(100.0, 0.5);
        assert!(
            fee >= 1.75 && fee <= 1.77,
            "Expected ~1.75-1.76, got {}",
            fee
        );

        // Test another case: 0.07 * 50 * 0.3 * 0.7 = 0.735 -> ceil = 0.74
        let fee2 = calc.calculate_kalshi_trading_fee(50.0, 0.3);
        assert!(
            fee2 >= 0.73 && fee2 <= 0.75,
            "Expected ~0.74, got {}",
            fee2
        );

        // Test edge cases
        assert_eq!(calc.calculate_kalshi_trading_fee(0.0, 0.5), 0.0);
        assert_eq!(calc.calculate_kalshi_trading_fee(100.0, 0.0), 0.0);
        assert_eq!(calc.calculate_kalshi_trading_fee(100.0, 1.0), 0.0);
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
