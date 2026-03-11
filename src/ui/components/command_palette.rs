//! Command palette — fuzzy-search jump to any view with `:`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};
use std::cell::RefCell;

use crate::app::{AppView, ResourceRef};
use crate::policy::{DetailAction, ResourceActionContext};

/// Actions emitted by the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    None,
    Navigate(AppView),
    Execute(DetailAction, ResourceRef),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    Navigate(AppView),
    Action(DetailAction),
}

#[derive(Debug, Clone)]
pub struct ActionEntry {
    pub action: DetailAction,
    pub aliases: &'static [&'static str],
}

const ACTION_ALIASES: &[(DetailAction, &[&str])] = &[
    (DetailAction::ViewYaml, &["yaml", "manifest"]),
    (DetailAction::ViewEvents, &["events", "event"]),
    (DetailAction::Logs, &["logs", "log"]),
    (DetailAction::Exec, &["exec", "shell", "terminal"]),
    (
        DetailAction::PortForward,
        &["port-forward", "forward", "tunnel", "pf"],
    ),
    (DetailAction::Probes, &["probes", "health", "probe"]),
    (DetailAction::Scale, &["scale", "replicas"]),
    (DetailAction::Restart, &["restart", "rollout"]),
    (DetailAction::FluxReconcile, &["reconcile", "flux"]),
    (DetailAction::EditYaml, &["edit", "modify"]),
    (DetailAction::Delete, &["delete", "remove"]),
    (DetailAction::Trigger, &["trigger", "run"]),
    (
        DetailAction::ViewRelationships,
        &[
            "relations",
            "relationships",
            "related",
            "web",
            "tree",
            "deps",
        ],
    ),
    (DetailAction::Cordon, &["cordon", "unschedulable"]),
    (DetailAction::Uncordon, &["uncordon", "schedulable"]),
    (DetailAction::Drain, &["drain", "evict"]),
];

pub fn action_entries_for_resource(resource: Option<&ResourceActionContext>) -> Vec<ActionEntry> {
    let Some(resource) = resource else {
        return Vec::new();
    };
    ACTION_ALIASES
        .iter()
        .filter(|(action, _)| resource.supports_action(*action))
        .map(|(action, aliases)| ActionEntry {
            action: *action,
            aliases,
        })
        .collect()
}

/// All navigable commands — each maps a set of aliases to a target view.
struct Command {
    view: AppView,
    aliases: &'static [&'static str],
}

