# Ensure rustup-managed nightly cargo is used (system /usr/bin cargo is stable)
export PATH := home_directory() / ".cargo/bin" + ":" + env("PATH", "")

# Default recipe to display help
default:
  @just --list

# Format all code
format:
  rumdl fmt .
  cargo sort -w -g
  cargo fmt --all

# Auto-fix linting issues
fix:
  rumdl check --fix .
  RUSTC_WRAPPER= cargo clippy --fix --all --all-targets --allow-dirty

# Run all lints
lint:
  rumdl check .
  cargo sort -w -g -c
  cargo fmt --all -- --check
  RUSTC_WRAPPER= cargo clippy --all --all-targets -- -D warnings
  cargo workspace-inheritance-check
  just check-agents-md
  just check-config-docs
  just check-cn
  just no-test-deps
  just check-mutation-scope

# Check that AGENTS.md documents every workspace crate
check-agents-md:
  #!/usr/bin/env bash
  set -euo pipefail
  errors=0
  for manifest in crates/*/Cargo.toml bin/*/Cargo.toml; do
    crate="$(sed -n 's/^name = "\([^"]*\)"$/\1/p' "$manifest" | head -1)"
    [ -n "$crate" ] || continue
    if ! grep -qF -- "$crate" AGENTS.md; then
      echo "MISSING: workspace crate '${crate}' is not mentioned in AGENTS.md" >&2
      errors=$((errors + 1))
    fi
  done
  for rule in .agents/skills/hpx-rust-style/SKILL.md crates/hpx-browser/AGENTS.md crates/hpx-h3/AGENTS.md; do
    if [ ! -f "$rule" ]; then
      echo "MISSING: AGENTS.md references '$rule' but the file does not exist" >&2
      errors=$((errors + 1))
    fi
  done
  if [ "$errors" -gt 0 ]; then
    echo "FAIL: ${errors} documentation gap(s) found" >&2
    exit 1
  fi
  echo "OK: AGENTS.md documents every workspace crate and every referenced rule file"

# Verify that config keys claimed in AGENTS.md are active, not commented out
check-config-docs:
  #!/usr/bin/env bash
  set -euo pipefail
  errors=0
  for key in rustc-wrapper incompatible-rust-versions global-min-publish-age; do
    if ! grep -Eq "^[[:space:]]*${key}[[:space:]]*=" .cargo/config.toml; then
      echo "MISSING: .cargo/config.toml does not activate '${key}'" >&2
      errors=$((errors + 1))
    fi
  done
  if ! grep -Eq '^[[:space:]]*rustflags[[:space:]]*=.*polonius' .cargo/config.toml; then
    echo "MISSING: .cargo/config.toml does not activate polonius" >&2
    errors=$((errors + 1))
  fi
  if [ "$errors" -gt 0 ]; then
    echo "FAIL: ${errors} documented config key(s) are inactive" >&2
    exit 1
  fi
  echo "OK: all documented config keys are active"

# Keep test-only crates out of the default dependency graph
no-test-deps:
  #!/usr/bin/env bash
  set -euo pipefail
  if cargo tree --package hpx -e=features,no-dev 2>/dev/null | grep -Eq 'arbitrary|proptest|test-fuzz'; then
    echo "error: test-only dependency leaked into the default hpx graph" >&2
    exit 1
  fi
  echo "OK: no test-only dependencies in the default hpx graph"

# Diagnostic only: check each feature combination for hpx-streams.
#
# This is NOT a CI gate. Known pre-existing debt: `hpx --no-default-features`
# has 18 compile errors, and `hpx-streams --features hotpath` alone trips
# cargo::unused_dependencies because no codec feature is enabled to use it.
# Do not weaken `build.warnings` to make this pass. `check-publish-features`
# is the blocking gate for isolated per-crate builds.
check-feature-matrix:
  cargo hack check -p hpx-streams --each-feature --locked

# Check that features propagate consistently across workspace crates
check-feature-propagation:
  zepter run check

# Run the prek commit hooks on every tracked file (requires prek)
hooks:
  prek run --all-files

# Run the fast test suite (nextest default filter)
test:
  cargo nextest run --workspace --all-features

# Run only the slow benchmark-style tests excluded by the default filter
test-slow:
  cargo nextest run --workspace --all-features --ignore-default-filter -E 'test(/memory_benchmark|reload_html_100_times_no_leak|create_and_drop_100_pages_no_leak/)'

# Doctests are skipped by nextest but are part of the contract
doctest:
  cargo test --workspace --all-features --doc

# Full test suite: fast path, slow path, and doctests
test-all: test test-slow doctest

# Run the hpx tests against the rustls backend.
#
# `--all-features` enables `boring-tls`, which compiles `tls/rustls.rs` out, so
# `just test` never runs the rustls backend. This feature set mirrors the one
# downstream rustls users build with, plus `keylog`.
test-rustls:
  cargo nextest run -p hpx --no-default-features --features rustls-tls,http1,http2,ws,keylog

# Build the workspace documentation
build-docs:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "$(uname)" = "Darwin" ]; then
    RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --workspace --no-deps --document-private-items
  else
    RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --workspace --no-deps --document-private-items --all-features
  fi

# Run tests with coverage
test-coverage:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "$(uname)" = "Darwin" ]; then
    cargo tarpaulin --workspace --timeout 300
  else
    cargo tarpaulin --all-features --workspace --timeout 300
  fi

