//! Kalshi REST market data: batched quotes, the REST order book veto, single-market reads.

use super::*;

pub(super) const KALSHI_MARKET_QUOTE_BATCH_SIZE: usize = 100;

/// Authoritative price-only quote from Kalshi's REST market endpoint.
///
/// This intentionally excludes quoted sizes: REST prices must not be used to
/// fabricate order-book depth for trading safeguards.
#[derive(Debug, Clone)]
pub struct KalshiMarketQuote {
    pub market_id: String,
    pub yes_ask: f64,
    pub no_ask: f64,
    /// Kalshi's own market lifecycle status: "unopened", "open", "paused",
    /// "closed", or "settled". Used to detect a closed/settled market
    /// immediately rather than waiting on the extreme-price heuristic in
    /// `market_lifecycle.rs`, which can miss a market that closes without
    /// its price ever drifting to an extreme value.
    pub status: String,
}

impl KalshiMarketQuote {
    /// A marketable limit price (cents) for a manual, user-initiated,
    /// known-quantity order on a ticker we hold no WebSocket book for (e.g.
    /// a position whose market has fallen out of the scanner's actively
    /// tracked/subscribed set). REST only carries asks, so a sell price is
    /// approximated as the complement of the opposite side's ask - the same
    /// yes/no complement convention already used throughout this codebase
    /// (e.g. `OrderBook::limit_price_for_order`,
    /// `paired_execution::kalshi_buy_levels`). This is a price-only
    /// fallback for a single manual order with an already-known contract
    /// count; it must never be used to size or feed depth for auto-trade,
    /// which stays exclusively on the fresh WebSocket book.
    pub fn limit_price_for_order(&self, side: &str, action: &str) -> Option<i32> {
        let yes_ask_cents = (self.yes_ask * 100.0).round() as i32;
        let no_ask_cents = (self.no_ask * 100.0).round() as i32;
        match (action, side) {
            ("buy", "yes") => Some(yes_ask_cents),
            ("buy", "no") => Some(no_ask_cents),
            ("sell", "yes") => 100_i32.checked_sub(no_ask_cents),
            ("sell", "no") => 100_i32.checked_sub(yes_ask_cents),
            _ => None,
        }
    }
}

/// Parse a Kalshi fixed-point dollar quote without accepting lossy or
/// out-of-range external values.
pub(super) fn parse_quote_dollars(value: &Value, field: &str) -> Result<f64> {
    let raw = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Kalshi {field} must be a fixed-point string"))?;
    let (whole, fractional) = raw.split_once('.').unwrap_or((raw, ""));

    if whole.is_empty()
        || fractional.len() > 6
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("Invalid Kalshi {field} fixed-point quote: {raw}");
    }

    let price = raw
        .parse::<f64>()
        .with_context(|| format!("Invalid Kalshi {field} quote: {raw}"))?;
    if !price.is_finite() || !(0.0..=1.0).contains(&price) {
        bail!("Kalshi {field} quote is outside [0, 1]: {raw}");
    }

    Ok(price)
}

