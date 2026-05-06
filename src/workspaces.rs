//! Saved workspace layouts, named banks, and configurable hotkey bindings.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppView, NavGroup},
    workbench::DEFAULT_WORKBENCH_HEIGHT,
};

fn default_workbench_height() -> u16 {
    DEFAULT_WORKBENCH_HEIGHT
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub namespace: String,
    pub view: AppView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collapsed_groups: Vec<NavGroup>,
    #[serde(default)]
    pub workbench_open: bool,
    #[serde(default = "default_workbench_height")]
    pub workbench_height: u16,
    #[serde(default)]
    pub workbench_maximized: bool,
    #[serde(default)]
    pub action_history_tab: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    pub snapshot: WorkspaceSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBank {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub namespace: String,
    pub view: AppView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
}

impl WorkspaceBank {
    pub fn to_snapshot(&self) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            context: normalized_optional_text(self.context.as_deref()),
            namespace: normalized_required_text(&self.namespace, "all"),
            view: self.view,
            search_query: normalized_optional_text(self.search_query.as_deref()),
            collapsed_groups: Vec::new(),
            workbench_open: false,
            workbench_height: DEFAULT_WORKBENCH_HEIGHT,
            workbench_maximized: false,
            action_history_tab: false,
        }
    }
}

pub fn normalized_workspace_snapshot(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        context: normalized_optional_text(snapshot.context.as_deref()),
        namespace: normalized_required_text(&snapshot.namespace, "all"),
        view: snapshot.view,
        search_query: normalized_optional_text(snapshot.search_query.as_deref()),
        collapsed_groups: snapshot.collapsed_groups.clone(),
        workbench_open: snapshot.workbench_open,
        workbench_height: snapshot.workbench_height,
        workbench_maximized: snapshot.workbench_maximized,
        action_history_tab: snapshot.action_history_tab,
    }
}

fn normalized_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized_required_text(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HotkeyTarget {
    View { view: AppView },
    Action { action: HotkeyAction },
    Workspace { name: String },
    Bank { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    OpenCommandPalette,
    RefreshData,
    OpenActionHistory,
    OpenNamespacePicker,
    OpenContextPicker,
    SaveWorkspace,
    ApplyPreviousWorkspace,
    ApplyNextWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub key: String,
    pub target: HotkeyTarget,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspacePreferences {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<SavedWorkspace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub banks: Vec<WorkspaceBank>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hotkeys: Vec<HotkeyBinding>,
}

pub fn hotkey_matches(spec: &str, key: KeyEvent) -> bool {
    let normalized = normalize_hotkey_spec(spec);
    if normalized.is_empty() {
        return false;
    }

    let (required_modifiers, required_key) = match parse_hotkey_spec(&normalized) {
        Some(parsed) => parsed,
        None => return false,
    };

    let Some(actual_modifiers) = hotkey_modifiers(key) else {
        return false;
    };
    required_modifiers == actual_modifiers && key_code_matches(required_key, key.code)
}

pub fn display_hotkey(spec: &str) -> String {
    normalize_hotkey_spec(spec)
}

fn normalize_hotkey_spec(spec: &str) -> String {
    spec.trim().to_ascii_lowercase().replace(' ', "")
}

fn parse_hotkey_spec(spec: &str) -> Option<(KeyModifiers, HotkeyToken)> {
    let mut modifiers = KeyModifiers::empty();
    let mut tokens = spec.split('+').peekable();
    let mut key_token = None;

    while let Some(token) = tokens.next() {
        match token {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "" => return None,
            other => {
                if tokens.peek().is_some() {
                    return None;
                }
                key_token = Some(parse_key_token(other)?);
            }
        }
    }

    key_token.map(|token| (modifiers, token))
}

fn hotkey_modifiers(key: KeyEvent) -> Option<KeyModifiers> {
    let allowed = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
    key.modifiers
        .difference(allowed)
        .is_empty()
        .then_some(key.modifiers & allowed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyToken {
    Char(char),
    F(u8),
}

fn parse_key_token(token: &str) -> Option<HotkeyToken> {
    if let Some(rest) = token.strip_prefix('f') {
        let value = rest.parse::<u8>().ok()?;
        return (1..=12).contains(&value).then_some(HotkeyToken::F(value));
    }

    let mut chars = token.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(HotkeyToken::Char(ch))
}

fn key_code_matches(expected: HotkeyToken, actual: KeyCode) -> bool {
    match (expected, actual) {
        (HotkeyToken::Char(left), KeyCode::Char(right)) => left.eq_ignore_ascii_case(&right),
        (HotkeyToken::F(left), KeyCode::F(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_matches_simple_alt_digit() {
        assert!(hotkey_matches(
            "alt+1",
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT)
        ));
    }

    #[test]
    fn hotkey_matches_function_key() {
        assert!(hotkey_matches(
            "f3",
            KeyEvent::new(KeyCode::F(3), KeyModifiers::empty())
        ));
    }

    #[test]
    fn hotkey_rejects_wrong_modifier() {
        assert!(!hotkey_matches(
            "ctrl+k",
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::ALT)
        ));
    }

    #[test]
    fn hotkey_rejects_extra_system_modifier() {
        assert!(!hotkey_matches(
            "ctrl+k",
            KeyEvent::new(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL | KeyModifiers::SUPER
            )
        ));
        assert!(!hotkey_matches(
            "alt+1",
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT | KeyModifiers::META)
        ));
    }

    #[test]
    fn hotkey_matches_shift_modifier() {
        assert!(hotkey_matches(
            "shift+k",
            KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT)
        ));
    }

    #[test]
    fn bank_snapshot_stays_lightweight() {
        let bank = WorkspaceBank {
            name: "prod pods".into(),
            context: Some("prod".into()),
            namespace: "payments".into(),
            view: AppView::Pods,
            hotkey: Some("alt+1".into()),
            search_query: Some("checkout".into()),
        };

        let snapshot = bank.to_snapshot();
        assert_eq!(snapshot.context.as_deref(), Some("prod"));
        assert_eq!(snapshot.namespace, "payments");
        assert_eq!(snapshot.view, AppView::Pods);
        assert_eq!(snapshot.search_query.as_deref(), Some("checkout"));
        assert!(!snapshot.workbench_open);
        assert!(!snapshot.action_history_tab);
    }

    #[test]
    fn bank_snapshot_normalizes_text_fields() {
        let bank = WorkspaceBank {
            name: "prod pods".into(),
            context: Some(" prod ".into()),
            namespace: " payments ".into(),
            view: AppView::Pods,
            hotkey: Some("alt+1".into()),
            search_query: Some(" checkout ".into()),
        };

        let snapshot = bank.to_snapshot();

        assert_eq!(snapshot.context.as_deref(), Some("prod"));
        assert_eq!(snapshot.namespace, "payments");
        assert_eq!(snapshot.search_query.as_deref(), Some("checkout"));
    }

    #[test]
    fn display_hotkey_normalizes_case_and_spacing() {
        assert_eq!(display_hotkey(" Alt + 1 "), "alt+1");
    }
}
