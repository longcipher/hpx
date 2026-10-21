# Ensure rustup-managed nightly cargo is used (system /usr/bin cargo is stable)
export PATH := home_directory() / ".cargo/bin" + ":" + env("PATH", "")

# Default recipe to display help
default:
  @just --list

# Format all code
format:
  rumdl fmt .
  cargo sort -w -g
  cargo +nightly fmt --all

# Auto-fix linting issues
fix:
  rumdl check --fix .
  RUSTC_WRAPPER= cargo +nightly clippy --fix --all --all-targets --allow-dirty

# Run all lints
lint:
  rumdl check .
  cargo sort -w -g -c
  cargo +nightly fmt --all -- --check
  RUSTC_WRAPPER= cargo +nightly clippy --all --all-targets -- -D warnings
  cargo shear
  cargo +nightly workspace-inheritance-check
  just check-agents-md

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
  if [ "$errors" -gt 0 ]; then
    echo "FAIL: ${errors} workspace crate(s) are not documented in AGENTS.md" >&2
    exit 1
  fi
  echo "OK: AGENTS.md documents every workspace crate"

# Run tests
test:
  cargo nextest run --workspace --all-features

# Run the full test suite
test-full:
  cargo nextest run --workspace --all-features

# Run the full test suite
test-all: test-full

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

# Run mutation testing (cargo-mutants) to measure test-suite kill rate
mutate:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ "$(uname)" = "Darwin" ]; then
    cargo mutants --workspace
  else
    cargo mutants --workspace --all-features
  fi

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

# Check all targets compile
check:
  cargo check --all-targets --all-features

# Build entire workspace
build:
  cargo build --workspace

# Verify all workspace crates package from their published layout
publish-check:
  cargo package --workspace --allow-dirty

# Publish all crates to crates.io in dependency order
publish:
  #!/usr/bin/env bash
  set -euo pipefail
  VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
  echo "Publishing workspace crates v$VERSION..."
  echo ""
  # Dependency order: hpx-yawc, hpx-h3 → hpx → {hpx-browser, hpx-dl, hpx-emulation, hpx-streams} → {hpxless, hpx-cli}
  CRATES="hpx-yawc hpx-h3 hpx hpx-browser hpx-dl hpx-emulation hpx-streams hpxless hpx-cli"
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
ci: lint test-all build-docs

# ============================================================
# Maintenance & Tools
# ============================================================

# Clean build artifacts
clean:
  cargo clean

# Install all required development tools
setup:
  cargo install cargo-shear
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
  RUSTC_WRAPPER= cargo +nightly clippy --all --all-targets --all-features -- -W clippy::pedantic -W clippy::nursery -W clippy::cargo

# Run tests with nextest
test-nextest:
  cargo nextest run --workspace --all-features

# Run miri on unsafe code
miri:
  cargo +nightly miri test --all-features

# Check for unused dependencies
unused-deps:
  cargo shear

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
