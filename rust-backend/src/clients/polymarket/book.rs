//! Polymarket US order book: normalized native book, origin-fresh REST book, per-market constraints.

use super::*;

/// Fresh, normalized native Polymarket US LONG-price order book.
#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketMarketBook {
    pub success: bool,
    pub market_slug: String,
    pub state: String,
    pub transact_time: Option<String>,
    pub fetched_at_ms: i64,
    pub bids: Vec<PolymarketBookLevel>,
    pub offers: Vec<PolymarketBookLevel>,
    /// Per-market minimum tradeable quantity (contracts). None means the
    /// order service could not look it up - callers must treat that as
    /// "unknown minimum", never as "no minimum".
    #[serde(default)]
    pub minimum_trade_qty: Option<f64>,
    /// Per-market required price increment. None means unknown, same
    /// caveat as `minimum_trade_qty`.
    #[serde(default)]
    pub price_tick_size: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketBookLevel {
    pub price: f64,
    pub quantity: f64,
}

impl PolymarketMarketBook {
    /// Total executable buy-side depth for a competitor's native position,
    /// from this same fresh CLOB book. Read-only reporting for tracking/UI
    /// visibility — not used on the order-submission path.
    ///
    /// Mirrors the buy-side selection and complement pricing the execution
    /// path applies: Long buys consume native LONG offers ascending; Short
    /// buys consume native LONG bids descending at complement price.
    /// Only levels priced at or better than `limit_price` (the price the
    /// opportunity was quoted at) count, so this is the same "contracts at
    /// the quoted price or better" measure Kalshi depth uses.
    /// Returns (usd_notional, token_size).
    pub fn buy_depth(&self, position_side: PolymarketPositionSide, limit_price: f64) -> (f64, f64) {
        let levels: &[PolymarketBookLevel] = match position_side {
            PolymarketPositionSide::Long => &self.offers,
            PolymarketPositionSide::Short => &self.bids,
        };
        levels
            .iter()
            .fold((0.0_f64, 0.0_f64), |(usd, size), level| {
                let quantity = level.quantity.floor();
                if quantity <= 0.0 {
                    return (usd, size);
                }
                let price = match position_side {
                    PolymarketPositionSide::Long => level.price,
                    PolymarketPositionSide::Short => 1.0 - level.price,
                };
                if price > limit_price + 1e-9 {
                    return (usd, size);
                }
                (usd + quantity * price, size + quantity)
            })
    }
}

/// Build and validate a `PolymarketMarketBook` straight from the gateway's
/// raw `{marketData: {...}}` envelope.
pub fn build_market_book(response: &Value, requested_slug: &str) -> Result<PolymarketMarketBook> {
    let market_data = response
        .get("marketData")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            anyhow::anyhow!("Polymarket US market book did not contain a marketData envelope")
        })?;
    let market_slug = market_data
        .get("marketSlug")
        .and_then(Value::as_str)
        .filter(|slug| *slug == requested_slug)
        .ok_or_else(|| {
            anyhow::anyhow!("Polymarket US market book slug did not match the requested market")
        })?;
    let state = market_data
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| !state.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Polymarket US market book did not contain a state"))?;

    let mut bids = normalize_book_levels(market_data.get("bids"), "bids")?;
    let mut offers = normalize_book_levels(market_data.get("offers"), "offers")?;
    bids.sort_by(|a, b| b.price.total_cmp(&a.price));
    offers.sort_by(|a, b| a.price.total_cmp(&b.price));

    let transact_time = match market_data.get("transactTime") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(_) => anyhow::bail!("Polymarket US market book contained an invalid transactTime"),
    };

    Ok(PolymarketMarketBook {
        success: true,
        market_slug: market_slug.to_string(),
        state: state.to_string(),
        transact_time,
        fetched_at_ms: Utc::now().timestamp_millis(),
        bids,
        offers,
        minimum_trade_qty: None,
        price_tick_size: None,
    })
}

