//! Order Builder for Polymarket CLOB
//!
//! Handles order creation, amount calculation, and signing.

use alloy::primitives::{keccak256, Address, B256, U256};
use anyhow::{bail, Result};
use rand::Rng;
use tracing::{debug, info};

use super::config::{get_contract_config, to_token_decimals, ContractConfig};
use super::signer::Signer;
use super::types::{OrderArgs, LimitOrderArgs, Side, SignatureType, SignedOrder, OrderBookSummary, OrderType, MarketOrderArgs};
use crate::utils;

/// Rounding configuration for different tick sizes
#[derive(Debug, Clone, Copy)]
pub struct RoundConfig {
    pub price: i32,
    pub size: i32,
    pub amount: i32,
}

/// Get rounding config for a tick size
pub fn get_round_config(tick_size: f64) -> RoundConfig {
    if tick_size >= 0.1 {
        RoundConfig { price: 1, size: 2, amount: 3 }
    } else if tick_size >= 0.01 {
        RoundConfig { price: 2, size: 2, amount: 4 }
    } else if tick_size >= 0.001 {
        RoundConfig { price: 3, size: 2, amount: 5 }
    } else {
        RoundConfig { price: 4, size: 2, amount: 6 }
    }
}

/// Round down to specified decimal places
pub fn round_down(x: f64, sig_digits: i32) -> f64 {
    let multiplier = 10_f64.powi(sig_digits);
    (x * multiplier).floor() / multiplier
}

/// Round normally to specified decimal places
pub fn round_normal(x: f64, sig_digits: i32) -> f64 {
    let multiplier = 10_f64.powi(sig_digits);
    (x * multiplier).round() / multiplier
}

/// Round up to specified decimal places
pub fn round_up(x: f64, sig_digits: i32) -> f64 {
    let multiplier = 10_f64.powi(sig_digits);
    (x * multiplier).ceil() / multiplier
}

/// Count decimal places in a float
pub fn decimal_places(x: f64) -> i32 {
    let s = format!("{}", x);
    if let Some(pos) = s.find('.') {
        // Remove trailing zeros
        let decimal_part = s[pos + 1..].trim_end_matches('0');
        decimal_part.len() as i32
    } else {
        0
    }
}

/// Order builder for creating and signing orders
pub struct OrderBuilder {
    signer: Signer,
    sig_type: SignatureType,
    funder: Address,
    chain_id: u64,
}

impl OrderBuilder {
    /// Create a new order builder
    ///
    /// # Arguments
    /// * `signer` - The wallet signer
    /// * `sig_type` - Signature type (defaults to EOA)
    /// * `funder` - Address that holds funds (defaults to signer address)
    pub fn new(
        signer: Signer,
        sig_type: Option<SignatureType>,
        funder: Option<Address>,
    ) -> Self {
        let chain_id = signer.chain_id();
        let default_funder = signer.address();
        Self {
            signer,
            sig_type: sig_type.unwrap_or(SignatureType::Eoa),
            funder: funder.unwrap_or(default_funder),
            chain_id,
        }
    }

