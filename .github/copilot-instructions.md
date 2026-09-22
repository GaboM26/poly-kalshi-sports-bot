# Copilot Instructions

## Repository Layout

- `rust-backend/` is the Axum/Tokio API, arbitrage engine, exchange clients, SQLite storage, and WebSocket server.
- `web/` is the Vite, React, and TypeScript user interface.
- `poly-order-service/` is the FastAPI service that submits Polymarket US orders through `polymarket-us`.
- Root scripts start the development stack and build deployment packages. Keep the service ports aligned with the configuration: frontend `5173`, Rust API `8000`, and Python order service `8001`.

## Change Guidelines

- Treat all price, order, and position data as financial data. Preserve decimal precision, validate external inputs, and do not weaken order-size, profit-margin, duration, or execution-count safeguards.
- Keep secrets out of source control. Use `rust-backend/config.example.toml` for new configuration defaults and document required configuration in `README.md`; never add real credentials.
- Keep Rust API models, route handlers, frontend types, and API client calls consistent when an endpoint or payload changes.
- Keep the Python order-service request and response contracts compatible with the Rust Polymarket client before changing either service.
- Follow existing error-handling patterns. Surface exchange, signing, storage, and network failures rather than hiding them with fallback data.
- Preserve WebSocket message compatibility for the React client when changing the Rust WebSocket manager or opportunity models.

## Exchange Integration Status

- Kalshi uses the v2 API. Sign authenticated requests with the path only; exclude query parameters from the signed payload.
- The pinned `polymarket-us==0.1.2` SDK returns an order creation envelope with `executions[].order`; do not read an order ID, state, or fills from the top-level response.
- Polymarket US executable depth comes from the official SDK's
  `markets.book(market_slug)` endpoint, normalized by the local Python order
  service. The payload is a `marketData` envelope containing timestamped
  `bids` and `offers` with USD price and quantity. Do not substitute gateway
  display quotes, account caches, or the non-US CLOB API.
- Polymarket US portfolio positions are returned as a market-keyed map. Normalize them into the frontend's position array before rendering.
- Manual Polymarket orders are contract-sized FAK/IOC limit orders. A canceled or rejected order with zero `cumQuantity` is not a successful fill and should clearly report the exchange reason.
- The Polymarket US account endpoints are Cloudflare rate-limited. Balance and position caching is for dashboard reads only; it must not provide pricing, depth, or execution inputs.
- React position responses are external data. Confirm payload arrays and finite position sizes before rendering them. WebSocket cleanup must cancel pending reconnects so an intentionally closed socket does not reconnect after unmount.

## Automatic Paired Executor

- Automatic execution must preflight fresh executable books from both venues.
  Kalshi depth may only come from its fresh WebSocket book; Polymarket US depth
  must come from `markets.book(market_slug)`. Size against the worst level
  needed, not a displayed quote or aggregate dashboard liquidity.
- Preserve contract, profit-margin, duration, duplicate-trade, maximum amount,
  and execution-count limits. Kalshi depth is fixed-point: floor it when
  ordering whole contracts; never round up.
- Execute complementary outcomes only. For a tracked competitor, Kalshi YES
  pairs with Polymarket NO, and Kalshi NO pairs with Polymarket YES. Resolve
  the exact Polymarket native LONG/SHORT side from that selected competitor
  mapping. Do not always resolve the tracked competitor when the strategy
  requires the opponent's outcome.
- Parse current response fields: Kalshi fill quantity is `fill_count_fp`; a
  Kalshi NO fill price is the complement of its single YES-book price;
  Polymarket's average fill is `executions[].order.avgPx`.
- Persist a `submitting` record before either order, and durably update each
  acknowledgement before submitting the other leg. On startup, halt execution
  if an unresolved `submitting` record exists; reconcile before re-enabling.
- On unequal fills, attempt a bounded IOC/FAK neutralization only within the
  persisted `neutralization_max_loss_cents` setting (default: 5 cents per
  contract). If it cannot fully close the residual, record it durably, disable
  auto-trading, and alert through the configured Telegram client.

## Validation

- For Rust changes, run `cargo test` from `rust-backend/`.
- For frontend changes, run `npm run lint` and `npm run build` from `web/`.
- For Python order-service changes, run the offline
  `python3 -m pytest -q test_market_book.py`. Do not run `test_service.py` as
  routine validation; it can access live account data and order endpoints.
- Do not enable automatic trading or submit test orders as part of routine development validation.
