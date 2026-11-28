# hpx-browser Agent Instructions

Additive rules for the browser engine. The workspace `AGENTS.md` in the repository root still applies; if these rules conflict with it, the root file wins.

## Scope

- This crate owns the headless browser, CDP client/server, DOM/layout engines, and emulation integration. Do not move browser-specific policy into `crates/hpx`.
- Browser fingerprint profiles come from `hpx-emulation`. Never hard-code JA3/JA4 or TLS profile values in call sites here.
- The generated profile modules under `src/emulation` may use glob imports; every other module must name its imports.

## Event loops and async

- Every wait is bounded and observes shutdown. A spawned task enters the caller's span for the whole closure, not only as the parent of a debug child.
- Do not sleep to synchronize with the page. Wait on the condition (frame loaded, selector present, request finished) through the CDP harness.
- Log local faults with `error!` and peer/CDP faults with `warn!`. Do not log at `info!` inside polling loops.

## Tests

- Add missing helpers to the test harness instead of copying setup code into individual tests.
- Assert exact errors and CDP payloads, not `.contains(...)` on debug strings.
- Page and memory tests that take minutes are excluded by the default nextest filter; `just test-slow` runs them, and they stay in `just test-all`.

## Performance

- Reuse buffers across page loads; avoid cloning DOM subtrees just to read them.
- Cap any length read from CDP messages or page content before allocating.
- Benchmark changes with the existing `page_throughput`, `navigation`, and `memory` benches and report measured results.
