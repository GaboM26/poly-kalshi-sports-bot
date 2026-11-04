# Adding a strategy

A strategy is one backend folder, one frontend folder, a config section and
two registry lines. Nothing in `api/`, `clients/` or the shared UI changes.

## Backend (`rust-backend/src/strategies/<id>/`)

1. Create the folder with a `mod.rs` and a `strategy.rs` holding a struct that
   implements `strategies::Strategy`:
   - `id()`: the stable id, e.g. `"my_strategy"`. It is the URL prefix
     (`/api/my_strategy/...`) and the web registry key.
   - `routes()`: a `RouteTable` of relative paths, each handler
     `.with_state(self.clone())`. Handlers take `State<Arc<YourStrategy>>`.
   - `start()`: spawn the background loops. Called once at startup.
   - `feed_client(tx)`: send the strategy's initial state to one frontend
     WebSocket client, then spawn the per-client tasks that keep pushing its
     messages; return their handles. Messages are `models::WsMessage`
     variants; add a variant (and its TypeScript type) for new message kinds.
   - `hedge_pairs()`: only if the strategy holds Kalshi/Polymarket hedges
     that the shared positions view should group. The default is none.
2. Build it from the shared `Platform` (exchange clients, storage, metrics,
   Discord) in a `new(platform: &Arc<Platform>)`; never create another client.
3. Add one line to `strategies::build_all` in `strategies/mod.rs`.
4. Put its tunables in a `[<id>]` config section (`config/mod.rs`), with
   defaults, validation and a documented block in `config.example.toml`. The
   `the_example_config_parses_and_documents_the_defaults` test should cover it.
5. Persist state through `services::storage` (add a repo file there).

Safety rules from CLAUDE.md apply unchanged: fees only through `core::fees`,
fresh books only, persisted execution state before any order fires, bounded
recovery, and auto-trading off by default.

## Frontend (`web/src/strategies/<id>/`)

1. `api.ts` with a `<id>Url(path)` helper over `apiUrl('/api/<id>' + path)`.
2. A page component taking `StrategyPageProps` (`live` = the shared WebSocket
   data). Use `shared/components/OrderPanel` at the bottom to show positions.
3. Optional header controls component.
4. One entry in `web/src/strategies/registry.ts`.

## Verify

- `cargo test` (from `rust-backend/`) and `npm run build` (from `web/`).
- Restart the stack, check the log, and run
  `cargo run --example probe_api_shapes -- compare <before.json> <after.json>`
  to confirm the existing API and WebSocket contract did not change.
