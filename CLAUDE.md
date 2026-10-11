# CLAUDE.md

## What this project is

Polytaoli: a real-time cross-exchange arbitrage system between **Kalshi** and
**Polymarket US** prediction markets. It watches matched markets on both
venues, computes profit margins after fees, and can automatically execute
paired (hedged) orders across both exchanges. This is live financial software
handling real money and real exchange APIs. Precision and correctness in the
order-execution path matter more than anywhere else in the codebase.

## Current goal

The user is debugging the arbitrage and execution pipeline so they can start
executing real trades on detected opportunities with confidence. Priorities:

- Find and fix correctness bugs in price/quantity calculation, order sizing,
  and paired-execution logic before features or polish.
- Treat any bug in this path as potentially money-losing. Check fixes against
  the invariants below rather than assuming a plausible-looking change is
  correct.
- Check `git status` before reasoning about "current" behavior. This file
  describes the state of the code at the last update, not what is uncommitted.

## Architecture

Two services, started together via `./start_rust_stack.sh`:

- **`rust-backend/`** (Axum + Tokio, port `8000`), the core engine.
  - `core/`: `matcher.rs` (event/market matching between exchanges, incl.
    NBA logic in `nba_teams.rs`/`competitors.rs`), `calculator.rs` (profit
    margin math), `fees.rs` (the only place fee math lives).
  - `clients/`: `kalshi.rs` (REST + WebSocket, RSA-PSS-signed requests
    against `external-api.kalshi.com`), `polymarket.rs` (REST against
    `gateway.polymarket.us` for market data and `api.polymarket.us` for
    orders/account/portfolio, Ed25519-signed via `polymarket_auth.rs`),
    `polymarket_ws.rs` (authenticated market-data WebSocket). Both exchanges
    are called directly; there is no separate order-service process.
  - `services/`: `arbitrage.rs` (detection/control), `paired_execution.rs`
    (sizing, paired submission, neutralization), `positions.rs` (position
    economics and hedge pairing), `websocket_manager/` (live delivery to the
    frontend, quote caches, `auto_trade.rs`, `market_lifecycle.rs`),
    `storage/` (SQLite; `auto_trade_repo.rs` tracks the execution state
    machine, `hedge_pairs_repo.rs` persists verified hedges), `telegram.rs`.
  - `api/`: HTTP routes and the `mod.rs` request/response wiring.
- **`web/`** (Vite + React + TS, port `5173`), the dashboard: live
  opportunities, order forms, position/history views, WebSocket client
  (`hooks/useWebSocket.ts`).

The old Python `poly-order-service/` (FastAPI wrapper around the
`polymarket-us` SDK) was replaced by direct Rust calls on 2026-09-27 and
deleted on 2026-09-29. Nothing references it.

## Critical invariants (do not weaken without explicit user sign-off)

`.github/copilot-instructions.md` is the authoritative day-to-day rulebook and
may be more current than this summary. Read it too.

- **Kalshi signing**: sign the path only, exclude query params.
- **Polymarket order responses**: read fills from `executions[].order`
  (e.g. `.avgPx`), never from the top-level response envelope.
- **`ORD_REJECT_REASON_EXCHANGE_OPTION` is not a rejection.** It is the zero
  value of the reject-reason enum and appears on every execution, including
  the `EXECUTION_TYPE_NEW` acceptance. Only `EXECUTION_TYPE_REJECTED` /
  `ORDER_STATE_REJECTED` carries a real reason. An accepted IOC that finds no
  liquidity goes `NEW` then `EXPIRED` and is reported as "expired unfilled".
- **Polymarket depth** (changed 2026-10-01 with user sign-off): only via
  `services/paired_execution.rs::executable_polymarket_book`: the live
  WebSocket full book while its connection is provably current
  (`WebSocketManager::fresh_polymarket_book`), else one origin-fresh
  `/v1/markets/{market_slug}/book` (`get_market_book` cache-busts and refuses
  a CDN-cached response). Market must be `MARKET_STATE_OPEN`. Never a
  CDN-cached gateway response, gateway display quotes, account caches, or the
  non-US CLOB API. The gateway is Cloudflare-cached with `max-age=30`, so a
  plain GET can be up to 30s stale.
- **Polymarket prices are YES-book prices** on the wire and in `avgPx`: a
  SHORT order's NO price must be complemented before sending and its fill
  complemented back (`yes_book_price`), snapped to tick, sent as a decimal
  string, never a raw `f64`.
- **`manualOrderIndicator`**: send `MANUAL_ORDER_INDICATOR_AUTOMATIC`. The
  exchange records every API order as AUTOMATIC regardless, so this field is
  not a fix for anything; do not treat it as one.
- **Kalshi depth**: only from a fresh Kalshi WebSocket book at execution
  time. Kalshi size is fixed-point; floor when converting to whole contracts,
  never round up.
