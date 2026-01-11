//! Auto-trade state management
//!
//! Handles auto-trade eligibility checks, market exclusion, and depth validation.

use chrono::NaiveDate;
use tracing::info;

use crate::models::generate_market_key;
use crate::services::storage::AutoTradeState;
use super::WebSocketManager;

impl WebSocketManager {
    /// Get current auto-trade state
    pub fn get_auto_trade_state(&self) -> AutoTradeState {
        self.storage.get_auto_trade_state().unwrap_or_default()
    }

    /// Enable auto-trade
    pub fn enable_auto_trade(&self) -> anyhow::Result<()> {
        self.storage.set_auto_trade_enabled(true)?;
        info!("🤖 自动下单已开启");
        Ok(())
    }

    /// Disable auto-trade
    pub fn disable_auto_trade(&self) -> anyhow::Result<()> {
        self.storage.set_auto_trade_enabled(false)?;
        info!("🛑 自动下单已关闭");
        Ok(())
    }

    /// Reset trade count
    pub fn reset_trade_count(&self) -> anyhow::Result<()> {
        self.storage.reset_trade_count()?;
        self.auto_traded_opportunities.write().clear();
        info!("🔄 下单次数已重置");
        Ok(())
    }

    /// Update auto-trade settings
    #[allow(clippy::too_many_arguments)]
    pub fn update_auto_trade_settings(
        &self,
        max_amount: Option<f64>,
        min_duration_ms: Option<i64>,
        max_trade_count: Option<i32>,
        flexible_mode: Option<bool>,
        max_contracts: Option<i32>,
        min_contracts: Option<i32>,
    ) -> anyhow::Result<()> {
        self.storage.update_auto_trade_settings(
            max_amount, 
            min_duration_ms, 
            max_trade_count,
            flexible_mode,
            max_contracts,
            min_contracts,
        )?;
        Ok(())
    }

    /// Update application settings (hot-updatable)
    pub fn update_app_settings(
        &self,
        refresh_interval: Option<u64>,
        min_profit_margin: Option<f64>,
        default_bet_amount: Option<f64>,
        tracking_threshold: Option<f64>,
    ) -> anyhow::Result<()> {
        self.storage.update_app_settings(
            refresh_interval,
            min_profit_margin,
            default_bet_amount,
            tracking_threshold,
        )?;
        
        info!("📝 应用设置已更新到数据库");
        Ok(())
    }

    /// Check if an opportunity is eligible for auto-trade
    pub fn check_auto_trade_eligibility(&self, key: &str, duration_ms: i64) -> (bool, String) {
        let state = self.get_auto_trade_state();

        if !state.enabled {
            return (false, "自动下单未开启".to_string());
        }

        let normalized_key = key.to_uppercase();
        if self.excluded_markets.read().contains(&normalized_key) {
            return (false, "该市场已被排除".to_string());
        }

        if state.trade_count >= state.max_trade_count {
            return (false, format!("已达到最大下单次数 ({}/{})", state.trade_count, state.max_trade_count));
        }

        if duration_ms < state.min_duration_ms {
            return (false, format!("持续时间不足 ({}ms < {}ms)", duration_ms, state.min_duration_ms));
        }

        if self.auto_traded_opportunities.read().contains(key) {
            return (false, "该机会已自动下单".to_string());
        }

        (true, format!("可以下单 ({}/{})", state.trade_count + 1, state.max_trade_count))
    }
    
    /// Load excluded markets from database on startup
    pub fn load_excluded_markets(&self) {
        match self.storage.get_excluded_markets() {
            Ok(markets) => {
                let count = markets.len();
                *self.excluded_markets.write() = markets;
                if count > 0 {
                    info!("📋 从数据库加载了 {} 个排除的市场", count);
                }
            }
            Err(e) => {
                tracing::error!("加载排除市场列表失败: {}", e);
            }
        }
    }
    
    /// Exclude a market from auto-trade
    pub fn exclude_market(&self, event_name: &str, team_name: &str, game_date: Option<NaiveDate>) -> bool {
        let key = generate_market_key(event_name, game_date, team_name);
        
        match self.storage.exclude_market(event_name, team_name, game_date) {
            Ok(inserted) => {
                if inserted {
                    self.excluded_markets.write().insert(key);
                }
                inserted
            }
            Err(e) => {
                tracing::error!("保存排除市场到数据库失败: {}", e);
                false
            }
        }
    }
    
    /// Remove a market from exclusion list
    pub fn unexclude_market(&self, event_name: &str, team_name: &str, game_date: Option<NaiveDate>) -> bool {
        let key = generate_market_key(event_name, game_date, team_name);
        
        match self.storage.unexclude_market(event_name, team_name, game_date) {
            Ok(removed) => {
                if removed {
                    self.excluded_markets.write().remove(&key);
                }
                removed
            }
            Err(e) => {
                tracing::error!("从数据库移除排除市场失败: {}", e);
                false
            }
        }
    }
    