/// Validate SDK price/quantity objects without accepting lossy book data.
pub(super) fn normalize_book_levels(levels: Option<&Value>, field: &str) -> Result<Vec<PolymarketBookLevel>> {
    let levels = levels
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Polymarket US market book {} must be a list", field))?;

    let mut normalized = Vec::with_capacity(levels.len());
    for level in levels {
        let price = amount_as_f64(&level["px"]);
        let quantity = amount_as_f64(&level["qty"]);
        let (Some(price), Some(quantity)) = (price, quantity) else {
            anyhow::bail!("Polymarket US market book {} contains an invalid level", field);
        };
        if !(0.0 < price && price < 1.0) {
            anyhow::bail!("Polymarket US market book {} contains an invalid price", field);
        }
        if quantity <= 0.0 {
            anyhow::bail!("Polymarket US market book {} contains an invalid quantity", field);
        }
        normalized.push(PolymarketBookLevel { price, quantity });
    }
    Ok(normalized)
}

/// Why a response must not be trusted as fresh market data, if the CDN
/// served it from cache (`None` for an origin response). `cf-cache-status`
/// HIT/STALE/UPDATING all mean an edge copy was returned instead of origin.
pub(super) fn cdn_cached_reason(cache_status: Option<&str>, age: Option<&str>) -> Option<String> {
    let status = cache_status.map(|status| status.trim().to_ascii_uppercase());
    let age_secs = age.and_then(|age| age.trim().parse::<u64>().ok());
    if matches!(status.as_deref(), Some("HIT" | "STALE" | "UPDATING")) {
        return Some(format!(
            "served from the CDN cache (cf-cache-status {}, age {}s)",
            status.unwrap_or_default(),
            age_secs.map_or_else(|| "?".to_string(), |age| age.to_string())
        ));
    }
    match age_secs {
        Some(age) if age > 1 => Some(format!("a {}s-old cached copy", age)),
        _ => None,
    }
}

/// A query value no other request has used, so the CDN can't answer it
/// from cache.
pub(super) fn cache_busting_nonce() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{nanos}-{count}")
}

impl PolymarketClient {
    /// Retrieve a fresh book directly from the Polymarket US gateway.
    ///
    /// The gateway serves this endpoint through Cloudflare with
    /// `cache-control: public, max-age=30`, so a plain GET returns whatever
    /// snapshot the edge cached, up to 30s old (measured live on in-play
    /// markets) - the cause of the zero-fill IOC orders that surfaced as
    /// `ORD_REJECT_REASON_EXCHANGE_OPTION`. A unique query string forces an
    /// origin fetch, and the response's CDN headers are checked so that a
    /// cached copy is refused rather than silently trusted if that ever
    /// stops working.
    pub async fn get_market_book(&self, market_slug: &str) -> Result<PolymarketMarketBook> {
        if market_slug.trim().is_empty() {
            anyhow::bail!("Polymarket US market slug cannot be empty");
        }
        let base = self.config.base_url.trim_end_matches('/');
        let url = format!(
            "{}/v1/markets/{}/book?fresh={}",
            base,
            market_slug,
            cache_busting_nonce()
        );
        let (response, headers) = self
            .request_json_with_headers(Method::GET, &url, None, None)
            .await
            .context("Failed to retrieve Polymarket US market book")?;
        if let Some(reason) = cdn_cached_reason(
            headers.get("cf-cache-status").and_then(|v| v.to_str().ok()),
            headers.get("age").and_then(|v| v.to_str().ok()),
        ) {
            anyhow::bail!(
                "Polymarket US market book for {} was {}; refusing to treat it as fresh",
                market_slug,
                reason
            );
        }
        let mut book = build_market_book(&response, market_slug)?;

        // Cached after the first lookup, so this normally costs nothing
        // extra. A failure here must not silently look like "no minimum" -
        // leave both fields None and let the caller refuse to size an order
        // against them.
        match self.market_constraints(market_slug).await {
            Ok((min_qty, tick_size)) => {
                book.minimum_trade_qty = min_qty;
                book.price_tick_size = tick_size;
            }
            Err(error) => {
                warn!(
                    "Polymarket US market constraint lookup failed for {}: {}",
                    market_slug, error
                );
            }
        }

        Ok(book)
    }

