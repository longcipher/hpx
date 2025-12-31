# Rust + Leptos Project Instructions

## Project Structure

```text
project-root/
├── Cargo.toml              // Root workspace manifest
├── justfile                // Task runner commands
├── README.md
├── bin/                    // Executable crates (CLI tools)
│   ├── cli-app/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs
│   └── another-cli/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
└── crates/                 // Shared libraries and components
    ├── common/
    │   ├── Cargo.toml
    │   └── src/
    │       └── lib.rs
    ├── ui-components/
    │   ├── Cargo.toml
    │   └── src/
    │       └── lib.rs
    └── backend-core/
        ├── Cargo.toml
        └── src/
            └── lib.rs
```

## Cargo Workspace Rules

### Dependency Management (CRITICAL)

**NEVER** manually type versions into `Cargo.toml` files. Always use `cargo add` to ensure the latest versions and correct workspace inheritance.

1. **To add a dependency to the Root Workspace** (adds to `[workspace.dependencies]`):
    ```bash
    cargo add <crate_name> --workspace
    ```
2. **To add a dependency to a Sub-crate**:
    ```bash
    # This ensures the sub-crate uses 'workspace = true' and the version is managed at the root
    cargo add <crate_name> -p <sub-crate-name> --workspace
    ```

### Root `Cargo.toml` Requirements

1. **ONLY** use number versions (e.g., `"0.8.14"`)
2. **NO** features allowed in `[workspace.dependencies]`
3. Must define `workspace.package.version` and `workspace.package.edition`

Example:

```toml
[workspace]
members = ["bin/*", "crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
# Managed via: cargo add <name> --workspace
# Framework
leptos = "0.8.15"

# CLI
clap = "4.5.53"

# Configuration
config = "0.15.19"

# Build
shadow-rs = "1.5.0"

# Database
sqlx = "0.9.0-alpha.1"

# Utilities
singlestage = "0.4.0"
eyre = "0.6.12"
```

### Sub-crate `Cargo.toml` Requirements

1. **MUST** use `workspace = true` for all dependencies
2. **CAN** specify `features = [...]` for dependencies
3. **MUST** use `workspace = true` for `version` and `edition`
4. **NO** number versions allowed

Example:

```toml
[package]
name = "my-crate"
version.workspace = true
edition.workspace = true

[dependencies]
leptos = { workspace = true, features = ["csr", "nightly"] }
clap = { workspace = true, features = ["derive"] }
sqlx = { workspace = true, features = ["postgres", "runtime-tokio"] }
eyre.workspace = true
```

## Dependency Priority

When implementing functionality, **ALWAYS** check if these libraries can be used first:

### Core Dependencies (Mandatory Versions)

- **leptos** >= 0.8.15 - Web framework
- **clap** >= 4.5.53 - CLI parsing
- **config** >= 0.15.19 - Configuration management
- **shadow-rs** >= 1.5.0 - Build-time information
- **sqlx** >= 0.9.0-alpha.1 - Database access
- **singlestage** >= 0.4.0 - Single-stage deployment
- **eyre** >= 0.6.12 - Error handling
- **serde** >= 1.0.228 - Serialization/deserialization
- **tokio** >= 1.48.0 - Async runtime
- **tracing** >= 0.1.44 - Logging
- **thiserror** >= 2.0.17 - Error types

## File Operations

**CRITICAL**: Never use `cat`, `echo >`, or shell commands to create/modify files.

✅ **DO**: Use VS Code's built-in features:

- Create File
- Edit File
- File Explorer actions

❌ **DON'T**:

```bash
cat > file.txt << EOF
content
EOF
```

## Development Workflow

### After Every Feature or Bug Fix

Run these commands in order and fix ALL errors and warnings:

```bash
# 1. Format code
just format

# 2. Run linter and fix issues
just lint

# 3. Run tests
just test
```

**IMPORTANT**: Code is NOT complete until:

- ✅ All format checks pass
- ✅ All lint warnings fixed
- ✅ All tests pass

## Testing Requirements

### Unit Tests

- Add unit tests in the same file as the code
- Use `#[cfg(test)]` module
- Test both success and error cases
- Aim for >80% code coverage

Example:

```rust
// src/utils.rs
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
        assert_eq!(add(-1, 1), 0);
    }
}
```

### Integration Tests

- Create `tests/` directory at crate root
- Test public API functionality
- Test cross-crate interactions
- Use realistic scenarios

Example structure:

```text
crates/backend-core/
├── Cargo.toml
├── src/
│   └── lib.rs
└── tests/
    ├── api_tests.rs
    └── db_tests.rs
```

## Code Style

### Rust

- Use `snake_case` for functions, variables, modules
- Use `CamelCase` for types, traits, enums
- Prefer `?` over `unwrap()` or `expect()`
- Error Handling: NEVER use anyhow. Use eyre for application-level error handling and thiserror for library-level error types.
- Logging: NEVER use the log crate. Use tracing for all logging and instrumentation.
- SQLx Usage: STRICTLY FORBIDDEN to use compile-time checked macros (e.g., `sqlx::query!`, `sqlx::query_as!`). You MUST use runtime queries (e.g., `sqlx::query`, `sqlx::query_as`) to ensure builds do not require a live database connection.

### Frontend (Leptos + Singlestage UI + Tailwind CSS v4)

- **Leptos Semantics**:
  - **Signals**: Access signals as functions (e.g., `count()`, NOT `count.get()`).
  - **Reactivity**: Use `move ||` closures for ALL dynamic attributes in `view!`.
  - **Components**: Use `#[component]` macro. Props usually accept `impl Into<T>` or `Signal<T>`.
  - **Server Functions**: Use `#[server]` for backend communication instead of manual REST fetches where possible.
- **Tailwind CSS v4**:
  - **Configuration**: Do NOT create `tailwind.config.js`. Use CSS-first configuration with `@theme` blocks in your main CSS file.
  - **Syntax**: Use v4 arbitrary values and opacity modifiers syntax strictly.
- **Singlestage UI**:
  - Prioritize `singlestage` components over manual HTML elements.

## Quality Checklist

Before considering work complete:

- [ ] All files created using VS Code (not shell commands)
- [ ] `just format` passes with no changes
- [ ] `just lint` passes with no warnings
- [ ] `just test` passes all tests
- [ ] No instances of anyhow or log crates exist in the code.
- [ ] All SQLx queries use runtime functions (query/query_as), not macros with exclamation marks (query!).
- [ ] Unit tests added for new functions
- [ ] Integration tests added for new features
- [ ] README.md updated with changes
- [ ] All dependencies use `workspace = true` in sub-crates
- [ ] Only root `Cargo.toml` has number versions
- [ ] All mandatory library versions met