    /// Calculate order amounts based on side, size, and price
    ///
    /// Returns (side_value, maker_amount, taker_amount)
    pub fn get_order_amounts(
        &self,
        side: Side,
        size: f64,
        price: f64,
        round_config: RoundConfig,
    ) -> Result<(u8, u64, u64)> {
        let raw_price = round_normal(price, round_config.price);

        match side {
            Side::Buy => {
                let raw_taker_amt = round_down(size, round_config.size);
                let mut raw_maker_amt = raw_taker_amt * raw_price;

                if decimal_places(raw_maker_amt) > round_config.amount {
                    raw_maker_amt = round_up(raw_maker_amt, round_config.amount + 4);
                    if decimal_places(raw_maker_amt) > round_config.amount {
                        raw_maker_amt = round_down(raw_maker_amt, round_config.amount);
                    }
                }

                let maker_amount = to_token_decimals(raw_maker_amt);
                let taker_amount = to_token_decimals(raw_taker_amt);

                Ok((0, maker_amount, taker_amount)) // 0 = BUY
            }
            Side::Sell => {
                let raw_maker_amt = round_down(size, round_config.size);
                let mut raw_taker_amt = raw_maker_amt * raw_price;

                if decimal_places(raw_taker_amt) > round_config.amount {
                    raw_taker_amt = round_up(raw_taker_amt, round_config.amount + 4);
                    if decimal_places(raw_taker_amt) > round_config.amount {
                        raw_taker_amt = round_down(raw_taker_amt, round_config.amount);
                    }
                }

                let maker_amount = to_token_decimals(raw_maker_amt);
                let taker_amount = to_token_decimals(raw_taker_amt);

                Ok((1, maker_amount, taker_amount)) // 1 = SELL
            }
        }
    }

    /// Calculate market order amounts (matches py-clob-client get_market_order_amounts)
    ///
    /// Semantic (same as official SDK):
    /// - BUY: amount = USDC to spend, makerAmount = USDC, takerAmount = tokens
    /// - SELL: amount = tokens to sell, makerAmount = tokens, takerAmount = USDC
    pub fn get_market_order_amounts(
        &self,
        side: Side,
        amount: f64,
        price: f64,
        round_config: RoundConfig,
    ) -> Result<(u8, u64, u64)> {
        let raw_price = round_normal(price, round_config.price);
        
        debug!(
            "🔢 [市价单金额计算] side={:?}, amount={}, price={}, raw_price={}",
            side, amount, price, raw_price
        );

        match side {
            Side::Buy => {
                // BUY: amount = USDC to spend
                // makerAmount = USDC (what you pay)
                // takerAmount = tokens (what you receive) = USDC / price
                let raw_maker_amt = round_down(amount, round_config.size);
                let mut raw_taker_amt = raw_maker_amt / raw_price;

                if decimal_places(raw_taker_amt) > round_config.amount {
                    raw_taker_amt = round_up(raw_taker_amt, round_config.amount + 4);
                    if decimal_places(raw_taker_amt) > round_config.amount {
                        raw_taker_amt = round_down(raw_taker_amt, round_config.amount);
                    }
                }

                let maker_amount = to_token_decimals(raw_maker_amt);  // USDC
                let taker_amount = to_token_decimals(raw_taker_amt);  // tokens
                
                info!(
                    "🔢 [市价单-BUY] USDC={} -> maker_amount={}, tokens={} -> taker_amount={}",
                    raw_maker_amt, maker_amount, raw_taker_amt, taker_amount
                );

                Ok((0, maker_amount, taker_amount))
            }
            Side::Sell => {
                // SELL: amount = tokens to sell
                // makerAmount = tokens (what you sell)
                // takerAmount = USDC (what you receive) = tokens * price
                let raw_maker_amt = round_down(amount, round_config.size);
                let mut raw_taker_amt = raw_maker_amt * raw_price;

                if decimal_places(raw_taker_amt) > round_config.amount {
                    raw_taker_amt = round_up(raw_taker_amt, round_config.amount + 4);
                    if decimal_places(raw_taker_amt) > round_config.amount {
                        raw_taker_amt = round_down(raw_taker_amt, round_config.amount);
                    }
                }

                let maker_amount = to_token_decimals(raw_maker_amt);  // tokens
                let taker_amount = to_token_decimals(raw_taker_amt);  // USDC
                
                info!(
                    "🔢 [市价单-SELL] tokens={} -> maker_amount={}, USDC={} -> taker_amount={}",
                    raw_maker_amt, maker_amount, raw_taker_amt, taker_amount
                );

                Ok((1, maker_amount, taker_amount))
            }
        }
    }

