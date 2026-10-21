# hpx Agent Instructions

## Non-negotiables

- Surgical edits only: every changed line must trace to the request. No drive-by refactors, no reformatting untouched code, no deleting pre-existing dead code (mention it instead).
- Fix root causes. Never silence a warning, weaken a check, or patch a symptom to make a gate pass.
- Never report completion while any `just` gate below fails. Report the failure verbatim and stop.
- Stop and ask when a request is ambiguous, conflicts with existing conventions, or requires credentials, publishing, or destructive operations.
- When a correction reveals a missing rule, add one specific rule here ("always X for Y", never "be careful with Y"); delete rules that never catch real errors.

## Scope

- `crates/hpx` is the core HTTP client; `crates/hpx-browser` the headless browser engine; `crates/hpx-dl` the segmented download engine; `crates/hpx-emulation` the browser fingerprint profiles; `crates/hpx-h3` a vendored `hyperium/h3` fork; `crates/hpx-streams` the response-stream codecs; `crates/yawc` (package `hpx-yawc`) the WebSocket client/server.
- `bin/hpx-cli` is the CLI; `bin/hpxless` is the standalone CDP-compatible browser server.
- `crates/yawc/yawcc` is its own workspace root and is not part of this workspace.
- The `fuzz` directories under `crates/*/fuzz` are excluded from the workspace on purpose: they pin their own graph, need nightly, and use `libfuzzer-sys`.
- `specs/` holds feature design specs and `docs/` holds design docs; both are historical context, not runtime configuration.
- English only in code, comments, docs, and commit messages.

## Execution

- Parallelize aggressively: dispatch subagents for independent research, exploration, and implementation.
- NEVER run two `cargo` commands concurrently, and never overlap file edits with a running `cargo` command (`target/` file lock): finish edits, wait, then run.
- NEVER use `git worktree`. Edit the current branch in place.
- TDD for behavior changes: failing test first, smallest change to pass, then run `just mutate` and fix surviving mutants.

## Verification loop

After each feature or bug fix, in order; any failure means the task is not done:

```bash
just format
just lint
just test
just test-all
just mutate
```

`just test` runs `cargo nextest`, which skips doctests. Doctests are part of the contract: run `cargo test --workspace --all-features --doc` as well.

## Workspace rules

- Never hand-type dependency versions. Add per-crate deps with `cargo add <crate> -p <crate>`. Shared deps go into root `[workspace.dependencies]` — numeric versions only, no features — and are referenced with `dep = { workspace = true }`.
- The local crates in `[workspace.dependencies]` must keep literal `version` fields: Cargo cannot inherit `[workspace.package].version` there. On a version bump, update `[workspace.package]` and those lines together.
- The toolchain is pinned to nightly in `rust-toolchain.toml`. Use `cargo +nightly fmt` / `cargo +nightly clippy` as the Justfile does.
- `.cargo/config.toml` sets `rustc-wrapper = "kache"` and `-Zpolonius=next`. Unset the compiler wrapper when invoking clippy manually (`RUSTC_WRAPPER=`), exactly as the Justfile does; CI also clears `RUSTC_WRAPPER` and `CARGO_BUILD_RUSTC_WRAPPER`.
- On aarch64 macOS the config pins `clang` plus `ld64.lld`; do not switch the linker without re-measuring link times.
- Workspace lints deny `unwrap_used`, `expect_used`, `panic`, `allow_attributes`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, and `print_stderr`. The only escape hatch is `#[expect(lint, reason = "...")]`; the binaries use it for stdout output.
- `wildcard_imports` is allowed only for the generated browser profile modules; every other module must name its imports.
- `cognitive_complexity` is off on purpose: turning it on surfaces dozens of pre-existing violations in the protocol parsers and state machines and needs a separate, measured refactor.
- `cargo shear` is the unused-dependency gate; `getrandom` is declared for wasm builds only and is listed in `[workspace.metadata.cargo-shear]`.
- Mutation testing is configured in `.cargo/mutants.toml` (nextest as the test tool, vendored `crates/hpx-h3` excluded). A surviving mutant means a missing assertion.
- Building the browser stack needs the native packages `libfreetype-dev` and `libfontconfig1-dev`.

## Dependency choices

- This workspace *is* the HTTP client: never add `reqwest` or `hyper` as a new direct dependency.
- TLS backends: BoringSSL is the default, with `rustls-tls` and `openssl-tls` alternatives. Keep the backend behind the existing abstraction.
- WebSocket: the `hpx-yawc` crate, not `tokio-tungstenite`.
- Browser fingerprints: `hpx-emulation` supplies the profiles; do not hard-code JA3/JA4 values in call sites.
- Concurrent map/set: `scc`; read-heavy shared state: `arc-swap`. Never `dashmap`.
- Rate limiting: `governor`; parsers: `winnow`; network buffers: `bytes`.
- `hpx-dl` persists its queue in SQLite through `sqlx` runtime queries (`sqlx::query`, never the compile-time `query!` macros).
- Errors: `thiserror` in `crates/`, `eyre` in `bin/`. Concrete error types through `?`; no string errors.
- Logging: `tracing` only. Never `log`.
- Forbidden by default: `anyhow`, `log`, `reqwest`, `dashmap`.

## Code conventions

- Every new capability lands behind a `hpx` feature flag; the `hft` and `stealth` presets are the supported bundles, so a new flag must state whether it belongs in either.
- `--all-features` builds must stay green on macOS: BoringSSL FIPS is intentionally disabled because its `objcopy` step needs GNU `objcopy`.
- `hpx-h3` is a vendored fork: keep changes minimal and upstream-shaped, and never enable mutation testing on it.
- Fuzz targets are per crate under `crates/<crate>/fuzz`; build them with `just fuzz-build` and gate them with `just fuzz-smoke`. A crash is a finding, not a flake.
- Publishing order is dependency order: `hpx-yawc`, `hpx-h3`, `hpx`, `hpx-browser`, `hpx-dl`, `hpx-emulation`, `hpx-streams`, `hpxless`, `hpx-cli`. Use `just publish`.
