//! Kalshi market catalog: event/market fetching and parsing per sport category.

use super::*;

/// Event categories this client knows how to parse. A series is scanned
/// with one of these (see `[sports_arb] series` in the config).
pub const PARSEABLE_CATEGORIES: &[&str] = &["NBA", "TENNIS", "NHL", "MLB", "CRYPTO15"];

pub(super) fn parse_nba_event(event_data: &Value) -> Option<KalshiEvent> {
    let event_ticker = event_data["event_ticker"].as_str()?.to_string();
    let (mut team_a, mut team_b) = extract_teams_from_ticker(&event_ticker)?;
    if team_a > team_b {
        std::mem::swap(&mut team_a, &mut team_b);
    }

    let event_name = format!("{team_a}-{team_b}");
    let start_time = extract_event_start_time(event_data, &event_ticker);
    let mut event = KalshiEvent {
        event_id: event_ticker.clone(),
        name: event_name.clone(),
        team_a: team_a.clone(),
        team_b: team_b.clone(),
        start_time,
        category: "NBA".to_string(),
        markets: Vec::new(),
    };

    let Some(markets) = event_data["markets"].as_array() else {
        return Some(event);
    };
    for market_data in markets {
        let Some(ticker) = market_data["ticker"].as_str().map(ToOwned::to_owned) else {
            continue;
        };
        let Some(team_name) = extract_team_from_ticker(&ticker) else {
            continue;
        };
        let opponent_name = if team_name == team_a {
            &team_b
        } else {
            &team_a
        };
        if let Some(market) = build_kalshi_market(
            market_data,
            ticker,
            event_ticker.clone(),
            event_name.clone(),
            team_name,
            opponent_name.clone(),
            start_time,
        ) {
            event.markets.push(market);
        }
    }

    Some(event)
}

pub(super) fn parse_tennis_event(event_data: &Value) -> Option<KalshiEvent> {
    let event_ticker = event_data["event_ticker"].as_str()?.to_string();
    let market_data = event_data["markets"].as_array()?;

    // A match winner event must expose exactly two independent binary markets,
    // one for each competitor. Anything else is deliberately excluded.
    if market_data.len() != 2 {
        return None;
    }

    let mut parsed_markets = Vec::with_capacity(2);
    for market in market_data {
        let ticker = market["ticker"].as_str()?.to_string();
        let competitor = extract_tennis_competitor(market)?;
        parsed_markets.push((competitor, ticker, market));
    }

    if parsed_markets[0].0 == parsed_markets[1].0 {
        return None;
    }

    let mut competitors = [parsed_markets[0].0.clone(), parsed_markets[1].0.clone()];
    competitors.sort();
    let event_name = competitor_event_name(&competitors[0], &competitors[1])?;
    let start_time = extract_event_start_time(event_data, &event_ticker);
    let mut event = KalshiEvent {
        event_id: event_ticker.clone(),
        name: event_name.clone(),
        team_a: competitors[0].clone(),
        team_b: competitors[1].clone(),
        start_time,
        category: "TENNIS".to_string(),
        markets: Vec::with_capacity(2),
    };

    for (competitor, ticker, market) in parsed_markets {
        let opponent = if competitor == competitors[0] {
            competitors[1].clone()
        } else {
            competitors[0].clone()
        };
        event.markets.push(build_kalshi_market(
            market,
            ticker,
            event_ticker.clone(),
            event_name.clone(),
            competitor,
            opponent,
            start_time,
        )?);
    }

    Some(event)
}

/// NHL/MLB game: exactly two markets, one per team, the team being the
/// market ticker suffix (event tickers cannot be split: NHL has 2-letter
/// codes and MLB tickers embed the start time). The game date comes from the
/// event ticker (US local date), matching the Polymarket slug date.
pub(super) fn parse_team_game_event(event_data: &Value, category: &str) -> Option<KalshiEvent> {
    let event_ticker = event_data["event_ticker"].as_str()?.to_string();
    let market_data = event_data["markets"].as_array()?;
    if market_data.len() != 2 {
        return None;
    }

    let mut parsed_markets = Vec::with_capacity(2);
    for market in market_data {
        let ticker = market["ticker"].as_str()?.to_string();
        let (prefix, suffix) = ticker.rsplit_once('-')?;
        if prefix != event_ticker {
            return None;
        }
        let team = crate::core::canonical_team_code(category, suffix)?;
        parsed_markets.push((team, ticker, market));
    }

    let event_name =
        crate::core::team_game_event_name(category, &parsed_markets[0].0, &parsed_markets[1].0)?;
    let start_time = extract_game_date_from_ticker(&event_ticker);
    let mut teams = [parsed_markets[0].0.clone(), parsed_markets[1].0.clone()];
    teams.sort();
    let mut event = KalshiEvent {
        event_id: event_ticker.clone(),
        name: event_name.clone(),
        team_a: teams[0].clone(),
        team_b: teams[1].clone(),
        start_time,
        category: category.to_string(),
        markets: Vec::with_capacity(2),
    };

    for (team, ticker, market) in parsed_markets {
        let opponent = if team == teams[0] {
            teams[1].clone()
        } else {
            teams[0].clone()
        };
        event.markets.push(build_kalshi_market(
            market,
            ticker,
            event_ticker.clone(),
            event_name.clone(),
            team,
            opponent,
            start_time,
        )?);
    }

    Some(event)
}

