//! SQLite storage for arbitrage tracking and auto-trade state
//!
//! Provides async queue-based writing for non-blocking persistence.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::models::ArbitrageTrackingRecord;

/// Auto-trade state stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTradeState {
    pub enabled: bool,
    pub trade_count: i32,
    pub max_trade_count: i32,
    pub max_amount: f64,
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
    pub duration_ms: i64,
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

/// Storage command for async queue
pub enum StorageCommand {
    TrackStart(ArbitrageTrackingRecord),
    TrackUpdate {
        id: String,
        profit_margin: f64,
    },
    TrackEnd(String),
}

/// Arbitrage storage service
pub struct ArbitrageStorage {
    conn: Arc<Mutex<Connection>>,
    command_tx: mpsc::Sender<StorageCommand>,
}

impl ArbitrageStorage {
    /// Create a new storage instance
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database: {:?}", db_path.as_ref()))?;

        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS arbitrage_tracking (
                id TEXT PRIMARY KEY,
                event_name TEXT NOT NULL,
                team_name TEXT NOT NULL,
                kalshi_market_id TEXT NOT NULL,
                polymarket_market_id TEXT NOT NULL,
                start_time TEXT NOT NULL,
                end_time TEXT,
                initial_profit_margin REAL NOT NULL,
                max_profit_margin REAL NOT NULL,
                kalshi_side TEXT NOT NULL,
                polymarket_side TEXT NOT NULL,
                update_count INTEGER DEFAULT 0,
                poly_ask_depth REAL DEFAULT 0,
                poly_ask_size REAL DEFAULT 0,
                kalshi_ask_depth INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                kalshi_ask_price REAL DEFAULT 0,
                polymarket_ask_price REAL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_start_time ON arbitrage_tracking(start_time)",
            [],
        )?;

