# AGENTS.md

This project is a Rust workspace containing the `hpx` HTTP client library and related crates.

## Project Structure

- `Cargo.toml` - Workspace manifest
- `crates/` - Contains the workspace crates
- `Cargo.lock` - Locked dependencies

## Commands

### Testing

```bash
cargo nextest run --workspace --all-features
```

### Linting

```bash
# Full lint check
typos && rumdl check . && taplo fmt --check && cargo +nightly fmt --all -- --check && cargo +nightly clippy --all -- -D warnings && cargo machete
```

### Formatting

```bash
rumdl fmt . && taplo fmt && cargo +nightly fmt --all
```

### Building Documentation

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
```

### Full CI Check

```bash
just lint && just test
```

## Important Notes

- Requires nightly Rust for formatting and clippy
- Uses `cargo nextest` for test execution (not standard `cargo test`)
- Documentation is built with all warnings as errors
- This is a high-performance HTTP client library - be mindful of performance implications when making changes