const COMMANDS: &[Command] = &[
    Command {
        view: AppView::Dashboard,
        aliases: &["dashboard", "dash", "home"],
    },
    Command {
        view: AppView::Nodes,
        aliases: &["nodes", "node", "no"],
    },
    Command {
        view: AppView::Pods,
        aliases: &["pods", "pod", "po"],
    },
    Command {
        view: AppView::Deployments,
        aliases: &["deployments", "deployment", "deploy", "dep"],
    },
    Command {
        view: AppView::StatefulSets,
        aliases: &["statefulsets", "statefulset", "sts"],
    },
    Command {
        view: AppView::DaemonSets,
        aliases: &["daemonsets", "daemonset", "ds"],
    },
    Command {
        view: AppView::Jobs,
        aliases: &["jobs", "job"],
    },
    Command {
        view: AppView::CronJobs,
        aliases: &["cronjobs", "cronjob", "cj"],
    },
    Command {
        view: AppView::Services,
        aliases: &["services", "service", "svc"],
    },
    Command {
        view: AppView::ServiceAccounts,
        aliases: &["serviceaccounts", "serviceaccount", "sa"],
    },
    Command {
        view: AppView::Roles,
        aliases: &["roles", "role"],
    },
    Command {
        view: AppView::RoleBindings,
        aliases: &["rolebindings", "rolebinding", "rb"],
    },
    Command {
        view: AppView::ClusterRoles,
        aliases: &["clusterroles", "clusterrole", "cr"],
    },
    Command {
        view: AppView::ClusterRoleBindings,
        aliases: &["clusterrolebindings", "clusterrolebinding", "crb"],
    },
    Command {
        view: AppView::ResourceQuotas,
        aliases: &["resourcequotas", "resourcequota", "quota", "rq"],
    },
    Command {
        view: AppView::LimitRanges,
        aliases: &["limitranges", "limitrange", "limits", "lr"],
    },
    Command {
        view: AppView::PodDisruptionBudgets,
        aliases: &["poddisruptionbudgets", "pdb", "pdbs"],
    },
    Command {
        view: AppView::PortForwarding,
        aliases: &["portforwarding", "portforward", "pf", "tunnel", "tunnels"],
    },
    Command {
        view: AppView::FluxCDAll,
        aliases: &["flux", "fluxcd", "gitops", "flux all", "flux get all"],
    },
    Command {
        view: AppView::FluxCDAlertProviders,
        aliases: &[
            "flux alert-providers",
            "flux alertproviders",
            "alert-providers",
        ],
    },
    Command {
        view: AppView::FluxCDAlerts,
        aliases: &["flux alerts", "alerts"],
    },
    Command {
        view: AppView::FluxCDArtifacts,
        aliases: &["flux artifacts", "artifacts"],
    },
    Command {
        view: AppView::FluxCDHelmReleases,
        aliases: &["flux helmreleases", "flux hr", "helmreleases"],
    },
    Command {
        view: AppView::FluxCDHelmRepositories,
        aliases: &[
            "flux helmrepositories",
            "flux helmrepository",
            "helmrepositories",
            "helmrepository",
        ],
    },
    Command {
        view: AppView::FluxCDImages,
        aliases: &["flux images", "images"],
    },
    Command {
        view: AppView::FluxCDKustomizations,
        aliases: &["flux kustomizations", "kustomizations", "ks"],
    },
    Command {
        view: AppView::FluxCDReceivers,
        aliases: &["flux receivers", "receivers"],
    },
    Command {
        view: AppView::FluxCDSources,
        aliases: &["flux sources", "sources"],
    },
    Command {
        view: AppView::Extensions,
        aliases: &["extensions", "ext", "crd", "crds"],
    },
];

/// Fuzzy-match: returns true if every character of `needle` appears in
/// `haystack` in order (case-insensitive).
fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = haystack.chars();
    'outer: for nc in needle.chars() {
        let nc = nc.to_ascii_lowercase();
        loop {
            match chars.next() {
                Some(hc) if hc.to_ascii_lowercase() == nc => continue 'outer,
                Some(_) => {}
                None => return false,
            }
        }
    }
    true
}

/// Modal command palette for jumping directly to any view.
#[derive(Debug, Clone, Default)]
pub struct CommandPalette {
    query: String,
    selected_index: usize,
    is_open: bool,
    cached_filtered: RefCell<Option<Vec<PaletteEntry>>>,
    resource_context: Option<ResourceActionContext>,
}

impl CommandPalette {
    pub fn open(&mut self) {
        self.open_with_context(None);
    }

    pub fn open_with_context(&mut self, resource: Option<ResourceActionContext>) {
        self.query.clear();
        self.selected_index = 0;
        self.is_open = true;
        self.resource_context = resource;
        self.cached_filtered.borrow_mut().take();
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.resource_context = None;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn resource_context(&self) -> Option<&ResourceActionContext> {
        self.resource_context.as_ref()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CommandPaletteAction {
        if !self.is_open {
            return CommandPaletteAction::None;
        }

        match key.code {
            KeyCode::Esc => CommandPaletteAction::Close,
            KeyCode::Enter => {
                let entries = self.filtered();
                if let Some(entry) = entries.get(self.selected_index) {
                    match entry {
                        PaletteEntry::Navigate(view) => CommandPaletteAction::Navigate(*view),
                        PaletteEntry::Action(action) => {
                            if let Some(resource) = &self.resource_context {
                                CommandPaletteAction::Execute(*action, resource.resource.clone())
                            } else {
                                CommandPaletteAction::None
                            }
                        }
                    }
                } else {
                    CommandPaletteAction::None
                }
            }
            KeyCode::Down => {
                let len = self.filtered().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                CommandPaletteAction::None
            }
            KeyCode::Char('j') => {
                let len = self.filtered().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                CommandPaletteAction::None
            }
            KeyCode::Up => {
                let len = self.filtered().len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                }
                CommandPaletteAction::None
            }
            KeyCode::Char('k') => {
                let len = self.filtered().len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                }
                CommandPaletteAction::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected_index = 0;
                self.cached_filtered.borrow_mut().take();
                CommandPaletteAction::None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.selected_index = 0;
                self.cached_filtered.borrow_mut().take();
                CommandPaletteAction::None
            }
            _ => CommandPaletteAction::None,
        }
    }