/// One 15-minute crypto window: a single market whose YES is "Up" (end
/// average >= start average of the series' CF Benchmarks index). Anything
/// that does not read exactly that way is excluded.
pub(super) fn parse_crypto_window_event(event_data: &Value, series_ticker: &str) -> Option<KalshiEvent> {
    use crate::core::crypto_windows::{self, DOWN, UP};

    let series = crypto_windows::series_for_kalshi(series_ticker)?;
    let event_ticker = event_data["event_ticker"].as_str()?.to_string();
    let [market] = event_data["markets"].as_array()?.as_slice() else {
        return None;
    };
    let ticker = market["ticker"].as_str()?.to_string();
    if market["strike_type"].as_str() != Some("greater_or_equal")
        || !market["rules_primary"]
            .as_str()
            .is_some_and(|rules| rules.contains(series.kalshi_index) && rules.contains("is at least"))
    {
        return None;
    }
    let time = |field: &str| {
        market[field]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
    };
    let (open, close) = (time("open_time")?, time("close_time")?);
    if !crypto_windows::is_window(open, close) {
        return None;
    }

    let event_name = crypto_windows::crypto_window_event_name(series.coin, close);
    let market = build_kalshi_market(
        market,
        ticker,
        event_ticker.clone(),
        event_name.clone(),
        UP.to_string(),
        DOWN.to_string(),
        Some(close),
    )?;
    Some(KalshiEvent {
        event_id: event_ticker,
        name: event_name,
        team_a: DOWN.to_string(),
        team_b: UP.to_string(),
        start_time: Some(close),
        category: crypto_windows::CRYPTO_WINDOW_CATEGORY.to_string(),
        markets: vec![market],
    })
}

pub(super) fn build_kalshi_market(
    market_data: &Value,
    ticker: String,
    event_id: String,
    event_name: String,
    team_name: String,
    opponent_name: String,
    start_time: Option<DateTime<Utc>>,
) -> Option<KalshiMarket> {
    let yes_price = initial_yes_price(market_data)?;

    Some(KalshiMarket {
        market_id: ticker,
        event_id,
        event_name,
        team_name,
        opponent_name,
        yes_price,
        no_price: 1.0 - yes_price,
        start_time,
        volume: market_data["volume"].as_f64().or_else(|| {
            market_data["volume_fp"]
                .as_str()
                .and_then(|value| value.parse().ok())
        }),
        liquidity: market_data["open_interest"].as_f64().or_else(|| {
            market_data["open_interest_fp"]
                .as_str()
                .and_then(|value| value.parse().ok())
        }),
    })
}

pub(super) fn initial_yes_price(market_data: &Value) -> Option<f64> {
    if !market_data["yes_ask_dollars"].is_null() {
        return parse_quote_dollars(&market_data["yes_ask_dollars"], "yes_ask_dollars").ok();
    }
    if !market_data["last_price_dollars"].is_null() {
        return parse_quote_dollars(&market_data["last_price_dollars"], "last_price_dollars").ok();
    }

    market_data["yes_ask"]
        .as_f64()
        .or_else(|| market_data["last_price"].as_f64())
        .filter(|price| price.is_finite() && (0.0..=100.0).contains(price))
        .map(|price| price / 100.0)
}

pub(super) fn extract_tennis_competitor(market_data: &Value) -> Option<String> {
    market_data["yes_sub_title"]
        .as_str()
        .and_then(normalize_competitor_name)
        .or_else(|| {
            market_data["title"]
                .as_str()
                .and_then(extract_tennis_competitor_from_market_title)
        })
}

