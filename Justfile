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
  cargo +nightly clippy --workspace --all-targets --all-features -- -D warnings
  cargo machete
  build-docs
test:
  cargo nextest run --workspace --all-features
build-docs:
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
test-coverage:
  cargo tarpaulin --all-features --workspace --timeout 300
check-feature:
  cargo hack check --each-feature --no-dev-deps
check-cn:
  rg --line-number --column "\p{Han}"
# Full CI check
ci: lint test