//! Excluded markets repository
//!
//! Handles operations for the excluded_markets table.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use std::collections::HashSet;
use tracing::info;

use super::ArbitrageStorage;

impl ArbitrageStorage {
    /// Add a market to exclusion list
    /// Normalizes event_name and team_name to ensure consistency
    pub fn exclude_market(&self, event_name: &str, team_name: &str) -> Result<bool> {
        let conn = self.conn().lock();
        
        // Normalize to uppercase for consistency
        let normalized_event = event_name.to_uppercase();
        let normalized_team = team_name.to_uppercase();
        let market_key = format!("{}_{}", normalized_event, normalized_team);
        
        let result = conn.execute(
            "INSERT OR IGNORE INTO excluded_markets (event_name, team_name, market_key, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![normalized_event, normalized_team, market_key, Utc::now().to_rfc3339()],
        )?;
        
        if result > 0 {
            info!("🚫 市场已排除并保存到数据库: {} - {}", normalized_event, normalized_team);
        }
        
        Ok(result > 0)
    }

    /// Remove a market from exclusion list
    /// Normalizes event_name and team_name to ensure consistency
    pub fn unexclude_market(&self, event_name: &str, team_name: &str) -> Result<bool> {
        let conn = self.conn().lock();
        
        // Normalize to uppercase for consistency
        let normalized_event = event_name.to_uppercase();
        let normalized_team = team_name.to_uppercase();
        let market_key = format!("{}_{}", normalized_event, normalized_team);
        
        let result = conn.execute(
            "DELETE FROM excluded_markets WHERE market_key = ?1",
            params![market_key],
        )?;
        
        if result > 0 {
            info!("✅ 市场已取消排除: {} - {}", normalized_event, normalized_team);
        }
        
        Ok(result > 0)
    }

    /// Get all excluded market keys
    pub fn get_excluded_markets(&self) -> Result<HashSet<String>> {
        let conn = self.conn().lock();
        
        let mut stmt = conn.prepare(
            "SELECT market_key FROM excluded_markets"
        )?;
        
        let keys = stmt.query_map([], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .collect();
        
        Ok(keys)
    }

    /// Check if a market is excluded
    /// Normalizes event_name and team_name to ensure consistency
    pub fn is_market_excluded(&self, event_name: &str, team_name: &str) -> Result<bool> {
        let conn = self.conn().lock();
        
        // Normalize to uppercase for consistency
        let normalized_event = event_name.to_uppercase();
        let normalized_team = team_name.to_uppercase();
        let market_key = format!("{}_{}", normalized_event, normalized_team);
        
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM excluded_markets WHERE market_key = ?1",
            params![market_key],
            |row| row.get(0),
        )?;
        
        Ok(count > 0)
    }
}