# Mutation testing is intentionally absent from the default loop and from CI.
#
# Whole-workspace mutation runs are PROHIBITED here: a `cargo mutants` run over
# the whole workspace takes hours and rarely finishes, so it is never a gate and
# never a completion check. Only the lines the current change touched are
# mutated, within the /pb-build budget: at most 3 files, 60 mutants, 300 s.
#
#   just mutate                            # every changed .rs file
#   just mutate crates/core/src/parser.rs  # one specific file
#
# The budget is verified before the run starts, so an over-large change fails
# immediately instead of grinding for hours. A surviving mutant is a test
# failure: strengthen the assertion, or record an exemption in
# `.cargo/mutants.toml` under `exclude_re` with a one-line reason.
#
# Mutate the lines this change touched, within the /pb-build budget.
mutate *args:
  #!/usr/bin/env bash
  set -euo pipefail
  max_files=3
  max_mutants=60
  max_seconds=300

  base=""
  upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || true)
  for ref in "$upstream" origin/main origin/master main master; do
    [ -n "$ref" ] || continue
    if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
      base=$(git merge-base HEAD "$ref" 2>/dev/null || true)
      [ -n "$base" ] && break
    fi
  done
  [ -n "$base" ] || base=HEAD

  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@" | sed '/^[[:space:]]*$/d' > "$work/files"
  else
    git diff --name-only --diff-filter=ACMR "$base" -- '*.rs' | sort -u > "$work/files"
  fi
  file_count=$(grep -c . "$work/files" || true)
  if [ "$file_count" -eq 0 ]; then
    echo "error: no changed Rust files against $base; there is nothing to mutate." >&2
    echo "       Whole-workspace mutation is prohibited: it takes hours and rarely finishes." >&2
    exit 2
  fi
  if [ "$file_count" -gt "$max_files" ]; then
    echo "error: $file_count files exceed the ${max_files}-file mutation budget." >&2
    echo "       Mutate fewer files at a time; never raise the budget, never run the workspace." >&2
    exit 3
  fi

  : > "$work/diff"
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    git diff --no-ext-diff --no-textconv "$base" -- "$file" >> "$work/diff" || true
  done < "$work/files"
  if ! grep -q '^diff --git ' "$work/diff"; then
    echo "error: the diff against $base is empty; there is nothing to mutate." >&2
    exit 2
  fi
  if [ "$(uname)" = "Darwin" ]; then
    mutate_flags=""
  else
    mutate_flags="--all-features"
  fi
  RUSTC_WRAPPER= cargo mutants --workspace ${mutate_flags:+"$mutate_flags"} --in-diff "$work/diff" --list --no-times > "$work/list" 2>&1 \
    || { cat "$work/list" >&2; echo "error: cargo-mutants could not list the mutants." >&2; exit 1; }
  mutant_count=$(grep -Ec ':[0-9]+:[0-9]+:' "$work/list" || true)
  if [ "$mutant_count" -gt "$max_mutants" ]; then
    echo "error: $mutant_count mutants exceed the ${max_mutants}-mutant budget." >&2
    echo "       Narrow the change; never raise the budget, never fall back to a workspace run." >&2
    exit 3
  fi
  if [ "$mutant_count" -eq 0 ]; then
    echo "ok: no mutants in the changed lines."
    exit 0
  fi

  echo "mutating $mutant_count mutant(s) in $file_count file(s) (budget: $max_mutants mutants / ${max_seconds}s)"
  if command -v timeout >/dev/null 2>&1; then
    timeout "$max_seconds" env RUSTC_WRAPPER= cargo mutants --workspace ${mutate_flags:+"$mutate_flags"} --in-diff "$work/diff" --timeout-multiplier 2 --no-times
  elif command -v gtimeout >/dev/null 2>&1; then
    gtimeout "$max_seconds" env RUSTC_WRAPPER= cargo mutants --workspace ${mutate_flags:+"$mutate_flags"} --in-diff "$work/diff" --timeout-multiplier 2 --no-times
  else
    echo "warning: neither 'timeout' nor 'gtimeout' is available; the ${max_seconds}s wall-clock cap is not enforced." >&2
    echo "         Install coreutils ('brew install coreutils') to get 'gtimeout'." >&2
    env RUSTC_WRAPPER= cargo mutants --workspace ${mutate_flags:+"$mutate_flags"} --in-diff "$work/diff" --timeout-multiplier 2 --no-times
  fi

