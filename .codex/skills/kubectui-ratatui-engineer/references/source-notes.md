# Source Notes

This skill is intentionally a curated merge, not a full import of any one external skill pack.

## Keep

### `rust-cli-tui-developer`

Use the idea, not the full tree.

- Strongest fit for this repo because it is the only source in the candidate set centered on
  Rust terminal apps and Ratatui
- Useful pattern: consult official examples before inventing abstractions
- Useful scope: clap, inquire, ratatui, and phased terminal-app construction

Applied here as:

- "Prefer first-class Ratatui/Crossterm patterns"
- "Consult official Ratatui examples before inventing new widget behavior"

### `leonardomso/rust-skills`

Useful as a rule catalog, especially for hot-path Rust work.

- Borrow over clone
- Use `with_capacity`
- Profile before optimizing
- Avoid locks across `await`
- Keep docs and tests disciplined

Applied here as:

- render-path allocation rules
- centralized helper preference
- performance-first validation

### David Barsky Rust doc guidance

Useful for public Rust docs and unsafe documentation conventions.

Applied here as:

- preserve standard `# Examples`, `# Errors`, `# Panics`, and `# Safety` sections when public APIs
  are added or changed

## Do Not Import Wholesale

### `actionbook/rust-skills`

High-quality but too heavy for this repo-local use case.

- Requires a router-first workflow and a larger multi-skill ecosystem
- Assumes external agents and live dependency lookups
- Better as a standalone environment, not as a small project-local skill

### `minimaxir` Rust `CLAUDE.md` and `AGENTS.md`

Contains useful strictness, but too much unrelated policy for Kubectui.

- Forces unrelated framework preferences
- Mixes web, Python, data-frame, and packaging rules that do not belong in this repo-local TUI
  skill
- Would conflict with the project's existing AGENTS guidance

### `discover-tui`, `Rust Expert`, marketplace-only entries

Low signal for this repository.

- Good for discovery, but not strong enough as an operational skill source
- Either too generic or too marketing-oriented to drive day-to-day Kubectui changes

## External URLs

- https://playbooks.com/skills/bahayonghang/my-claude-code-settings/rust-cli-tui-developer
- https://github.com/bahayonghang/my-claude-code-settings/tree/HEAD/content/skills/tech-stack-skills/rust-cli-tui-developer
- https://github.com/leonardomso/rust-skills
- https://gist.github.com/davidbarsky/8fae6dc45c294297db582378284bd1f2
- https://github.com/actionbook/rust-skills
- https://gist.github.com/minimaxir/23ee55a83633ac0b6b92de635291ad80
- https://gist.github.com/minimaxir/068ef4137a1b6c1dcefa785349c91728
- https://www.aimcp.info/en/skills/ca1225f3-a2b8-4732-98d7-824eb7ee7fef
- https://www.antigravityskills.org/skill/rust-expert
- https://mcpmarket.com/es/tools/skills/ratatui-rust-tui-builder
