//! Polymarket WebSocket state: pushed quotes and books, connection epochs, and liveness.

use super::*;

/// A full Polymarket book pushed over the WebSocket, tagged with the
/// connection it arrived on.
#[derive(Debug, Clone)]
pub(crate) struct LivePolymarketBook {
    pub book: PolymarketMarketBook,
    pub epoch: u64,
}

/// Point-in-time WebSocket delivery counters (see `WebSocketManager::ws_stats`).
#[derive(Debug, Clone, Copy)]
pub struct WsStats {
    pub poly_quote_msgs: u64,
    pub poly_book_msgs: u64,
    pub kalshi_msgs: u64,
    pub poly_markets_recent: usize,
    pub kalshi_markets_recent: usize,
    pub markets_total: usize,
}

impl WebSocketManager {
    /// Apply a pushed Polymarket WebSocket quote and re-run detection for
    /// every matched market on that slug immediately. Returns the number of
    /// matched markets updated. Prices feed detection only; execution depth
    /// is still fetched from the real book at order time.
    pub fn update_polymarket_ws_quote(&self, slug: &str, long_quote: f64, short_quote: f64) -> usize {
        self.poly_ws_quote_msgs.fetch_add(1, Ordering::Relaxed);
        self.note_polymarket_ws_frame();
        let epoch = self.poly_ws_epoch.load(Ordering::SeqCst);
        let received_at = Instant::now();
        let mut updated_indices = Vec::new();
        let mut cache_updates: Vec<(String, f64, f64)> = Vec::new();

        {
            let mut matched_markets = self.matched_markets.write();
            for (idx, matched) in matched_markets.iter_mut().enumerate() {
                let pm = &mut matched.polymarket_market;
                if pm.market_slug != slug {
                    continue;
                }
                // Map each competitor to its own native side; never infer it
                // from outcome order.
                let side_quote = |side: crate::models::PolymarketPositionSide| match side {
                    crate::models::PolymarketPositionSide::Long => long_quote,
                    crate::models::PolymarketPositionSide::Short => short_quote,
                };
                pm.price_a = side_quote(pm.team_a_position);
                pm.price_b = side_quote(pm.team_b_position);
                let (price_a, price_b) = (pm.price_a, pm.price_b);
                let market_id = pm.market_id.clone();
                let Ok((yes_price, no_price)) = pm.get_price_for_team(&matched.team_name) else {
                    continue;
                };
                matched.poly_yes_price = yes_price;
                matched.poly_no_price = no_price;
                updated_indices.push(idx);
                cache_updates.push((market_id, price_a, price_b));
            }
        }

        if updated_indices.is_empty() {
            return 0;
        }

        {
            let mut cache = self.poly_rest_quotes.write();
            let mut ws_at = self.poly_ws_quote_at.write();
            let mut ws_epoch = self.poly_ws_quote_epoch.write();
            for (market_id, price_a, price_b) in cache_updates {
                ws_at.insert(market_id.clone(), received_at);
                ws_epoch.insert(market_id.clone(), epoch);
                cache.insert(
                    market_id,
                    PolymarketRestQuote { price_a, price_b, received_at },
                );
            }
        }
        *self.polymarket_last_update_time.write() = Some(Utc::now());
        *self.polymarket_update_count.write() += updated_indices.len() as u64;

        for idx in &updated_indices {
            self.calculate_and_notify(*idx);
        }
        updated_indices.len()
    }

    /// Snapshot for the periodic stats line: cumulative message counts plus
    /// how many matched markets received a WebSocket quote within `window`
    /// (quiet markets legitimately go silent, so this is a coverage hint,
    /// not an error count).
    pub fn ws_stats(&self, window: Duration) -> WsStats {
        let now = Instant::now();
        let recent = |map: &HashMap<String, Instant>| {
            map.values()
                .filter(|at| now.saturating_duration_since(**at) <= window)
                .count()
        };
        WsStats {
            poly_quote_msgs: self.poly_ws_quote_msgs.load(Ordering::Relaxed),
            poly_book_msgs: self.poly_ws_book_msgs.load(Ordering::Relaxed),
            kalshi_msgs: self.kalshi_ws_msgs.load(Ordering::Relaxed),
            poly_markets_recent: recent(&self.poly_ws_quote_at.read()),
            kalshi_markets_recent: recent(&self.kalshi_ws_quote_at.read()),
            markets_total: self.matched_markets.read().len(),
        }
    }

