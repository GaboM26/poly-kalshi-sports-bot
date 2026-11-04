//! Polymarket US market catalog: sports and series discovery, and market parsing per sport.

use super::*;

// Polymarket US sport codes for the supported tennis match-winner feeds.
// Series IDs are resolved from /v1/sports at runtime so they are not baked in.
pub(super) const TENNIS_SPORT_CODES: &[&str] = &["atp", "wta", "itfm", "itfw", "itfme", "itfwo", "atpcq"];

// (sport code, category, the only accepted `sportsMarketType`). NHL full-game
// winner includes OT/SO like Kalshi; regulation-winner and partial-game MLB
// markets settle differently and are never fetched.
pub(super) const TEAM_GAME_SPORTS: &[(&str, &str, &str)] = &[
    ("nhl", "NHL", "hockey_team_full_game_winner"),
    ("mlb", "MLB", "baseball_team_full_game_winner"),
];

pub(super) fn sport_code(sport: &Value) -> Option<&str> {
    ["sport", "code", "slug"]
        .iter()
        .find_map(|field| sport[*field].as_str())
}

pub(super) fn sport_has_code(sport: &Value, expected_code: &str) -> bool {
    ["sport", "code", "slug"]
        .iter()
        .filter_map(|field| sport[*field].as_str())
        .any(|value| value.eq_ignore_ascii_case(expected_code))
}

