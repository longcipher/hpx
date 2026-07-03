format:
    rumdl fmt .
    cargo sort -w -g
    cargo +nightly fmt --all
fix:
    rumdl check --fix .
lint:
    rumdl check .
    cargo sort -w -g -c
    cargo +nightly fmt --all -- --check
    cargo +nightly clippy --all -- -D warnings
    cargo shear
test:
    cargo nextest run --workspace --all-features
test-full:
    cargo nextest run --workspace --all-features
bdd:
    cargo test -p hpx-dl --test cucumber --all-features
test-all: test-full bdd
build-docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
test-coverage:
    cargo tarpaulin --all-features --workspace --timeout 300
check-feature:
    cargo check --workspace --all-features
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
    # Dependency order: hpx-yawc → hpx → {hpx-browser, hpx-dl, hpx-emulation, hpx-streams} → hpx-cli
    CRATES="hpx-yawc hpx hpx-browser hpx-dl hpx-emulation hpx-streams hpx-cli"
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