pub(super) fn extract_tennis_competitor_from_market_title(title: &str) -> Option<String> {
    let title = title.trim();
    let title_lower = title.to_ascii_lowercase();
    let prefix = "will ";
    let separator = " win the ";

    if title_lower.starts_with(prefix) {
        let name_end = title_lower[prefix.len()..].find(separator)? + prefix.len();
        return normalize_competitor_name(&title[prefix.len()..name_end]);
    }

    let suffix = " wins";
    let competitor = title_lower.strip_suffix(suffix)?;
    normalize_competitor_name(&title[..competitor.len()])
}

pub(super) fn extract_event_start_time(event_data: &Value, event_ticker: &str) -> Option<DateTime<Utc>> {
    ["expected_expiration_time", "close_time", "open_time"]
        .iter()
        .find_map(|field| {
            event_data[*field]
                .as_str()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
        })
        .or_else(|| extract_game_date_from_ticker(event_ticker))
}

/// Parse team names from event title
/// Extract team names from event_ticker
/// Example: "KXNBAGAME-26JAN07CLELAL" -> ("CLE", "LAL")
pub(super) fn extract_teams_from_ticker(event_ticker: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = event_ticker.split('-').collect();
    if parts.len() < 2 {
        return None;
    }

    let last_part = parts.last()?;
    if last_part.len() <= 7 {
        return None;
    }

    // Skip the date part (first 7 chars like "26JAN07")
    let teams_str = &last_part[7..];

    // Most common: 6 characters (3 + 3)
    if teams_str.len() == 6 {
        return Some((teams_str[..3].to_uppercase(), teams_str[3..].to_uppercase()));
    }

    // Handle 7+ characters by splitting in the middle
    if teams_str.len() >= 4 {
        let mid = teams_str.len() / 2;
        return Some((
            teams_str[..mid].to_uppercase(),
            teams_str[mid..].to_uppercase(),
        ));
    }

    None
}

/// Extract team from market ticker
/// Example: "KXNBAGAME-26JAN07CLELAL-CLE" -> "CLE"
pub(super) fn extract_team_from_ticker(ticker: &str) -> Option<String> {
    let parts: Vec<&str> = ticker.split('-').collect();
    if parts.len() < 3 {
        return None;
    }

    // Last part is the team abbreviation
    Some(parts.last()?.to_uppercase())
}

/// Extract game date from event ticker (e.g., "KXNBA-26JAN08-DAL-UTA" -> 2026-01-08)
///
/// Format: The second part contains the date as "YYMMMDD" where:
/// - YY: two-digit year (e.g., "26" for 2026)
/// - MMM: three-letter month abbreviation (e.g., "JAN")
/// - DD: two-digit day (e.g., "08")
pub(super) fn extract_game_date_from_ticker(event_ticker: &str) -> Option<DateTime<Utc>> {
    let parts: Vec<&str> = event_ticker.split('-').collect();
    if parts.len() < 2 {
        return None;
    }

    let date_part = parts[1];
    if date_part.len() < 7 {
        return None;
    }

    // Parse year (first 2 characters)
    let year_str = &date_part[..2];
    let year: i32 = match year_str.parse::<i32>() {
        Ok(y) => 2000 + y,
        Err(_) => return None,
    };

    // Parse month (characters 2-5, e.g., "JAN")
    let month_str = &date_part[2..5];
    let month: u32 = match month_str.to_uppercase().as_str() {
        "JAN" => 1,
        "FEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" => 8,
        "SEP" => 9,
        "OCT" => 10,
        "NOV" => 11,
        "DEC" => 12,
        _ => return None,
    };

    // Parse day (characters 5-7)
    let day_str = &date_part[5..7];
    let day: u32 = match day_str.parse() {
        Ok(d) => d,
        Err(_) => return None,
    };

    use chrono::NaiveDate;
    let naive_date = NaiveDate::from_ymd_opt(year, month, day)?;
    let naive_datetime = naive_date.and_hms_opt(12, 0, 0)?;

    Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc))
}

