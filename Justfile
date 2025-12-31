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
  cargo +nightly clippy --all
  cargo machete
test:
  cargo test --all-features
test-coverage:
  cargo tarpaulin --all-features --workspace --timeout 300
check-cn:
  rg --line-number --column "\p{Han}"
# Full CI check
ci: lint test