    /// Get list of excluded markets
    pub fn get_excluded_markets(&self) -> Vec<String> {
        self.excluded_markets.read().iter().cloned().collect()
    }
    
    /// Check if a market is excluded
    pub fn is_market_excluded(&self, event_name: &str, team_name: &str, game_date: Option<NaiveDate>) -> bool {
        let key = generate_market_key(event_name, game_date, team_name);
        self.excluded_markets.read().contains(&key)
    }

    /// Mark an opportunity as auto-traded
    pub fn mark_as_auto_traded(&self, key: &str) {
        self.auto_traded_opportunities.write().insert(key.to_string());
        self.clear_skip_records_for_market(key);
    }

    /// Check if a skip reason should be recorded (deduplication)
    pub fn should_record_skip(&self, market_key: &str, skip_reason: &str) -> bool {
        let simplified_reason = if skip_reason.contains("Polymarket 深度不足") {
            "poly_depth"
        } else if skip_reason.contains("Kalshi 深度不足") {
            "kalshi_depth"
        } else if skip_reason.contains("超过限额") {
            "over_limit"
        } else if skip_reason.contains("无法获取") {
            "token_not_found"
        } else if skip_reason.contains("价格和超过") {
            "price_sum_invalid"
        } else {
            "other"
        };
        
        let record_key = format!("{}:{}", market_key, simplified_reason);
        let mut recorded = self.recorded_skip_reasons.write();
        
        if recorded.contains(&record_key) {
            false
        } else {
            recorded.insert(record_key);
            true
        }
    }

    /// Clear skip records for a specific market
    pub fn clear_skip_records_for_market(&self, market_key: &str) {
        let mut recorded = self.recorded_skip_reasons.write();
        let prefix = format!("{}:", market_key);
        recorded.retain(|k| !k.starts_with(&prefix));
    }

    /// Increment trade count after successful auto-trade
    pub fn increment_trade_count(&self) -> anyhow::Result<i32> {
        self.storage.increment_trade_count()
    }

    /// Validate orderbook depth and price before auto-trade execution
    pub fn validate_auto_trade_depth(
        &self,
        kalshi_ticker: &str,
        kalshi_side: &str,
        poly_token: &str,
        required_contracts: i32,
    ) -> (bool, i32, f64, f64, f64, String) {
        let kalshi_book = match &self.kalshi_client {
            Some(client) => client.get_orderbook(kalshi_ticker),
            None => {
                return (false, 0, 0.0, 0.0, 0.0, "Kalshi client 未初始化".to_string());
            }
        };

        let poly_book = match &self.polymarket_client {
            Some(client) => client.get_orderbook(poly_token),
            None => {
                return (false, 0, 0.0, 0.0, 0.0, "Polymarket client 未初始化".to_string());
            }
        };

        let kalshi_book = match kalshi_book {
            Some(book) => book,
            None => {
                return (false, 0, 0.0, 0.0, 0.0, format!("Kalshi 订单簿不存在: {}", kalshi_ticker));
            }
        };

        let poly_book = match poly_book {
            Some(book) => book,
            None => {
                return (false, 0, 0.0, 0.0, 0.0, format!("Polymarket 订单簿不存在: {}", poly_token));
            }
        };

        let kalshi_depth = kalshi_book.ask_depth_for_side(kalshi_side, required_contracts);

        let kalshi_price = {
            let prices = self.kalshi_prices.read();
            match prices.get(kalshi_ticker) {
                Some((_, yes_ask, _, no_ask)) => {
                    if kalshi_side == "yes" { *yes_ask } else { *no_ask }
                }
                None => {
                    return (false, kalshi_depth, 0.0, 0.0, 0.0, 
                        format!("Kalshi 价格缓存不存在: {}", kalshi_ticker));
                }
            }
        };

        let (poly_price, poly_size) = match poly_book.best_ask() {
            Some((price, size)) => (price, size),
            None => {
                return (false, kalshi_depth, 0.0, kalshi_price, 0.0, 
                    "Polymarket 无可用 ask".to_string());
            }
        };

        let required_poly_amount = required_contracts as f64 * poly_price;
        let poly_depth = poly_price * poly_size;

        if kalshi_depth < required_contracts {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("Kalshi 深度不足: 需要 {} 合约, 可用 {} 合约", 
                    required_contracts, kalshi_depth));
        }

        if poly_depth < required_poly_amount {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("Polymarket 深度不足: 需要 ${:.2}, 可用 ${:.2}", 
                    required_poly_amount, poly_depth));
        }

        let price_sum = kalshi_price + poly_price;
        if price_sum >= 1.0 {
            return (false, kalshi_depth, poly_depth, kalshi_price, poly_price,
                format!("套利条件已消失: K={:.4} + P={:.4} = {:.4} >= 1", 
                    kalshi_price, poly_price, price_sum));
        }

        (true, kalshi_depth, poly_depth, kalshi_price, poly_price, 
            format!("验证通过: K深度={}, P深度=${:.2}, 价格和={:.4}", 
                kalshi_depth, poly_depth, price_sum))
    }
}