impl KalshiClient {
    /// Events and markets of every `(series ticker, category)` given.
    pub async fn get_supported_events_and_markets(
        &self,
        series: &[(&str, &str)],
    ) -> Result<(Vec<KalshiEvent>, Vec<KalshiMarket>)> {
        let mut events = Vec::new();
        let mut markets = Vec::new();

        for &(series_ticker, category) in series {
            let (mut series_events, mut series_markets) = self
                .get_series_events_and_markets(series_ticker, category)
                .await?;
            events.append(&mut series_events);
            markets.append(&mut series_markets);
        }

        info!(
            "Loaded {} supported Kalshi events and {} markets",
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }

    /// Fetch only the legacy NBA feed for callers that do not use the
    /// supported-sports scan.
    #[allow(dead_code)]
    pub async fn get_nba_events_and_markets(
        &self,
    ) -> Result<(Vec<KalshiEvent>, Vec<KalshiMarket>)> {
        self.get_series_events_and_markets("KXNBAGAME", "NBA").await
    }

    pub(super) async fn get_series_events_and_markets(
        &self,
        series_ticker: &str,
        category: &str,
    ) -> Result<(Vec<KalshiEvent>, Vec<KalshiMarket>)> {
        let mut events = Vec::new();
        let mut markets = Vec::new();
        let mut cursor = None;

        loop {
            let path = {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                query.append_pair("series_ticker", series_ticker);
                query.append_pair("status", "open");
                query.append_pair("with_nested_markets", "true");
                query.append_pair("limit", "200");
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
                format!("/events?{}", query.finish())
            };
            let response = self.get(&path).await?;
            let event_array = response["events"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid events response"))?;

            for event_data in event_array {
                let event = match category {
                    "NBA" => parse_nba_event(event_data),
                    "TENNIS" => parse_tennis_event(event_data),
                    "NHL" | "MLB" => parse_team_game_event(event_data, category),
                    "CRYPTO15" => parse_crypto_window_event(event_data, series_ticker),
                    _ => None,
                };

                if let Some(event) = event {
                    markets.extend(event.markets.iter().cloned());
                    events.push(event);
                }
            }

            let next_cursor = response["cursor"].as_str().unwrap_or_default();
            if next_cursor.is_empty() {
                break;
            }
            cursor = Some(next_cursor.to_string());
        }

        info!(
            "Loaded {} Kalshi {} events and {} markets",
            category,
            events.len(),
            markets.len()
        );

        Ok((events, markets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nhl_and_mlb_games_from_market_ticker_suffixes() {
        let nhl = json!({
            "event_ticker": "KXNHLGAME-26OCT27NYINSH",
            "markets": [
                {"ticker": "KXNHLGAME-26OCT27NYINSH-NYI", "yes_ask_dollars": "0.5500"},
                {"ticker": "KXNHLGAME-26OCT27NYINSH-NSH", "yes_ask_dollars": "0.4600"}
            ]
        });
        let event = parse_team_game_event(&nhl, "NHL").unwrap();
        assert_eq!(event.name, "NHL NSH VS NYI");
        assert_eq!(event.category, "NHL");
        assert_eq!(event.start_time.unwrap().date_naive().to_string(), "2026-10-27");
        assert_eq!(event.markets[0].team_name, "NYI");
        assert_eq!(event.markets[0].opponent_name, "NSH");

        let mlb = json!({
            "event_ticker": "KXMLBGAME-26OCT262000MILTB",
            "markets": [
                {"ticker": "KXMLBGAME-26OCT262000MILTB-MIL", "yes_ask_dollars": "0.4800"},
                {"ticker": "KXMLBGAME-26OCT262000MILTB-TB", "yes_ask_dollars": "0.5300"}
            ]
        });
        let event = parse_team_game_event(&mlb, "MLB").unwrap();
        assert_eq!(event.name, "MLB MIL VS TB");
        assert_eq!(event.start_time.unwrap().date_naive().to_string(), "2026-10-26");

        let three = json!({
            "event_ticker": "KXNHLGAME-26OCT27NYINSH",
            "markets": [
                {"ticker": "KXNHLGAME-26OCT27NYINSH-NYI", "yes_ask_dollars": "0.5"},
                {"ticker": "KXNHLGAME-26OCT27NYINSH-NSH", "yes_ask_dollars": "0.5"},
                {"ticker": "KXNHLGAME-26OCT27NYINSH-TIE", "yes_ask_dollars": "0.1"}
            ]
        });
        assert!(parse_team_game_event(&three, "NHL").is_none());
    }

    #[test]
    fn parses_a_15_minute_crypto_window() {
        let event = json!({
            "event_ticker": "KXBTC15M-26OCT261300",
            "markets": [{
                "ticker": "KXBTC15M-26OCT261300-00",
                "open_time": "2026-10-26T16:45:00Z",
                "close_time": "2026-10-26T17:00:00Z",
                "strike_type": "greater_or_equal",
                "yes_ask_dollars": "0.5200",
                "rules_primary": "If the simple average of the sixty seconds of CF Benchmarks' BRTI before 1:00 PM EDT on Oct 26, 2026 is at least the simple average of the sixty seconds of CF Benchmarks' BRTI before 12:45 PM EDT on October 26, 2026, then the market resolves to Yes."
            }]
        });
        let parsed = parse_crypto_window_event(&event, "KXBTC15M").unwrap();
        assert_eq!(parsed.name, "BTC 15M 17:00Z");
        assert_eq!(parsed.category, "CRYPTO15");
        assert_eq!(parsed.markets[0].team_name, "UP");
        assert_eq!(parsed.markets[0].opponent_name, "DOWN");

        let mut other_index = event.clone();
        other_index["markets"][0]["rules_primary"] = json!("... CF Benchmarks' ETHUSDRTI ... is at least ...");
        assert!(parse_crypto_window_event(&other_index, "KXBTC15M").is_none());
        let mut hourly = event.clone();
        hourly["markets"][0]["open_time"] = json!("2026-10-26T16:00:00Z");
        assert!(parse_crypto_window_event(&hourly, "KXBTC15M").is_none());
        assert!(parse_crypto_window_event(&event, "KXXRP15M").is_none());
    }

    #[test]
    fn extracts_full_tennis_identity_from_market_title() {
        assert_eq!(
            extract_tennis_competitor_from_market_title(
                "Will Juan Manuel Cerundolo win the Cerundolo vs Auger-Aliassime: Round Of 32 match?"
            ),
            Some("JUAN MANUEL CERUNDOLO".to_string())
        );
        assert_eq!(
            extract_tennis_competitor_from_market_title(
                "Will Cerundolo win the Cerundolo vs Auger-Aliassime match?"
            ),
            None
        );
        assert_eq!(
            extract_tennis_competitor_from_market_title("Jesper De Jong wins"),
            Some("JESPER DE JONG".to_string())
        );
    }

    #[test]
    fn parses_only_two_competitor_tennis_events() {
        let event = json!({
            "event_ticker": "KXATPMATCH-26AUG18TEST",
            "markets": [
                {
                    "ticker": "KXATPMATCH-26AUG18TEST-CERUNDOLO",
                    "title": "Will Juan Manuel Cerundolo win the Cerundolo vs Auger-Aliassime match?",
                    "yes_ask": 45
                },
                {
                    "ticker": "KXATPMATCH-26AUG18TEST-AUGER",
                    "title": "Will Felix Auger-Aliassime win the Cerundolo vs Auger-Aliassime match?",
                    "yes_ask": 55
                }
            ]
        });

        let parsed = parse_tennis_event(&event).unwrap();
        assert_eq!(
            parsed.name,
            "FELIX AUGER ALIASSIME VS JUAN MANUEL CERUNDOLO"
        );
        assert_eq!(parsed.markets.len(), 2);
    }

    #[test]
    fn parses_current_tennis_market_titles_and_dollar_prices() {
        let event = json!({
            "event_ticker": "KXATPMATCH-26AUG30DEJPAS",
            "markets": [
                {
                    "ticker": "KXATPMATCH-26AUG30DEJPAS-DEJ",
                    "title": "Jesper De Jong wins",
                    "yes_sub_title": "Jesper De Jong",
                    "yes_ask_dollars": "0.4400",
                    "volume_fp": "740891.24",
                    "open_interest_fp": "404822.66"
                },
                {
                    "ticker": "KXATPMATCH-26AUG30DEJPAS-PAS",
                    "title": "Francesco Passaro wins",
                    "yes_sub_title": "Francesco Passaro",
                    "yes_ask_dollars": "0.5600"
                }
            ]
        });

        let parsed = parse_tennis_event(&event).unwrap();
        assert_eq!(parsed.name, "FRANCESCO PASSARO VS JESPER DE JONG");
        assert_eq!(parsed.markets[0].yes_price, 0.44);
        assert_eq!(parsed.markets[0].volume, Some(740891.24));
        assert_eq!(parsed.markets[0].liquidity, Some(404822.66));
    }

    #[test]
    fn rejects_current_tennis_events_with_invalid_prices() {
        let event = json!({
            "event_ticker": "KXATPMATCH-26AUG30DEJPAS",
            "markets": [
                {
                    "ticker": "KXATPMATCH-26AUG30DEJPAS-DEJ",
                    "yes_sub_title": "Jesper De Jong",
                    "yes_ask_dollars": "1.1"
                },
                {
                    "ticker": "KXATPMATCH-26AUG30DEJPAS-PAS",
                    "yes_sub_title": "Francesco Passaro",
                    "yes_ask_dollars": "0.5600"
                }
            ]
        });

        assert!(parse_tennis_event(&event).is_none());
    }
}
