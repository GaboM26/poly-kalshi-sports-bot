//! SQLite storage for arbitrage tracking
//!
//! Provides async queue-based writing for non-blocking persistence.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::models::ArbitrageTrackingRecord;

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
                update_count INTEGER DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_start_time ON arbitrage_tracking(start_time)",
            [],
        )?;

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
                     kalshi_side, polymarket_side, update_count)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                conn.execute(
                    "UPDATE arbitrage_tracking SET end_time = ?1 WHERE id = ?2",
                    params![Utc::now().to_rfc3339(), id],
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
                    kalshi_side, polymarket_side, update_count
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
}

/// Storage statistics
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub total_records: usize,
    pub active_records: usize,
}
