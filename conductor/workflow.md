# Workflow

## TDD Policy

**Strict** — Tests required before implementation.

- Write tests first, then implement to pass them
- No merging without passing test suite
- Performance profiling required for render-path changes

## Commit Strategy

**Conventional Commits** with semantic prefixes:

- `feat:`, `fix:`, `chore:`, `refactor:`, `perf:`, `docs:`, `test:`
- Imperative mood ("add feature" not "added feature")
- Subject lines under 72 characters
- One measurable objective per commit
- No `Co-Authored-By` lines

## Git Workflow

- **Never push directly to main**
- All changes: new branch → open PR → merge PR to main
- Branch naming: `fix/`, `feat/`, `chore/`, `refactor/`
- Use `gh pr create` and `gh pr merge --squash --delete-branch`

## Quality Gate (before every commit)

1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets --all-features`
4. Performance profiling check for changed render paths

## Performance Validation Protocol

- Run profiling test 5 times baseline, 5 times candidate
- Compare medians per key metrics: `render`, `sidebar`, `header`
- Keep change only if global `render` median improves
- No critical hotspot may regress without explicit reason

## Verification Checkpoints

- Required after each phase completion
- Phase verification confirms all tasks pass tests and acceptance criteria
- Track completion requires all phases verified

## Task Lifecycle

1. `[ ]` Pending — not started
2. `[~]` In Progress — actively being worked on
3. `[x]` Complete — implemented, tested, verified
4. `[!]` Blocked — waiting on dependency
