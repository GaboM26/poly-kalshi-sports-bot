//! Team identity for NHL and MLB moneylines, joined on team abbreviations.
//!
//! Kalshi codes (market ticker suffix) are canonical. Polymarket US
//! `marketSides[].team.abbreviation` codes are mapped onto them; every MLB
//! code already agrees, NHL differs for four teams.

/// Polymarket US NHL abbreviation -> Kalshi NHL abbreviation.
const NHL_POLYMARKET_TO_KALSHI: &[(&str, &str)] = &[
    ("MON", "MTL"),
    ("NAS", "NSH"),
    ("VEG", "VGK"),
    ("WAS", "WSH"),
];

/// Canonical (Kalshi) team code for an NHL/MLB abbreviation from either
/// exchange. Rejects anything that is not a plain 2-3 letter code.
pub fn canonical_team_code(category: &str, raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_uppercase();
    if !(2..=3).contains(&code.len()) || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mapped = match category {
        "NHL" => NHL_POLYMARKET_TO_KALSHI
            .iter()
            .find(|(poly, _)| *poly == code)
            .map(|(_, kalshi)| kalshi.to_string()),
        "MLB" => None,
        _ => return None,
    };
    Some(mapped.unwrap_or(code))
}

/// Order-independent event name. The sport prefix keeps market keys from
/// colliding with another league's game between same-coded teams that day.
pub fn team_game_event_name(category: &str, first: &str, second: &str) -> Option<String> {
    if first == second {
        return None;
    }
    let (a, b) = if first < second { (first, second) } else { (second, first) };
    Some(format!("{category} {a} VS {b}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_polymarket_nhl_codes_onto_kalshi_codes() {
        assert_eq!(canonical_team_code("NHL", "mon").as_deref(), Some("MTL"));
        assert_eq!(canonical_team_code("NHL", "NAS").as_deref(), Some("NSH"));
        assert_eq!(canonical_team_code("NHL", "veg").as_deref(), Some("VGK"));
        assert_eq!(canonical_team_code("NHL", "WAS").as_deref(), Some("WSH"));
        assert_eq!(canonical_team_code("NHL", "la").as_deref(), Some("LA"));
        assert_eq!(canonical_team_code("MLB", "tb").as_deref(), Some("TB"));
    }

    #[test]
    fn rejects_malformed_codes_and_unknown_sports() {
        assert_eq!(canonical_team_code("NHL", "L"), None);
        assert_eq!(canonical_team_code("NHL", "LAKE"), None);
        assert_eq!(canonical_team_code("NHL", "L1"), None);
        assert_eq!(canonical_team_code("NBA", "LAL"), None);
    }

    #[test]
    fn event_name_is_order_independent_and_sport_scoped() {
        assert_eq!(
            team_game_event_name("NHL", "NYR", "LA").as_deref(),
            Some("NHL LA VS NYR")
        );
        assert_eq!(team_game_event_name("NHL", "LA", "NYR"), team_game_event_name("NHL", "NYR", "LA"));
        assert_eq!(team_game_event_name("MLB", "TB", "TB"), None);
    }
}
