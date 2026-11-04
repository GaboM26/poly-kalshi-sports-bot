//! Matched-market registry: the market list, lookups, and the ticker/slug sets the feeds subscribe to.

use super::*;

impl WebSocketManager {
    /// Return the currently matched Kalshi tickers for the REST quote poller.
    pub fn get_kalshi_quote_tickers(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.matched_markets
            .read()
            .iter()
            .filter_map(|market| {
                let ticker = &market.kalshi_market.market_id;
                seen.insert(ticker.clone()).then(|| ticker.clone())
            })
            .collect()
    }

    /// Set matched markets and build lookup tables
    pub fn set_matched_markets(&self, markets: Vec<MatchedMarket>) {
        let matcher = EventMatcher::new(24);
        let sub_info = matcher.get_subscription_info(&markets);

        *self.matched_markets.write() = markets;
        *self.market_lookup.write() = sub_info.market_lookup;
        self.kalshi_rest_quotes.write().clear();
        *self.kalshi_rest_connected.write() = false;
        *self.kalshi_rest_last_success.write() = None;
        self.poly_rest_quotes.write().clear();
        *self.polymarket_rest_connected.write() = false;
        *self.polymarket_rest_last_success.write() = None;

        info!(
            "WebSocket 管理器已配置 {} 个匹配的市场",
            self.matched_markets.read().len()
        );
    }

    /// The matched market carrying a Kalshi ticker, if any.
    pub fn matched_market_for_kalshi_ticker(&self, ticker: &str) -> Option<MatchedMarket> {
        self.matched_markets
            .read()
            .iter()
            .find(|m| m.kalshi_market.market_id == ticker)
            .cloned()
    }

    /// Unique native slugs of every matched Polymarket market, for the
    /// WebSocket subscription.
    pub fn get_polymarket_slugs(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.matched_markets
            .read()
            .iter()
            .map(|m| m.polymarket_market.market_slug.clone())
            .filter(|slug| seen.insert(slug.clone()))
            .collect()
    }

    /// Merge newly discovered markets and rebuild the Kalshi subscription lookup.
    ///
    /// Polymarket US market-side IDs are not subscriptions. Fresh native slug
    /// and long/short mappings replace the previous market atomically.
    pub fn add_matched_markets(
        &self,
        new_markets: Vec<MatchedMarket>,
        _new_lookup: std::collections::HashMap<String, Vec<usize>>,
    ) -> usize {
        if new_markets.is_empty() {
            return 0;
        }

        let mut added = 0;
        {
            let mut markets = self.matched_markets.write();
            for new_market in new_markets {
                if let Some(existing) = markets
                    .iter_mut()
                    .find(|existing| existing.market_key() == new_market.market_key())
                {
                    *existing = new_market;
                } else {
                    markets.push(new_market);
                    added += 1;
                }
            }
            let matcher = EventMatcher::new(24);
            *self.market_lookup.write() = matcher.get_subscription_info(&markets).market_lookup;
        }

        if added > 0 {
            info!("📊 Added {} matched markets", added);
        }
        added
    }

    /// Return current Kalshi tickers for WebSocket subscriptions.
    pub fn get_kalshi_subscription_ids(&self) -> Vec<String> {
        let markets = self.matched_markets.read();
        EventMatcher::new(24)
            .get_subscription_info(&markets)
            .kalshi_tickers
    }
}
