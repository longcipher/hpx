# hpx-transport Refactoring Task List

## Phase 1: Preparation and Module Cleanup

- [x] Initial workspace state check (`just lint`, `just test`)
- [x] Create refactored design document (`docs/hpx-transport-design.md`)
- [x] Remove redundant modules:
  - [x] `src/middleware.rs`
  - [x] `src/hooks.rs`
  - [x] `src/transport.rs`
  - [x] `src/http.rs`
- [x] Update `src/lib.rs` exports
- [x] Cleanup `Cargo.toml` dependencies (remove unused `tower`, `httpdate`, etc.)

## Phase 2: Implementation of New Components

- [x] Implementation of `src/error.rs` (Refined Error types)
- [x] Implementation of `src/auth.rs` (Refined Authentication trait and common strategies)
- [x] Implementation of `src/typed.rs` (Refined TypedResponse)
- [x] Implementation of `src/exchange.rs` (ExchangeClient and RestClient)
- [x] Implementation of `src/rate_limit.rs` (Token Bucket rate limiter)
- [x] Implementation of `src/websocket.rs` (Reconnecting Actor implementation)

## Phase 3: Verification and Integration

- [x] Fix internal code references and unit tests
- [x] Update examples (`examples/transport_example.rs`) to use new API
- [x] Fix type safety and trait object issues (e.g., `Authentication` clone and dyn compatibility)
- [x] Ensure `Cargo.toml` workspace inheritance is correct
- [x] Run `just format`
- [x] Run `just lint` across the workspace
- [x] Run `just test` across the workspace

## Phase 4: Documentation

- [x] Finalize English documentation
- [x] Update `README.md` if necessary
