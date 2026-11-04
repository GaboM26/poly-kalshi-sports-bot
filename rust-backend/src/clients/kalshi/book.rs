//! Kalshi order book: fixed-point levels, deltas, and fresh-book access.

use super::*;

/// Order book structure
#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    /// Whole-contract view (price_cents, floored quantity), ascending, best
    /// last. Derived from the exact levels; a level under one whole contract
    /// is left out, since an order can only take whole contracts.
    pub yes: Vec<(i32, i32)>,
    pub no: Vec<(i32, i32)>,
    /// Exact levels in hundredths of a contract (Kalshi sizes are
    /// fixed-point). Deltas apply here: flooring each delta on its own made
    /// the cached book drift from the exchange's.
    yes_exact: Vec<(i32, i64)>,
    no_exact: Vec<(i32, i64)>,
    pub(super) updated_at: Option<Instant>,
    /// The WebSocket connection epoch this book was last written under. 0
    /// (the Default) never matches a real connection's epoch (which starts
    /// at 1), so a book that was never epoch-tagged is correctly untrusted.
    pub(super) epoch: u64,
}

impl OrderBook {
    pub(super) fn from_exact(yes_exact: Vec<(i32, i64)>, no_exact: Vec<(i32, i64)>) -> Self {
        // Sub-cent levels floor onto one whole-cent price, so merge them:
        // deltas then update that single aggregated level.
        let merge = |levels: Vec<(i32, i64)>| -> Vec<(i32, i64)> {
            let mut merged = std::collections::BTreeMap::new();
            for (price, quantity) in levels {
                *merged.entry(price).or_insert(0) += quantity;
            }
            merged.into_iter().filter(|(_, quantity)| *quantity > 0).collect()
        };
        let mut book = OrderBook {
            yes_exact: merge(yes_exact),
            no_exact: merge(no_exact),
            ..OrderBook::default()
        };
        book.refresh_whole_levels();
        book
    }

    fn refresh_whole_levels(&mut self) {
        let whole = |levels: &[(i32, i64)]| -> Vec<(i32, i32)> {
            levels
                .iter()
                .filter_map(|(price, centi)| {
                    let contracts = i32::try_from(centi / 100).ok()?;
                    (contracts > 0).then_some((*price, contracts))
                })
                .collect()
        };
        self.yes = whole(&self.yes_exact);
        self.no = whole(&self.no_exact);
    }

    /// Apply one `orderbook_delta` (exact hundredths of a contract).
    pub(super) fn apply_delta(&mut self, side: &str, price: i32, delta_centi: i64) {
        let levels = if side == "yes" {
            &mut self.yes_exact
        } else {
            &mut self.no_exact
        };
        if let Some(pos) = levels.iter().position(|(p, _)| *p == price) {
            let new_qty = levels[pos].1 + delta_centi;
            if new_qty <= 0 {
                levels.remove(pos);
            } else {
                levels[pos].1 = new_qty;
            }
        } else if delta_centi > 0 {
            levels.push((price, delta_centi));
            levels.sort_by_key(|(p, _)| *p);
        }
        self.refresh_whole_levels();
    }

    /// 计算 yes 侧的 ask 深度（买入 yes 时使用）
    /// Kalshi: yes_ask = 1 - no_bid，所以买 yes 的深度看 no 侧的 bid
    /// no 按价格升序排列，last 是最高买价（best no_bid）
    /// 只看最优档位的深度，避免累加多档位导致过于乐观的深度估计
    pub fn yes_ask_depth(&self, _max_contracts: i32) -> i32 {
        // 只返回最优档位（No 侧最高 bid）的深度
        self.no.last().map(|(_, qty)| *qty).unwrap_or(0)
    }

    /// 计算 no 侧的 ask 深度（买入 no 时使用）
    /// no_ask = 1 - yes_bid，所以买 no 的深度看 yes 侧的 bid
    /// 只看最优档位的深度，避免累加多档位导致过于乐观的深度估计
    pub fn no_ask_depth(&self, _max_contracts: i32) -> i32 {
        // 只返回最优档位（Yes 侧最高 bid）的深度
        self.yes.last().map(|(_, qty)| *qty).unwrap_or(0)
    }

    /// 根据 side 获取对应的 ask 深度
    pub fn ask_depth_for_side(&self, side: &str, max_contracts: i32) -> i32 {
        match side.to_lowercase().as_str() {
            "yes" => self.yes_ask_depth(max_contracts),
            "no" => self.no_ask_depth(max_contracts),
            _ => 0,
        }
    }

    /// Return the executable limit price for an order using the current best quote.
    pub fn limit_price_for_order(&self, side: &str, action: &str) -> Option<i32> {
        let yes_bid = self.yes.last().map(|(price, _)| *price);
        let no_bid = self.no.last().map(|(price, _)| *price);

        match (action, side) {
            ("buy", "yes") => no_bid.and_then(|price| 100_i32.checked_sub(price)),
            ("buy", "no") => yes_bid.and_then(|price| 100_i32.checked_sub(price)),
            ("sell", "yes") => yes_bid,
            ("sell", "no") => no_bid,
            _ => None,
        }
    }
}

