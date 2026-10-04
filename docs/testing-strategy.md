# Testing Strategy

This document describes the test layers in the `hpx` workspace, what each layer
is *for*, and how to run them. It is the reference for adding new tests: pick the
layer that matches the property you need to pin.

## Layers at a glance

| Layer | Tool | Where | What it proves |
|---|---|---|---|
| Unit | `#[test]` / `#[tokio::test]` | next to the code | Behaviour of one function on known inputs |
| Property | `proptest` | colocated `mod properties` | Invariants that hold for *every* input |
| Integration / E2E | `#[tokio::test]` in `crates/*/tests` | real loopback sockets | Whole-stack behaviour including framing and chunk boundaries |
| Fuzzing | `cargo-fuzz` / libFuzzer | `crates/*/fuzz` | Absence of panics and invariant violations on hostile input |
| Mutation | `cargo-mutants` | `.cargo/mutants.toml` | That the test suite actually *detects* faults |
| Coverage | `cargo-llvm-cov` | — | Where the gaps are (see "Known gaps") |

```bash
just test        # nextest across the workspace
just test-all    # same; kept for parity with `just ci`
just lint        # fmt, clippy, cargo-shear, inheritance, AGENTS.md version check
just build-docs  # rustdoc with -D warnings
just fuzz-build  # build every fuzz target
just fuzz-list   # list every fuzz target
just fuzz-smoke  # 20s per target; the CI gate
just fuzz 300    # 300s per target
just fuzz-one hpx-streams json_array 120
just mutate      # cargo-mutants over the workspace
just test-coverage
```

## Choosing a layer

**Unit test** when you can name the exact input and the exact expected output.
This is the default; most code should need nothing else.

**Property test** when the interesting cases are combinatorial: encoding
round-trips, chunk-boundary independence, "never panics on arbitrary input",
"preserves ordering and length". `proptest` shrinks a failure to a minimal
reproducer, which is why the workspace favours it over hand-enumerated examples
for codec and parser work.

Two hard rules learned the hard way here:

1. **Do not let `proptest` own a boundary.** A generated range can simply fail
   to sample the value at the limit, and the off-by-one mutant survives. See
   `csv_stream::tests::length_limits` — those cases are spelled out by hand
   precisely because `len in 1..8` never produced `len == 8` in 64 runs.
2. **Bound the helper loop.** Helpers that feed a codec chunk-by-chunk must have
   an absolute iteration cap (`MAX_HELPER_STEPS`). Without one, mutation testing
   turns every non-terminating mutant into a 25-second timeout instead of a fast
   failure.

**Integration / E2E test** when the property is about *how bytes arrive*, not
just what they mean. Every decoder in `hpx-streams` is a
`tokio_util::codec::Decoder` driven by chunk arrival, and the bugs live in the
state carried between `decode` calls. `crates/hpx-streams/tests/support/mod.rs`
serves a body in caller-specified slices over a real loopback socket, which is
the only way to reach those states deterministically.

**Fuzz test** when the input is genuinely unbounded — network bytes, `Set-Cookie`
headers, font data, metalink XML. See below.

## Fuzzing layout

Fuzz crates live in `crates/<crate>/fuzz` and are **excluded from the workspace**
(`exclude` in the root `Cargo.toml`). Without that exclusion every `cargo`
command inside a fuzz directory fails with *"current package believes it's in a
workspace when it's not"*.

Each target is a separate `[[bin]]` with `test = false`. Current targets:

| Target | Crate | Attack surface |
|---|---|---|
| `ws_frame` | `hpx-yawc` | RFC 6455 frame decoder, both roles, bulk and byte-at-a-time feeding |
| `ws_frame_roundtrip` | `hpx-yawc` | client-encode → server-decode must preserve opcode/fin/payload; all three mask implementations must agree |
| `ws_mask` | `hpx-yawc` | the three XOR-mask paths must agree with the byte-at-a-time reference at every alignment |
| `h1_response` | `hpx` | hostile HTTP/1 responses over a real socket, end to end through `hpx::Client` |
| `cookie_jar` | `hpx` | `Set-Cookie` parsing and read-back consistency |
| `metalink` | `hpx-dl` | Metalink v4 XML parser (RFC 5854) |

Fuzz crates pin their own dependency versions rather than using
`{ workspace = true }`, because they are outside the workspace.

### Requirements and gotchas

- **Nightly + `cargo-fuzz`** (`taiki-e/install-action` in CI).
- **`libxml2` must be installed.** libFuzzer uses `llvm-symbolizer` to render
  stack traces; if it cannot start, the target dies with `SIGPIPE` *before
  fuzzing a single input*, which looks like a crash but is not one. The `fuzz`
  CI job installs it.
- **`just fuzz-smoke` invokes the built binary directly**, not
  `cargo fuzz run`, because cargo-fuzz does not reliably reap the target: a
  `-max_total_time` run would sit there after libFuzzer had already exited.
- **A crash is a non-zero exit plus an input under
  `crates/<crate>/fuzz/artifacts/<target>/`.** CI uploads that directory on
  failure. The corpus directories are gitignored; seed corpora that are worth
  keeping should be committed deliberately.

## Mutation testing

`just mutate` (or `cargo mutants -p <crate>`) injects faults and checks that a
test fails. It is the only layer that answers *"would my tests notice?"*.

Two configuration traps, both fixed:

- `additional_cargo_test_args` in `.cargo/mutants.toml` used `--skip <name>`,
  which nextest removed. Every run aborted with *"unexpected argument `--skip`
  found"* before testing a single mutant. It now uses a filter expression.
- `cargo mutants` copies the build tree into `$TMPDIR`. On a machine where
  `/tmp` is a small tmpfs, that copy can exhaust the filesystem; set `TMPDIR` to
  a path on real disk.

## Doctests

`just test` uses nextest, which **does not run doctests**. They are therefore a
separate CI step (`cargo test --workspace --all-features --doc`). Several were
silently broken: `hpx-streams` used a `?` on `Client::new()` after that API
stopped returning a `Result`, and every doctest in `hpx-yawc`'s `frame.rs` still
imported `yawc::` after the crate was renamed to `hpx-yawc`.

## Known coverage gaps

Overall line coverage sits near 80%. The largest remaining gaps, in order of
missed lines:

- `hpx-browser`: `canvas/canvas2d.rs` (533), `dom.rs` (266),
  `chrome/browser.rs` (233), `canvas/text/mod.rs` (173)
- `hpx-dl`: `engine.rs` (413)
- `hpx`: `client/http/builder.rs` (293), `client/ws/backend_yawc.rs` (231),
  `client/request.rs` (238)
- `hpx-h3`: `connection.rs` (219), `quic.rs` (164)

`hpx-h3` is vendored and excluded from mutation testing (`exclude_globs` in
`.cargo/mutants.toml`).
