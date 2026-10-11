//! Auto-trade repository
//!
//! Handles operations for auto_trade_state and auto_trade_history tables.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::ArbitrageStorage;
use crate::core::fees::{kalshi_taker_fee, polymarket_taker_fee};

/// Auto-trade state stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTradeState {
    pub enabled: bool,
    pub trade_count: i32,
    /// Maximum number of auto-trade executions
    pub max_trade_count: i32,
    /// Maximum trade amount per execution (USD)
    pub max_amount: f64,
    /// Minimum arbitrage opportunity duration threshold (milliseconds)
    pub min_duration_ms: i64,
    /// Whether flexible auto-trade mode is enabled
    pub flexible_mode: bool,
    /// Maximum contracts per trade
    pub max_contracts: i32,
    /// Minimum required contracts; do not trade below this depth
    pub min_contracts: i32,
    /// Maximum tolerated adverse close price per contract after an unpaired fill.
    pub neutralization_max_loss_cents: i32,
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
            flexible_mode: false,
            max_contracts: 100,
            min_contracts: 10,
            neutralization_max_loss_cents: 5,
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
    /// Modeled Kalshi taker fee for the planned size.
    pub kalshi_fee: f64,
    /// Modeled Polymarket taker fee for the planned size; `None` for rows
    /// written before it was modeled.
    pub polymarket_fee: Option<f64>,
    pub polymarket_amount: f64,
    pub polymarket_price: f64,
    pub total_amount: f64,
    pub profit_margin: f64,
    /// Opportunity duration before order placement (milliseconds)
    pub duration_ms: i64,
    /// Total time from opportunity detection to order completion (milliseconds)
    pub total_duration_ms: i64,
    pub kalshi_success: bool,
    pub polymarket_success: bool,
    pub kalshi_order_id: Option<String>,
    pub polymarket_order_id: Option<String>,
    pub kalshi_error: Option<String>,
    pub polymarket_error: Option<String>,
    /// Kalshi API latency in milliseconds
    pub kalshi_latency_ms: Option<i64>,
    /// Polymarket API latency in milliseconds
    pub poly_latency_ms: Option<i64>,
    /// Status: "executed", "skipped", "partial"
    pub status: String,
    /// Reason for skipping (if status is "skipped")
    pub skip_reason: Option<String>,
    pub kalshi_filled_contracts: i32,
    pub polymarket_filled_contracts: i32,
    pub kalshi_order_status: Option<String>,
    pub polymarket_order_status: Option<String>,
    pub neutralization_leg: Option<String>,
    pub neutralization_success: Option<bool>,
    pub neutralization_order_id: Option<String>,
    pub neutralization_filled_contracts: i32,
    pub neutralization_error: Option<String>,
    pub residual_leg: Option<String>,
    pub residual_contracts: i32,
    pub created_at: String,
}

/// Durable outcome of a paired automatic execution, including any recovery.
#[derive(Debug, Clone)]
pub struct AutoTradeExecutionRecord {
    pub event_name: String,
    pub team_name: String,
    pub kalshi_market_id: String,
    pub polymarket_market_id: String,
    pub kalshi_side: String,
    pub polymarket_side: String,
    pub contracts: i32,
    pub kalshi_price: f64,
    pub kalshi_fee: f64,
    /// Modeled Polymarket taker fee for the planned size.
    pub polymarket_fee: f64,
    pub polymarket_price: f64,
    pub profit_margin: f64,
    pub duration_ms: i64,
    pub total_duration_ms: i64,
    pub kalshi_success: bool,
    pub polymarket_success: bool,
    pub kalshi_order_id: Option<String>,
    pub polymarket_order_id: Option<String>,
    pub kalshi_error: Option<String>,
    pub polymarket_error: Option<String>,
    pub kalshi_latency_ms: Option<i64>,
    pub poly_latency_ms: Option<i64>,
    pub kalshi_filled_contracts: i32,
    pub polymarket_filled_contracts: i32,
    pub kalshi_order_status: Option<String>,
    pub polymarket_order_status: Option<String>,
    pub neutralization_leg: Option<String>,
    pub neutralization_success: Option<bool>,
    pub neutralization_order_id: Option<String>,
    pub neutralization_filled_contracts: i32,
    pub neutralization_error: Option<String>,
    pub residual_leg: Option<String>,
    pub residual_contracts: i32,
    pub status: String,
}