        // Migrate existing tables: add new columns if they don't exist
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN poly_ask_depth REAL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN poly_ask_size REAL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN kalshi_ask_depth INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN duration_ms INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN kalshi_ask_price REAL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE arbitrage_tracking ADD COLUMN polymarket_ask_price REAL DEFAULT 0",
            [],
        );

        // Create auto_trade_state table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS auto_trade_state (
                id INTEGER PRIMARY KEY DEFAULT 1,
                enabled INTEGER DEFAULT 0,
                trade_count INTEGER DEFAULT 0,
                max_trade_count INTEGER DEFAULT 2,
                max_amount REAL DEFAULT 10.0,
                min_duration_ms INTEGER DEFAULT 500,
                last_trade_time TEXT,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Migrate auto_trade_state: add new columns if they don't exist
        let _ = conn.execute(
            "ALTER TABLE auto_trade_state ADD COLUMN min_duration_ms INTEGER DEFAULT 500",
            [],
        );

        // Initialize auto_trade_state with default row if not exists
        conn.execute(
            "INSERT OR IGNORE INTO auto_trade_state (id, enabled, trade_count, max_trade_count, max_amount, min_duration_ms) 
             VALUES (1, 0, 0, 2, 10.0, 500)",
            [],
        )?;

        // Create auto_trade_history table for recording executed and skipped trades
        conn.execute(
            "CREATE TABLE IF NOT EXISTS auto_trade_history (
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
                kalshi_success INTEGER NOT NULL,
                polymarket_success INTEGER NOT NULL,
                kalshi_order_id TEXT,
                polymarket_order_id TEXT,
                kalshi_error TEXT,
                polymarket_error TEXT,
                status TEXT DEFAULT 'executed',
                skip_reason TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Migrate existing auto_trade_history table: add new columns if they don't exist
        let _ = conn.execute(
            "ALTER TABLE auto_trade_history ADD COLUMN kalshi_fee REAL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE auto_trade_history ADD COLUMN status TEXT DEFAULT 'executed'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE auto_trade_history ADD COLUMN skip_reason TEXT",
            [],
        );

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_auto_trade_created_at ON auto_trade_history(created_at)",
            [],
        )?;

        info!("📦 数据库初始化完成，包含 auto_trade_state 和 auto_trade_history 表");

        let conn = Arc::new(Mutex::new(conn));

        // Create command channel
        let (command_tx, mut command_rx) = mpsc::channel::<StorageCommand>(1000);

        // Spawn background writer
        let conn_clone = conn.clone();
        tokio::spawn(async move {
            while let Some(cmd) = command_rx.recv().await {
                let conn = conn_clone.lock();
                if let Err(e) = Self::execute_command(&conn, cmd) {
                    error!("存储错误: {}", e);
                }
            }
        });

        Ok(Self { conn, command_tx })
    }

    /// Execute a storage command
    fn execute_command(conn: &Connection, cmd: StorageCommand) -> Result<()> {
        match cmd {
            StorageCommand::TrackStart(record) => {
                conn.execute(
                    "INSERT OR REPLACE INTO arbitrage_tracking 
                    (id, event_name, team_name, kalshi_market_id, polymarket_market_id, 
                     start_time, initial_profit_margin, max_profit_margin, 
                     kalshi_side, polymarket_side, update_count,
                     poly_ask_depth, poly_ask_size, kalshi_ask_depth, duration_ms,
                     kalshi_ask_price, polymarket_ask_price)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        record.id,
                        record.event_name,
                        record.team_name,
                        record.kalshi_market_id,
                        record.polymarket_market_id,
                        record.start_time.to_rfc3339(),
                        record.initial_profit_margin,
                        record.max_profit_margin,
                        record.kalshi_side,
                        record.polymarket_side,
                        record.update_count,
                        record.poly_ask_depth,
                        record.poly_ask_size,
                        record.kalshi_ask_depth,
                        record.duration_ms,
                        record.kalshi_ask_price,
                        record.polymarket_ask_price,
                    ],
                )?;
            }
            StorageCommand::TrackUpdate { id, profit_margin } => {
                conn.execute(
                    "UPDATE arbitrage_tracking 
                    SET max_profit_margin = MAX(max_profit_margin, ?1),
                        update_count = update_count + 1
                    WHERE id = ?2",
                    params![profit_margin, id],
                )?;
            }
            StorageCommand::TrackEnd(id) => {
                let now = Utc::now();
                // Calculate duration_ms from start_time
                let duration_ms: i64 = conn
                    .query_row(
                        "SELECT start_time FROM arbitrage_tracking WHERE id = ?1",
                        params![&id],
                        |row| {
                            let start_str: String = row.get(0)?;
                            let start_dt = DateTime::parse_from_rfc3339(&start_str)
                                .map(|dt| dt.with_timezone(&Utc))
                                .unwrap_or(now);
                            Ok(now.signed_duration_since(start_dt).num_milliseconds())
                        },
                    )
                    .unwrap_or(0);

                conn.execute(
                    "UPDATE arbitrage_tracking SET end_time = ?1, duration_ms = ?2 WHERE id = ?3",
                    params![now.to_rfc3339(), duration_ms, id],
                )?;
            }
        }
        Ok(())
    }

    /// Start tracking an arbitrage opportunity
    pub fn track_start(&self, record: ArbitrageTrackingRecord) {
        let _ = self.command_tx.try_send(StorageCommand::TrackStart(record));
    }

    /// Update tracking with new profit margin
    pub fn track_update(&self, id: &str, profit_margin: f64) {
        let _ = self.command_tx.try_send(StorageCommand::TrackUpdate {
            id: id.to_string(),
            profit_margin,
        });
    }

    /// End tracking for an opportunity
    pub fn track_end(&self, id: &str) {
        let _ = self
            .command_tx
            .try_send(StorageCommand::TrackEnd(id.to_string()));
    }

    /// Get recent arbitrage history
    pub fn get_history(&self, limit: usize) -> Result<Vec<ArbitrageTrackingRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, event_name, team_name, kalshi_market_id, polymarket_market_id,
                    start_time, end_time, initial_profit_margin, max_profit_margin,
                    kalshi_side, polymarket_side, update_count,
                    poly_ask_depth, poly_ask_size, kalshi_ask_depth, duration_ms,
                    kalshi_ask_price, polymarket_ask_price
             FROM arbitrage_tracking
             ORDER BY start_time DESC
             LIMIT ?1",
        )?;

        let records = stmt
            .query_map([limit], |row| {
                Ok(ArbitrageTrackingRecord {
                    id: row.get(0)?,
                    event_name: row.get(1)?,
                    team_name: row.get(2)?,
                    kalshi_market_id: row.get(3)?,
                    polymarket_market_id: row.get(4)?,
                    start_time: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    end_time: row
                        .get::<_, Option<String>>(6)?
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    initial_profit_margin: row.get(7)?,
                    max_profit_margin: row.get(8)?,
                    kalshi_side: row.get(9)?,
                    polymarket_side: row.get(10)?,
                    update_count: row.get(11)?,
                    poly_ask_depth: row.get(12).unwrap_or(0.0),
                    poly_ask_size: row.get(13).unwrap_or(0.0),
                    kalshi_ask_depth: row.get(14).unwrap_or(0),
                    duration_ms: row.get(15).unwrap_or(0),
                    kalshi_ask_price: row.get(16).unwrap_or(0.0),
                    polymarket_ask_price: row.get(17).unwrap_or(0.0),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Get storage statistics
    pub fn get_stats(&self) -> StorageStats {
        let conn = self.conn.lock();

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM arbitrage_tracking", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM arbitrage_tracking WHERE end_time IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        StorageStats {
            total_records: total as usize,
            active_records: active as usize,
        }
    }

    /// Search records with filters
    pub fn search_records(
        &self,
        min_profit: Option<f64>,
        max_profit: Option<f64>,
        min_duration: Option<f64>,
        max_duration: Option<f64>,
        event_name: Option<String>,
        team_name: Option<String>,
        sort_by: Option<String>,
        sort_order: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
        _include_history: Option<bool>,
    ) -> Result<serde_json::Value> {
        let conn = self.conn.lock();

        // Build WHERE clause
        let mut where_clauses = vec!["end_time IS NOT NULL"];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(min) = min_profit {
            where_clauses.push("max_profit_margin >= ?");
            params.push(Box::new(min));
        }
        if let Some(max) = max_profit {
            where_clauses.push("max_profit_margin <= ?");
            params.push(Box::new(max));
        }
        if let Some(name) = event_name {
            where_clauses.push("event_name LIKE ?");
            params.push(Box::new(format!("%{}%", name)));
        }
        if let Some(team) = team_name {
            where_clauses.push("team_name LIKE ?");
            params.push(Box::new(format!("%{}%", team)));
        }

        let where_clause = where_clauses.join(" AND ");

        // Build ORDER BY clause
        let sort_field = sort_by.unwrap_or_else(|| "start_time".to_string());
        let sort_dir = sort_order.unwrap_or_else(|| "desc".to_string());
        let order_clause = format!("{} {}", sort_field, sort_dir);

        // Count total matching records
        let count_query = format!("SELECT COUNT(*) FROM arbitrage_tracking WHERE {}", where_clause);
        let total: i64 = {
            let mut stmt = conn.prepare(&count_query)?;
            let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            stmt.query_row(&params_refs[..], |row| row.get(0))?
        };

        // Fetch records
        let limit_val = limit.unwrap_or(50);
        let offset_val = offset.unwrap_or(0);

        let query = format!(
            "SELECT id, event_name, team_name, kalshi_market_id, polymarket_market_id,
                    start_time, end_time, initial_profit_margin, max_profit_margin,
                    kalshi_side, polymarket_side, update_count,
                    poly_ask_depth, poly_ask_size, kalshi_ask_depth, duration_ms,
                    kalshi_ask_price, polymarket_ask_price
             FROM arbitrage_tracking
             WHERE {}
             ORDER BY {}
             LIMIT ? OFFSET ?",
            where_clause, order_clause
        );

        let mut stmt = conn.prepare(&query)?;
        params.push(Box::new(limit_val as i64));
        params.push(Box::new(offset_val as i64));
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let records: Vec<serde_json::Value> = stmt
            .query_map(&params_refs[..], |row| {
                let start_time: String = row.get(5)?;
                let end_time: Option<String> = row.get(6)?;
                let poly_depth: f64 = row.get(12).unwrap_or(0.0);
                let poly_size: f64 = row.get(13).unwrap_or(0.0);
                let kalshi_depth: i32 = row.get(14).unwrap_or(0);
                let duration_ms: i64 = row.get(15).unwrap_or(0);
                let kalshi_ask_price: f64 = row.get(16).unwrap_or(0.0);
                let polymarket_ask_price: f64 = row.get(17).unwrap_or(0.0);

                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "event_name": row.get::<_, String>(1)?,
                    "team_name": row.get::<_, String>(2)?,
                    "kalshi_market_id": row.get::<_, String>(3)?,
                    "polymarket_market_id": row.get::<_, String>(4)?,
                    "kalshi_side": row.get::<_, String>(9)?,
                    "polymarket_side": row.get::<_, String>(10)?,
                    "start_time": start_time,
                    "end_time": end_time,
                    "duration_ms": duration_ms,
                    "duration_seconds": duration_ms / 1000,
                    "max_profit_margin": row.get::<_, f64>(8)?,
                    "poly_ask_depth": poly_depth,
                    "poly_ask_size": poly_size,
                    "kalshi_ask_depth": kalshi_depth,
                    "kalshi_ask_price": kalshi_ask_price,
                    "polymarket_ask_price": polymarket_ask_price,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(serde_json::json!({
            "records": records,
            "total": total,
            "limit": limit_val,
            "offset": offset_val,
            "has_more": (offset_val + records.len()) < total as usize,
        }))
    }

    /// Get comprehensive statistics
    pub fn get_statistics(&self) -> Result<serde_json::Value> {
        let conn = self.conn.lock();

        // Basic stats
        let total_records: i64 = conn.query_row(
            "SELECT COUNT(*) FROM arbitrage_tracking WHERE end_time IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        if total_records == 0 {
            return Ok(serde_json::json!({
                "total_records": 0,
                "avg_profit": 0.0,
                "max_profit": 0.0,
                "min_profit": 0.0,
                "avg_duration": 0.0,
                "max_duration": 0.0,
                "min_duration": 0.0,
                "top_events": [],
                "top_teams": [],
                "profit_distribution": [],
            }));
        }

        // Profit stats
        let (avg_profit, max_profit, min_profit): (f64, f64, f64) = conn.query_row(
            "SELECT AVG(max_profit_margin), MAX(max_profit_margin), MIN(max_profit_margin) 
             FROM arbitrage_tracking WHERE end_time IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        // Duration stats (calculate from timestamps)
        let mut duration_stmt = conn.prepare(
            "SELECT start_time, end_time FROM arbitrage_tracking WHERE end_time IS NOT NULL"
        )?;
        
        let durations: Vec<i64> = duration_stmt
            .query_map([], |row| {
                let start: String = row.get(0)?;
                let end: String = row.get(1)?;
                let start_dt = DateTime::parse_from_rfc3339(&start)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let end_dt = DateTime::parse_from_rfc3339(&end)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(end_dt.signed_duration_since(start_dt).num_seconds())
            })?
            .filter_map(|r| r.ok())
            .collect();

        let avg_duration = if !durations.is_empty() {
            durations.iter().sum::<i64>() as f64 / durations.len() as f64
        } else {
            0.0
        };
        let max_duration = durations.iter().max().copied().unwrap_or(0) as f64;
        let min_duration = durations.iter().min().copied().unwrap_or(0) as f64;

        // Top events
        let mut event_stmt = conn.prepare(
            "SELECT event_name, COUNT(*) as count, AVG(max_profit_margin) as avg_profit
             FROM arbitrage_tracking WHERE end_time IS NOT NULL
             GROUP BY event_name ORDER BY count DESC LIMIT 10"
        )?;
        let top_events: Vec<serde_json::Value> = event_stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "event_name": row.get::<_, String>(0)?,
                    "count": row.get::<_, i64>(1)?,
                    "avg_profit": row.get::<_, f64>(2)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Top teams
        let mut team_stmt = conn.prepare(
            "SELECT team_name, COUNT(*) as count, AVG(max_profit_margin) as avg_profit
             FROM arbitrage_tracking WHERE end_time IS NOT NULL
             GROUP BY team_name ORDER BY count DESC LIMIT 10"
        )?;
        let top_teams: Vec<serde_json::Value> = team_stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "team_name": row.get::<_, String>(0)?,
                    "count": row.get::<_, i64>(1)?,
                    "avg_profit": row.get::<_, f64>(2)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Profit distribution
        let profit_ranges = vec![
            ("0-2%", 0.0, 2.0),
            ("2-5%", 2.0, 5.0),
            ("5-10%", 5.0, 10.0),
            ("10-20%", 10.0, 20.0),
            ("20%+", 20.0, 1000.0),
        ];

        let mut profit_distribution = Vec::new();
        for (label, min, max) in profit_ranges {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM arbitrage_tracking 
                 WHERE end_time IS NOT NULL AND max_profit_margin >= ? AND max_profit_margin < ?",
                params![min, max],
                |row| row.get(0),
            )?;
            profit_distribution.push(serde_json::json!({
                "range": label,
                "count": count,
            }));
        }

        Ok(serde_json::json!({
            "total_records": total_records,
            "avg_profit": avg_profit,
            "max_profit": max_profit,
            "min_profit": min_profit,
            "avg_duration": avg_duration,
            "max_duration": max_duration,
            "min_duration": min_duration,
            "top_events": top_events,
            "top_teams": top_teams,
            "profit_distribution": profit_distribution,
        }))
    }
}

/// Storage statistics
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub total_records: usize,
    pub active_records: usize,
}

// ==================== Auto-Trade State Methods ====================

impl ArbitrageStorage {
    /// Get current auto-trade state
    pub fn get_auto_trade_state(&self) -> Result<AutoTradeState> {
        let conn = self.conn.lock();
        
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
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE auto_trade_state SET enabled = ?, updated_at = ? WHERE id = 1",
            params![enabled as i32, Utc::now().to_rfc3339()],
        )?;
        info!("🔄 自动下单状态已更新: enabled = {}", enabled);
        Ok(())
    }

    /// Increment trade count after successful execution
    pub fn increment_trade_count(&self) -> Result<i32> {
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
        
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
        kalshi_success: bool,
        polymarket_success: bool,
        kalshi_order_id: Option<&str>,
        polymarket_order_id: Option<&str>,
        kalshi_error: Option<&str>,
        polymarket_error: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock();
        
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
        let conn = self.conn.lock();
        
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
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, event_name, team_name, kalshi_market_id, polymarket_market_id,
                    kalshi_side, polymarket_side, kalshi_contracts, kalshi_price, kalshi_fee,
                    polymarket_amount, polymarket_price, total_amount, profit_margin,
                    duration_ms, kalshi_success, polymarket_success,
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
                    kalshi_success: row.get::<_, i32>(15)? != 0,
                    polymarket_success: row.get::<_, i32>(16)? != 0,
                    kalshi_order_id: row.get(17)?,
                    polymarket_order_id: row.get(18)?,
                    kalshi_error: row.get(19)?,
                    polymarket_error: row.get(20)?,
                    status: row.get::<_, Option<String>>(21)?.unwrap_or_else(|| "executed".to_string()),
                    skip_reason: row.get(22)?,
                    created_at: row.get(23)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(records)
    }
}
