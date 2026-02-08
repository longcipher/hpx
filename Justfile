format:
  rumdl fmt .
  taplo fmt
  cargo +nightly fmt --all
fix:
  rumdl check --fix .
lint:
  typos
  rumdl check .
  taplo fmt --check
  cargo +nightly fmt --all -- --check
  cargo +nightly clippy --all -- -D warnings
  cargo machete
  just build-docs
test:
  cargo nextest run --workspace --all-features
build-docs:
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
test-coverage:
  cargo tarpaulin --all-features --workspace --timeout 300
check-feature:
  cargo hack check --each-feature --no-dev-deps --exclude-no-default-features
check-cn:
  rg --line-number --column "\p{Han}"
# Full CI check
ci: lint test
publish:
  cargo publish --workspace