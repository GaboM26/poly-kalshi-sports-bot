//! Auto-trade repository
//!
//! Handles operations for auto_trade_state and auto_trade_history tables.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::ArbitrageStorage;

/// Auto-trade state stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTradeState {
    pub enabled: bool,
    pub trade_count: i32,
    /// 自动下单最大执行次数
    pub max_trade_count: i32,
    /// 自动下单单次最大金额（美元）
    pub max_amount: f64,
    /// 套利机会持续时间阈值（毫秒）
    pub min_duration_ms: i64,
    pub last_trade_time: Option<String>,
    pub updated_at: Option<String>,
}

impl Default for AutoTradeState {
    fn default() -> Self {
        Self {
            enabled: false,
            trade_count: 0,
            max_trade_count: 2,
            max_amount: 10.0,
            min_duration_ms: 500,
            last_trade_time: None,
            updated_at: None,
        }
    }
}

/// Auto-trade execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTradeRecord {
    pub id: i64,
    pub event_name: String,
    pub team_name: String,
    pub kalshi_market_id: String,
    pub polymarket_market_id: String,
    pub kalshi_side: String,
    pub polymarket_side: String,
    pub kalshi_contracts: i32,
    pub kalshi_price: f64,
    pub kalshi_fee: f64,
    pub polymarket_amount: f64,
    pub polymarket_price: f64,
    pub total_amount: f64,
    pub profit_margin: f64,
    /// 机会持续时间（下单前，毫秒）
    pub duration_ms: i64,
    /// 从发现机会到下单完成的总耗时（毫秒）
    pub total_duration_ms: i64,
    pub kalshi_success: bool,
    pub polymarket_success: bool,
    pub kalshi_order_id: Option<String>,
    pub polymarket_order_id: Option<String>,
    pub kalshi_error: Option<String>,
    pub polymarket_error: Option<String>,
    /// Status: "executed", "skipped", "partial"
    pub status: String,
    /// Reason for skipping (if status is "skipped")
    pub skip_reason: Option<String>,
    pub created_at: String,
}