    /// Create and sign an order
    pub async fn create_order(
        &self,
        order_args: &OrderArgs,
        tick_size: f64,
        neg_risk: bool,
    ) -> Result<SignedOrder> {
        let round_config = get_round_config(tick_size);
        let (side_value, maker_amount, taker_amount) =
            self.get_order_amounts(order_args.side, order_args.size, order_args.price, round_config)?;

        // Generate nonce if not provided
        let nonce = order_args.nonce.unwrap_or_else(|| rand::thread_rng().gen());
        let expiration = order_args.expiration.unwrap_or(0);
        let fee_rate_bps = order_args.fee_rate_bps.unwrap_or(0);

        // Get contract config
        let contract_config = get_contract_config(self.chain_id)
            .ok_or_else(|| anyhow::anyhow!("Unsupported chain ID: {}", self.chain_id))?;

        // Build order data
        let order_data = OrderData {
            maker: self.funder,
            taker: Address::ZERO,
            token_id: order_args.token_id.clone(),
            maker_amount,
            taker_amount,
            side: side_value,
            fee_rate_bps,
            nonce,
            signer: self.signer.address(),
            expiration,
            signature_type: self.sig_type as u8,
        };

        // Sign the order with correct exchange address based on neg_risk
        self.sign_order(&order_data, &contract_config, neg_risk).await
    }

    /// Create and sign a market order (matches py-clob-client create_market_order)
    ///
    /// For market orders:
    /// - BUY: amount = USDC to spend (e.g., 100 means spend $100)
    /// - SELL: amount = tokens/shares to sell
    pub async fn create_market_order(
        &self,
        order_args: &MarketOrderArgs,
        price: f64,
        tick_size: f64,
        neg_risk: bool,
    ) -> Result<SignedOrder> {
        let round_config = get_round_config(tick_size);
        let (side_value, maker_amount, taker_amount) =
            self.get_market_order_amounts(order_args.side, order_args.amount, price, round_config)?;

        let nonce = order_args.nonce.unwrap_or_else(|| rand::thread_rng().gen());
        let fee_rate_bps = order_args.fee_rate_bps.unwrap_or(0);

        let contract_config = get_contract_config(self.chain_id)
            .ok_or_else(|| anyhow::anyhow!("Unsupported chain ID: {}", self.chain_id))?;

        info!(
            "🔐 [市价单创建] token_id={}, side={:?}, amount={}, price={}, maker_amt={}, taker_amt={}",
            order_args.token_id, order_args.side, order_args.amount, price, maker_amount, taker_amount
        );

        let order_data = OrderData {
            maker: self.funder,
            taker: Address::ZERO,
            token_id: order_args.token_id.clone(),
            maker_amount,
            taker_amount,
            side: side_value,
            fee_rate_bps,
            nonce,
            signer: self.signer.address(),
            expiration: 0, // Market orders have no expiration
            signature_type: self.sig_type as u8,
        };

        self.sign_order(&order_data, &contract_config, neg_risk).await
    }

    /// Create and sign a limit order using LimitOrderArgs
    pub async fn create_limit_order(
        &self,
        order_args: &LimitOrderArgs,
        tick_size: f64,
        neg_risk: bool,
    ) -> Result<SignedOrder> {
        let round_config = get_round_config(tick_size);
        let (side_value, maker_amount, taker_amount) =
            self.get_order_amounts(order_args.side, order_args.size, order_args.price, round_config)?;

        let nonce = order_args.nonce.unwrap_or_else(|| rand::thread_rng().gen());
        let expiration = order_args.expiration.unwrap_or(0);
        let fee_rate_bps = order_args.fee_rate_bps.unwrap_or(0);

        let contract_config = get_contract_config(self.chain_id)
            .ok_or_else(|| anyhow::anyhow!("Unsupported chain ID: {}", self.chain_id))?;

        info!(
            "🔐 [限价单创建] token_id={}, side={:?}, price={}, size={}, maker_amt={}, taker_amt={}",
            order_args.token_id, order_args.side, order_args.price, order_args.size, maker_amount, taker_amount
        );

        let order_data = OrderData {
            maker: self.funder,
            taker: Address::ZERO,
            token_id: order_args.token_id.clone(),
            maker_amount,
            taker_amount,
            side: side_value,
            fee_rate_bps,
            nonce,
            signer: self.signer.address(),
            expiration,
            signature_type: self.sig_type as u8,
        };

        self.sign_order(&order_data, &contract_config, neg_risk).await
    }

