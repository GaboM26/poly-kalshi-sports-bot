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
- Polymarket US portfolio positions are returned as a market-keyed map. Normalize them into the frontend's position array before rendering.
- Manual Polymarket orders are contract-sized FAK/IOC limit orders. A canceled or rejected order with zero `cumQuantity` is not a successful fill and should clearly report the exchange reason.
- The Polymarket US account endpoints are Cloudflare rate-limited. Balance and position caching is for dashboard reads only; it must not provide pricing, depth, or execution inputs.
- React position responses are external data. Confirm payload arrays and finite position sizes before rendering them. WebSocket cleanup must cancel pending reconnects so an intentionally closed socket does not reconnect after unmount.

## Next Priority: Arbitrage Executor

- Automatic paired execution is currently intentionally blocked before submission when Polymarket US executable liquidity cannot be verified. Do not remove this gate merely because a displayed quote is available.
- Before enabling execution, establish a live, executable quote/depth source for both venues and calculate quantity plus worst-case fill price from that data.
- Keep execution decisions independent of cached dashboard balances and positions. Use direct order acknowledgements and fills to update inventory, then reconcile account state at an exchange-supported rate.
- Preserve contract, profit-margin, duration, duplicate-trade, and execution-count limits. Record each submission, acceptance, fill, cancellation, rejection, and any one-leg exposure durably.
- Define and implement explicit partial-fill and one-leg-failure handling before submitting a pair. The executor must stop or neutralize residual exposure according to a deliberate, price-bounded policy rather than silently continuing.

## Validation

- For Rust changes, run `cargo test` from `rust-backend/`.
- For frontend changes, run `npm run lint` and `npm run build` from `web/`.
- For Python order-service changes, run `python3 test_service.py` only against an intentionally configured local service; it can access live account data and order endpoints.
- Do not enable automatic trading or submit test orders as part of routine development validation.
