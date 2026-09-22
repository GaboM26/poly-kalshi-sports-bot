# CLAUDE.md

## What this project is

Polytaoli: a real-time cross-exchange arbitrage system between **Kalshi** and
**Polymarket US** prediction markets. It watches matched markets on both
venues, computes profit margins after fees, and can automatically execute
paired (hedged) orders across both exchanges. This is live financial software
handling real money and real exchange APIs — precision and correctness in the
order-execution path matter more than anywhere else in the codebase.

## Current goal

The user is debugging the arbitrage/execution pipeline so they can start
publishing (executing) real trades on detected arbitrage opportunities with
confidence. Priorities when helping:

- Find and fix correctness bugs in price/quantity calculation, order sizing,
  and paired-execution logic before worrying about features or polish.
- Treat any bug in this path as potentially money-losing. Confirm fixes
  against the invariants below rather than assuming a plausible-looking
  change is correct.
- There is currently an in-progress uncommitted diff across the Rust
  backend, order service, and docs (`git status` shows modified files in
  `api/`, `clients/kalshi.rs`, `clients/polymarket.rs`,
  `services/storage/auto_trade_repo.rs`, `services/websocket_manager/`, and
  `poly-order-service/`). Assume this is active work-in-progress, not junk —
  read the diff before reasoning about "current" behavior.

## Architecture

Three services, started together via `./start_rust_stack.sh`:

- **`rust-backend/`** (Axum + Tokio, port `8000`) — the core engine.
  - `core/`: `matcher.rs` (event/market matching between exchanges, incl.
    NBA-specific logic in `nba_teams.rs`/`competitors.rs`), `calculator.rs`
    (profit-margin math).
  - `clients/`: `kalshi.rs` (REST + WebSocket, RSA-signed requests),
    `polymarket.rs` (REST + WebSocket, calls out to the Python order
    service for order submission).
  - `services/`: `arbitrage.rs` (opportunity detection/control),
    `websocket_manager/` (live delivery to frontend, `auto_trade.rs` for
    automated paired execution, `market_lifecycle.rs`), `storage/`
    (SQLite persistence, `auto_trade_repo.rs` tracks execution state
    machine), `telegram.rs` (alerts).
  - `api/`: HTTP routes and the `mod.rs` request/response wiring.
- **`web/`** (Vite + React + TS, port `5173`) — dashboard: live
  opportunities, order forms, position/history views, WebSocket client
  (`hooks/useWebSocket.ts`).
- **`poly-order-service/`** (FastAPI, port `8001`) — thin wrapper around the
  official `polymarket-us` SDK (pinned `0.1.2`) for order submission and
  market-book depth; the Rust backend calls this instead of hitting
  Polymarket directly for order execution.

## Critical invariants (do not weaken without explicit user sign-off)

These come from `.github/copilot-instructions.md` — read that file too, it's
the authoritative day-to-day rulebook and may be more current than this
summary.

- **Kalshi signing**: sign the path only, exclude query params.
- **Polymarket order responses**: read fills from `executions[].order`
  (e.g. `.avgPx`), never from the top-level response envelope.
- **Polymarket depth**: only from the SDK's `markets.book(market_slug)`
  (normalized by `poly-order-service`) — never gateway display quotes,
  account caches, or the non-US CLOB API.
- **Kalshi depth**: only from a fresh Kalshi WebSocket book at execution
  time. Kalshi size is fixed-point — floor when converting to whole
  contracts, never round up.
- **Pairing logic**: Kalshi YES ↔ Polymarket NO, Kalshi NO ↔ Polymarket YES,
  for the *tracked competitor*. The native Polymarket LONG/SHORT side must
  be resolved from that competitor mapping, never inferred from a raw
  outcome index.
- **Execution state machine**: persist a `submitting` record before either
  leg fires; durably update each acknowledgement before submitting the
  second leg. On startup, halt if an unresolved `submitting` record exists
  until it's reconciled.
- **Neutralization**: on unequal fills, attempt a bounded IOC/FAK offset
  only within `neutralization_max_loss_cents` (default 5¢/contract). If it
  can't fully close, record the residual durably, disable auto-trading, and
  alert via Telegram — don't silently retry or hide it.
- **Safeguards**: never weaken order-size, profit-margin, duration,
  duplicate-trade, or execution-count limits without being asked.
- Preserve WebSocket message compatibility between the Rust manager and the
  React client when touching opportunity models.

## Validation

- Rust: `cargo test` from `rust-backend/`.
- Frontend: `npm run lint && npm run build` from `web/`.
- Python: `python3 -m pytest -q test_market_book.py` from
  `poly-order-service/` (offline-safe). **Do not run `test_service.py`** —
  it hits live account/order endpoints.
- **Never enable automatic trading or submit live/test orders** as part of
  routine development or debugging. Config default is `auto_trade.enabled =
  false`; keep it that way unless the user explicitly asks to test live.

## Config & secrets

- `rust-backend/config.toml` is gitignored and holds real Kalshi/Polymarket
  credentials — never read it back into chat output or commit it.
  `config.example.toml` is the template to update when adding new settings.
- Default dev ports: frontend `5173`, Rust API `8000`, Python order service
  `8001`. Keep these aligned across services if changed.