    /// Sign an order
    /// 
    /// Uses the correct exchange contract address based on neg_risk:
    /// - neg_risk=false → CTF Exchange (0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E)
    /// - neg_risk=true → NegRiskCtfExchange (0xC5d563A36AE78145C45a50134d48A1215220f80a)
    async fn sign_order(
        &self,
        data: &OrderData,
        contract_config: &ContractConfig,
        neg_risk: bool,
    ) -> Result<SignedOrder> {
        // Select the correct exchange address based on neg_risk flag
        // This is CRITICAL: using the wrong address will cause "Invalid order payload" error
        let exchange = if neg_risk {
            &contract_config.neg_risk_exchange
        } else {
            &contract_config.exchange
        };
        
        // 🔍 详细日志：签名过程开始
        info!(
            "🔐 [Poly签名开始] token_id={}, neg_risk={}, 使用合约: {:?}",
            data.token_id, neg_risk, exchange
        );
        info!(
            "🔐 [Poly签名参数] maker={:?}, signer={:?}, taker={:?}, maker_amt={}, taker_amt={}, side={}, fee_rate_bps={}, nonce={}, expiration={}, sig_type={}",
            data.maker, data.signer, data.taker, data.maker_amount, data.taker_amount,
            data.side, data.fee_rate_bps, data.nonce, data.expiration, data.signature_type
        );

        // #region agent log - 签名详情 (假设 A, D, E)
        {
            use std::io::Write;
            let debug_log = serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "location": "order_builder.rs:sign_order",
                "hypothesisId": "A,D,E",
                "message": "签名过程详情",
                "data": {
                    "token_id": &data.token_id,
                    "neg_risk": neg_risk,
                    "exchange_address": format!("{:?}", exchange),
                    "chain_id": self.chain_id,
                    "maker": format!("{:?}", data.maker),
                    "signer": format!("{:?}", data.signer),
                    "maker_amount": data.maker_amount,
                    "taker_amount": data.taker_amount,
                    "side": data.side,
                    "nonce_salt": data.nonce
                }
            });
            let path = utils::get_debug_log_path();
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{}", debug_log.to_string());
            }
        }
        // #endregion

        // Calculate struct hash
        let struct_hash = data.struct_hash();
        info!("🔐 [Poly签名] struct_hash={:?}", struct_hash);

        // Calculate domain separator
        let domain_separator = calculate_domain_separator(exchange, self.chain_id);
        info!("🔐 [Poly签名] domain_separator={:?}, chain_id={}", domain_separator, self.chain_id);

        // Calculate EIP-712 hash
        let mut eip712_data = Vec::with_capacity(66);
        eip712_data.extend_from_slice(&[0x19, 0x01]);
        eip712_data.extend_from_slice(domain_separator.as_slice());
        eip712_data.extend_from_slice(struct_hash.as_slice());
        let hash = keccak256(&eip712_data);
        info!("🔐 [Poly签名] eip712_hash={:?}", hash);

        // Sign
        let signature = self.signer.sign_hash(&hash).await?;
        info!("🔐 [Poly签名] signature={}", signature);

        let signed_order = SignedOrder {
            salt: data.nonce.to_string(),
            maker: format!("{:?}", data.maker),
            signer: format!("{:?}", data.signer),
            taker: format!("{:?}", data.taker),
            token_id: data.token_id.clone(),
            maker_amount: data.maker_amount.to_string(),
            taker_amount: data.taker_amount.to_string(),
            expiration: data.expiration.to_string(),
            nonce: "0".to_string(), // Protocol nonce, different from salt
            fee_rate_bps: data.fee_rate_bps.to_string(),
            side: data.side.to_string(),
            signature_type: data.signature_type.to_string(),
            signature,
        };
        
        info!(
            "🔐 [Poly签名完成] token_id={}, side={}, maker_amt={}, taker_amt={}, neg_risk={}",
            signed_order.token_id,
            if signed_order.side == "0" { "BUY" } else { "SELL" },
            signed_order.maker_amount,
            signed_order.taker_amount,
            neg_risk
        );
        
        Ok(signed_order)
    }

    /// Calculate the cost (in USDC) to buy a specific number of tokens
    /// Returns (total_usdc_cost, worst_price_encountered, tokens_available)
    /// 
    /// Polymarket API returns asks in descending order (high to low)
    /// So we use .rev() to traverse from best_ask (last) to worst (first)
    pub fn calculate_buy_cost_for_tokens(
        &self,
        order_book: &OrderBookSummary,
        target_tokens: f64,
        slippage: f64,  // e.g., 0.05 for 5%
    ) -> Result<(f64, f64, f64)> {
        if order_book.asks.is_empty() {
            bail!("No asks in order book");
        }

        let mut total_usdc = 0.0;
        let mut total_tokens = 0.0;
        let mut worst_price = 0.0;
        
        // API returns asks: [high...low], so use .rev() to start from best_ask (last)
        for (price, size) in order_book.asks.iter().rev() {
            let tokens_at_this_level = (*size).min(target_tokens - total_tokens);
            total_tokens += tokens_at_this_level;
            total_usdc += tokens_at_this_level * price;
            worst_price = *price;
            
            info!(
                "📊 [订单簿遍历] price={:.4}, size={:.4}, 累计tokens={:.4}, 累计USDC={:.4}",
                price, size, total_tokens, total_usdc
            );
            
            if total_tokens >= target_tokens {
                break;
            }
        }
        
        if total_tokens == 0.0 {
            bail!("No liquidity available in order book");
        }
        
        // Add slippage to the total cost
        let total_usdc_with_slippage = total_usdc * (1.0 + slippage);
        
        info!(
            "📊 [买入计算] 目标tokens={:.4}, 可用tokens={:.4}, 需要USDC={:.4}, 含滑点USDC={:.4}, 最高价={:.4}",
            target_tokens, total_tokens, total_usdc, total_usdc_with_slippage, worst_price
        );
        
        Ok((total_usdc_with_slippage, worst_price, total_tokens))
    }

    /// Calculate buy market price from order book with 5% slippage
    /// API returns asks: [high...low], best_ask at last
    /// 
    /// Returns price rounded to 2 decimal places (cents), with 5% markup for aggressive fill
    pub fn calculate_buy_market_price(
        &self,
        order_book: &OrderBookSummary,
        amount_to_match: f64,
        _order_type: OrderType,
    ) -> Result<f64> {
        if order_book.asks.is_empty() {
            bail!("No asks in order book");
        }

        let mut sum = 0.0;
        let mut worst_price = 0.0;
        
        // API returns asks: [high...low], use .rev() to start from best_ask (last)
        for (price, size) in order_book.asks.iter().rev() {
            sum += size * price;
            worst_price = *price;
            if sum >= amount_to_match {
                break;
            }
        }

        // Add 5% slippage for aggressive fill
        let price_with_slippage = worst_price * 1.05;
        
        // Round to 2 decimal places (cents) and cap at 0.99
        let final_price = (price_with_slippage * 100.0).ceil() / 100.0;
        let final_price = final_price.min(0.99);
        
        Ok(final_price)
    }

    /// Calculate sell market price from order book with 5% slippage
    /// API returns bids: [low...high], best_bid at last
    /// 
    /// Returns price rounded to 2 decimal places (cents), with 5% discount for aggressive fill
    pub fn calculate_sell_market_price(
        &self,
        order_book: &OrderBookSummary,
        amount_to_match: f64,
        _order_type: OrderType,
    ) -> Result<f64> {
        if order_book.bids.is_empty() {
            bail!("No bids in order book");
        }

        let mut sum = 0.0;
        let mut worst_price = 1.0;
        
        // API returns bids: [low...high], use .rev() to start from best_bid (last)
        for (price, size) in order_book.bids.iter().rev() {
            sum += size;
            worst_price = *price;
            if sum >= amount_to_match {
                break;
            }
        }

        // Subtract 5% slippage for aggressive fill (sell at lower price)
        let price_with_slippage = worst_price * 0.95;
        
        // Round to 2 decimal places (cents) and ensure minimum 0.01
        let final_price = (price_with_slippage * 100.0).floor() / 100.0;
        let final_price = final_price.max(0.01);
        
        Ok(final_price)
    }
}