pub(super) fn parse_market_rest_quote(market: &Value) -> Result<KalshiMarketQuote> {
    let market_id = market["ticker"]
        .as_str()
        .filter(|ticker| !ticker.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Kalshi market quote is missing ticker"))?
        .to_string();
    // Asks round UP to the cent, matching the floored WS book bids (a 9.7c
    // crypto ask is 10c here), so REST never shows a better price than the
    // whole-cent limit we could actually send.
    let ask_cents_up = |price: f64| ((price * 100.0) - 1e-9).ceil() / 100.0;
    let yes_ask = parse_quote_dollars(&market["yes_ask_dollars"], "yes_ask_dollars")
        .map(ask_cents_up)
        .with_context(|| format!("Invalid Kalshi REST quote for {market_id}"))?;
    let no_ask = parse_quote_dollars(&market["no_ask_dollars"], "no_ask_dollars")
        .map(ask_cents_up)
        .with_context(|| format!("Invalid Kalshi REST quote for {market_id}"))?;
    let status = market["status"].as_str().unwrap_or("").to_string();

    Ok(KalshiMarketQuote {
        market_id,
        yes_ask,
        no_ask,
        status,
    })
}

impl KalshiClient {
    /// Fetch authoritative current ask quotes for the requested market tickers.
    ///
    /// Kalshi supports a comma-separated `tickers` filter. Requests are
    /// bounded so a large matched-market set cannot create an oversized URL.
    /// Sizes are deliberately not returned because only the WebSocket
    /// order-book cache is allowed to satisfy depth checks.
    pub async fn get_market_quotes(&self, tickers: &[String]) -> Result<Vec<KalshiMarketQuote>> {
        let mut seen = HashSet::new();
        let unique_tickers: Vec<String> = tickers
            .iter()
            .filter(|ticker| seen.insert((*ticker).clone()))
            .cloned()
            .collect();
        let mut quotes = Vec::with_capacity(unique_tickers.len());

        for ticker_batch in unique_tickers.chunks(KALSHI_MARKET_QUOTE_BATCH_SIZE) {
            let path = {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                query.append_pair("tickers", &ticker_batch.join(","));
                query.append_pair("limit", &ticker_batch.len().to_string());
                format!("/markets?{}", query.finish())
            };
            let response = self.get(&path).await?;
            let markets = response["markets"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid Kalshi markets quote response"))?;

            for market in markets {
                quotes.push(parse_market_rest_quote(market)?);
            }
        }

        Ok(quotes)
    }

    /// Kalshi's own REST order book for one ticker (ascending levels, best
    /// last, like the WebSocket book). Used only as an independent veto
    /// against a stale WebSocket book, never as sizing or execution depth.
    pub async fn get_rest_orderbook(&self, ticker: &str) -> Result<OrderBook> {
        let response = self.get(&format!("/markets/{ticker}/orderbook")).await?;
        let book = response
            .get("orderbook_fp")
            .ok_or_else(|| anyhow::anyhow!("Kalshi REST order book has no orderbook_fp"))?;
        Ok(OrderBook::from_exact(
            parse_price_levels(book, "yes_dollars", "yes"),
            parse_price_levels(book, "no_dollars", "no"),
        ))
    }

    pub async fn get_market(&self, ticker: &str) -> Result<Value> {
        let escaped = url::form_urlencoded::byte_serialize(ticker.as_bytes()).collect::<String>();
        self.get(&format!("/markets/{escaped}")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(yes_ask: f64, no_ask: f64) -> KalshiMarketQuote {
        KalshiMarketQuote {
            market_id: "KXLAL".to_string(),
            yes_ask,
            no_ask,
            status: "open".to_string(),
        }
    }

    #[test]
    fn rest_quote_buy_prices_use_the_direct_ask() {
        let q = quote(0.45, 0.58);
        assert_eq!(q.limit_price_for_order("yes", "buy"), Some(45));
        assert_eq!(q.limit_price_for_order("no", "buy"), Some(58));
    }

    #[test]
    fn rest_quote_sell_prices_use_the_opposite_asks_complement() {
        let q = quote(0.45, 0.58);
        // Selling YES needs a YES bid, approximated as 100 - no_ask.
        assert_eq!(q.limit_price_for_order("yes", "sell"), Some(42));
        assert_eq!(q.limit_price_for_order("no", "sell"), Some(55));
    }

    #[test]
    fn rest_quote_rejects_an_unknown_side_or_action() {
        let q = quote(0.45, 0.58);
        assert_eq!(q.limit_price_for_order("yes", "hold"), None);
        assert_eq!(q.limit_price_for_order("maybe", "sell"), None);
    }

    #[test]
    fn parses_rest_quote_asks_as_dollar_prices() {
        let quote = parse_market_rest_quote(&json!({
            "ticker": "KXATPMATCH-26AUG18CERAUG-CER",
            "yes_bid_dollars": "0.2500",
            "yes_ask_dollars": "0.2600",
            "no_bid_dollars": "0.7400",
            "no_ask_dollars": "0.7500"
        }))
        .unwrap();

        assert_eq!(quote.market_id, "KXATPMATCH-26AUG18CERAUG-CER");
        assert_eq!(quote.yes_ask, 0.26);
        assert_eq!(quote.no_ask, 0.75);
    }

    #[test]
    fn rejects_invalid_rest_quote_price() {
        let error = parse_market_rest_quote(&json!({
            "ticker": "KXATPMATCH-26AUG18CERAUG-CER",
            "yes_ask_dollars": "1.0000001",
            "no_ask_dollars": "0.7500"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("Invalid Kalshi REST quote"));
    }
}