# Fail if an unbounded mutation command reappears in a build or CI file.
# Whole-workspace mutation runs take hours and rarely finish, so they are
# prohibited: the only permitted forms are a diff-scoped (`--in-diff`), a
# file-scoped (`--file`), or a branch-scoped (`--since`) run.
#
# Check that no build or CI file regains an unbounded mutation command.
check-mutation-scope:
  #!/usr/bin/env bash
  set -euo pipefail
  status=0
  for file in Justfile justfile Makefile makefile .github/workflows/*.yml .github/workflows/*.yaml; do
    [ -f "$file" ] || continue
    offenders=$(grep -nE 'cargo[[:space:]]+mutants' "$file" \
      | grep -vE '^[0-9]+:[[:space:]]*#' \
      | grep -vE -- '--in-diff|--file|--list|--since|-f[[:space:]]' \
      || true)
    if [ -n "$offenders" ]; then
      echo "error: $file runs an unbounded mutation command:" >&2
      printf '%s\n' "$offenders" >&2
      echo "       Whole-workspace mutation runs are prohibited. Use the diff-scoped 'just mutate'." >&2
      status=1
    fi
  done
  exit "$status"

# Run mutation testing (canonical alias of `mutate`)
mutation: mutate

# Check that the workspace compiles with the supported feature set
check-feature:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "$(uname)" = "Darwin" ]; then
    cargo check --workspace
  else
    cargo check --workspace --all-features
  fi

# Check each publishable crate against its *own* declared feature set.
#
# `cargo check --workspace --all-features` unifies features across the whole
# workspace, so a cfg pair that only holds when one crate enables a feature and
# another does not is invisible to it. `cargo publish --verify` instead builds
# each packaged crate in isolation against registry dependencies, which is where
# those combinations surface: releases v2.5.33 and v2.5.34 both failed at
# `hpx-cli` because `hpx/http3` without `hpx/socks` was never compiled by CI.
#
# Do not add `--all-targets` here: dev-dependencies enable extra features and
# would mask the very combinations this gate exists to catch.
check-publish-features:
  #!/usr/bin/env bash
  set -euo pipefail
  crates=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name' | sort)
  failures=0
  for crate in $crates; do
    if RUSTC_WRAPPER= cargo check -p "$crate" --locked; then
      echo "  OK: $crate"
    else
      echo "  FAIL: $crate" >&2
      failures=$((failures + 1))
    fi
  done
  if [ "$failures" -ne 0 ]; then
    echo "FAIL: $failures crate(s) do not compile in isolation; fix before publishing" >&2
    exit 1
  fi
  echo "OK: every publishable crate compiles with its own feature set"

# Check all targets compile
check:
  cargo check --all-targets --all-features

# Build entire workspace
build:
  cargo build --workspace

# Optimized build with symbols, for profiling
profiling:
  RUSTFLAGS="-C target-cpu=native" cargo build --workspace --profile profiling

# Maximum-performance build (fat LTO, one codegen unit)
maxperf:
  RUSTFLAGS="-C target-cpu=native" cargo build --workspace --profile maxperf

# Verify all workspace crates package from their published layout
publish-check:
  cargo package --workspace --allow-dirty

# Publish all crates to crates.io in dependency order
publish: check-publish-features
  #!/usr/bin/env bash
  set -euo pipefail
  VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
  echo "Publishing workspace crates v$VERSION..."
  echo ""
  # Dependency order: hpx-yawc, hpx-h3 → hpx → {hpx-browser, hpx-dl, hpx-emulation, hpx-streams, hpx-transport} → {hpxless, hpx-cli}
  CRATES="hpx-yawc hpx-h3 hpx hpx-browser hpx-dl hpx-emulation hpx-streams hpx-transport hpxless hpx-cli"
  for crate in $CRATES; do
  	# Check if already published at this version
  	if cargo search "$crate" --limit 1 2>/dev/null | grep -q "^$crate = \"$VERSION\""; then
  		echo "  ✓ $crate@$VERSION already published, skipping"
  		continue
  	fi
  	echo "  Publishing $crate..."
  	OUTPUT=$(cargo publish -p "$crate" --allow-dirty 2>&1) && RC=0 || RC=$?
  	if [ $RC -eq 0 ] || echo "$OUTPUT" | grep -qi "already exists"; then
  		echo "  ✓ $crate published (or already exists)"
  		sleep 30
  	else
  		echo "  ✗ $crate failed:"
  		echo "$OUTPUT"
  		exit 1
  	fi
  done
  echo ""
  echo "All crates published."

# Check for Chinese characters in English-only sources and docs
check-cn:
  #!/usr/bin/env bash
  set -euo pipefail
  if rg --line-number --column "\p{Han}" --glob '*.md' --glob '*.toml' --glob '!target/**' --glob '!specs/**'; then
    echo "error: Chinese characters found in English-only files" >&2
    exit 1
  fi

# Full CI check
ci: lint test-all build-docs check-publish-features

# ============================================================
# Maintenance & Tools
# ============================================================

# Clean build artifacts
clean:
  cargo clean

# Install all required development tools
setup:
  cargo install cargo-sort
  cargo install cargo-workspace-inheritance-check
  cargo install cargo-nextest
  cargo install cargo-llvm-cov
  cargo install cargo-deny
  cargo install cargo-mutants
  cargo install cargo-tarpaulin
  cargo install cargo-fuzz
  cargo install cargo-audit
  cargo install cargo-outdated
  cargo install cargo-hack
  cargo install zepter
  cargo install rumdl

# Generate documentation for the workspace
docs: build-docs

# Run security audit
audit:
  cargo audit

# Check for outdated dependencies
outdated:
  cargo outdated

# Run clippy with all lints
clippy-all:
  RUSTC_WRAPPER= cargo clippy --all --all-targets --all-features -- -W clippy::pedantic -W clippy::nursery -W clippy::cargo

# Run tests with nextest
test-nextest:
  cargo nextest run --workspace --all-features

# Run miri on unsafe code
miri:
  cargo miri test --all-features

# Update all dependencies
update-deps:
  cargo update

# Check licenses
license-check:
  cargo deny check licenses

# Check advisories
advisory-check:
  cargo deny check advisories

# Check bans
ban-check:
  cargo deny check bans

# Full check (lint + test + build + audit)
full-check: lint test build audit

# ---------------------------------------------------------------------------
# Fuzzing
# ---------------------------------------------------------------------------
#
# Fuzz crates live in `crates/<crate>/fuzz` and are excluded from the main
# workspace (see `exclude` in Cargo.toml), so each one needs an explicit recipe.
#
# `cargo fuzz run` is used for building and listing, but the smoke recipe invokes
# the compiled binary directly: cargo-fuzz does not always reap the target
# process promptly, so a `-max_total_time` run would sit there after libFuzzer
# has already exited.

# Build every fuzz target without running it
fuzz-build:
  #!/usr/bin/env bash
  set -euo pipefail
  # ASAN is incompatible with statically-linked musl libc, so never build
  # fuzz targets for a `-musl` target even if the toolchain host defaults to
  # one (some CI runners do). `cargo fuzz` hard-codes the host target and
  # ignores CARGO_BUILD_TARGET, so we pass `--target` explicitly; the glibc
  # target keeps ASAN working.
  triple="$(rustc -vV | sed -n 's/^host: //p')"
  triple="${triple/%musl/gnu}"
  rustup target add "$triple" 2>/dev/null || true
  export CARGO_BUILD_TARGET="$triple"
  for dir in crates/*/fuzz; do
    [ -d "$dir" ] || continue
    # cargo-fuzz refuses to run when the corpus or artifact directory is
    # missing, so create them up front.
    for target in $(cargo fuzz list --fuzz-dir "$dir"); do
      mkdir -p "$dir/corpus/$target" "$dir/artifacts/$target"
    done
    echo "==> building $dir"
    cargo fuzz build --target "$triple" --fuzz-dir "$dir"
  done