    /// `minimumTradeQty` / `orderPriceMinTickSize` for a market, cached for
    /// an hour (docs.polymarket.us/api-reference/orders/overview:
    /// "constraints are market-dependent - retrieve ... before order
    /// submission"). The cache lock is not held across the HTTP call, so a
    /// first-time lookup for one market never stalls a cached lookup for
    /// another on the order path.
    pub async fn market_constraints(
        &self,
        market_slug: &str,
    ) -> Result<(Option<f64>, Option<f64>)> {
        if let Some((cached_at, min_qty, tick_size)) =
            self.market_constraints_cache.lock().await.get(market_slug)
        {
            if cached_at.elapsed() < self.config.cache.market_constraints_ttl() {
                return Ok((*min_qty, *tick_size));
            }
        }

        let base = self.config.base_url.trim_end_matches('/');
        let url = format!("{}/v1/market/slug/{}", base, market_slug);
        let response = self.request_json(Method::GET, &url, None, None).await?;
        let market = response.get("market");
        let min_qty = market.and_then(|m| amount_as_f64(&m["minimumTradeQty"]));
        let tick_size = market.and_then(|m| amount_as_f64(&m["orderPriceMinTickSize"]));

        self.market_constraints_cache
            .lock()
            .await
            .insert(market_slug.to_string(), (Instant::now(), min_qty, tick_size));
        Ok((min_qty, tick_size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(bids: Vec<(f64, f64)>, offers: Vec<(f64, f64)>) -> PolymarketMarketBook {
        PolymarketMarketBook {
            success: true,
            market_slug: "test-market".to_string(),
            state: "ACTIVE".to_string(),
            transact_time: None,
            fetched_at_ms: 0,
            bids: bids
                .into_iter()
                .map(|(price, quantity)| PolymarketBookLevel { price, quantity })
                .collect(),
            offers: offers
                .into_iter()
                .map(|(price, quantity)| PolymarketBookLevel { price, quantity })
                .collect(),
            minimum_trade_qty: None,
            price_tick_size: None,
        }
    }

    #[test]
    fn long_buy_depth_sums_native_offers_at_native_price() {
        let book = book(vec![(0.50, 10.0)], vec![(0.22, 4.0), (0.25, 6.5)]);
        let (usd, size) = book.buy_depth(PolymarketPositionSide::Long, 1.0);
        // quantity floors to whole tokens: 4 + 6 = 10; usd = 4*0.22 + 6*0.25
        assert_eq!(size, 10.0);
        assert!((usd - (4.0 * 0.22 + 6.0 * 0.25)).abs() < 1e-9);
    }

    #[test]
    fn short_buy_depth_sums_native_bids_at_complement_price() {
        let book = book(vec![(0.70, 3.0)], vec![(0.22, 4.0)]);
        let (usd, size) = book.buy_depth(PolymarketPositionSide::Short, 1.0);
        // Short buys consume native bids at 1 - price.
        assert_eq!(size, 3.0);
        assert!((usd - 3.0 * (1.0 - 0.70)).abs() < 1e-9);
    }

    #[test]
    fn buy_depth_ignores_non_positive_quantity_levels() {
        let book = book(vec![], vec![(0.30, 0.0), (0.40, 2.0)]);
        let (usd, size) = book.buy_depth(PolymarketPositionSide::Long, 1.0);
        assert_eq!(size, 2.0);
        assert!((usd - 2.0 * 0.40).abs() < 1e-9);
    }

    #[test]
    fn cdn_cached_market_data_is_refused() {
        assert!(cdn_cached_reason(Some("HIT"), Some("12")).is_some());
        assert!(cdn_cached_reason(Some("hit"), None).is_some());
        assert!(cdn_cached_reason(Some("STALE"), None).is_some());
        assert!(cdn_cached_reason(Some("UPDATING"), None).is_some());
        assert!(cdn_cached_reason(None, Some("25")).is_some());
        // Origin responses.
        assert!(cdn_cached_reason(Some("MISS"), None).is_none());
        assert!(cdn_cached_reason(Some("EXPIRED"), None).is_none());
        assert!(cdn_cached_reason(Some("REVALIDATED"), None).is_none());
        assert!(cdn_cached_reason(Some("BYPASS"), None).is_none());
        assert!(cdn_cached_reason(Some("DYNAMIC"), Some("0")).is_none());
        assert!(cdn_cached_reason(None, None).is_none());
    }

    #[test]
    fn cache_busting_nonces_never_repeat() {
        let nonces: HashSet<String> = (0..1000).map(|_| cache_busting_nonce()).collect();
        assert_eq!(nonces.len(), 1000);
    }
}
