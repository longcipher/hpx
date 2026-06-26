format:
    rumdl fmt .
    taplo fmt
    cargo +nightly fmt --all
fix:
    rumdl check --fix .
lint:
    rumdl check .
    taplo fmt --check
    cargo +nightly fmt --all -- --check
    cargo +nightly clippy --all -- -D warnings
    cargo machete
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
    echo "Publishing workspace crates in dependency order..."
    CRATES=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name')
    REMAINING="$CRATES"
    for i in {1..7}; do
    	if [ -z "$REMAINING" ]; then break; fi
    	NEXT=""
    	for crate in $REMAINING; do
    		echo "[$i/7] Publishing $crate..."
    		if cargo publish -p "$crate" --allow-dirty 2>&1; then
    			echo "  ✓ $crate published"
    			sleep 30
    		else
    			echo "  → $crate deferred (retrying next round)"
    			NEXT="$NEXT $crate"
    			sleep 5
    		fi
    	done
    	REMAINING="$NEXT"
    	if [ -n "$REMAINING" ]; then
    		echo "Waiting 60s before next round..."
    		sleep 60
    	fi
    done
    if [ -n "$REMAINING" ]; then
    	echo "ERROR: Failed to publish: $REMAINING"
    	exit 1
    fi
    echo "All crates published."