- **Fees**: every margin is net of BOTH exchanges' taker fees, computed only
  by `core/fees.rs` (Kalshi ceil-to-cent 0.07·C·p·(1−p); Polymarket US
  banker's-rounded 0.0695·C·p·(1−p), rounded on the cumulative exact fee per
  order). Never re-implement fee math elsewhere, and never drop either fee
  from detection, sizing, or stored estimates. A drift check compares the
  modeled fee with `commissionNotionalTotalCollected` on every fill and logs
  `Polymarket fee model drift` if they diverge.
- **Pairing logic**: Kalshi YES ↔ Polymarket NO, Kalshi NO ↔ Polymarket YES,
  for the *tracked competitor*. The native Polymarket LONG/SHORT side must be
  resolved from that competitor mapping, never inferred from a raw outcome
  index.
- **Execution state machine**: persist a `submitting` record before either
  leg fires; durably update each acknowledgement before submitting the second
  leg. On startup, halt if an unresolved `submitting` record exists until it
  is reconciled.
- **Neutralization**: on unequal fills, attempt a bounded IOC/FAK offset only
  within `neutralization_max_loss_cents` (default 5¢/contract). If it cannot
  fully close, record the residual durably, disable auto-trading, and alert
  via Telegram. Do not silently retry or hide it.
- **Safeguards**: never weaken order-size, profit-margin, duration,
  duplicate-trade, or execution-count limits without being asked.
- **Error logging**: log `anyhow::Error` with `{:#}`, never `{}`. Bare `{}`
  truncates to the outermost context and hid root causes in the past.
- Preserve WebSocket message compatibility between the Rust manager and the
  React client when touching opportunity models.

## Data sources and freshness

- **Polymarket**: the authenticated WebSocket
  (`wss://api.polymarket.us/v1/ws/markets`) streams quotes
  (`SUBSCRIPTION_TYPE_MARKET_DATA_LITE`) and full books
  (`SUBSCRIPTION_TYPE_MARKET_DATA`); at most 100 slugs per subscription.
  Connection epoch and last-frame liveness gate whether a WS value is trusted.
  REST polling stays as fallback and drives ended-market detection and
  `live`/`period` fields, but must not overwrite a quote from the live
  connection.
- **Kalshi**: the WebSocket book feeds `apply_kalshi_ws_quote`, which
  triggers detection when best prices change. Kalshi REST market data is
  served from a server-side cache (values step every ~8–10s). The Kalshi WS
  book has no `seq` gap detection.
- WS-derived depth shown on the detail page and in tracking is display-only.
  Order sizing always fetches its own fresh book.
- Closed markets: Kalshi `closed`/`settled` status ends a market immediately;
  a Polymarket market missing from the active feed for 2 consecutive cycles
  is treated as ended. Both feed `confirmed_ended_markets`.
- Manual Sell falls back to a fresh REST quote when the WS book is empty
  (`KalshiMarketQuote::limit_price_for_order`). That fallback is price-only
  for one manual order; never use it for auto-trade sizing or depth.
- Retries: Kalshi GETs retry transport failures (1s/2s, 3 attempts); Kalshi
  `post()` (order placement) never retries, to avoid duplicate orders.
  Polymarket retries only clean 429s (1s/2s/4s). Never retry a real order
  rejection.

## Positions and fees in the UI

- `/api/positions/unified` returns server-side grouped cards; the frontend is
  a pure renderer. `services/positions.rs` holds the economics and pairing.
- Pairing: bot trade history first (`executed`/`neutralized`/`partial_paired`,
  matched on the full `(kalshi_market_id, kalshi_side, polymarket_market_id,
  polymarket_side)` tuple), then the matcher's market mapping for manual
  hedges (`paired_by`: `bot_trade` / `market_match`). Verified mapped pairs
  persist in `hedge_pairs`. A manual hedge on a market the matcher never
  matched stays standalone.
- Both legs use an all-in cost basis (cost includes fees; entry price excludes
  them). Polymarket `cost` = `baseCost` + `fees`; Kalshi
  `market_exposure_dollars` excludes fees (`fees_paid_dollars` is separate). A
  null fee shows "n/a", never $0.00. Paired cards show settlement P&L
  `min(leg contracts) − total all-in cost`.
- Details page: `ArbitrageCalculator::preview_single` shows fees and margin
  for any market with fresh prices on both sides. It is display-only; only the
  gated calculator decides `has_opportunity`, and nothing may trade on a
  preview.
- Open risk: Kalshi `market_exposure_dollars` sign for NO positions is
  undocumented. The code uses `abs(...)`; verify against a real held NO
  position.

## Open issues (found, not fixed)

