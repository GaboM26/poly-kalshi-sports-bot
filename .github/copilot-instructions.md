# Copilot Instructions

## Repository Layout

- `rust-backend/` is the Axum/Tokio API, arbitrage engine, exchange clients, SQLite storage, and WebSocket server.
- `web/` is the Vite, React, and TypeScript user interface.
- `poly-order-service/` is the FastAPI service that signs and submits Polymarket CLOB orders through `py-clob-client`.
- Root scripts start the development stack and build deployment packages. Keep the service ports aligned with the configuration: frontend `5173`, Rust API `8000`, and Python order service `8001`.

## Change Guidelines

- Treat all price, order, and position data as financial data. Preserve decimal precision, validate external inputs, and do not weaken order-size, profit-margin, duration, or execution-count safeguards.
- Keep secrets out of source control. Use `rust-backend/config.example.toml` for new configuration defaults and document required configuration in `README.md`; never add real credentials.
- Keep Rust API models, route handlers, frontend types, and API client calls consistent when an endpoint or payload changes.
- Keep the Python order-service request and response contracts compatible with the Rust Polymarket client before changing either service.
- Follow existing error-handling patterns. Surface exchange, signing, storage, and network failures rather than hiding them with fallback data.
- Preserve WebSocket message compatibility for the React client when changing the Rust WebSocket manager or opportunity models.

## Validation

- For Rust changes, run `cargo test` from `rust-backend/`.
- For frontend changes, run `npm run lint` and `npm run build` from `web/`.
- For Python order-service changes, run `python3 test_service.py` only against an intentionally configured local service; it can access live account data and order endpoints.
- Do not enable automatic trading or submit test orders as part of routine development validation.