/// Parse current fixed-point levels and the legacy integer-cent representation.
/// Levels as (price_cents, quantity in hundredths of a contract).
pub(super) fn parse_price_levels(message: &Value, fixed_point_key: &str, legacy_key: &str) -> Vec<(i32, i64)> {
    let Some(levels) = message
        .get(fixed_point_key)
        .or_else(|| message.get(legacy_key))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let fixed_point = message.get(fixed_point_key).is_some();
    levels
        .iter()
        .filter_map(|entry| {
            let price = entry.get(0)?;
            let price = if fixed_point {
                book_bid_cents(price)?
            } else {
                price.as_i64()? as i32
            };
            let quantity = entry.get(1).and_then(fixed_point_to_centi)?;
            if quantity < 0 {
                return None;
            }
            Some((price, quantity))
        })
        .collect()
}

/// Convert a Kalshi book BID price (fixed-point dollars) to whole cents,
/// flooring a sub-cent price (crypto markets tick in 0.1c). Every cached book
/// level is a bid and an ask is `100 - bid`, so flooring only ever shows a
/// bid lower and an ask higher than the exchange: depth and edges are never
/// overstated, and a whole-cent limit is still a valid Kalshi price.
pub(super) fn book_bid_cents(value: &Value) -> Option<i32> {
    let price = value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string))?;
    let (dollars, fractional) = price.split_once('.').unwrap_or((&price, ""));
    let dollars = dollars.parse::<i32>().ok()?;
    let mut cents = fractional.chars().take(2).collect::<String>();
    while cents.len() < 2 {
        cents.push('0');
    }
    if !fractional.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }

    let cents = dollars
        .checked_mul(100)?
        .checked_add(cents.parse::<i32>().ok()?)?;
    (0..=100).contains(&cents).then_some(cents)
}

/// A Kalshi fixed-point contract quantity in exact hundredths of a contract
/// (Kalshi sizes carry at most two decimals). Whole contracts are floored
/// only when the book's whole-contract view is built.
pub(super) fn fixed_point_to_centi(value: &Value) -> Option<i64> {
    let centi = value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .and_then(|quantity| quantity.parse::<f64>().ok())
        })?
        * 100.0;
    let centi = centi.round();
    (centi.is_finite() && centi.abs() < 1e15).then_some(centi as i64)
}

impl KalshiClient {
    /// Get order book from cache
    pub fn get_orderbook(&self, ticker: &str) -> Option<OrderBook> {
        self.orderbook_cache.read().get(ticker).cloned()
    }

    /// Return only a websocket book we can vouch for. REST quotes are
    /// intentionally not used for executable depth or automatic execution.
    ///
    /// "Fresh" means: this book was last written under the *currently
    /// active* connection (a reconnect bumps the epoch and invalidates
    /// every book until it's resnapshotted), not merely "changed recently" -
    /// Kalshi only sends a delta on a real book change, so a quiet market
    /// can go untouched for a while while still being perfectly accurate.
    /// `max_age` backstops that against a connection that's gone silent
    /// without erroring (e.g. a hung read): it's checked against the last
    /// message received on *any* ticker, not this one, since a quiet book
    /// isn't evidence of a dead connection.
    pub fn get_fresh_orderbook(&self, ticker: &str, max_age: Duration) -> Option<OrderBook> {
        let current_epoch = self.connection_epoch.load(Ordering::SeqCst);
        let connection_alive = self
            .last_message_at
            .read()
            .is_some_and(|last| last.elapsed() <= max_age);
        if !connection_alive {
            return None;
        }
        self.get_orderbook(ticker)
            .filter(|book| book.epoch == current_epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_fractional_orderbook_depth_for_whole_contract_orders() {
        assert_eq!(fixed_point_to_centi(&json!("9.99")), Some(999));
        let merged = OrderBook::from_exact(vec![(90, 150), (90, 60)], vec![]);
        assert_eq!(merged.yes, vec![(90, 2)]);
        let book = OrderBook::from_exact(vec![(40, 999), (41, 50)], vec![]);
        // 9.99 floors to 9 whole contracts; 0.50 is not a tradable level.
        assert_eq!(book.yes, vec![(40, 9)]);
    }

    #[test]
    fn sub_cent_book_bids_floor_and_rest_asks_round_up() {
        assert_eq!(book_bid_cents(&json!("0.9030")), Some(90));
        assert_eq!(book_bid_cents(&json!("0.0950")), Some(9));
        assert_eq!(book_bid_cents(&json!("0.45")), Some(45));
        assert_eq!(book_bid_cents(&json!("1.0000")), Some(100));
        assert_eq!(book_bid_cents(&json!("0.9x")), None);
        let quote = parse_market_rest_quote(&json!({
            "ticker": "KXBTC15M-26OCT261315-15",
            "yes_ask_dollars": "0.0970",
            "no_ask_dollars": "0.9100",
            "status": "active"
        }))
        .unwrap();
        assert!((quote.yes_ask - 0.10).abs() < 1e-9);
        assert!((quote.no_ask - 0.91).abs() < 1e-9);
    }

    #[test]
    fn fractional_deltas_do_not_drift_the_book() {
        let mut book = OrderBook::from_exact(vec![], vec![(55, 200)]);
        // +1.50 then -1.50 must return to exactly 2 contracts; flooring each
        // delta separately (+1, -2) left 1.
        book.apply_delta("no", 55, 150);
        book.apply_delta("no", 55, -150);
        assert_eq!(book.no, vec![(55, 2)]);
        // A level that only ever reaches 0.70 never shows as a best price.
        book.apply_delta("no", 60, 70);
        assert_eq!(book.no.last(), Some(&(55, 2)));
        book.apply_delta("no", 60, 40);
        assert_eq!(book.no.last(), Some(&(60, 1)));
    }
}
