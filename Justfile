# Ensure rustup-managed nightly cargo is used (system /usr/bin cargo is stable)
export PATH := home_directory() / ".cargo/bin" + ":" + env("PATH", "")

format:
    rumdl fmt .
    cargo sort -w -g
    cargo fmt --all
fix:
    rumdl check --fix .
lint:
    rumdl check .
    cargo sort -w -g -c
    cargo fmt --all -- --check
    cargo clippy --all -- -D warnings
    cargo shear
    cargo +nightly workspace-inheritance-check
    just check-agents-md
test:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        cargo nextest run --workspace --all-features
    else
        cargo nextest run --workspace --all-features
    fi
test-full:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        cargo nextest run --workspace --all-features
    else
        cargo nextest run --workspace --all-features
    fi
test-all: test-full
build-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --workspace --no-deps --document-private-items
    else
        RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --workspace --no-deps --document-private-items --all-features
    fi
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
# Check that AGENTS.md dependency versions match Cargo.toml
check-agents-md:
    #!/usr/bin/env bash
    errors=0
    while IFS= read -r line; do
        crate=$(echo "$line" | sed -n 's/.*`\([^ ]*\) = ".*/\1/p')
        agents_ver=$(echo "$line" | sed -n 's/.*"\([^"]*\)".*/\1/p')
        if [ -z "$crate" ] || [ -z "$agents_ver" ]; then
            continue
        fi
        cargo_ver=$(grep -E "^${crate} = " Cargo.toml 2>/dev/null | sed -n 's/.*version = "\([^"]*\)".*/\1/p' | head -1)
        if [ -z "$cargo_ver" ]; then
            cargo_ver=$(grep -E "^${crate} = " Cargo.toml 2>/dev/null | sed -n 's/[^"]*"\([^"]*\)".*/\1/p' | head -1)
        fi
        if [ -z "$cargo_ver" ]; then
            continue
        fi
        if [ "$agents_ver" != "$cargo_ver" ]; then
            echo "MISMATCH: ${crate}: AGENTS.md=${agents_ver} Cargo.toml=${cargo_ver}"
            errors=$((errors + 1))
        fi
    done < <(sed -n '/^## Preferred Dependencies/,/^##/p' AGENTS.md | grep '^\-.*\`.*=.*"')
    if [ $errors -gt 0 ]; then
        echo "FAIL: ${errors} version mismatch(es) between AGENTS.md and Cargo.toml"
        exit 1
    fi
    echo "OK: AGENTS.md versions match Cargo.toml"
check-feature:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        cargo check --workspace
    else
        cargo check --workspace --all-features
    fi
# ---------------------------------------------------------------------------
# Fuzzing
# ---------------------------------------------------------------------------
# Fuzz crates live in `crates/<crate>/fuzz` and are excluded from the main
# workspace (see `exclude` in Cargo.toml), so each one needs an explicit recipe.

# ---------------------------------------------------------------------------
# Fuzzing
# ---------------------------------------------------------------------------
# Fuzz crates live in `crates/<crate>/fuzz` and are excluded from the main
# workspace (see `exclude` in Cargo.toml), so each one needs an explicit recipe.
#
# `cargo fuzz run` is used for building and listing, but the smoke recipe invokes
# the compiled binary directly: cargo-fuzz does not always reap the target
# process promptly, so a `-max_total_time` run would sit there after libFuzzer
# has already exited.

# Build every fuzz target without running it.
fuzz-build:
    #!/usr/bin/env bash
    set -euo pipefail
    for dir in crates/*/fuzz; do
        [ -d "$dir" ] || continue
        # cargo-fuzz refuses to run when the corpus or artifact directory is
        # missing, so create them up front.
        for target in $(cargo fuzz list --fuzz-dir "$dir"); do
            mkdir -p "$dir/corpus/$target" "$dir/artifacts/$target"
        done
        echo "==> building $dir"
        cargo fuzz build --fuzz-dir "$dir"
    done

# List every fuzz target across the workspace.
fuzz-list:
    #!/usr/bin/env bash
    set -euo pipefail
    for dir in crates/*/fuzz; do
        [ -d "$dir" ] || continue
        crate=$(basename "$(dirname "$dir")")
        echo "==> $crate"
        cargo fuzz list --fuzz-dir "$dir"
    done

# Smoke-run every target for `fuzz-time` seconds (default 20).
#
# This is the CI gate: long enough to catch a trivially reachable panic without
# pretending to be a real campaign. A non-zero exit is a finding, not a flake --
# libFuzzer exits non-zero on a crash and writes the input to the artifact dir.
#
# The compiled binary is invoked directly rather than through `cargo fuzz run`:
# cargo-fuzz does not reliably reap the target process, so a `-max_total_time`
# run would hang after libFuzzer has already exited.
fuzz-smoke fuzz-time="20":
    #!/usr/bin/env bash
    set -uo pipefail
    triple=$(rustc -vV | sed -n 's/^host: //p')
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
                -timeout=25 \
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

# Longer local campaign: build, then smoke every target.
fuzz fuzz-time="300":
    #!/usr/bin/env bash
    set -euo pipefail
    just fuzz-build
    just fuzz-smoke "{{fuzz-time}}"
check-cn:
    rg --line-number --column "\p{Han}"
# Full CI check
ci: lint test-all build-docs
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