impl ArbitrageStorage {
    /// Get current auto-trade state
    pub fn get_auto_trade_state(&self) -> Result<AutoTradeState> {
        let conn = self.conn().lock();
        
        let result = conn.query_row(
            "SELECT enabled, trade_count, max_trade_count, max_amount, min_duration_ms, last_trade_time, updated_at
             FROM auto_trade_state WHERE id = 1",
            [],
            |row| {
                Ok(AutoTradeState {
                    enabled: row.get::<_, i32>(0)? != 0,
                    trade_count: row.get(1)?,
                    max_trade_count: row.get(2)?,
                    max_amount: row.get(3)?,
                    min_duration_ms: row.get(4)?,
                    last_trade_time: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );

        match result {
            Ok(state) => Ok(state),
            Err(_) => Ok(AutoTradeState::default()),
        }
    }

    /// Set auto-trade enabled state
    pub fn set_auto_trade_enabled(&self, enabled: bool) -> Result<()> {
        let conn = self.conn().lock();
        conn.execute(
            "UPDATE auto_trade_state SET enabled = ?, updated_at = ? WHERE id = 1",
            params![enabled as i32, Utc::now().to_rfc3339()],
        )?;
        info!("🔄 自动下单状态已更新: enabled = {}", enabled);
        Ok(())
    }

    /// Increment trade count after successful execution
    pub fn increment_trade_count(&self) -> Result<i32> {
        let conn = self.conn().lock();
        conn.execute(
            "UPDATE auto_trade_state SET trade_count = trade_count + 1, last_trade_time = ?, updated_at = ? WHERE id = 1",
            params![Utc::now().to_rfc3339(), Utc::now().to_rfc3339()],
        )?;
        
        let new_count: i32 = conn.query_row(
            "SELECT trade_count FROM auto_trade_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        
        info!("📈 自动下单次数已递增: {}", new_count);
        Ok(new_count)
    }

    /// Reset trade count (for testing)
    pub fn reset_trade_count(&self) -> Result<()> {
        let conn = self.conn().lock();
        conn.execute(
            "UPDATE auto_trade_state SET trade_count = 0, updated_at = ? WHERE id = 1",
            params![Utc::now().to_rfc3339()],
        )?;
        info!("🔄 自动下单次数已重置为 0");
        Ok(())
    }

    /// Update auto-trade settings
    pub fn update_auto_trade_settings(
        &self,
        max_amount: Option<f64>,
        min_duration_ms: Option<i64>,
        max_trade_count: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn().lock();
        
        if let Some(amount) = max_amount {
            conn.execute(
                "UPDATE auto_trade_state SET max_amount = ?, updated_at = ? WHERE id = 1",
                params![amount, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 max_amount 已更新: {}", amount);
        }
        
        if let Some(duration) = min_duration_ms {
            conn.execute(
                "UPDATE auto_trade_state SET min_duration_ms = ?, updated_at = ? WHERE id = 1",
                params![duration, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 min_duration_ms 已更新: {}", duration);
        }
        
        if let Some(max_count) = max_trade_count {
            conn.execute(
                "UPDATE auto_trade_state SET max_trade_count = ?, updated_at = ? WHERE id = 1",
                params![max_count, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 max_trade_count 已更新: {}", max_count);
        }
        
        Ok(())
    }

    /// Check if auto-trade can execute (enabled and within limits)
    pub fn can_auto_trade(&self) -> Result<(bool, String)> {
        let state = self.get_auto_trade_state()?;
        
        if !state.enabled {
            return Ok((false, "自动下单未开启".to_string()));
        }
        
        if state.trade_count >= state.max_trade_count {
            return Ok((false, format!(
                "已达到最大下单次数限制 ({}/{})", 
                state.trade_count, state.max_trade_count
            )));
        }
        
        Ok((true, format!(
            "可以下单 ({}/{})",
            state.trade_count, state.max_trade_count
        )))
    }

    /// Save an auto-trade execution record
    #[allow(clippy::too_many_arguments)]
    pub fn save_auto_trade_record(
        &self,
        event_name: &str,
        team_name: &str,
        kalshi_market_id: &str,
        polymarket_market_id: &str,
        kalshi_side: &str,
        polymarket_side: &str,
        kalshi_contracts: i32,
        kalshi_price: f64,
        kalshi_fee: f64,
        polymarket_amount: f64,
        polymarket_price: f64,
        total_amount: f64,
        profit_margin: f64,
        duration_ms: i64,
        total_duration_ms: i64,
        kalshi_success: bool,
        polymarket_success: bool,
        kalshi_order_id: Option<&str>,
        polymarket_order_id: Option<&str>,
        kalshi_error: Option<&str>,
        polymarket_error: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn().lock();
        
        // Determine status based on success flags
        let status = if kalshi_success && polymarket_success {
            "executed"
        } else {
            "partial"
        };
        
        conn.execute(
            "INSERT INTO auto_trade_history (
                event_name, team_name, kalshi_market_id, polymarket_market_id,
                kalshi_side, polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee,
                polymarket_amount, polymarket_price, total_amount, profit_margin,
                duration_ms, total_duration_ms, kalshi_success, polymarket_success,
                kalshi_order_id, polymarket_order_id, kalshi_error, polymarket_error,
                status, skip_reason, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                event_name,
                team_name,
                kalshi_market_id,
                polymarket_market_id,
                kalshi_side,
                polymarket_side,
                kalshi_contracts,
                kalshi_price,
                kalshi_fee,
                polymarket_amount,
                polymarket_price,
                total_amount,
                profit_margin,
                duration_ms,
                total_duration_ms,
                kalshi_success as i32,
                polymarket_success as i32,
                kalshi_order_id,
                polymarket_order_id,
                kalshi_error,
                polymarket_error,
                status,
                Option::<String>::None, // skip_reason is None for executed orders
                Utc::now().to_rfc3339(),
            ],
        )?;
        
        let id = conn.last_insert_rowid();
        info!("📝 自动下单记录已保存: ID={}, 事件={}, 状态={}, K:{}/P:{}", 
            id, event_name, status,
            if kalshi_success { "成功" } else { "失败" },
            if polymarket_success { "成功" } else { "失败" }
        );
        Ok(id)
    }

    /// Save a skipped auto-trade record (for debugging/analysis)
    #[allow(clippy::too_many_arguments)]
    pub fn save_skipped_auto_trade_record(
        &self,
        event_name: &str,
        team_name: &str,
        kalshi_market_id: &str,
        polymarket_market_id: &str,
        kalshi_side: &str,
        polymarket_side: &str,
        kalshi_contracts: i32,
        kalshi_price: f64,
        polymarket_price: f64,
        profit_margin: f64,
        duration_ms: i64,
        skip_reason: &str,
    ) -> Result<i64> {
        let conn = self.conn().lock();
        
        // Calculate estimated amounts for reference
        let kalshi_fee = (0.07 * kalshi_contracts as f64 * kalshi_price * (1.0 - kalshi_price) * 100.0).ceil() / 100.0;
        let polymarket_amount = kalshi_contracts as f64 * polymarket_price;
        let total_amount = kalshi_contracts as f64 * kalshi_price + kalshi_fee + polymarket_amount;
        
        conn.execute(
            "INSERT INTO auto_trade_history (
                event_name, team_name, kalshi_market_id, polymarket_market_id,
                kalshi_side, polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee,
                polymarket_amount, polymarket_price, total_amount, profit_margin,
                duration_ms, kalshi_success, polymarket_success,
                kalshi_order_id, polymarket_order_id, kalshi_error, polymarket_error,
                status, skip_reason, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                event_name,
                team_name,
                kalshi_market_id,
                polymarket_market_id,
                kalshi_side,
                polymarket_side,
                kalshi_contracts,
                kalshi_price,
                kalshi_fee,
                polymarket_amount,
                polymarket_price,
                total_amount,
                profit_margin,
                duration_ms,
                0, // kalshi_success = false (not executed)
                0, // polymarket_success = false (not executed)
                Option::<String>::None, // no order_id
                Option::<String>::None,
                Option::<String>::None, // no error (just skipped)
                Option::<String>::None,
                "skipped",
                Some(skip_reason),
                Utc::now().to_rfc3339(),
            ],
        )?;
        
        let id = conn.last_insert_rowid();
        info!("📝 自动下单跳过记录: ID={}, 事件={}, 原因={}", id, event_name, skip_reason);
        Ok(id)
    }

    /// Get auto-trade execution history
    pub fn get_auto_trade_history(&self, limit: usize) -> Result<Vec<AutoTradeRecord>> {
        let conn = self.conn().lock();
        let mut stmt = conn.prepare(
            "SELECT id, event_name, team_name, kalshi_market_id, polymarket_market_id,
                    kalshi_side, polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee,
                    polymarket_amount, polymarket_price, total_amount, profit_margin,
                    duration_ms, total_duration_ms, kalshi_success, polymarket_success,
                    kalshi_order_id, polymarket_order_id, kalshi_error, polymarket_error,
                    status, skip_reason, created_at
             FROM auto_trade_history
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;

        let records = stmt
            .query_map([limit], |row| {
                Ok(AutoTradeRecord {
                    id: row.get(0)?,
                    event_name: row.get(1)?,
                    team_name: row.get(2)?,
                    kalshi_market_id: row.get(3)?,
                    polymarket_market_id: row.get(4)?,
                    kalshi_side: row.get(5)?,
                    polymarket_side: row.get(6)?,
                    kalshi_contracts: row.get(7)?,
                    kalshi_price: row.get(8)?,
                    kalshi_fee: row.get(9)?,
                    polymarket_amount: row.get(10)?,
                    polymarket_price: row.get(11)?,
                    total_amount: row.get(12)?,
                    profit_margin: row.get(13)?,
                    duration_ms: row.get(14)?,
                    total_duration_ms: row.get::<_, i64>(15).unwrap_or(0),
                    kalshi_success: row.get::<_, i32>(16)? != 0,
                    polymarket_success: row.get::<_, i32>(17)? != 0,
                    kalshi_order_id: row.get(18)?,
                    polymarket_order_id: row.get(19)?,
                    kalshi_error: row.get(20)?,
                    polymarket_error: row.get(21)?,
                    status: row.get::<_, Option<String>>(22)?.unwrap_or_else(|| "executed".to_string()),
                    skip_reason: row.get(23)?,
                    created_at: row.get(24)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(records)
    }
}
