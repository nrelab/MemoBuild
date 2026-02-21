# MemoBuild Code Style & Quality Guidelines

To ensure a highly maintainable, safe, and collaborative environment, MemoBuild strictly adheres to the standard Rust idioms along with the following supplemental guidelines.

## 1. Error Handling (`?` vs `.unwrap()`)

- **Default to `?`:** All fallible functions should return a `Result<T, E>` and bubble errors using the `?` operator. This ensures the caller can decide how to handle the failure contextually.
- **Never `.unwrap()` in Production paths:** Hard unwraps should be reserved for logical impossibilities or test scaffolding. Use `expect("Detailed explanation of why this panic is unreachable")` if you must forcibly unwrap something.

## 2. Naming Conventions

Follow standard Rust naming conventions (`camelCase` for types, `snake_case` for variables/functions).
- **Channels**: Use standard TX/RX prefixes: `tx_events` and `rx_events`. Do not use `event_tx`.
- **Hashes**: Always name strings holding BLAKE3 digests `hash` or `digest` (e.g., `input_manifest_hash`).

## 3. Magic Numbers & Constants

**Do not use magic numbers.** Any arbitrary buffer sizes, defaults, or timeout scalars must be pulled into a module-level constant with documentation explaining *why* the number was chosen.

```rust
// BAD
tokio::time::sleep(Duration::from_millis(5000)).await;

// GOOD
/// The default timeout for remote cache retrieval before switching contexts.
pub const DEFAULT_CACHE_TIMEOUT_MS: u64 = 5000;
tokio::time::sleep(Duration::from_millis(DEFAULT_CACHE_TIMEOUT_MS)).await;
```

## 4. Clippy Allowlist

Run `cargo clippy --all-targets --all-features`. We target zero warnings on default clippy rules.
Allowed exceptions include specific scenarios documented locally in modules via `#[allow(clippy::too_many_arguments)]` when strictly bounding builder patterns is less legible than executing raw parameter maps, but this should be rare.

## 5. Module Structure & Feature Gating

Code layout operations should not be gated by testing frameworks. Feature gates (`#[cfg(feature = "server")]`) should only be applied securely around application components that contain heavy dependency trees (like `axum` or `tonic`) to keep CLI execution binaries small. 

If a module needs to be tested under a different feature flag, ensure its test payloads sit inside `#[cfg(test)]`.