    /// Store a full Polymarket book snapshot pushed over the WebSocket,
    /// tagged with the connection it arrived on.
    pub fn update_polymarket_ws_book(&self, book: PolymarketMarketBook) {
        self.poly_ws_book_msgs.fetch_add(1, Ordering::Relaxed);
        self.note_polymarket_ws_frame();
        let epoch = self.poly_ws_epoch.load(Ordering::SeqCst);
        self.poly_live_books
            .write()
            .insert(book.market_slug.clone(), LivePolymarketBook { book, epoch });
    }

    /// Latest WebSocket book for a slug, if one has arrived - on any
    /// connection, so for display and tracking only. Execution must use
    /// `fresh_polymarket_book`.
    pub fn polymarket_live_book(&self, slug: &str) -> Option<PolymarketMarketBook> {
        self.poly_live_books
            .read()
            .get(slug)
            .map(|live| live.book.clone())
    }

    /// A new Polymarket WebSocket connection is up: bump the epoch so
    /// nothing stored from the previous connection is trusted until this one
    /// re-sends it (the server sends a full book on subscribe).
    pub fn on_polymarket_ws_connected(&self) {
        let epoch = self.poly_ws_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.poly_ws_last_frame_at.write() = Some(Instant::now());
        self.poly_ws_connected.store(true, Ordering::SeqCst);
        info!("📡 Polymarket WS connected (connection epoch {})", epoch);
    }

    /// The Polymarket WebSocket connection died; until it reconnects and
    /// re-sends, no stored book or quote is current.
    pub fn on_polymarket_ws_disconnected(&self) {
        self.poly_ws_connected.store(false, Ordering::SeqCst);
    }

    /// Any frame proves the connection is alive, even a heartbeat.
    pub fn note_polymarket_ws_frame(&self) {
        *self.poly_ws_last_frame_at.write() = Some(Instant::now());
    }

    /// The current connection epoch, if that connection is up and has
    /// delivered a frame within `max_silence`.
    pub fn live_polymarket_ws_epoch(&self, max_silence: Duration) -> Option<u64> {
        let alive = self.poly_ws_connected.load(Ordering::SeqCst)
            && self
                .poly_ws_last_frame_at
                .read()
                .is_some_and(|last| last.elapsed() <= max_silence);
        alive.then(|| self.poly_ws_epoch.load(Ordering::SeqCst))
    }

    /// Whether `market_id`'s Polymarket price came from the WebSocket on the
    /// connection that is live now. Anything else is a gateway REST quote,
    /// which Cloudflare can serve up to 30s old: a new crypto window's
    /// listing quote was 12c off the real book on 2026-10-27, seconds before
    /// its WS subscription landed. Only a live WS price may start an
    /// auto-trade.
    pub fn polymarket_price_is_live(&self, market_id: &str) -> bool {
        self.live_polymarket_ws_epoch(self.settings.polymarket_ws_max_silence())
            .is_some_and(|epoch| self.poly_ws_quote_epoch.read().get(market_id) == Some(&epoch))
    }

    /// The WebSocket book for `slug`, but only if it is provably current:
    /// it arrived on the connection that is up right now, and that
    /// connection has delivered a frame within `max_silence`. The server
    /// pushes a full snapshot on every change, so a quiet market's book can
    /// be old and still exact; a dead or replaced connection is what makes
    /// it untrustworthy. Mirrors `KalshiClient::get_fresh_orderbook`.
    pub fn fresh_polymarket_book(
        &self,
        slug: &str,
        max_silence: Duration,
    ) -> Option<PolymarketMarketBook> {
        let epoch = self.live_polymarket_ws_epoch(max_silence)?;
        self.poly_live_books
            .read()
            .get(slug)
            .filter(|live| live.epoch == epoch)
            .map(|live| live.book.clone())
    }
}