pub(super) fn sport_series_id(sport: &Value) -> Option<String> {
    ["series", "seriesId", "series_id"]
        .iter()
        .find_map(|field| match &sport[*field] {
            Value::String(value) if !value.is_empty() => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

pub(super) fn parse_nba_market(
    event_title: &str,
    event_date: Option<DateTime<Utc>>,
    market_data: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    let question = market_data["question"].as_str().unwrap_or(event_title);
    if question != event_title {
        return None;
    }

    let (outcomes, sides) = parse_binary_outcomes_and_market_sides(market_data, |outcome| {
        if matches!(
            outcome.trim().to_ascii_lowercase().as_str(),
            "yes" | "no" | "over" | "under"
        ) {
            return None;
        }
        let normalized = normalize_team_name(outcome);
        (!normalized.is_empty()).then_some(normalized)
    })?;
    let team_a = outcomes[0].clone();
    let team_b = outcomes[1].clone();
    if team_a == team_b {
        return None;
    }

    let event_name = if team_a < team_b {
        format!("{team_a}-{team_b}")
    } else {
        format!("{team_b}-{team_a}")
    };
    build_competitor_market(
        market_data,
        event_name,
        team_a,
        team_b,
        sides[0],
        sides[1],
        event_date,
        "NBA",
    )
}

pub(super) fn parse_tennis_match_winner_market(
    event_date: Option<DateTime<Utc>>,
    market_data: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    if market_data["sportsMarketType"].as_str() != Some("tennis_match_winner")
        || market_data["sportsMarketTypeV2"].as_str() != Some("SPORTS_MARKET_TYPE_MONEYLINE")
    {
        return None;
    }

    let (outcomes, sides) =
        parse_binary_outcomes_and_market_sides(market_data, normalize_competitor_name)?;
    let competitor_a = outcomes[0].clone();
    let competitor_b = outcomes[1].clone();
    let event_name = competitor_event_name(&competitor_a, &competitor_b)?;

    build_competitor_market(
        market_data,
        event_name,
        competitor_a,
        competitor_b,
        sides[0],
        sides[1],
        event_date,
        "TENNIS",
    )
}

/// NHL/MLB full-game moneyline. Teams come from
/// `marketSides[].team.abbreviation` (mapped to Kalshi codes); each side's
/// own `long` flag gives its native position, never the outcome order.
pub(super) fn parse_team_game_market(
    category: &str,
    market_type: &str,
    event_date: Option<DateTime<Utc>>,
    market_data: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    if market_data["sportsMarketType"].as_str() != Some(market_type)
        || market_data["sportsMarketTypeV2"].as_str() != Some("SPORTS_MARKET_TYPE_MONEYLINE")
    {
        return None;
    }

    let market_sides = market_data["marketSides"].as_array()?;
    if market_sides.len() != 2 {
        return None;
    }
    let mut parsed = Vec::with_capacity(2);
    for market_side in market_sides {
        let team = crate::core::canonical_team_code(
            category,
            market_side["team"]["abbreviation"].as_str()?,
        )?;
        let quote = market_side["quote"]["value"]
            .as_str()
            .and_then(|price| price.parse::<f64>().ok())
            .or_else(|| market_side["quote"]["value"].as_f64())?;
        if !quote.is_finite() || !(0.01..=0.99).contains(&quote) {
            return None;
        }
        let position_side = if market_side["long"].as_bool()? {
            PolymarketPositionSide::Long
        } else {
            PolymarketPositionSide::Short
        };
        parsed.push((team, MarketSide { quote, position_side }));
    }
    // Exactly one LONG and one SHORT side, for two different teams.
    if parsed[0].0 == parsed[1].0 || parsed[0].1.position_side == parsed[1].1.position_side {
        return None;
    }

    let event_name = crate::core::team_game_event_name(category, &parsed[0].0, &parsed[1].0)?;
    build_competitor_market(
        market_data,
        event_name,
        parsed[0].0.clone(),
        parsed[1].0.clone(),
        parsed[0].1,
        parsed[1].1,
        event_date,
        category,
    )
}

/// One 15-minute crypto window event: a single Yes(LONG)=Up / No(SHORT)=Down
/// market whose description names the series' index and the ">=" rule.
pub(super) fn parse_crypto_window_market(
    series: &crate::core::crypto_windows::CryptoWindowSeries,
    api_event: &Value,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    use crate::core::crypto_windows::{self, DOWN, UP};

    let time = |field: &str| {
        api_event[field]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
    };
    let (start, end) = (time("startDate")?, time("endDate")?);
    if !crypto_windows::is_window(start, end) {
        return None;
    }
    let [market_data] = api_event["markets"].as_array()?.as_slice() else {
        return None;
    };
    let description = market_data["description"].as_str()?;
    if !description.contains(series.polymarket_index)
        || !description.contains("settle to Up if")
        || !description.contains("greater than or equal to")
    {
        return None;
    }

    let market_sides = market_data["marketSides"].as_array()?;
    if market_sides.len() != 2 {
        return None;
    }
    let mut up = None;
    let mut down = None;
    for market_side in market_sides {
        let quote = market_side["quote"]["value"]
            .as_str()
            .and_then(|price| price.parse::<f64>().ok())
            .or_else(|| market_side["quote"]["value"].as_f64())
            .filter(|quote| quote.is_finite() && (0.01..=0.99).contains(quote));
        match (market_side["description"].as_str()?, market_side["long"].as_bool()?) {
            ("Yes", true) if up.is_none() => up = Some(quote),
            ("No", false) if down.is_none() => down = Some(quote),
            _ => return None,
        }
    }
    // A side near 0/1 can lack a quote; the live feed fills it in later.
    let (up_quote, down_quote) = match (up?, down?) {
        (Some(up), Some(down)) => (up, down),
        (Some(up), None) => (up, 1.0 - up),
        (None, Some(down)) => (1.0 - down, down),
        (None, None) => return None,
    };

    build_competitor_market(
        market_data,
        crypto_windows::crypto_window_event_name(series.coin, end),
        UP.to_string(),
        DOWN.to_string(),
        MarketSide {
            quote: up_quote,
            position_side: PolymarketPositionSide::Long,
        },
        MarketSide {
            quote: down_quote,
            position_side: PolymarketPositionSide::Short,
        },
        Some(end),
        crypto_windows::CRYPTO_WINDOW_CATEGORY,
    )
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MarketSide {
    quote: f64,
    position_side: PolymarketPositionSide,
}

/// Parse a binary market and associate every competitor with its own gateway
/// market side. The outcome array supplies competitor names only; it never
/// determines a LONG or SHORT position.
pub(super) fn parse_binary_outcomes_and_market_sides<F>(
    market_data: &Value,
    normalizer: F,
) -> Option<([String; 2], [MarketSide; 2])>
where
    F: Fn(&str) -> Option<String>,
{
    let outcomes = parse_json_array(&market_data["outcomes"])?
        .iter()
        .map(|value| value.as_str().and_then(|name| normalizer(name)))
        .collect::<Option<Vec<_>>>()?;
    let outcomes: [String; 2] = outcomes.try_into().ok()?;
    if outcomes[0] == outcomes[1] {
        return None;
    }

    let market_sides = market_data["marketSides"].as_array()?;
    let mut outcome_sides: [Option<MarketSide>; 2] = [None, None];
    for market_side in market_sides {
        let description = market_side["description"].as_str()?;
        let Some(competitor) = normalizer(description) else {
            continue;
        };
        let outcome_index = if competitor == outcomes[0] {
            0
        } else if competitor == outcomes[1] {
            1
        } else {
            continue;
        };

        let quote = market_side["quote"]["value"]
            .as_str()
            .and_then(|price| price.parse::<f64>().ok())
            .or_else(|| market_side["quote"]["value"].as_f64())?;
        if !quote.is_finite() || !(0.01..=0.99).contains(&quote) {
            return None;
        }
        let position_side = if market_side["long"].as_bool()? {
            PolymarketPositionSide::Long
        } else {
            PolymarketPositionSide::Short
        };

        // A competitor must map to exactly one side. Continuing would make
        // execution ambiguous and could choose the wrong position.
        if outcome_sides[outcome_index].is_some() {
            return None;
        }
        outcome_sides[outcome_index] = Some(MarketSide {
            quote,
            position_side,
        });
    }

    Some((outcomes, [outcome_sides[0]?, outcome_sides[1]?]))
}

pub(super) fn parse_json_array(value: &Value) -> Option<Vec<Value>> {
    if let Some(serialized) = value.as_str() {
        serde_json::from_str(serialized).ok()
    } else {
        value.as_array().cloned()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_competitor_market(
    market_data: &Value,
    event_name: String,
    first_competitor: String,
    second_competitor: String,
    first_side: MarketSide,
    second_side: MarketSide,
    start_time: Option<DateTime<Utc>>,
    category: &str,
) -> Option<(PolymarketEvent, PolymarketMarket)> {
    if first_competitor == second_competitor {
        return None;
    }

    let market_id = market_data["id"].as_str().unwrap_or("");
    let condition_id = market_data["conditionId"]
        .as_str()
        .or_else(|| market_data["condition_id"].as_str())
        .unwrap_or(market_id)
        .to_string();
    if condition_id.is_empty() {
        return None;
    }

    let market_slug = market_data["slug"].as_str()?.trim();
    if market_slug.is_empty() {
        return None;
    }

    let volume = market_data["volume"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| market_data["volume"].as_f64());

    let (team_a, team_b, side_a, side_b) = if first_competitor < second_competitor {
        (first_competitor, second_competitor, first_side, second_side)
    } else {
        (second_competitor, first_competitor, second_side, first_side)
    };

    let market = PolymarketMarket {
        market_id: condition_id.clone(),
        market_slug: market_slug.to_string(),
        event_name: event_name.clone(),
        team_a: team_a.clone(),
        team_b: team_b.clone(),
        price_a: side_a.quote,
        price_b: side_b.quote,
        team_a_position: side_a.position_side,
        team_b_position: side_b.position_side,
        start_time,
        volume,
        live: false,
        period: None,
    };
    let event = PolymarketEvent {
        event_id: condition_id,
        name: event_name,
        team_a,
        team_b,
        start_time,
        category: category.to_string(),
        market: Some(market.clone()),
    };

    Some((event, market))
}

/// Extract date from slug (e.g., "lakers-vs-grizzlies-2026-01-07" -> 2026-01-07)
pub(super) fn extract_date_from_slug(slug: &str) -> Option<DateTime<Utc>> {
    let parts: Vec<&str> = slug.split('-').collect();
    if parts.len() >= 3 {
        let year_str = parts[parts.len() - 3];
        let month_str = parts[parts.len() - 2];
        let day_str = parts[parts.len() - 1];

        if let (Ok(year), Ok(month), Ok(day)) = (
            year_str.parse::<i32>(),
            month_str.parse::<u32>(),
            day_str.parse::<u32>(),
        ) {
            use chrono::NaiveDate;
            if let Some(naive_date) = NaiveDate::from_ymd_opt(year, month, day) {
                let naive_datetime = naive_date.and_hms_opt(12, 0, 0)?;
                return Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc));
            }
        }
    }
    None
}

impl PolymarketClient {
    /// Get all supported NBA and tennis match-winner events and markets.
    pub async fn get_supported_events_and_markets(
        &self,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let sports = self.get_sports().await?;
        let mut events = Vec::new();
        let mut markets = Vec::new();
        let mut fetched_series = HashSet::new();

        let nba_sport = sports
            .iter()
            .find(|sport| {
                let name = sport_code(sport).unwrap_or_default().to_ascii_uppercase();
                name.contains("NBA") && !name.contains("WNBA")
            })
            .ok_or_else(|| anyhow::anyhow!("NBA league not found"))?;
        let nba_series =
            sport_series_id(nba_sport).ok_or_else(|| anyhow::anyhow!("NBA series_id not found"))?;
        self.append_series_events_and_markets(
            &nba_series,
            "NBA",
            None,
            &mut fetched_series,
            &mut events,
            &mut markets,
        )
        .await?;

        for &(sport_code_name, category, market_type) in TEAM_GAME_SPORTS {
            let Some(series_id) = sports
                .iter()
                .find(|sport| sport_has_code(sport, sport_code_name))
                .and_then(sport_series_id)
            else {
                warn!(
                    "Polymarket US sport code {} is unavailable; skipping it",
                    sport_code_name
                );
                continue;
            };
            // One sport failing (the gateway times out regularly) must not
            // drop NBA and tennis from the scan.
            if let Err(error) = self
                .append_series_events_and_markets(
                    &series_id,
                    category,
                    Some(market_type),
                    &mut fetched_series,
                    &mut events,
                    &mut markets,
                )
                .await
            {
                warn!("Polymarket US {} events unavailable: {:#}", category, error);
            }
        }

        for sport_code_name in TENNIS_SPORT_CODES {
            let Some(sport) = sports
                .iter()
                .find(|sport| sport_has_code(sport, sport_code_name))
            else {
                warn!(
                    "Polymarket US tennis sport code {} is unavailable; skipping it",
                    sport_code_name
                );
                continue;
            };

            let Some(series_id) = sport_series_id(sport) else {
                warn!(
                    "Polymarket US tennis sport code {} has no series ID; skipping it",
                    sport_code_name
                );
                continue;
            };

            self.append_series_events_and_markets(
                &series_id,
                "TENNIS",
                None,
                &mut fetched_series,
                &mut events,
                &mut markets,
            )
            .await?;
        }

        for series in crate::core::crypto_windows::CRYPTO_WINDOW_SERIES {
            match self.get_crypto_window_events_and_markets(series).await {
                Ok((mut series_events, mut series_markets)) => {
                    events.append(&mut series_events);
                    markets.append(&mut series_markets);
                }
                Err(error) => warn!(
                    "Polymarket US {} events unavailable: {:#}",
                    series.polymarket_series_slug, error
                ),
            }
        }

        info!(
            "✅ Polymarket supported sports: {} events, {} markets",
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
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let sports = self.get_sports().await?;
        let nba_sport = sports
            .iter()
            .find(|sport| {
                let name = sport_code(sport).unwrap_or_default().to_ascii_uppercase();
                name.contains("NBA") && !name.contains("WNBA")
            })
            .ok_or_else(|| anyhow::anyhow!("NBA league not found"))?;
        let nba_series =
            sport_series_id(nba_sport).ok_or_else(|| anyhow::anyhow!("NBA series_id not found"))?;
        self.get_series_events_and_markets(&nba_series, "NBA", None).await
    }

    /// Open windows of one 15-minute crypto series. The series is not in
    /// /v1/sports, so its ID is resolved from its slug.
    pub(super) async fn get_crypto_window_events_and_markets(
        &self,
        series: &crate::core::crypto_windows::CryptoWindowSeries,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let series_url = format!(
            "{}/v1/series?slug={}",
            self.config.base_url, series.polymarket_series_slug
        );
        let response = self.http.get(&series_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to get series: {}", response.status());
        }
        let body = response.json::<Value>().await?;
        let series_id = body["series"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["slug"].as_str() == Some(series.polymarket_series_slug))
            .and_then(|entry| entry["id"].as_str().map(str::to_string))
            .ok_or_else(|| anyhow::anyhow!("series ID not found"))?;

        let events_url = format!(
            "{}/v1/events?seriesId={}&active=true&closed=false&limit=100",
            self.config.base_url, series_id
        );
        let response = self.http.get(&events_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to get events: {}", response.status());
        }
        let body = response.json::<Value>().await?;
        let mut events = Vec::new();
        let mut markets = Vec::new();
        for api_event in body["events"].as_array().into_iter().flatten() {
            if let Some((event, market)) = parse_crypto_window_market(series, api_event) {
                events.push(event);
                markets.push(market);
            }
        }
        Ok((events, markets))
    }

    pub(super) async fn get_sports(&self) -> Result<Vec<Value>> {
        let sports_url = format!("{}/v1/sports", self.config.base_url);
        let response = self.http.get(&sports_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to get sports leagues: {}", response.status());
        }

        response
            .json::<Value>()
            .await?
            .get("sports")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Polymarket US sports response is missing sports"))
    }

    pub(super) async fn append_series_events_and_markets(
        &self,
        series_id: &str,
        category: &str,
        market_type: Option<&str>,
        fetched_series: &mut HashSet<String>,
        events: &mut Vec<PolymarketEvent>,
        markets: &mut Vec<PolymarketMarket>,
    ) -> Result<()> {
        if !fetched_series.insert(series_id.to_string()) {
            return Ok(());
        }

        let (mut series_events, mut series_markets) = self
            .get_series_events_and_markets(series_id, category, market_type)
            .await?;
        events.append(&mut series_events);
        markets.append(&mut series_markets);
        Ok(())
    }

    pub(super) async fn get_series_events_and_markets(
        &self,
        series_id: &str,
        category: &str,
        market_type: Option<&str>,
    ) -> Result<(Vec<PolymarketEvent>, Vec<PolymarketMarket>)> {
        let mut events_url = format!(
            "{}/v1/events?seriesId={}&active=true&closed=false&limit=100",
            self.config.base_url, series_id
        );
        // Unfiltered NHL is ~22 MB per request; fetch only the moneyline.
        if let Some(market_type) = market_type {
            events_url.push_str("&sportsMarketTypes=");
            events_url.push_str(market_type);
        }
        let response = self.http.get(&events_url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to get Polymarket {} events: {}",
                category,
                response.status()
            );
        }

        let api_events: Vec<Value> = response
            .json::<Value>()
            .await?
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Polymarket US events response is missing events"))?;
        debug!(
            "📥 Retrieved {} Polymarket {} events",
            api_events.len(),
            category
        );

        let mut events = Vec::new();
        let mut markets = Vec::new();
        for api_event in &api_events {
            let event_title = api_event["title"].as_str().unwrap_or("");
            let event_date = extract_date_from_slug(api_event["slug"].as_str().unwrap_or(""));

            for market_data in api_event["markets"].as_array().into_iter().flatten() {
                let parsed = match category {
                    "NBA" => parse_nba_market(event_title, event_date, market_data),
                    "TENNIS" => parse_tennis_match_winner_market(event_date, market_data),
                    "NHL" | "MLB" => market_type.and_then(|market_type| {
                        parse_team_game_market(category, market_type, event_date, market_data)
                    }),
                    _ => None,
                };
                if let Some((mut event, mut market)) = parsed {
                    // In-play state lives on the event, not the market.
                    let live = api_event["live"].as_bool().unwrap_or(false);
                    let period = api_event["period"]
                        .as_str()
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(str::to_string);
                    for m in event.market.iter_mut().chain(std::iter::once(&mut market)) {
                        m.live = live;
                        m.period = period.clone();
                    }
                    events.push(event);
                    markets.push(market);
                }
            }
        }

        Ok((events, markets))
    }

    // ========================================================================
    // ORDER PLACEMENT API (direct, Ed25519-signed against api.polymarket.us)
    // ========================================================================
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_tennis_match_winner_outcomes() {
        let market = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-cerundolo-auger-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Juan Manuel Cerundolo\", \"Felix Auger-Aliassime\"]",
            "marketSides": [
                {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}},
                {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}}
            ]
        });

        let (event, parsed_market) = parse_tennis_match_winner_market(None, &market).unwrap();
        assert_eq!(event.name, "FELIX AUGER ALIASSIME VS JUAN MANUEL CERUNDOLO");
        assert_eq!(parsed_market.team_a, "FELIX AUGER ALIASSIME");
        assert_eq!(parsed_market.price_a, 0.56);
        assert_eq!(parsed_market.market_slug, "atp-cerundolo-auger-2026-08-17");
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Felix Auger-Aliassime")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Long
        );
    }

    #[test]
    fn outcome_order_does_not_determine_position_side() {
        let market = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-auger-cerundolo-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Felix Auger-Aliassime\", \"Juan Manuel Cerundolo\"]",
            "marketSides": [
                {"description": "Juan Manuel Cerundolo", "long": true, "quote": {"value": "0.46"}},
                {"description": "Felix Auger-Aliassime", "long": false, "quote": {"value": "0.56"}}
            ]
        });

        let (_, parsed_market) = parse_tennis_match_winner_market(None, &market).unwrap();
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Felix Auger-Aliassime")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Short
        );
        assert_eq!(
            parsed_market
                .us_execution_for_competitor("Juan Manuel Cerundolo")
                .unwrap()
                .position_side,
            PolymarketPositionSide::Long
        );
    }

    #[test]
    fn rejects_missing_ambiguous_or_slugless_side_mappings() {
        let base = json!({
            "id": "market-id",
            "conditionId": "condition-id",
            "slug": "atp-auger-cerundolo-2026-08-17",
            "sportsMarketType": "tennis_match_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Felix Auger-Aliassime\", \"Juan Manuel Cerundolo\"]",
            "marketSides": [
                {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}},
                {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}}
            ]
        });

        let mut missing_slug = base.clone();
        missing_slug.as_object_mut().unwrap().remove("slug");
        assert!(parse_tennis_match_winner_market(None, &missing_slug).is_none());

        let mut missing_side = base.clone();
        missing_side["marketSides"] = json!([
            {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}}
        ]);
        assert!(parse_tennis_match_winner_market(None, &missing_side).is_none());

        let mut ambiguous = base;
        ambiguous["marketSides"] = json!([
            {"description": "Felix Auger-Aliassime", "long": true, "quote": {"value": "0.56"}},
            {"description": "Felix Auger-Aliassime", "long": false, "quote": {"value": "0.56"}},
            {"description": "Juan Manuel Cerundolo", "long": false, "quote": {"value": "0.46"}}
        ]);
        assert!(parse_tennis_match_winner_market(None, &ambiguous).is_none());
    }

    #[test]
    fn parses_nhl_moneyline_from_team_abbreviations() {
        let market = json!({
            "id": "1669874",
            "slug": "aec-nhl-mon-nas-2026-10-27",
            "sportsMarketType": "hockey_team_full_game_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_MONEYLINE",
            "outcomes": "[\"Canadiens\",\"Predators\"]",
            "marketSides": [
                {"description": "Canadiens", "long": true, "quote": {"value": "0.4500"}, "team": {"abbreviation": "mon"}},
                {"description": "Predators", "long": false, "quote": {"value": "0.56"}, "team": {"abbreviation": "nas"}}
            ]
        });
        let (event, parsed) = parse_team_game_market(
            "NHL",
            "hockey_team_full_game_winner",
            None,
            &market,
        )
        .unwrap();
        assert_eq!(event.name, "NHL MTL VS NSH");
        assert_eq!(event.category, "NHL");
        assert_eq!(parsed.team_a, "MTL");
        assert_eq!(parsed.team_a_position, PolymarketPositionSide::Long);
        assert_eq!(parsed.team_b, "NSH");
        assert_eq!(parsed.team_b_position, PolymarketPositionSide::Short);
        assert!((parsed.price_b - 0.56).abs() < 1e-9);

        // Regulation-winner (no OT/SO) settles differently: never accepted.
        let mut regulation = market.clone();
        regulation["sportsMarketType"] = json!("hockey_team_regulation_winner");
        assert!(parse_team_game_market("NHL", "hockey_team_full_game_winner", None, &regulation).is_none());

        // Two LONG sides would make the position ambiguous.
        let mut both_long = market.clone();
        both_long["marketSides"][1]["long"] = json!(true);
        assert!(parse_team_game_market("NHL", "hockey_team_full_game_winner", None, &both_long).is_none());
    }

    #[test]
    fn parses_a_15_minute_crypto_window() {
        let series = &crate::core::crypto_windows::CRYPTO_WINDOW_SERIES[0];
        let event = json!({
            "startDate": "2026-10-26T16:45:00Z",
            "endDate": "2026-10-26T17:00:00Z",
            "markets": [{
                "id": "1",
                "slug": "cpc-btc-updown-15m-2026-10-26-1645z",
                "description": "This market will settle to Up if the Bitcoin price at 1:00PM ET on Oct 26, 2026 is greater than or equal to the price at 12:45PM ET on Oct 26, 2026. Otherwise, this market will settle to Down. Outcome sourced from CF Benchmarks' Bitcoin Real-Time Index (BRTI).",
                "marketSides": [
                    {"description": "Yes", "long": true, "quote": null},
                    {"description": "No", "long": false, "quote": {"value": "0.4500"}}
                ]
            }]
        });
        let (parsed_event, market) = parse_crypto_window_market(series, &event).unwrap();
        assert_eq!(parsed_event.name, "BTC 15M 17:00Z");
        assert_eq!(parsed_event.category, "CRYPTO15");
        // team_a/team_b are sorted: DOWN, UP.
        assert_eq!(market.team_a, "DOWN");
        assert_eq!(market.team_a_position, PolymarketPositionSide::Short);
        assert_eq!(market.team_b, "UP");
        assert_eq!(market.team_b_position, PolymarketPositionSide::Long);
        assert!((market.price_b - 0.55).abs() < 1e-9);

        let mut eth = event.clone();
        eth["markets"][0]["description"] = json!("settle to Up if ... greater than or equal to ... (ETHUSD_RTI)");
        assert!(parse_crypto_window_market(series, &eth).is_none());
        let mut swapped = event.clone();
        swapped["markets"][0]["marketSides"][0]["long"] = json!(false);
        assert!(parse_crypto_window_market(series, &swapped).is_none());
    }

    #[test]
    fn rejects_non_moneyline_tennis_markets() {
        let market = json!({
            "sportsMarketType": "tennis_tournament_winner",
            "sportsMarketTypeV2": "SPORTS_MARKET_TYPE_FUTURES",
            "outcomes": "[\"Juan Manuel Cerundolo\", \"Felix Auger-Aliassime\"]",
            "outcomePrices": "[\"0.45\", \"0.55\"]"
        });

        assert!(parse_tennis_match_winner_market(None, &market).is_none());
    }
}
