//! 15-minute crypto "Up or Down" windows listed on both exchanges.
//!
//! Kalshi `KX{COIN}15M` and Polymarket US `{coin}-updown-15m` settle on the
//! same CF Benchmarks real-time index, each endpoint being the simple average
//! of the 60 index prices in the minute before it, Up/Yes when end >= start.
//! Pairs are joined on the window's end time. XRP is left out: Kalshi rounds
//! its average to 4 decimals and Polymarket to 5, so a near-flat window can
//! settle differently.

use chrono::{DateTime, Duration, Utc};

pub const CRYPTO_WINDOW_CATEGORY: &str = "CRYPTO15";
pub const WINDOW_MINUTES: i64 = 15;

/// Outcome names: the Kalshi YES / Polymarket LONG side is "UP".
pub const UP: &str = "UP";
pub const DOWN: &str = "DOWN";

pub struct CryptoWindowSeries {
    pub coin: &'static str,
    pub kalshi_series: &'static str,
    /// Index name as written in Kalshi `rules_primary`.
    pub kalshi_index: &'static str,
    pub polymarket_series_slug: &'static str,
    /// Index name as written in the Polymarket US market description.
    pub polymarket_index: &'static str,
}

pub const CRYPTO_WINDOW_SERIES: &[CryptoWindowSeries] = &[
    CryptoWindowSeries {
        coin: "BTC",
        kalshi_series: "KXBTC15M",
        kalshi_index: "BRTI",
        polymarket_series_slug: "btc-updown-15m",
        polymarket_index: "(BRTI)",
    },
    CryptoWindowSeries {
        coin: "ETH",
        kalshi_series: "KXETH15M",
        kalshi_index: "ETHUSDRTI",
        polymarket_series_slug: "eth-updown-15m",
        polymarket_index: "(ETHUSD_RTI)",
    },
    CryptoWindowSeries {
        coin: "SOL",
        kalshi_series: "KXSOL15M",
        kalshi_index: "SOLUSDRTI",
        polymarket_series_slug: "sol-updown-15m",
        polymarket_index: "(SOLUSD_RTI)",
    },
];

pub fn series_for_kalshi(kalshi_series: &str) -> Option<&'static CryptoWindowSeries> {
    CRYPTO_WINDOW_SERIES
        .iter()
        .find(|series| series.kalshi_series == kalshi_series)
}

/// Event name shared by both exchanges for one window, e.g. "BTC 15M 17:00Z".
/// The date is carried separately (the matcher's game date).
pub fn crypto_window_event_name(coin: &str, window_end: DateTime<Utc>) -> String {
    format!("{coin} 15M {}Z", window_end.format("%H:%M"))
}

/// Whether `start`..`end` is exactly one 15-minute window.
pub fn is_window(start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
    end - start == Duration::minutes(WINDOW_MINUTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_window_by_its_end_time() {
        let end = DateTime::parse_from_rfc3339("2026-10-26T17:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(crypto_window_event_name("BTC", end), "BTC 15M 17:00Z");
        assert!(is_window(end - Duration::minutes(15), end));
        assert!(!is_window(end - Duration::minutes(60), end));
    }

    #[test]
    fn xrp_is_not_paired() {
        assert!(series_for_kalshi("KXXRP15M").is_none());
        assert!(series_for_kalshi("KXBTC15M").is_some());
    }
}
