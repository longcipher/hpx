# hpx-h3 Agent Instructions

Additive rules for the vendored `hyperium/h3` fork. The workspace `AGENTS.md` in the repository root still applies; if these rules conflict with it, the root file wins.

## Scope

- This is a vendored fork: keep every change minimal and upstream-shaped. Do not restructure modules or rename upstream items for local style preferences.
- Mutation testing is disabled here by design. Do not add a mutation recipe for this crate or remove it from `exclude_globs` in `.cargo/mutants.toml`.
- Protocol changes must cite the relevant RFC section in the PR description and carry a conformance test.

## Tests and performance

- Use property tests for header block round-trips and fuzz tests for parsing and serialization.
- Assert exact decoded values and error codes; do not assert only that decoding failed.
- Do not add allocations to hot codec paths without a measured reason. Benchmark with the existing throughput bench before and after the change.