impl ArbitrageStorage {
    /// Get current auto-trade state
    pub fn get_auto_trade_state(&self) -> Result<AutoTradeState> {
        let conn = self.conn().lock();

        let result = conn.query_row(
            "SELECT enabled, trade_count, max_trade_count, max_amount, min_duration_ms, 
                    flexible_mode, max_contracts, min_contracts, neutralization_max_loss_cents, last_trade_time, updated_at
             FROM auto_trade_state WHERE id = 1",
            [],
            |row| {
                Ok(AutoTradeState {
                    enabled: row.get::<_, i32>(0)? != 0,
                    trade_count: row.get(1)?,
                    max_trade_count: row.get(2)?,
                    max_amount: row.get(3)?,
                    min_duration_ms: row.get(4)?,
                    flexible_mode: row.get::<_, i32>(5).unwrap_or(0) != 0,
                    max_contracts: row.get(6).unwrap_or(100),
                    min_contracts: row.get(7).unwrap_or(10),
                    neutralization_max_loss_cents: row.get(8).unwrap_or(5),
                    last_trade_time: row.get(9)?,
                    updated_at: row.get(10)?,
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
        info!("🔄 Auto-trade status updated: enabled = {}", enabled);
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

        info!("📈 Auto-trade count incremented: {}", new_count);
        Ok(new_count)
    }

    /// Reset trade count (for testing)
    pub fn reset_trade_count(&self) -> Result<()> {
        let conn = self.conn().lock();
        conn.execute(
            "UPDATE auto_trade_state SET trade_count = 0, updated_at = ? WHERE id = 1",
            params![Utc::now().to_rfc3339()],
        )?;
        info!("🔄 Auto-trade count reset to 0");
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
        neutralization_max_loss_cents: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn().lock();

        if let Some(amount) = max_amount {
            conn.execute(
                "UPDATE auto_trade_state SET max_amount = ?, updated_at = ? WHERE id = 1",
                params![amount, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 max_amount updated: {}", amount);
        }

        if let Some(duration) = min_duration_ms {
            conn.execute(
                "UPDATE auto_trade_state SET min_duration_ms = ?, updated_at = ? WHERE id = 1",
                params![duration, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 min_duration_ms updated: {}", duration);
        }

        if let Some(max_count) = max_trade_count {
            conn.execute(
                "UPDATE auto_trade_state SET max_trade_count = ?, updated_at = ? WHERE id = 1",
                params![max_count, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 max_trade_count updated: {}", max_count);
        }

        if let Some(flexible) = flexible_mode {
            conn.execute(
                "UPDATE auto_trade_state SET flexible_mode = ?, updated_at = ? WHERE id = 1",
                params![flexible as i32, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 flexible_mode updated: {}", flexible);
        }

        if let Some(max_c) = max_contracts {
            conn.execute(
                "UPDATE auto_trade_state SET max_contracts = ?, updated_at = ? WHERE id = 1",
                params![max_c, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 max_contracts updated: {}", max_c);
        }

        if let Some(min_c) = min_contracts {
            conn.execute(
                "UPDATE auto_trade_state SET min_contracts = ?, updated_at = ? WHERE id = 1",
                params![min_c, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 min_contracts updated: {}", min_c);
        }
        if let Some(cents) = neutralization_max_loss_cents {
            conn.execute(
                "UPDATE auto_trade_state SET neutralization_max_loss_cents = ?, updated_at = ? WHERE id = 1",
                params![cents, Utc::now().to_rfc3339()],
            )?;
            info!("🔄 neutralization_max_loss_cents updated: {}", cents);
        }

        Ok(())
    }

    /// Persist every outcome that follows an automatic paired submission.
    pub fn save_auto_trade_execution(&self, record: &AutoTradeExecutionRecord) -> Result<i64> {
        let conn = self.conn().lock();
        let polymarket_amount = record.contracts as f64 * record.polymarket_price;
        let total_amount = record.contracts as f64 * record.kalshi_price
            + record.kalshi_fee
            + polymarket_amount
            + record.polymarket_fee;
        conn.execute(
            "INSERT INTO auto_trade_history (
                event_name, team_name, kalshi_market_id, polymarket_market_id, kalshi_side,
                polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee, polymarket_amount,
                polymarket_price, total_amount, profit_margin, duration_ms, total_duration_ms,
                kalshi_success, polymarket_success, kalshi_order_id, polymarket_order_id,
                kalshi_error, polymarket_error, kalshi_latency_ms, poly_latency_ms, status,
                kalshi_filled_contracts, polymarket_filled_contracts, kalshi_order_status,
                polymarket_order_status, neutralization_leg, neutralization_success,
                neutralization_order_id, neutralization_filled_contracts, neutralization_error,
                residual_leg, residual_contracts, created_at, polymarket_fee
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                      ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29,
                      ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37)",
            params![
                record.event_name,
                record.team_name,
                record.kalshi_market_id,
                record.polymarket_market_id,
                record.kalshi_side,
                record.polymarket_side,
                record.contracts,
                record.kalshi_price,
                record.kalshi_fee,
                polymarket_amount,
                record.polymarket_price,
                total_amount,
                record.profit_margin,
                record.duration_ms,
                record.total_duration_ms,
                record.kalshi_success as i32,
                record.polymarket_success as i32,
                record.kalshi_order_id,
                record.polymarket_order_id,
                record.kalshi_error,
                record.polymarket_error,
                record.kalshi_latency_ms,
                record.poly_latency_ms,
                record.status,
                record.kalshi_filled_contracts,
                record.polymarket_filled_contracts,
                record.kalshi_order_status,
                record.polymarket_order_status,
                record.neutralization_leg,
                record.neutralization_success.map(i32::from),
                record.neutralization_order_id,
                record.neutralization_filled_contracts,
                record.neutralization_error,
                record.residual_leg,
                record.residual_contracts,
                Utc::now().to_rfc3339(),
                record.polymarket_fee,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Finalize a durable `submitting` record after both legs and any bounded
    /// recovery attempt have reported their outcomes.
    pub fn finish_auto_trade_execution(
        &self,
        id: i64,
        record: &AutoTradeExecutionRecord,
    ) -> Result<()> {
        let conn = self.conn().lock();
        let polymarket_amount = record.contracts as f64 * record.polymarket_price;
        let total_amount = record.contracts as f64 * record.kalshi_price
            + record.kalshi_fee
            + polymarket_amount
            + record.polymarket_fee;
        conn.execute(
            "UPDATE auto_trade_history SET kalshi_contracts=?2, kalshi_price=?3, kalshi_fee=?4,
                polymarket_amount=?5, polymarket_price=?6, total_amount=?7, profit_margin=?8,
                duration_ms=?9, total_duration_ms=?10, kalshi_success=?11, polymarket_success=?12,
                kalshi_order_id=?13, polymarket_order_id=?14, kalshi_error=?15, polymarket_error=?16,
                kalshi_latency_ms=?17, poly_latency_ms=?18, status=?19, kalshi_filled_contracts=?20,
                polymarket_filled_contracts=?21, kalshi_order_status=?22, polymarket_order_status=?23,
                neutralization_leg=?24, neutralization_success=?25, neutralization_order_id=?26,
                neutralization_filled_contracts=?27, neutralization_error=?28, residual_leg=?29,
                residual_contracts=?30, polymarket_fee=?31 WHERE id=?1",
            params![
                id, record.contracts, record.kalshi_price, record.kalshi_fee, polymarket_amount,
                record.polymarket_price, total_amount, record.profit_margin, record.duration_ms,
                record.total_duration_ms, record.kalshi_success as i32, record.polymarket_success as i32,
                record.kalshi_order_id, record.polymarket_order_id, record.kalshi_error,
                record.polymarket_error, record.kalshi_latency_ms, record.poly_latency_ms,
                record.status, record.kalshi_filled_contracts, record.polymarket_filled_contracts,
                record.kalshi_order_status, record.polymarket_order_status, record.neutralization_leg,
                record.neutralization_success.map(i32::from), record.neutralization_order_id,
                record.neutralization_filled_contracts, record.neutralization_error, record.residual_leg,
                record.residual_contracts, record.polymarket_fee,
            ],
        )?;
        Ok(())
    }

    /// Persist an acknowledged leg before submitting or recovering the other leg.
    #[allow(clippy::too_many_arguments)]
    pub fn update_auto_trade_leg(
        &self,
        id: i64,
        leg: &str,
        success: bool,
        filled_contracts: i32,
        order_id: Option<&str>,
        status: Option<&str>,
        error: Option<&str>,
        latency_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn().lock();
        match leg {
            "kalshi" => {
                conn.execute(
                    "UPDATE auto_trade_history SET kalshi_success=?2, kalshi_filled_contracts=?3,
                     kalshi_order_id=?4, kalshi_order_status=?5, kalshi_error=?6,
                     kalshi_latency_ms=?7 WHERE id=?1",
                    params![
                        id,
                        success as i32,
                        filled_contracts,
                        order_id,
                        status,
                        error,
                        latency_ms
                    ],
                )?;
            }
            "polymarket" => {
                conn.execute(
                    "UPDATE auto_trade_history SET polymarket_success=?2, polymarket_filled_contracts=?3,
                     polymarket_order_id=?4, polymarket_order_status=?5, polymarket_error=?6,
                     poly_latency_ms=?7 WHERE id=?1",
                    params![id, success as i32, filled_contracts, order_id, status, error, latency_ms],
                )?;
            }
            _ => anyhow::bail!("Unknown auto-trade execution leg: {leg}"),
        }
        Ok(())
    }

    /// A restarted executor must not submit new pairs while a prior process
    /// may have been interrupted between durable intent and final outcome.
    pub fn has_submitting_auto_trade_execution(&self) -> Result<bool> {
        let conn = self.conn().lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM auto_trade_history WHERE status = 'submitting'",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
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
        kalshi_latency_ms: i64,
        poly_latency_ms: i64,
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
                kalshi_latency_ms, poly_latency_ms, status, skip_reason,
                kalshi_filled_contracts, polymarket_filled_contracts, kalshi_order_status,
                polymarket_order_status, neutralization_leg, neutralization_success,
                neutralization_order_id, neutralization_filled_contracts, neutralization_error,
                residual_leg, residual_contracts, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
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
                kalshi_latency_ms,
                poly_latency_ms,
                status,
                Option::<String>::None, // skip_reason is None for executed orders
                Utc::now().to_rfc3339(),
            ],
        )?;

        let id = conn.last_insert_rowid();
        info!("📝 Auto-trade record saved: ID={}, event={}, status={}, K:{}/P:{}, latency: K={}ms/P={}ms", 
            id, event_name, status,
            if kalshi_success { "success" } else { "failed" },
            if polymarket_success { "success" } else { "failed" },
            kalshi_latency_ms, poly_latency_ms
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
        let kalshi_fee = kalshi_taker_fee(kalshi_contracts as f64, kalshi_price);
        let polymarket_fee = polymarket_taker_fee(kalshi_contracts as f64, polymarket_price);
        let polymarket_amount = kalshi_contracts as f64 * polymarket_price;
        let total_amount =
            kalshi_contracts as f64 * kalshi_price + kalshi_fee + polymarket_amount + polymarket_fee;

        conn.execute(
            "INSERT INTO auto_trade_history (
                event_name, team_name, kalshi_market_id, polymarket_market_id,
                kalshi_side, polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee,
                polymarket_amount, polymarket_price, total_amount, profit_margin,
                duration_ms, kalshi_success, polymarket_success,
                kalshi_order_id, polymarket_order_id, kalshi_error, polymarket_error,
                status, skip_reason, created_at, polymarket_fee
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
                0, // kalshi_success = false (not executed)
                0, // polymarket_success = false (not executed)
                Option::<String>::None, // no order_id
                Option::<String>::None,
                Option::<String>::None, // no error (just skipped)
                Option::<String>::None,
                "skipped",
                Some(skip_reason),
                Utc::now().to_rfc3339(),
                polymarket_fee,
            ],
        )?;

        let id = conn.last_insert_rowid();
        info!(
            "📝 Auto-trade skip record: ID={}, event={}, reason={}",
            id, event_name, skip_reason
        );
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
                    kalshi_latency_ms, poly_latency_ms, status, skip_reason,
                    kalshi_filled_contracts, polymarket_filled_contracts,
                    kalshi_order_status, polymarket_order_status,
                    neutralization_leg, neutralization_success, neutralization_order_id,
                    neutralization_filled_contracts, neutralization_error,
                    residual_leg, residual_contracts, created_at, polymarket_fee
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
                    polymarket_fee: row.get::<_, Option<f64>>(38)?,
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
                    kalshi_latency_ms: row.get::<_, Option<i64>>(22).ok().flatten(),
                    poly_latency_ms: row.get::<_, Option<i64>>(23).ok().flatten(),
                    status: row
                        .get::<_, Option<String>>(24)?
                        .unwrap_or_else(|| "executed".to_string()),
                    skip_reason: row.get(25)?,
                    kalshi_filled_contracts: row.get::<_, Option<i32>>(26)?.unwrap_or(0),
                    polymarket_filled_contracts: row.get::<_, Option<i32>>(27)?.unwrap_or(0),
                    kalshi_order_status: row.get(28)?,
                    polymarket_order_status: row.get(29)?,
                    neutralization_leg: row.get(30)?,
                    neutralization_success: row.get::<_, Option<i32>>(31)?.map(|value| value != 0),
                    neutralization_order_id: row.get(32)?,
                    neutralization_filled_contracts: row.get::<_, Option<i32>>(33)?.unwrap_or(0),
                    neutralization_error: row.get(34)?,
                    residual_leg: row.get(35)?,
                    residual_contracts: row.get::<_, Option<i32>>(36)?.unwrap_or(0),
                    created_at: row.get(37)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(polymarket_fee: f64) -> AutoTradeExecutionRecord {
        AutoTradeExecutionRecord {
            event_name: "ARYAN SHAH VS EVAN BYNOE".into(),
            team_name: "ARYAN SHAH".into(),
            kalshi_market_id: "KXITFMATCH-26OCT01SHABYN-SHA".into(),
            polymarket_market_id: "aec-itfme-arysha-evabyn-2026-10-01".into(),
            kalshi_side: "no".into(),
            polymarket_side: "long".into(),
            contracts: 10,
            kalshi_price: 0.46,
            kalshi_fee: 0.18,
            polymarket_fee,
            polymarket_price: 0.50,
            profit_margin: 0.5,
            duration_ms: 0,
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
            status: "submitting".into(),
        }
    }

    #[tokio::test]
    async fn the_polymarket_fee_is_stored_finalized_and_part_of_the_total() {
        let storage = ArbitrageStorage::new(":memory:").unwrap();
        let id = storage.save_auto_trade_execution(&execution(0.17)).unwrap();

        let row = &storage.get_auto_trade_history(10).unwrap()[0];
        assert_eq!(row.polymarket_fee, Some(0.17));
        assert!((row.kalshi_fee - 0.18).abs() < 1e-9);
        // 10 x 0.46 + 0.18 + 10 x 0.50 + 0.17
        assert!((row.total_amount - 9.95).abs() < 1e-9, "{}", row.total_amount);

        // Finalising rewrites it from the finished record.
        let mut finished = execution(0.19);
        finished.status = "executed".into();
        storage.finish_auto_trade_execution(id, &finished).unwrap();
        let row = &storage.get_auto_trade_history(10).unwrap()[0];
        assert_eq!(row.polymarket_fee, Some(0.19));
        assert!((row.total_amount - 9.97).abs() < 1e-9, "{}", row.total_amount);
    }

    #[tokio::test]
    async fn legacy_rows_have_no_polymarket_fee_rather_than_a_fake_zero() {
        let storage = ArbitrageStorage::new(":memory:").unwrap();
        // A row exactly as written before Polymarket's fee was modeled: the
        // column exists (added by the migration) but was never set.
        storage
            .conn()
            .lock()
            .execute(
                "INSERT INTO auto_trade_history (
                    event_name, team_name, kalshi_market_id, polymarket_market_id, kalshi_side,
                    polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee, polymarket_amount,
                    polymarket_price, total_amount, profit_margin, duration_ms, kalshi_success,
                    polymarket_success, status, created_at
                 ) VALUES ('E', 'T', 'K', 'P', 'yes', 'short', 10, 0.5, 0.18, 4.4, 0.44, 9.58,
                           2.0, 0, 1, 1, 'executed', '2026-09-29T00:43:42+00:00')",
                [],
            )
            .unwrap();
        let row = &storage.get_auto_trade_history(10).unwrap()[0];
        assert_eq!(row.polymarket_fee, None);
    }

    #[tokio::test]
    async fn the_migration_adds_the_column_to_an_existing_database() {
        // Build a database with the exact schema the live app had before
        // Polymarket's fee was modeled (copied from the real
        // arbitrage_history.db), then open it with the app.
        let path = std::env::temp_dir().join(format!(
            "polytaoli-fee-migration-{}-{}.db",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute(
                r#"CREATE TABLE auto_trade_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_name TEXT NOT NULL,
                team_name TEXT NOT NULL,
                kalshi_market_id TEXT NOT NULL,
                polymarket_market_id TEXT NOT NULL,
                kalshi_side TEXT NOT NULL,
                polymarket_side TEXT NOT NULL,
                kalshi_contracts INTEGER NOT NULL,
                kalshi_price REAL NOT NULL,
                kalshi_fee REAL DEFAULT 0,
                polymarket_amount REAL NOT NULL,
                polymarket_price REAL NOT NULL,
                total_amount REAL NOT NULL,
                profit_margin REAL NOT NULL,
                duration_ms INTEGER NOT NULL,
                total_duration_ms INTEGER DEFAULT 0,
                kalshi_success INTEGER NOT NULL,
                polymarket_success INTEGER NOT NULL,
                kalshi_order_id TEXT,
                polymarket_order_id TEXT,
                kalshi_error TEXT,
                polymarket_error TEXT,
                status TEXT DEFAULT 'executed',
                skip_reason TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            , kalshi_latency_ms INTEGER DEFAULT 0, poly_latency_ms INTEGER DEFAULT 0, kalshi_filled_contracts INTEGER DEFAULT 0, polymarket_filled_contracts INTEGER DEFAULT 0, kalshi_order_status TEXT, polymarket_order_status TEXT, neutralization_leg TEXT, neutralization_success INTEGER, neutralization_order_id TEXT, neutralization_filled_contracts INTEGER DEFAULT 0, neutralization_error TEXT, residual_leg TEXT, residual_contracts INTEGER DEFAULT 0)"#,
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO auto_trade_history (event_name, team_name, kalshi_market_id,
                    polymarket_market_id, kalshi_side, polymarket_side, kalshi_contracts,
                    kalshi_price, polymarket_amount, polymarket_price, total_amount,
                    profit_margin, duration_ms, kalshi_success, polymarket_success, created_at)
                 VALUES ('OLD', 'T', 'K', 'P', 'yes', 'short', 10, 0.5, 4.4, 0.44, 9.4, 2.0, 0, 1, 1,
                         '2026-09-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
        }

        let storage = ArbitrageStorage::new(&path).unwrap();
        let history = storage.get_auto_trade_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event_name, "OLD");
        assert_eq!(history[0].polymarket_fee, None);

        // And new rows write the column on the migrated table.
        storage.save_auto_trade_execution(&execution(0.17)).unwrap();
        let fees: Vec<Option<f64>> = storage
            .get_auto_trade_history(10)
            .unwrap()
            .iter()
            .map(|r| r.polymarket_fee)
            .collect();
        assert!(fees.contains(&Some(0.17)) && fees.contains(&None), "{fees:?}");

        drop(storage);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn skipped_records_estimate_both_fees() {
        let storage = ArbitrageStorage::new(":memory:").unwrap();
        storage
            .save_skipped_auto_trade_record(
                "E", "T", "K", "P", "no", "long", 10, 0.46, 0.50, 0.5, 0, "No profitable size",
            )
            .unwrap();
        let row = &storage.get_auto_trade_history(10).unwrap()[0];
        assert_eq!(row.status, "skipped");
        assert!((row.kalshi_fee - 0.18).abs() < 1e-9);
        assert_eq!(row.polymarket_fee, Some(0.17));
        assert!((row.total_amount - 9.95).abs() < 1e-9, "{}", row.total_amount);
    }
}