/// Internal order data structure for signing
struct OrderData {
    maker: Address,
    taker: Address,
    token_id: String,
    maker_amount: u64,
    taker_amount: u64,
    side: u8,
    fee_rate_bps: i64,
    nonce: u64,
    signer: Address,
    expiration: u64,
    signature_type: u8,
}

impl OrderData {
    /// Calculate struct hash for EIP-712
    fn struct_hash(&self) -> B256 {
        // Order type hash
        // keccak256("Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)")
        let type_hash = keccak256(
            b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)"
        );

        let token_id = U256::from_str_radix(&self.token_id, 10).unwrap_or_default();

        // Encode all fields
        let mut data = Vec::with_capacity(416);
        data.extend_from_slice(type_hash.as_slice());
        data.extend_from_slice(&U256::from(self.nonce).to_be_bytes::<32>()); // salt
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(self.maker.as_slice());
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(self.signer.as_slice());
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(self.taker.as_slice());
        data.extend_from_slice(&token_id.to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(self.maker_amount).to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(self.taker_amount).to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(self.expiration).to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(0u64).to_be_bytes::<32>()); // protocol nonce
        data.extend_from_slice(&U256::from(self.fee_rate_bps as u64).to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(self.side).to_be_bytes::<32>());
        data.extend_from_slice(&U256::from(self.signature_type).to_be_bytes::<32>());