    /// Returns palette entries whose aliases fuzzy-match the current query.
    /// Actions (if a resource context exists) come first, then navigation entries.
    pub fn filtered(&self) -> Vec<PaletteEntry> {
        if let Some(cached) = self.cached_filtered.borrow().as_ref() {
            return cached.clone();
        }

        let mut result = Vec::new();

        // Actions section (only if resource context exists)
        if let Some(resource) = &self.resource_context {
            let actions = action_entries_for_resource(Some(resource));
            for entry in &actions {
                if self.query.is_empty()
                    || entry
                        .aliases
                        .iter()
                        .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::Action(entry.action));
                }
            }
        }

        // Navigation section
        for cmd in COMMANDS {
            if self.query.is_empty()
                || cmd
                    .aliases
                    .iter()
                    .any(|alias| fuzzy_match(alias, &self.query))
            {
                result.push(PaletteEntry::Navigate(cmd.view));
            }
        }

        *self.cached_filtered.borrow_mut() = Some(result.clone());
        result
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.is_open {
            return;
        }

        use crate::ui::components::default_theme;
        let theme = default_theme();

        let popup_width = (area.width * 2 / 5).clamp(44, 60);
        let popup_height = (area.height / 2).clamp(16, 24);
        let popup = Rect {
            x: (area.width.saturating_sub(popup_width)) / 2,
            y: area.height.saturating_sub(popup_height) / 3,
            width: popup_width,
            height: popup_height,
        };

        frame.render_widget(Clear, popup);

        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_active_style())
            .style(Style::default().bg(theme.bg));
        frame.render_widget(outer, popup);

        let inner = Rect {
            x: popup.x + 1,
            y: popup.y + 1,
            width: popup.width.saturating_sub(2),
            height: popup.height.saturating_sub(2),
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(inner);

        let title = Line::from(vec![
            Span::styled(" ⌘ ", theme.title_style()),
            Span::styled("Action Palette", theme.title_style()),
            Span::styled("  · type to filter", theme.inactive_style()),
        ]);
        let title_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.header_bg));
        frame.render_widget(Paragraph::new(title).block(title_block), chunks[0]);

        let search_content = if self.query.is_empty() {
            Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled("scale, logs, pods, deploy…", theme.inactive_style()),
            ])
        } else {
            Line::from(vec![
                Span::styled("  : ", theme.title_style()),
                Span::styled(self.query.clone(), Style::default().fg(theme.fg)),
                Span::styled("█", theme.title_style()),
            ])
        };
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if self.query.is_empty() {
                theme.border_style()
            } else {
                theme.border_active_style()
            })
            .style(Style::default().bg(theme.bg_surface));
        frame.render_widget(
            Paragraph::new(search_content).block(search_block),
            chunks[1],
        );

        let matches = self.filtered();
        let mut items: Vec<ListItem> = Vec::new();

        if matches.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  No matches",
                theme.inactive_style(),
            ))));
        } else {
            let mut seen_action = false;
            let mut seen_nav = false;

            for (selectable_idx, entry) in matches.iter().enumerate() {
                match entry {
                    PaletteEntry::Action(_) if !seen_action => {
                        seen_action = true;
                        items.push(ListItem::new(Line::from(Span::styled(
                            " ── Actions ──",
                            theme.muted_style(),
                        ))));
                    }
                    PaletteEntry::Navigate(_) if !seen_nav => {
                        seen_nav = true;
                        items.push(ListItem::new(Line::from(Span::styled(
                            " ── Navigate ──",
                            theme.muted_style(),
                        ))));
                    }
                    _ => {}
                }

                let (name, right_label) = match entry {
                    PaletteEntry::Navigate(view) => {
                        (view.label(), view.group().label().to_string())
                    }
                    PaletteEntry::Action(action) => (action.label(), action.key_hint().to_string()),
                };

                let is_selected = selectable_idx == self.selected_index;
                if is_selected {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled(" ▶ ", theme.title_style()),
                        Span::styled(
                            name,
                            Style::default()
                                .fg(theme.selection_fg)
                                .bg(theme.selection_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("  {right_label}"), theme.inactive_style()),
                    ])));
                } else {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("   ", theme.inactive_style()),
                        Span::styled(name, Style::default().fg(theme.fg_dim)),
                        Span::styled(format!("  {right_label}"), theme.inactive_style()),
                    ])));
                }
            }
        }

        let count = matches.len();
        let list_block = Block::default()
            .title(Span::styled(
                format!(" {count} results "),
                theme.muted_style(),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.bg));
        frame.render_widget(List::new(items).block(list_block), chunks[2]);

        let footer = Line::from(vec![
            Span::styled(" [↑↓/jk] ", theme.keybind_key_style()),
            Span::styled("navigate  ", theme.keybind_desc_style()),
            Span::styled("[Enter] ", theme.keybind_key_style()),
            Span::styled("select  ", theme.keybind_desc_style()),
            Span::styled("[Esc] ", theme.keybind_key_style()),
            Span::styled("close", theme.keybind_desc_style()),
        ]);
        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.statusbar_bg));
        frame.render_widget(Paragraph::new(footer).block(footer_block), chunks[3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ResourceActionContext;

    fn ctx(resource: ResourceRef, node_unschedulable: Option<bool>) -> ResourceActionContext {
        ResourceActionContext {
            resource,
            node_unschedulable,
        }
    }

    #[test]
    fn fuzzy_match_exact() {
        assert!(fuzzy_match("pods", "pods"));
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("deployments", "dply"));
        assert!(fuzzy_match("serviceaccounts", "svacc"));
    }

    #[test]
    fn fuzzy_match_empty_needle_matches_all() {
        assert!(fuzzy_match("anything", ""));
    }

    #[test]
    fn fuzzy_match_no_match() {
        assert!(!fuzzy_match("pods", "xyz"));
    }

    #[test]
    fn filtered_empty_query_returns_all() {
        let p = CommandPalette::default();
        assert_eq!(p.filtered().len(), COMMANDS.len());
    }

    #[test]
    fn filtered_po_matches_pods() {
        let mut p = CommandPalette::default();
        p.open();
        for c in "po".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let results = p.filtered();
        assert!(results.contains(&PaletteEntry::Navigate(AppView::Pods)));
    }

    #[test]
    fn filtered_svc_matches_services() {
        let mut p = CommandPalette::default();
        p.open();
        for c in "svc".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert!(
            p.filtered()
                .contains(&PaletteEntry::Navigate(AppView::Services))
        );
    }

    #[test]
    fn enter_navigates_to_selected() {
        let mut p = CommandPalette::default();
        p.open();
        for c in "deploy".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let action = p.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, CommandPaletteAction::Navigate(AppView::Deployments));
    }

    #[test]
    fn esc_emits_close() {
        let mut p = CommandPalette::default();
        p.open();
        assert_eq!(
            p.handle_key(KeyEvent::from(KeyCode::Esc)),
            CommandPaletteAction::Close
        );
    }

    #[test]
    fn navigation_wraps() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(p.selected_index, COMMANDS.len() - 1);
    }

    #[test]
    fn vim_navigation_moves_selection() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(p.selected_index, 1);
        p.handle_key(KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(p.selected_index, 0);
    }

    #[test]
    fn palette_entry_action_aliases_match() {
        let entries = action_entries_for_resource(None);
        assert!(entries.is_empty(), "No actions without resource");
    }

    #[test]
    fn palette_entry_action_aliases_pod() {
        let resource = ctx(ResourceRef::Pod("test".into(), "default".into()), None);
        let entries = action_entries_for_resource(Some(&resource));
        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
        assert!(entries.iter().any(|e| e.action == DetailAction::Exec));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Scale));
    }

    #[test]
    fn palette_entry_action_aliases_deployment() {
        let resource = ctx(
            ResourceRef::Deployment("api".into(), "default".into()),
            None,
        );
        let entries = action_entries_for_resource(Some(&resource));
        assert!(entries.iter().any(|e| e.action == DetailAction::Scale));
        assert!(entries.iter().any(|e| e.action == DetailAction::Restart));
        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Exec));
    }

    #[test]
    fn palette_entry_node_actions_follow_unschedulable_state() {
        let schedulable = ctx(ResourceRef::Node("node-a".into()), Some(false));
        let entries = action_entries_for_resource(Some(&schedulable));
        assert!(entries.iter().any(|e| e.action == DetailAction::Cordon));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Uncordon));

        let unschedulable = ctx(ResourceRef::Node("node-a".into()), Some(true));
        let entries = action_entries_for_resource(Some(&unschedulable));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Cordon));
        assert!(entries.iter().any(|e| e.action == DetailAction::Uncordon));
        assert!(entries.iter().any(|e| e.action == DetailAction::Drain));
    }

    #[test]
    fn palette_set_context_enables_actions() {
        let mut palette = CommandPalette::default();
        let resource = ctx(ResourceRef::Pod("test".into(), "default".into()), None);
        palette.open_with_context(Some(resource.clone()));
        assert!(palette.is_open());
        assert!(palette.resource_context().is_some());
    }

    #[test]
    fn palette_open_without_context_has_no_actions() {
        let mut palette = CommandPalette::default();
        palette.open_with_context(None);
        assert!(palette.is_open());
        assert!(palette.resource_context().is_none());
    }

    #[test]
    fn filtered_returns_actions_then_navigation() {
        let mut palette = CommandPalette::default();
        let resource = ctx(
            ResourceRef::Deployment("api".into(), "default".into()),
            None,
        );
        palette.open_with_context(Some(resource));
        let entries = palette.filtered();
        let first_action_idx = entries
            .iter()
            .position(|e| matches!(e, PaletteEntry::Action(_)));
        let first_nav_idx = entries
            .iter()
            .position(|e| matches!(e, PaletteEntry::Navigate(_)));
        assert!(first_action_idx.is_some());
        assert!(first_nav_idx.is_some());
        assert!(first_action_idx.unwrap() < first_nav_idx.unwrap());
    }

    #[test]
    fn filtered_with_query_matches_actions_and_views() {
        let mut palette = CommandPalette::default();
        let resource = ctx(
            ResourceRef::Deployment("api".into(), "default".into()),
            None,
        );
        palette.open_with_context(Some(resource));
        // Type "scl" which should fuzzy-match "scale"
        for c in "scl".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let entries = palette.filtered();
        assert!(
            entries
                .iter()
                .any(|e| matches!(e, PaletteEntry::Action(DetailAction::Scale)))
        );
    }

    #[test]
    fn filtered_no_context_has_no_actions() {
        let mut palette = CommandPalette::default();
        palette.open_with_context(None);
        let entries = palette.filtered();
        assert!(
            entries
                .iter()
                .all(|e| matches!(e, PaletteEntry::Navigate(_)))
        );
    }

    #[test]
    fn handle_key_enter_on_action_returns_execute() {
        let mut palette = CommandPalette::default();
        let resource = ResourceRef::Pod("test".into(), "default".into());
        palette.open_with_context(Some(ctx(resource.clone(), None)));
        // First entry should be an action (ViewYaml for Pod)
        let result = palette.handle_key(KeyEvent::from(KeyCode::Enter));
        match result {
            CommandPaletteAction::Execute(_, ref res) => {
                assert_eq!(*res, resource);
            }
            other => panic!("Expected Execute, got {:?}", other),
        }
    }

    #[test]
    fn empty_query_with_context_shows_actions_and_views() {
        let mut palette = CommandPalette::default();
        let resource = ctx(
            ResourceRef::Deployment("api".into(), "default".into()),
            None,
        );
        palette.open_with_context(Some(resource));
        let entries = palette.filtered();
        let has_actions = entries.iter().any(|e| matches!(e, PaletteEntry::Action(_)));
        let has_nav = entries
            .iter()
            .any(|e| matches!(e, PaletteEntry::Navigate(_)));
        assert!(
            has_actions,
            "Should have action entries with resource context"
        );
        assert!(has_nav, "Should have navigation entries");
        assert!(
            entries.len() > COMMANDS.len(),
            "Should have more entries than navigation alone"
        );
    }
}
