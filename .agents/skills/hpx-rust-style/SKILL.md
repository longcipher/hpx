---
name: hpx-rust-style
description: Review rules enforced beyond AGENTS.md. Use when writing, refactoring, or reviewing Rust in hpx before opening or updating a PR.
---

# hpx Rust Style

AGENTS.md still applies. This file only adds rules that reviewers enforce.

## Scope

- Before pushing, read the full diff and revert every hunk the PR description does not explain.
- Refactors keep behavior identical. If behavior must change, say so in the PR description.
- No unused functions, parameters, variants, fields, config knobs, CLI flags, or cargo features.
- Do not add backwards-compatibility aliases, deprecated wrappers, or compatibility shims.

## Reuse before writing

- Use constructors the type already has instead of struct literals.
- Prefer the existing abstractions: `scc` for concurrent maps, `arc-swap` for read-heavy shared state, `bytes` for network buffers, `winnow` for parsers, `governor` for rate limits.
- When the same logic appears twice, extract one helper.

## Keep it small

- No wrapper, trait, enum, or type alias with a single use. The exception is a type alias that replaces `#[allow(clippy::type_complexity)]`.
- No caches, interior mutability, or format versioning without a measured need.
- Remove unused parameters instead of `_`-prefixing them.
- Keep items `pub(crate)` or private unless another crate uses them.

## Public API and errors

- A new trait method needs a default or no-op impl unless every implementor must provide it.
- Re-export types that appear in public signatures from the crate that exposes the signature.
- Use typed errors (`thiserror` in crates, `eyre` in binaries); never `Result<_, String>`.
- Never discard a fallible result with `.ok()` or `let _ =`. Ignore only the specific variant you expect.
- Use `checked_*` and return an error for index and length math; `try_from` for conversions, `as` only when truncation is impossible.
- Use `#[expect(lint, reason = "...")]` instead of `#[allow]`.

## Comments and docs

- A comment states one non-obvious fact. Do not restate the code or narrate the change.
- Comments end with periods, except URLs.
- When behavior changes, update doc comments, step lists, and error messages in the same PR.
- Put doc comments before attributes (`///` before `#[derive]`); module docs go at the top of the module file (`//!`).
- Never put imports inside a function unless `#[cfg(...)]` gating requires it. In test modules, use `use super::*;`.

## Control flow and types

- Prefer `let Some(x) = x else { return };`.
- Use combined `if let` chains (`if let Some(x) = x && let Some(y) = y`) instead of nesting.
- Never use `ref` / `ref mut` in patterns as the first resort.
- Prefer map entry APIs (`entry`, `or_insert`, `or_insert_with`) and turbofish over type annotations.
- Make invalid states unrepresentable: put variant-specific data inside enum variants, not correlated `bool`/`Option` fields.
- A function that does not use `self` is a free function.

## Tests

- Every new branch, guard, error path, and unwind or reopen path gets a test in the same PR.
- Assert the exact error or message, not just `is_err()`.
- In e2e tests, wait on conditions instead of sleeping, and add missing helpers to the harness instead of copying them into tests.
- Before deleting a test as redundant, show that another test fails when the line it guarded breaks.
- Use fuzz tests for parsing and serialization, and property tests for invariants.
- Do not commit `#[ignore]` manual perf tests. Write a bench.

## Performance

- Do not clone large values to read them. Borrow, `take`, or share through `Arc`; getters return references.
- Cap any length from untrusted input before `Vec::with_capacity` or a loop.
- Do not read the same state value twice; build optional data lazily.
- Every `unsafe` block carries a `// SAFETY:` comment stating the invariant that makes it sound.

## Dependencies and CI

- Workspace dependencies have default features off where the upstream crate supports it. Check that each touched crate builds on its own, with and without its features.
- Dependency bumps respect the workspace MSRV and never downgrade a dependency to silence a build error.
- Do not add `cargo test` to commit hooks; tests stay in `just test` and `just test-all`.