        keccak256(&data)
    }
}

/// Calculate EIP-712 domain separator for CTF Exchange
fn calculate_domain_separator(exchange: &Address, chain_id: u64) -> B256 {
    // EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
    let type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    );
    let name_hash = keccak256(b"Polymarket CTF Exchange");
    let version_hash = keccak256(b"1");

    let mut data = Vec::with_capacity(160);
    data.extend_from_slice(type_hash.as_slice());
    data.extend_from_slice(name_hash.as_slice());
    data.extend_from_slice(version_hash.as_slice());
    data.extend_from_slice(&U256::from(chain_id).to_be_bytes::<32>());
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(exchange.as_slice());

    keccak256(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_functions() {
        assert!((round_down(1.234, 2) - 1.23).abs() < 0.001);
        assert!((round_up(1.234, 2) - 1.24).abs() < 0.001);
        assert!((round_normal(1.235, 2) - 1.24).abs() < 0.001);
        assert!((round_normal(1.234, 2) - 1.23).abs() < 0.001);
    }

    #[test]
    fn test_decimal_places() {
        assert_eq!(decimal_places(1.0), 0);
        assert_eq!(decimal_places(1.1), 1);
        assert_eq!(decimal_places(1.12), 2);
        assert_eq!(decimal_places(1.123), 3);
    }

    #[test]
    fn test_get_round_config() {
        let config = get_round_config(0.01);
        assert_eq!(config.price, 2);
        assert_eq!(config.size, 2);
        assert_eq!(config.amount, 4);
    }
}
