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
  cargo publish --workspace