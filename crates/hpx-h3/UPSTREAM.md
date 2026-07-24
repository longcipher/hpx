# Upstream Tracking

## Source

- Upstream: <https://github.com/hyperium/h3>
- Fork point: h3 v0.0.8 (2025-05-06) / h3-quinn from the same repository
- Vendored: 2026-07-21 (commit `d0829a0`) as `hpx-h3`
- Quinn transport merged: 2026-07-21 (commit `ce4247a`), consolidating `hpx-h3-quinn` into `hpx-h3`

## Custom Patches

- RFC 9220 WebSocket over HTTP/3 support
- Quinn QUIC transport backend (merged from hpx-h3-quinn)
- Is0rtt trait for 0-RTT detection
- Edition 2024 migration
- hotpath profiling instrumentation (feature-gated)

## Merge Strategy

- Manual cherry-pick of upstream changes
- Review upstream CHANGELOG before merging
- Run full test suite after merge (`cargo nextest run -p hpx-h3 --all-features`)

## Known Divergences

- Lint configuration: project-specific clippy allows in `Cargo.toml` (single source of truth)
- Feature gates: added `quinn`, `hotpath`; retained `i-implement-a-third-party-backend-and-opt-into-breaking-changes`
- Workspace dependencies: uses `workspace = true` for all shared deps
- Edition: 2024 (upstream uses 2021)