# List every fuzz target across the workspace
fuzz-list:
  #!/usr/bin/env bash
  set -euo pipefail
  for dir in crates/*/fuzz; do
    [ -d "$dir" ] || continue
    crate=$(basename "$(dirname "$dir")")
    echo "==> $crate"
    cargo fuzz list --fuzz-dir "$dir"
  done

# The compiled binary is invoked directly rather than through `cargo fuzz run`:
# cargo-fuzz does not reliably reap the target process, so a `-max_total_time`
# run would hang after libFuzzer has already exited.

# This is the CI gate: long enough to catch a trivially reachable panic without
# pretending to be a real campaign. A non-zero exit is a finding, not a flake --
# libFuzzer exits non-zero on a crash and writes the input to the artifact dir.

# Smoke-run every target for `fuzz-time` seconds (default 20)
fuzz-smoke fuzz-time="20":
  #!/usr/bin/env bash
  set -uo pipefail
  triple=$(rustc -vV | sed -n 's/^host: //p')
  # ASAN is incompatible with statically-linked musl libc; mirror the
  # fuzz-build override so the prebuilt binaries are found at the right path.
  triple="${triple/%musl/gnu}"
  export CARGO_BUILD_TARGET="$triple"
  failures=0
  for dir in crates/*/fuzz; do
    [ -d "$dir" ] || continue
    crate=$(basename "$(dirname "$dir")")
    for target in $(cargo fuzz list --fuzz-dir "$dir" 2>/dev/null); do
      binary=""
      for candidate in \
        "$dir/target/$triple/release/$target" \
        "$dir/target/release/$target"
      do
        if [ -x "$candidate" ]; then
          binary="$candidate"
          break
        fi
      done
      if [ -z "$binary" ]; then
        echo "==> $crate/$target: SKIP (not built; run 'just fuzz-build')"
        continue
      fi
      echo "==> $crate/$target for {{fuzz-time}}s"
      if "$binary" \
        -artifact_prefix="$dir/artifacts/$target/" \
        -max_total_time="{{fuzz-time}}" \
        -timeout=40 \
        -rss_limit_mb=4096 \
        "$dir/corpus/$target" >/dev/null 2>&1
      then
        echo "    ok"
      else
        echo "    FAIL: crash input written to $dir/artifacts/$target"
        failures=$((failures + 1))
      fi
    done
  done
  if [ "$failures" -ne 0 ]; then
    echo "FAIL: $failures fuzz target(s) crashed"
    exit 1
  fi
  echo "OK: all fuzz targets survived the smoke run"

# Run a single target: just fuzz-one <crate> <target> [seconds]
fuzz-one crate target seconds="60":
  #!/usr/bin/env bash
  set -euo pipefail
  cargo fuzz build --fuzz-dir "crates/{{crate}}/fuzz"
  triple=$(rustc -vV | sed -n 's/^host: //p')
  binary=""
  for candidate in \
    "crates/{{crate}}/fuzz/target/$triple/release/{{target}}" \
    "crates/{{crate}}/fuzz/target/release/{{target}}"
  do
    if [ -x "$candidate" ]; then
      binary="$candidate"
      break
    fi
  done
  if [ -z "$binary" ]; then
    echo "no built binary for {{target}}" >&2
    exit 1
  fi
  exec "$binary" \
    -artifact_prefix="crates/{{crate}}/fuzz/artifacts/{{target}}/" \
    -max_total_time="{{seconds}}" \
    -timeout=25 \
    -rss_limit_mb=4096 \
    "crates/{{crate}}/fuzz/corpus/{{target}}"

# Longer local campaign: build, then smoke every target
fuzz fuzz-time="300":
  #!/usr/bin/env bash
  set -euo pipefail
  just fuzz-build
  just fuzz-smoke "{{fuzz-time}}"