Ordered by money risk. (Fixed 2026-10-11: sizing now re-checks `min_profit_margin` on the fresh books, neutralization's loss bound includes the closing taker fee, and auto-trade skips `SUS` games.)

1. Kalshi REST can overwrite a newer WS quote (detection-only phantom risk;
   execution re-sizes from the WS book). Add `seq`-gap detection and a
   resnapshot before making the WS own prices.
2. Not yet verified live: the `SELL_SHORT` price conversion (inferred from the
   docs and BUY_SHORT evidence; none sent yet) and WS-driven detection
   end-to-end in the app's own logs.
3. `orderPriceMinTickSize` is now used to snap prices; `minimumTradeQty` is
   `0.01` on every market checked, so the minimum-quantity gate in
   `find_profitable_contract_size` is effectively a no-op.
4. Log noise: every UI API call logs at INFO (`TraceLayer`). Revisit if too
   verbose.
5. `polymarket.rs` and `positions.rs` have low graph cohesion (~0.06) and
   `WebSocketManager` is a 68-edge hub. Candidates for splitting; do not
   refactor without being asked.
6. The user reported something "seems to disconnect every few seconds"
    (2026-09-27). Never triaged: could be the Kalshi WS, the frontend WS
    client, or a backend restart.

## Next up: extend market coverage beyond NBA + tennis

Not started. Keep it read-only (detection/display) until settlement
equivalence is verified; auto-trade safeguards do not check it.

- Polymarket US `/v1/sports` lists nba, cbb, nhl, epl, ucl, mls, ufc, uefa,
  mlb, f1, boxing, wta/atp, poly-open. Per-sport series ids come from that
  response. Fetch events with `&sportsMarketTypes=<type>` (unfiltered NHL is
  ~22 MB per request).
- Two-outcome moneylines (nhl, mlb, cbb, ufc, boxing) fit the existing binary
  model. Soccer is three-way (draw): a Kalshi YES / Polymarket NO pairing is
  not a clean hedge there.
- **Identity**: join on team abbreviations (Kalshi ticker suffix, Polymarket
  `marketSides[].team.abbreviation`). NHL differences: Kalshi→Polymarket
  `MTL→MON`, `NSH→NAS`, `VGK→VEG`, `WSH→WAS`. MLB: all 30 agree. Do not reuse
  `extract_teams_from_ticker` (NBA 3+3 split); NHL has 2-letter codes and MLB
  tickers embed the game time. Parse like tennis: two markets per event.
- **Allowlist exact Polymarket market types**: NHL `hockey_team_full_game_
  winner` (includes OT/SO; never `hockey_team_regulation_winner`), MLB
  `baseball_team_full_game_winner` (not `first_five`/`inningN_winner`).
- **Settlement**: NHL equivalent on both exchanges. MLB mismatch: Kalshi
  settles at a fair price if a postponed game is not played within 2 days;
  Polymarket waits up to 2 weeks. Decide before auto-trading MLB (per-sport
  switch).
- **Doubleheaders**: the matcher takes the first free Polymarket candidate for
  a (category, teams, date) key. Add an ambiguity guard (refuse when more than
  one candidate) before the regular season.
- Edges after both fees are rare and transient (a live snapshot of 35 NHL + 5
  MLB games had none positive), which is where WS-driven detection matters.

## Validation

- Rust: `cargo test` from `rust-backend/`.
- Frontend: `npm run build` from `web/`. `npm run lint` is broken independent
  of recent changes (the `--ext` flag is unsupported by the flat eslint
  config).
- `cargo run --example verify_polymarket_direct` (from `rust-backend/`) hits
  read-only live Polymarket endpoints with real credentials. Manual check
  only; never routine. Other read-only probes: `probe_polymarket_get`,
  `probe_poly_book_freshness`, `probe_polymarket_ws`, `probe_positions_view`.
- **Never enable automatic trading or submit live/test orders** as part of
  routine development or debugging. The config default is
  `auto_trade.enabled = false`; keep it that way unless the user explicitly
  asks to test live.

## Config & secrets

- `rust-backend/config.toml` is gitignored and holds real Kalshi/Polymarket
  credentials. Never read it back into chat output or commit it.
  `config.example.toml` is the template to update when adding settings.
  `POLYMARKET_KEY_ID` / `POLYMARKET_SECRET_KEY` env vars take precedence over
  config.
- Default dev ports: frontend `5173`, Rust API `8000`. Keep them aligned
  across services if changed.

## Lessons from past debugging (so they are not repeated)

- `ORD_REJECT_REASON_EXCHANGE_OPTION` was chased through five wrong theories
  (minimumTradeQty, float quantity, SDK/IPC, zero slippage buffer,
  `manualOrderIndicator`). The real causes were a 30s Cloudflare-cached book
  and BUY_SHORT orders priced on the wrong side of the YES book. A staleness
  check "ruled out" the cache only because the check read the same cached
  copy. When verifying a theory, make sure the check does not share the
  suspected fault.
- A first fill after a change is not proof the change worked (a pre-game
  order filled after the `AUTOMATIC` change; pre-game orders always filled).
- Check the exchange's own records (`GET /v1/order/{id}`,
  `/v1/portfolio/activities`) before trusting a log line.
- Read logs in full before theorizing: no `FILLED` line existed in 14 files of
  history, which ruled out "compare to a successful payload".

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
