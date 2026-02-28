//! Command palette — fuzzy-search jump to any view with `:`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::AppView;

/// Actions emitted by the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    None,
    Navigate(AppView),
    Close,
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
}

impl CommandPalette {
    pub fn open(&mut self) {
        self.is_open = true;
        self.query.clear();
        self.selected_index = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CommandPaletteAction {
        if !self.is_open {
            return CommandPaletteAction::None;
        }

        match key.code {
            KeyCode::Esc => CommandPaletteAction::Close,
            KeyCode::Enter => {
                let matches = self.filtered();
                matches
                    .get(self.selected_index)
                    .map(|v| CommandPaletteAction::Navigate(*v))
                    .unwrap_or(CommandPaletteAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.filtered().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                }
                CommandPaletteAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
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
                CommandPaletteAction::None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.selected_index = 0;
                CommandPaletteAction::None
            }
            _ => CommandPaletteAction::None,
        }
    }

    /// Returns views whose aliases fuzzy-match the current query.
    pub fn filtered(&self) -> Vec<AppView> {
        if self.query.is_empty() {
            return COMMANDS.iter().map(|c| c.view).collect();
        }
        COMMANDS
            .iter()
            .filter(|cmd| {
                cmd.aliases
                    .iter()
                    .any(|alias| fuzzy_match(alias, &self.query))
            })
            .map(|c| c.view)
            .collect()
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
            Span::styled("Command Palette", theme.title_style()),
            Span::styled("  · type to jump", theme.inactive_style()),
        ]);
        let title_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.header_bg));
        frame.render_widget(Paragraph::new(title).block(title_block), chunks[0]);

        let search_content = if self.query.is_empty() {
            Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled("pods, svc, deploy, sa, crd…", theme.inactive_style()),
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
        let items: Vec<ListItem> = if matches.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "  No matches",
                theme.inactive_style(),
            )))]
        } else {
            matches
                .iter()
                .enumerate()
                .map(|(idx, view)| {
                    let group_label = view.group().label();
                    if idx == self.selected_index {
                        ListItem::new(Line::from(vec![
                            Span::styled(" ▶ ", theme.title_style()),
                            Span::styled(
                                view.label(),
                                Style::default()
                                    .fg(theme.selection_fg)
                                    .bg(theme.selection_bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(format!("  {group_label}"), theme.inactive_style()),
                        ]))
                    } else {
                        ListItem::new(Line::from(vec![
                            Span::styled("   ", theme.inactive_style()),
                            Span::styled(view.label(), Style::default().fg(theme.fg_dim)),
                            Span::styled(format!("  {group_label}"), theme.inactive_style()),
                        ]))
                    }
                })
                .collect()
        };

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
            Span::styled("jump  ", theme.keybind_desc_style()),
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
        assert!(results.contains(&AppView::Pods));
    }

    #[test]
    fn filtered_svc_matches_services() {
        let mut p = CommandPalette::default();
        p.open();
        for c in "svc".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert!(p.filtered().contains(&AppView::Services));
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
}
