# Rust Style Guide

## Formatting

- Use `cargo fmt --all` (rustfmt defaults)
- No manual formatting overrides

## Linting

- `cargo clippy --all-targets --all-features -- -D warnings` must pass
- All warnings treated as errors

## Safety

- Prefer safe Rust; isolate any `unsafe` behind minimal, documented abstractions
- Follow ownership-first design
- Zero-cost abstractions over runtime overhead
- Predictable allocation behavior on hot paths

## Error Handling

- Keep APIs panic-safe for runtime paths
- Fail fast on invalid invariants (debug builds)
- Use `Result<T, E>` for fallible operations
- No `.unwrap()` on user-facing paths

## Code Style

- Prefer explicit, readable code over dense one-liners
- No commented-out code — delete it
- Self-documenting code over excessive comments
- Don't add features, refactoring, or "improvements" beyond what was asked

## Testing

- Tests go in `#[cfg(test)] mod tests` blocks inside source files
- Integration tests in `tests/` directory
- Performance benchmarks in `benches/` using Criterion
- Strict TDD: write tests before implementation

## Naming

- Types: `PascalCase`
- Functions/methods: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Modules: `snake_case`
- Enum variants: `PascalCase`

## Dependencies

- Minimize dependency count
- Prefer well-maintained crates with active communities
- Pin major versions in `Cargo.toml`
- `Cargo.lock` committed (binary project)

## Performance

- Profile before optimizing — use median-based comparisons
- Cache expensive computations keyed on snapshot version
- Minimize allocations on hot render paths
- Row virtualization for large datasets
