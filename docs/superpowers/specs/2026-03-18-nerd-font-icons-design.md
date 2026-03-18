# Nerd Font Icon System Design

## Goal
Centralized, configurable icon system for all 46 resource views, 9 sidebar groups, 9 workbench tabs, and status indicators. Three modes: Nerd Font (default), Emoji, Plain text.

## Architecture

### New module: `src/icons.rs`

```rust
pub enum IconMode { Nerd, Emoji, Plain }

pub struct ViewIcon {
    pub nerd: &'static str,
    pub emoji: &'static str,
    pub plain: &'static str,
}

impl ViewIcon {
    pub fn for_mode(&self, mode: IconMode) -> &'static str;
}
```

Global state: `active_icon_mode()` / `set_icon_mode()` (same pattern as theme system).

### Registries
- `view_icon(AppView) -> &ViewIcon` — 48 views
- `group_icon(NavGroup) -> &ViewIcon` — 9 sidebar groups
- `tab_icon(WorkbenchTabKind) -> &ViewIcon` — 9 tab types
- `status_icon(StatusKind) -> &ViewIcon` — 4 status indicators

### Config persistence
- `icon_mode: "nerd" | "emoji" | "plain"` in `AppConfig` (kubectui-config.json)
- Runtime toggle: `:icons` action palette command cycles modes
- Default: `nerd`

### Callsite changes
- Table title headers: use `view_icon(view).for_mode(mode)` instead of hardcoded emoji strings
- Sidebar: use `view_icon` / `group_icon` instead of hardcoded Nerd Font strings
- Workbench tabs: prepend `tab_icon` to tab labels
- Status indicators: use `status_icon` for issue center + bookmarks
- Dashboard: use `view_icon(Dashboard)`

## Icon Mapping
See approved mapping in brainstorming session — covers all 48 views, 9 groups, 9 tabs, 4 status indicators.
