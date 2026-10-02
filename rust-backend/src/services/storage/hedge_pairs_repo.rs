//! Hedge pairs repository
//!
//! Handles operations for the hedge_pairs table: which Kalshi leg and
//! Polymarket leg offset each other, as defined by the matcher's market
//! mapping. The Positions view derives hedges from the *currently matched*
//! markets, but a game drops out of that set when it closes - often well
//! before it settles and the positions disappear - so a verified pairing is
//! remembered here to keep a hedge from flipping back to "unhedged" at the
//! final whistle or across a backend restart. A row is only a statement
//! about the markets; a card still renders as paired only while both legs
//! are actually held.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};

use super::ArbitrageStorage;

/// A Kalshi leg and the Polymarket leg that hedges it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HedgePair {
    pub event_name: String,
    pub kalshi_market_id: String,
    /// "yes" / "no"
    pub kalshi_side: String,
    pub polymarket_market_id: String,
    /// "long" / "short"
    pub polymarket_side: String,
}

impl ArbitrageStorage {
    pub(super) fn init_hedge_pairs_table(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS hedge_pairs (
                kalshi_market_id TEXT NOT NULL,
                kalshi_side TEXT NOT NULL,
                polymarket_market_id TEXT NOT NULL,
                polymarket_side TEXT NOT NULL,
                event_name TEXT NOT NULL,
                first_seen TEXT NOT NULL,
                PRIMARY KEY (kalshi_market_id, kalshi_side, polymarket_market_id, polymarket_side)
            )",
            [],
        )?;
        Ok(())
    }

    /// Remember verified pairs. Re-remembering a known pair is a no-op.
    pub fn remember_hedge_pairs(&self, pairs: &[HedgePair]) -> Result<()> {
        let conn = self.conn().lock();
        for pair in pairs {
            conn.execute(
                "INSERT OR IGNORE INTO hedge_pairs
                    (kalshi_market_id, kalshi_side, polymarket_market_id, polymarket_side, event_name, first_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    pair.kalshi_market_id,
                    pair.kalshi_side.to_ascii_lowercase(),
                    pair.polymarket_market_id,
                    pair.polymarket_side.to_ascii_lowercase(),
                    pair.event_name,
                    Utc::now().to_rfc3339()
                ],
            )?;
        }
        Ok(())
    }

    /// Every remembered pair.
    pub fn get_hedge_pairs(&self) -> Result<Vec<HedgePair>> {
        let conn = self.conn().lock();
        let mut stmt = conn.prepare(
            "SELECT event_name, kalshi_market_id, kalshi_side, polymarket_market_id, polymarket_side
             FROM hedge_pairs",
        )?;
        let pairs = stmt
            .query_map([], |row| {
                Ok(HedgePair {
                    event_name: row.get(0)?,
                    kalshi_market_id: row.get(1)?,
                    kalshi_side: row.get(2)?,
                    polymarket_market_id: row.get(3)?,
                    polymarket_side: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(kalshi_side: &str) -> HedgePair {
        HedgePair {
            event_name: "AZK VS SIN".into(),
            kalshi_market_id: "KXITFMATCH-26OCT02AZKSIN-SIN".into(),
            kalshi_side: kalshi_side.into(),
            polymarket_market_id: "aec-itfme-ardazk-digsin-2026-10-02".into(),
            polymarket_side: "short".into(),
        }
    }

    #[tokio::test]
    async fn remembers_pairs_once_and_returns_them() {
        let storage = ArbitrageStorage::new(":memory:").unwrap();
        assert!(storage.get_hedge_pairs().unwrap().is_empty());

        storage.remember_hedge_pairs(&[pair("no")]).unwrap();
        storage.remember_hedge_pairs(&[pair("no"), pair("yes")]).unwrap();
        let mut stored = storage.get_hedge_pairs().unwrap();
        stored.sort_by(|a, b| a.kalshi_side.cmp(&b.kalshi_side));
        assert_eq!(stored, vec![pair("no"), pair("yes")]);
    }

    #[tokio::test]
    async fn sides_are_stored_lowercase() {
        let storage = ArbitrageStorage::new(":memory:").unwrap();
        let mut shouty = pair("NO");
        shouty.polymarket_side = "SHORT".into();
        storage.remember_hedge_pairs(&[shouty]).unwrap();
        assert_eq!(storage.get_hedge_pairs().unwrap(), vec![pair("no")]);
    }
}
