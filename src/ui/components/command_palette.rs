//! Command palette — fuzzy-search jump to any view with `:`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Style},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{cell::RefCell, cmp::Ordering, collections::HashSet, sync::Arc};

use crate::ai_actions::LoadedAiAction;
use crate::app::{AppState, AppView, RecentJumpTarget, ResourceRef};
use crate::extensions::LoadedExtensionAction;
use crate::global_search::GlobalResourceSearchEntry;
use crate::policy::{DetailAction, ResourceActionContext};
use crate::resource_templates::ResourceTemplateKind;
use crate::runbooks::LoadedRunbook;
use crate::ui::components::render_vertical_scrollbar;
use crate::ui::theme::Theme;
use crate::ui::{
    clear_input_at_cursor, cursor_visible_input_line, delete_char_left_at_cursor,
    delete_char_right_at_cursor, insert_char_at_cursor, move_cursor_end, move_cursor_home,
    move_cursor_left, move_cursor_right, wrap_span_groups, wrapped_line_count,
};
use crate::workbench::WorkbenchTabKey;
use crate::workspaces::display_hotkey;

const TEMPLATE_INTENT_ALIASES: &[&str] =
    &["create", "new", "template", "scaffold", "apply", "manifest"];
const MAX_ACTIVITY_RESULTS: usize = 16;
const MAX_RESOURCE_RESULTS: usize = 40;
const COMPACT_PALETTE_WIDTH: u16 = 48;
const COMPACT_PALETTE_HEIGHT: u16 = 12;

fn plain_shortcut(key: KeyEvent) -> bool {
    key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

fn edit_key(key: KeyEvent) -> bool {
    key.modifiers.is_empty()
}

fn ctrl_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key
            .modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            .is_empty()
}

fn command_palette_popup(area: Rect) -> Rect {
    let preferred_width = (area.width * 2 / 5).clamp(44, 60);
    let preferred_height = (area.height / 2).clamp(16, 24);
    let popup = crate::ui::bounded_popup_rect(area, preferred_width, preferred_height, 1, 1);
    Rect {
        y: area.y + area.height.saturating_sub(popup.height) / 3,
        ..popup
    }
}

fn use_compact_command_palette_layout(popup: Rect) -> bool {
    popup.width < COMPACT_PALETTE_WIDTH || popup.height < COMPACT_PALETTE_HEIGHT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteSection {
    Activity,
    Resource,
    Action,
    Ai,
    Extension,
    Runbook,
    Workspace,
    Template,
    Bank,
    Column,
    Navigate,
}

impl PaletteSection {
    const fn title(self) -> &'static str {
        match self {
            Self::Activity => " ── Recent Activity ──",
            Self::Resource => " ── Resources ──",
            Self::Action => " ── Actions ──",
            Self::Ai => " ── AI ──",
            Self::Extension => " ── Extensions ──",
            Self::Runbook => " ── Runbooks ──",
            Self::Workspace => " ── Workspaces ──",
            Self::Template => " ── Templates ──",
            Self::Bank => " ── Banks ──",
            Self::Column => " ── Columns ──",
            Self::Navigate => " ── Navigate ──",
        }
    }
}

/// Actions emitted by the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    None,
    Navigate(AppView),
    JumpToResource(ResourceRef),
    ActivateWorkbenchTab(WorkbenchTabKey),
    Execute(DetailAction, ResourceRef),
    ExecuteAi(String, ResourceRef),
    ExecuteExtension(String, ResourceRef),
    OpenRunbook(String, Option<ResourceRef>),
    ToggleColumn(String),
    SaveWorkspace,
    ApplyWorkspace(String),
    ActivateWorkspaceBank(String),
    OpenTemplateDialog(ResourceTemplateKind),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    Activity(PaletteActivityEntry),
    Resource(PaletteResourceEntry),
    Navigate(AppView),
    Action(DetailAction),
    AiAction(PaletteAiAction),
    ExtensionAction(PaletteExtensionAction),
    Runbook(PaletteRunbookAction),
    SaveWorkspace,
    Template(ResourceTemplateKind),
    Workspace(String),
    WorkspaceBank {
        name: String,
        hotkey: Option<String>,
    },
    ColumnToggle {
        id: String,
        label: String,
        visible: bool,
    },
}

fn palette_entries_same_identity(left: &PaletteEntry, right: &PaletteEntry) -> bool {
    match (left, right) {
        (PaletteEntry::Activity(left), PaletteEntry::Activity(right)) => {
            left.target == right.target
        }
        (PaletteEntry::Resource(left), PaletteEntry::Resource(right)) => {
            left.resource == right.resource
        }
        (PaletteEntry::AiAction(left), PaletteEntry::AiAction(right)) => left.id == right.id,
        (PaletteEntry::ExtensionAction(left), PaletteEntry::ExtensionAction(right)) => {
            left.id == right.id
        }
        (PaletteEntry::Runbook(left), PaletteEntry::Runbook(right)) => {
            left.id == right.id && left.resource == right.resource
        }
        (
            PaletteEntry::ColumnToggle { id: left, .. },
            PaletteEntry::ColumnToggle { id: right, .. },
        ) => left == right,
        (
            PaletteEntry::WorkspaceBank { name: left, .. },
            PaletteEntry::WorkspaceBank { name: right, .. },
        ) => left == right,
        _ => left == right,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteActivityTarget {
    Navigate(AppView),
    Resource(ResourceRef),
    WorkbenchTab(WorkbenchTabKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteActivityEntry {
    pub title: String,
    pub subtitle: String,
    pub aliases: Vec<String>,
    pub badge_label: String,
    pub target: PaletteActivityTarget,
}

pub type PaletteResourceEntry = GlobalResourceSearchEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteAiAction {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub shortcut: Option<String>,
    pub badge_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteExtensionAction {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub shortcut: Option<String>,
    pub badge_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRunbookAction {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub shortcut: Option<String>,
    pub resource: Option<ResourceRef>,
}

#[derive(Debug, Clone)]
pub struct ActionEntry {
    pub action: DetailAction,
    pub aliases: &'static [&'static str],
}

const ACTION_ALIASES: &[(DetailAction, &[&str])] = &[
    (DetailAction::ViewYaml, &["yaml", "manifest"]),
    (
        DetailAction::ViewConfigDrift,
        &["drift", "diff", "config drift", "live vs applied"],
    ),
    (
        DetailAction::ViewRollout,
        &[
            "rollout",
            "rollout center",
            "rollout status",
            "undo rollout",
        ],
    ),
    (
        DetailAction::ViewHelmHistory,
        &[
            "helm",
            "helm history",
            "history",
            "rollback",
            "release history",
        ],
    ),
    (
        DetailAction::ViewDecodedSecret,
        &["decoded", "decode", "secret data", "reveal"],
    ),
    (DetailAction::ToggleBookmark, &["bookmark", "pin", "save"]),
    (DetailAction::ViewEvents, &["events", "event"]),
    (
        DetailAction::ViewAccessReview,
        &[
            "access",
            "rbac",
            "permissions",
            "authorization",
            "why denied",
        ],
    ),
    (DetailAction::Logs, &["logs", "log"]),
    (DetailAction::Exec, &["exec", "shell", "terminal"]),
    (
        DetailAction::DebugContainer,
        &["debug", "debug container", "ephemeral", "kubectl debug"],
    ),
    (
        DetailAction::NodeDebugShell,
        &[
            "node debug",
            "node shell",
            "debug node",
            "host shell",
            "node troubleshoot",
        ],
    ),
    (
        DetailAction::PortForward,
        &["port-forward", "forward", "tunnel", "pf"],
    ),
    (DetailAction::Probes, &["probes", "health", "probe"]),
    (DetailAction::Scale, &["scale", "replicas"]),
    (
        DetailAction::Restart,
        &["restart", "restart rollout", "rollout restart"],
    ),
    (DetailAction::FluxReconcile, &["reconcile", "flux"]),
    (DetailAction::EditYaml, &["edit", "modify"]),
    (DetailAction::Delete, &["delete", "remove"]),
    (DetailAction::Trigger, &["trigger", "run"]),
    (
        DetailAction::SuspendCronJob,
        &["suspend", "pause", "stop schedule"],
    ),
    (
        DetailAction::ResumeCronJob,
        &["resume", "unpause", "start schedule"],
    ),
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
    (
        DetailAction::ViewNetworkPolicies,
        &[
            "network policy",
            "network policies",
            "netpol",
            "policy view",
        ],
    ),
    (
        DetailAction::CheckNetworkConnectivity,
        &[
            "connectivity",
            "reachability",
            "can reach",
            "network reachability",
            "check connectivity",
        ],
    ),
    (
        DetailAction::ViewTrafficDebug,
        &[
            "traffic",
            "traffic debug",
            "service debug",
            "ingress trace",
            "endpoint audit",
            "dns debug",
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

pub fn collect_activity_entries(app: &AppState) -> Vec<PaletteActivityEntry> {
    let mut entries = Vec::new();
    let mut seen_workbench_tabs = HashSet::new();
    let mut seen_resource_targets = HashSet::new();
    let mut seen_views = HashSet::new();
    let current_scope = app.activity_scope();

    for entry in app.action_history().entries() {
        let Some(target) = entry.target.as_ref() else {
            continue;
        };
        if entry.scope != current_scope {
            continue;
        }
        if !seen_resource_targets.insert(target.resource.clone()) {
            continue;
        }
        let title = format!("{} {}", entry.kind.label(), entry.resource_label);
        entries.push(PaletteActivityEntry {
            title: title.clone(),
            subtitle: entry.message.clone(),
            aliases: vec![
                title.to_ascii_lowercase(),
                entry.message.to_ascii_lowercase(),
                target.resource.kind().to_ascii_lowercase(),
                target.resource.name().to_ascii_lowercase(),
                target
                    .resource
                    .namespace()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                target.resource.summary_label().to_ascii_lowercase(),
            ],
            badge_label: entry.status.label().to_string(),
            target: PaletteActivityTarget::Resource(target.resource.clone()),
        });
    }

    for jump in app.recent_jumps() {
        if jump.scope != current_scope {
            continue;
        }
        match &jump.target {
            RecentJumpTarget::Resource(resource) => {
                if !seen_resource_targets.insert(resource.clone()) {
                    continue;
                }
                let title = resource.summary_label();
                entries.push(PaletteActivityEntry {
                    title: title.clone(),
                    subtitle: "Recent resource jump".to_string(),
                    aliases: vec![
                        title.to_ascii_lowercase(),
                        resource.kind().to_ascii_lowercase(),
                        resource.name().to_ascii_lowercase(),
                        resource
                            .namespace()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    ],
                    badge_label: "Recent".to_string(),
                    target: PaletteActivityTarget::Resource(resource.clone()),
                });
            }
            RecentJumpTarget::View(view) => {
                if !seen_views.insert(*view) {
                    continue;
                }
                entries.push(PaletteActivityEntry {
                    title: view.label().to_string(),
                    subtitle: "Recent view jump".to_string(),
                    aliases: vec![
                        view.label().to_ascii_lowercase(),
                        view.group().label().to_ascii_lowercase(),
                        "recent".to_string(),
                    ],
                    badge_label: "Recent".to_string(),
                    target: PaletteActivityTarget::Navigate(*view),
                });
            }
        }
    }

    for tab in &app.workbench().tabs {
        let key = tab.state.key();
        if !seen_workbench_tabs.insert(key.clone()) {
            continue;
        }
        let title = tab.state.title();
        entries.push(PaletteActivityEntry {
            title: title.clone(),
            subtitle: "Open workbench tab".to_string(),
            aliases: vec![
                title.to_ascii_lowercase(),
                "workbench".to_string(),
                tab.state.kind().title().to_ascii_lowercase(),
            ],
            badge_label: "Tab".to_string(),
            target: PaletteActivityTarget::WorkbenchTab(key),
        });
    }

    entries
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
        view: AppView::Governance,
        aliases: &["governance", "cost", "cost center", "finops", "efficiency"],
    },
    Command {
        view: AppView::Bookmarks,
        aliases: &["bookmarks", "bookmark", "saved", "pinned"],
    },
    Command {
        view: AppView::HealthReport,
        aliases: &["health", "health report", "sanitizer", "lint"],
    },
    Command {
        view: AppView::Vulnerabilities,
        aliases: &[
            "vulnerabilities",
            "vulnerability",
            "security",
            "security center",
            "trivy",
        ],
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
        view: AppView::Endpoints,
        aliases: &["endpoints", "endpoint", "ep"],
    },
    Command {
        view: AppView::Ingresses,
        aliases: &["ingresses", "ingress", "ing"],
    },
    Command {
        view: AppView::IngressClasses,
        aliases: &["ingressclasses", "ingressclass", "ic"],
    },
    Command {
        view: AppView::GatewayClasses,
        aliases: &["gatewayclasses", "gatewayclass", "gwc", "gateway class"],
    },
    Command {
        view: AppView::Gateways,
        aliases: &["gateways", "gateway", "gw"],
    },
    Command {
        view: AppView::HttpRoutes,
        aliases: &["httproutes", "http route", "http routes", "hroute"],
    },
    Command {
        view: AppView::GrpcRoutes,
        aliases: &["grpcroutes", "grpc route", "grpc routes", "groute"],
    },
    Command {
        view: AppView::ReferenceGrants,
        aliases: &[
            "referencegrants",
            "referencegrant",
            "grant",
            "reference grant",
        ],
    },
    Command {
        view: AppView::NetworkPolicies,
        aliases: &["networkpolicies", "networkpolicy", "netpol", "netpols"],
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

fn starts_with_ascii_ci(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .get(..needle.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle))
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    haystack.char_indices().find_map(|(idx, _)| {
        let end = idx.checked_add(needle.len())?;
        haystack
            .as_bytes()
            .get(idx..end)
            .and_then(|window| window.eq_ignore_ascii_case(needle).then_some(idx))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AliasScore<'a> {
    rank: u8,
    metric: usize,
    alias: &'a str,
}

fn push_ranked_match<'a, F>(
    matches: &mut Vec<(AliasScore<'a>, usize)>,
    candidate: (AliasScore<'a>, usize),
    limit: usize,
    mut compare: F,
) where
    F: FnMut(&(AliasScore<'a>, usize), &(AliasScore<'a>, usize)) -> Ordering,
{
    if limit == 0 {
        return;
    }
    if matches.len() < limit {
        matches.push(candidate);
        return;
    }

    if let Some((worst_index, _)) = matches
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| compare(left, right))
        && compare(&candidate, &matches[worst_index]).is_lt()
    {
        matches[worst_index] = candidate;
    }
}

fn compare_activity_match(
    entries: &[PaletteActivityEntry],
    left: &(AliasScore<'_>, usize),
    right: &(AliasScore<'_>, usize),
) -> Ordering {
    let left_entry = &entries[left.1];
    let right_entry = &entries[right.1];
    left.0
        .cmp(&right.0)
        .then_with(|| left_entry.title.cmp(&right_entry.title))
        .then_with(|| left_entry.subtitle.cmp(&right_entry.subtitle))
}

fn compare_resource_match(
    entries: &[PaletteResourceEntry],
    left: &(AliasScore<'_>, usize),
    right: &(AliasScore<'_>, usize),
) -> Ordering {
    let left_entry = &entries[left.1];
    let right_entry = &entries[right.1];
    left.0
        .cmp(&right.0)
        .then_with(|| left_entry.title.cmp(&right_entry.title))
        .then_with(|| left_entry.subtitle.cmp(&right_entry.subtitle))
}

fn ranked_alias_score<'a>(aliases: &'a [String], query: &str) -> Option<AliasScore<'a>> {
    let query = query.trim();
    if query.is_empty() {
        return Some(AliasScore {
            rank: u8::MAX,
            metric: 0,
            alias: "",
        });
    }

    aliases
        .iter()
        .filter_map(|alias| {
            let alias = alias.as_str();
            if alias.eq_ignore_ascii_case(query) {
                Some(AliasScore {
                    rank: 0,
                    metric: alias.len(),
                    alias,
                })
            } else if starts_with_ascii_ci(alias, query) {
                Some(AliasScore {
                    rank: 1,
                    metric: alias.len(),
                    alias,
                })
            } else if let Some(position) = find_ascii_ci(alias, query) {
                Some(AliasScore {
                    rank: 2,
                    metric: position,
                    alias,
                })
            } else if fuzzy_match(alias, query) {
                Some(AliasScore {
                    rank: 3,
                    metric: alias.len(),
                    alias,
                })
            } else {
                None
            }
        })
        .min()
}

fn palette_entry_section(entry: &PaletteEntry) -> PaletteSection {
    match entry {
        PaletteEntry::Activity(_) => PaletteSection::Activity,
        PaletteEntry::Resource(_) => PaletteSection::Resource,
        PaletteEntry::Action(_) => PaletteSection::Action,
        PaletteEntry::AiAction(_) => PaletteSection::Ai,
        PaletteEntry::ExtensionAction(_) => PaletteSection::Extension,
        PaletteEntry::Runbook(_) => PaletteSection::Runbook,
        PaletteEntry::SaveWorkspace | PaletteEntry::Workspace(_) => PaletteSection::Workspace,
        PaletteEntry::Template(_) => PaletteSection::Template,
        PaletteEntry::WorkspaceBank { .. } => PaletteSection::Bank,
        PaletteEntry::ColumnToggle { .. } => PaletteSection::Column,
        PaletteEntry::Navigate(_) => PaletteSection::Navigate,
    }
}

fn palette_item_lines(
    entry: &PaletteEntry,
    theme: &Theme,
    is_selected: bool,
    section_header: Option<&'static str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(3);
    if let Some(header) = section_header {
        lines.push(Line::from(Span::styled(header, theme.muted_style())));
    }

    if let PaletteEntry::ColumnToggle { label, visible, .. } = entry {
        let check = if *visible { "[x]" } else { "[ ]" };
        let style = if is_selected {
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_dim)
        };
        let prefix = if is_selected { " ▶ " } else { "   " };
        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                if is_selected {
                    theme.title_style()
                } else {
                    theme.inactive_style()
                },
            ),
            Span::styled(format!("{check} {label}"), style),
        ]));
        return lines;
    }

    let (name, right_label, subtitle) = match entry {
        PaletteEntry::Activity(entry) => (
            entry.title.clone(),
            entry.badge_label.clone(),
            Some(entry.subtitle.clone()),
        ),
        PaletteEntry::Resource(entry) => (
            entry.title.clone(),
            entry.badge_label.clone(),
            Some(entry.subtitle.clone()),
        ),
        PaletteEntry::Navigate(view) => (
            view.label().to_string(),
            view.group().label().to_string(),
            None,
        ),
        PaletteEntry::Action(action) => (
            action.label().to_string(),
            action.key_hint().to_string(),
            None,
        ),
        PaletteEntry::AiAction(action) => (
            action.title.clone(),
            action
                .shortcut
                .as_deref()
                .map(display_hotkey)
                .unwrap_or_else(|| action.badge_label.clone()),
            None,
        ),
        PaletteEntry::ExtensionAction(action) => (
            action.title.clone(),
            action
                .shortcut
                .as_deref()
                .map(display_hotkey)
                .unwrap_or_else(|| action.badge_label.clone()),
            None,
        ),
        PaletteEntry::Runbook(runbook) => (
            runbook.title.clone(),
            runbook
                .shortcut
                .as_deref()
                .map(display_hotkey)
                .unwrap_or_else(|| "Runbook".to_string()),
            None,
        ),
        PaletteEntry::SaveWorkspace => (
            "Save current workspace".to_string(),
            "[W]".to_string(),
            None,
        ),
        PaletteEntry::Template(kind) => (
            format!("Create {}", kind.label()),
            "Template".to_string(),
            None,
        ),
        PaletteEntry::Workspace(name) => (name.clone(), "Workspace".to_string(), None),
        PaletteEntry::WorkspaceBank { name, hotkey } => (
            name.clone(),
            hotkey
                .as_deref()
                .map(display_hotkey)
                .unwrap_or_else(|| "Bank".to_string()),
            None,
        ),
        PaletteEntry::ColumnToggle { .. } => unreachable!(),
    };

    let prefix = if is_selected { " ▶ " } else { "   " };
    let name_style = if is_selected {
        Style::default()
            .fg(theme.selection_fg)
            .bg(theme.selection_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    lines.push(Line::from(vec![
        Span::styled(
            prefix,
            if is_selected {
                theme.title_style()
            } else {
                theme.inactive_style()
            },
        ),
        Span::styled(name, name_style),
        Span::styled(format!("  {right_label}"), theme.inactive_style()),
    ]));
    if let Some(subtitle) = subtitle {
        lines.push(Line::from(vec![
            Span::styled("   ", theme.inactive_style()),
            Span::styled(subtitle, theme.inactive_style()),
        ]));
    }

    lines
}

fn compute_palette_offset(
    item_heights: &[usize],
    selected_index: usize,
    viewport_height: usize,
) -> usize {
    if viewport_height == 0 || item_heights.is_empty() || selected_index >= item_heights.len() {
        return 0;
    }

    let mut offset = selected_index;
    let mut used_height = item_heights[selected_index];
    while offset > 0 {
        let next_height = item_heights[offset - 1];
        if used_height + next_height > viewport_height {
            break;
        }
        offset -= 1;
        used_height += next_height;
    }
    offset
}

fn palette_scroll_metrics(item_heights: &[usize], offset: usize) -> (usize, usize) {
    if item_heights.is_empty() {
        return (1, 0);
    }

    let clamped_offset = offset.min(item_heights.len().saturating_sub(1));
    let total = item_heights.iter().sum::<usize>().max(1);
    let position = item_heights[..clamped_offset].iter().sum::<usize>();
    (total, position)
}

/// Modal command palette for jumping directly to any view.
#[derive(Debug, Clone, Default)]
pub struct CommandPalette {
    query: String,
    query_cursor: usize,
    selected_index: usize,
    selection_anchor: Option<PaletteEntry>,
    is_open: bool,
    cached_filtered: RefCell<Option<Arc<Vec<PaletteEntry>>>>,
    activity_entries: Vec<PaletteActivityEntry>,
    resource_entries: Arc<Vec<PaletteResourceEntry>>,
    resource_context: Option<ResourceActionContext>,
    /// Column toggle info for current view: (id, label, currently_visible).
    columns_info: Option<Vec<(String, String, bool)>>,
    extension_actions: Vec<PaletteExtensionAction>,
    ai_actions: Vec<PaletteAiAction>,
    runbooks: Vec<PaletteRunbookAction>,
    saved_workspaces: Vec<String>,
    workspace_banks: Vec<(String, Option<String>)>,
}

impl CommandPalette {
    pub fn open(&mut self) {
        self.open_with_context(None);
    }

    pub fn open_with_context(&mut self, resource: Option<ResourceActionContext>) {
        clear_input_at_cursor(&mut self.query, &mut self.query_cursor);
        self.selected_index = 0;
        self.selection_anchor = None;
        self.is_open = true;
        self.resource_context = resource;
        self.cached_filtered.borrow_mut().take();
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.resource_context = None;
        self.columns_info = None;
        self.activity_entries.clear();
        self.resource_entries = Arc::default();
        self.extension_actions.clear();
        self.ai_actions.clear();
        self.runbooks.clear();
        self.selection_anchor = None;
        self.cached_filtered.borrow_mut().take();
    }

    pub fn resource_context_resource(&self) -> Option<&ResourceRef> {
        self.resource_context
            .as_ref()
            .map(|context| &context.resource)
    }

    fn selected_entry_snapshot(&self) -> Option<PaletteEntry> {
        let filtered = self.filtered_entries();
        filtered.get(self.selected_index).cloned()
    }

    fn selected_entry_anchor(&self) -> Option<PaletteEntry> {
        self.selection_anchor
            .clone()
            .or_else(|| self.selected_entry_snapshot())
    }

    fn restore_selected_entry(&mut self, selected_entry: Option<PaletteEntry>) {
        self.cached_filtered.borrow_mut().take();
        let filtered = self.filtered_entries();
        let matched_index = selected_entry.as_ref().and_then(|entry| {
            filtered
                .iter()
                .position(|candidate| palette_entries_same_identity(candidate, entry))
        });
        self.selected_index = matched_index.unwrap_or_else(|| {
            if selected_entry.is_some() {
                0
            } else {
                self.selected_index.min(filtered.len().saturating_sub(1))
            }
        });
        self.selection_anchor = matched_index
            .and_then(|index| filtered.get(index).cloned())
            .or(selected_entry)
            .or_else(|| filtered.get(self.selected_index).cloned());
    }

    fn update_loaded_entries(&mut self, selected_entry: Option<PaletteEntry>) {
        if self.is_open {
            let had_selected_entry = selected_entry.is_some();
            self.restore_selected_entry(selected_entry);
            if had_selected_entry {
                let filtered = self.filtered_entries();
                if !filtered.is_empty() {
                    self.selection_anchor = filtered.get(self.selected_index).cloned();
                }
            }
        } else {
            self.cached_filtered.borrow_mut().take();
            self.selection_anchor = None;
        }
    }

    pub fn set_activity_entries(&mut self, entries: Vec<PaletteActivityEntry>) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.activity_entries = entries;
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_resource_entries(&mut self, entries: impl Into<Arc<Vec<PaletteResourceEntry>>>) {
        let entries = entries.into();
        if Arc::ptr_eq(&self.resource_entries, &entries) {
            return;
        }
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.resource_entries = entries;
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_columns_info(&mut self, info: Option<Vec<(String, String, bool)>>) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.columns_info = info;
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_extension_actions(&mut self, actions: Vec<LoadedExtensionAction>) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.extension_actions = actions
            .into_iter()
            .map(|action| {
                let badge_label = action.badge_label();
                PaletteExtensionAction {
                    id: action.id,
                    title: action.title,
                    aliases: action.aliases,
                    shortcut: action.shortcut,
                    badge_label,
                }
            })
            .collect();
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_ai_actions(&mut self, actions: Vec<LoadedAiAction>) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.ai_actions = actions
            .into_iter()
            .map(|action| {
                let badge_label = action.badge_label();
                PaletteAiAction {
                    id: action.id,
                    title: action.title,
                    aliases: action.aliases,
                    shortcut: action.shortcut,
                    badge_label,
                }
            })
            .collect();
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_runbooks(&mut self, runbooks: Vec<LoadedRunbook>, resource: Option<ResourceRef>) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.runbooks = runbooks
            .into_iter()
            .map(|runbook| {
                let mut aliases = runbook.aliases;
                aliases.push(runbook.title.to_ascii_lowercase());
                aliases.push("runbook".into());
                aliases.push("incident".into());
                aliases.sort();
                aliases.dedup();
                PaletteRunbookAction {
                    id: runbook.id,
                    title: runbook.title,
                    aliases,
                    shortcut: runbook.shortcut,
                    resource: resource.clone(),
                }
            })
            .collect();
        self.update_loaded_entries(selected_entry);
    }

    pub fn set_workspace_info(
        &mut self,
        saved_workspaces: Vec<String>,
        workspace_banks: Vec<(String, Option<String>)>,
    ) {
        let selected_entry = self.is_open.then(|| self.selected_entry_anchor()).flatten();
        self.saved_workspaces = saved_workspaces;
        self.workspace_banks = workspace_banks;
        self.update_loaded_entries(selected_entry);
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
            KeyCode::Esc if plain_shortcut(key) => CommandPaletteAction::Close,
            KeyCode::Enter if plain_shortcut(key) => {
                let entries = self.filtered_entries();
                if let Some(entry) = entries.get(self.selected_index) {
                    match entry {
                        PaletteEntry::Activity(entry) => match &entry.target {
                            PaletteActivityTarget::Navigate(view) => {
                                CommandPaletteAction::Navigate(*view)
                            }
                            PaletteActivityTarget::Resource(resource) => {
                                CommandPaletteAction::JumpToResource(resource.clone())
                            }
                            PaletteActivityTarget::WorkbenchTab(key) => {
                                CommandPaletteAction::ActivateWorkbenchTab(key.clone())
                            }
                        },
                        PaletteEntry::Resource(entry) => {
                            CommandPaletteAction::JumpToResource(entry.resource.clone())
                        }
                        PaletteEntry::Navigate(view) => CommandPaletteAction::Navigate(*view),
                        PaletteEntry::Action(action) => {
                            if let Some(resource) = &self.resource_context {
                                CommandPaletteAction::Execute(*action, resource.resource.clone())
                            } else {
                                CommandPaletteAction::None
                            }
                        }
                        PaletteEntry::AiAction(action) => {
                            if let Some(resource) = &self.resource_context {
                                CommandPaletteAction::ExecuteAi(
                                    action.id.clone(),
                                    resource.resource.clone(),
                                )
                            } else {
                                CommandPaletteAction::None
                            }
                        }
                        PaletteEntry::ExtensionAction(action) => {
                            if let Some(resource) = &self.resource_context {
                                CommandPaletteAction::ExecuteExtension(
                                    action.id.clone(),
                                    resource.resource.clone(),
                                )
                            } else {
                                CommandPaletteAction::None
                            }
                        }
                        PaletteEntry::Runbook(runbook) => CommandPaletteAction::OpenRunbook(
                            runbook.id.clone(),
                            runbook.resource.clone(),
                        ),
                        PaletteEntry::SaveWorkspace => CommandPaletteAction::SaveWorkspace,
                        PaletteEntry::Template(kind) => {
                            CommandPaletteAction::OpenTemplateDialog(*kind)
                        }
                        PaletteEntry::Workspace(name) => {
                            CommandPaletteAction::ApplyWorkspace(name.clone())
                        }
                        PaletteEntry::WorkspaceBank { name, .. } => {
                            CommandPaletteAction::ActivateWorkspaceBank(name.clone())
                        }
                        PaletteEntry::ColumnToggle { id, .. } => {
                            CommandPaletteAction::ToggleColumn(id.clone())
                        }
                    }
                } else {
                    CommandPaletteAction::None
                }
            }
            KeyCode::Down if plain_shortcut(key) => {
                let len = self.filtered_entries().len();
                if len > 0 {
                    self.selected_index = (self.selected_index + 1) % len;
                    self.selection_anchor = self.selected_entry_snapshot();
                }
                CommandPaletteAction::None
            }
            KeyCode::Up if plain_shortcut(key) => {
                let len = self.filtered_entries().len();
                if len > 0 {
                    self.selected_index = if self.selected_index == 0 {
                        len - 1
                    } else {
                        self.selected_index - 1
                    };
                    self.selection_anchor = self.selected_entry_snapshot();
                }
                CommandPaletteAction::None
            }
            KeyCode::Backspace if edit_key(key) => {
                if self.query_cursor > 0 {
                    let selected_entry = self.selected_entry_anchor();
                    delete_char_left_at_cursor(&mut self.query, &mut self.query_cursor);
                    self.restore_selected_entry(selected_entry);
                }
                CommandPaletteAction::None
            }
            KeyCode::Delete if edit_key(key) => {
                let previous_len = self.query.len();
                let selected_entry = self.selected_entry_anchor();
                delete_char_right_at_cursor(&mut self.query, self.query_cursor);
                if self.query.len() != previous_len {
                    self.restore_selected_entry(selected_entry);
                }
                CommandPaletteAction::None
            }
            KeyCode::Left if edit_key(key) => {
                move_cursor_left(&mut self.query_cursor);
                CommandPaletteAction::None
            }
            KeyCode::Right if edit_key(key) => {
                move_cursor_right(&mut self.query_cursor, &self.query);
                CommandPaletteAction::None
            }
            KeyCode::Home if edit_key(key) => {
                move_cursor_home(&mut self.query_cursor);
                CommandPaletteAction::None
            }
            KeyCode::End if edit_key(key) => {
                move_cursor_end(&mut self.query_cursor, &self.query);
                CommandPaletteAction::None
            }
            KeyCode::Char('u') if ctrl_shortcut(key) => {
                if !self.query.is_empty() {
                    let selected_entry = self.selected_entry_anchor();
                    clear_input_at_cursor(&mut self.query, &mut self.query_cursor);
                    self.restore_selected_entry(selected_entry);
                }
                CommandPaletteAction::None
            }
            KeyCode::Char(c) if plain_shortcut(key) => {
                let selected_entry = self.selected_entry_anchor();
                insert_char_at_cursor(&mut self.query, &mut self.query_cursor, c);
                self.restore_selected_entry(selected_entry);
                CommandPaletteAction::None
            }
            _ => CommandPaletteAction::None,
        }
    }

    /// Returns palette entries whose aliases fuzzy-match the current query.
    /// Actions (if a resource context exists) come first, then navigation entries.
    fn filtered_entries(&self) -> Arc<Vec<PaletteEntry>> {
        if let Some(cached) = self.cached_filtered.borrow().as_ref() {
            return Arc::clone(cached);
        }

        let mut result = Vec::new();
        let q_lower = self.query.to_ascii_lowercase();

        if q_lower.is_empty() {
            result.extend(
                self.activity_entries
                    .iter()
                    .take(MAX_ACTIVITY_RESULTS)
                    .cloned()
                    .map(PaletteEntry::Activity),
            );
        } else {
            let mut matched_activities = Vec::with_capacity(MAX_ACTIVITY_RESULTS);
            for (index, entry) in self.activity_entries.iter().enumerate() {
                if let Some(score) = ranked_alias_score(&entry.aliases, &q_lower) {
                    push_ranked_match(
                        &mut matched_activities,
                        (score, index),
                        MAX_ACTIVITY_RESULTS,
                        |left, right| compare_activity_match(&self.activity_entries, left, right),
                    );
                }
            }
            matched_activities
                .sort_by(|left, right| compare_activity_match(&self.activity_entries, left, right));
            result.extend(
                matched_activities
                    .into_iter()
                    .map(|(_, index)| PaletteEntry::Activity(self.activity_entries[index].clone())),
            );
        }

        if !q_lower.is_empty() {
            let mut matched_resources = Vec::with_capacity(MAX_RESOURCE_RESULTS);
            for (index, entry) in self.resource_entries.iter().enumerate() {
                if let Some(score) = ranked_alias_score(&entry.aliases, &q_lower) {
                    push_ranked_match(
                        &mut matched_resources,
                        (score, index),
                        MAX_RESOURCE_RESULTS,
                        |left, right| compare_resource_match(&self.resource_entries, left, right),
                    );
                }
            }
            matched_resources
                .sort_by(|left, right| compare_resource_match(&self.resource_entries, left, right));
            result.extend(
                matched_resources
                    .into_iter()
                    .map(|(_, index)| PaletteEntry::Resource(self.resource_entries[index].clone())),
            );
        }

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
            for action in &self.extension_actions {
                if self.query.is_empty()
                    || action
                        .aliases
                        .iter()
                        .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::ExtensionAction(action.clone()));
                }
            }
            for action in &self.ai_actions {
                if self.query.is_empty()
                    || action
                        .aliases
                        .iter()
                        .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::AiAction(action.clone()));
                }
            }
            for runbook in &self.runbooks {
                if self.query.is_empty()
                    || runbook
                        .aliases
                        .iter()
                        .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::Runbook(runbook.clone()));
                }
            }
        }

        if self.resource_context.is_none() {
            for runbook in &self.runbooks {
                if self.query.is_empty()
                    || runbook
                        .aliases
                        .iter()
                        .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::Runbook(runbook.clone()));
                }
            }
        }

        if self.query.is_empty()
            || fuzzy_match("workspace", &q_lower)
            || fuzzy_match("save workspace", &q_lower)
            || fuzzy_match("layout", &q_lower)
        {
            result.push(PaletteEntry::SaveWorkspace);
        }

        if query_indicates_template_intent(&q_lower) {
            for kind in ResourceTemplateKind::ALL {
                if kind
                    .aliases()
                    .iter()
                    .any(|alias| fuzzy_match(alias, &self.query))
                {
                    result.push(PaletteEntry::Template(kind));
                }
            }
        }

        for name in &self.saved_workspaces {
            let lower = name.to_ascii_lowercase();
            if self.query.is_empty()
                || fuzzy_match(&lower, &q_lower)
                || fuzzy_match("workspace", &q_lower)
            {
                result.push(PaletteEntry::Workspace(name.clone()));
            }
        }

        for (name, hotkey) in &self.workspace_banks {
            let lower = name.to_ascii_lowercase();
            if self.query.is_empty()
                || fuzzy_match(&lower, &q_lower)
                || fuzzy_match("bank", &q_lower)
                || fuzzy_match("workspace bank", &q_lower)
            {
                result.push(PaletteEntry::WorkspaceBank {
                    name: name.clone(),
                    hotkey: hotkey.clone(),
                });
            }
        }

        // Column toggles (when query matches "columns", "toggle", or a column label)
        if let Some(cols) = &self.columns_info {
            for (id, label, visible) in cols {
                let label_lower = label.to_ascii_lowercase();
                if q_lower.is_empty()
                    || fuzzy_match("columns", &q_lower)
                    || fuzzy_match("toggle", &q_lower)
                    || fuzzy_match(&label_lower, &q_lower)
                {
                    result.push(PaletteEntry::ColumnToggle {
                        id: id.clone(),
                        label: label.clone(),
                        visible: *visible,
                    });
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

        let result = Arc::new(result);
        *self.cached_filtered.borrow_mut() = Some(Arc::clone(&result));
        result
    }

    /// Returns palette entries whose aliases fuzzy-match the current query.
    /// Actions (if a resource context exists) come first, then navigation entries.
    pub fn filtered(&self) -> Vec<PaletteEntry> {
        self.filtered_entries().as_ref().clone()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.is_open {
            return;
        }

        use crate::ui::components::default_theme;
        let theme = default_theme();
        let popup = command_palette_popup(area);
        let compact = use_compact_command_palette_layout(popup);

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

        let footer_groups = if compact {
            vec![
                vec![Span::styled(" [Enter] ", theme.keybind_key_style())],
                vec![Span::styled("select  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("close", theme.keybind_desc_style())],
            ]
        } else {
            vec![
                vec![Span::styled(" [↑↓] ", theme.keybind_key_style())],
                vec![Span::styled("navigate  ", theme.keybind_desc_style())],
                vec![Span::styled("[Enter] ", theme.keybind_key_style())],
                vec![Span::styled("select  ", theme.keybind_desc_style())],
                vec![Span::styled("[Esc] ", theme.keybind_key_style())],
                vec![Span::styled("close", theme.keybind_desc_style())],
            ]
        };
        let footer_lines = wrap_span_groups(&footer_groups, inner.width.max(1));
        let footer_height = wrapped_line_count(&footer_lines, inner.width.max(1)).max(1) as u16 + 1;

        let title = Line::from(vec![
            Span::styled(" ⌘ ", theme.title_style()),
            Span::styled("Action Palette", theme.title_style()),
            if compact {
                Span::raw("")
            } else {
                Span::styled("  · type to filter", theme.inactive_style())
            },
        ]);
        let title_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.header_bg));
        let title_lines = vec![title];
        let title_height = wrapped_line_count(&title_lines, inner.width.max(1)).max(1) as u16
            + u16::from(!compact);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(title_height),
                Constraint::Length(3),
                Constraint::Min(if compact { 1 } else { 3 }),
                Constraint::Length(footer_height),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(title_lines)
                .block(title_block)
                .wrap(Wrap { trim: false }),
            chunks[0],
        );

        let search_content = if self.query.is_empty() {
            Line::from(vec![
                Span::styled("  ", theme.inactive_style()),
                Span::styled("pods, api, history, rollout…", theme.inactive_style()),
            ])
        } else {
            cursor_visible_input_line(
                &[Span::styled("  : ".to_string(), theme.title_style())],
                &self.query,
                Some(self.query_cursor),
                Style::default().fg(theme.fg),
                theme.title_style(),
                &[],
                usize::from(chunks[1].width.saturating_sub(2).max(1)),
            )
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

        let matches = self.filtered_entries();
        let mut items: Vec<ListItem> = Vec::new();
        let mut item_heights = Vec::with_capacity(matches.len());

        if matches.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  No matches",
                theme.inactive_style(),
            ))));
        } else {
            let mut previous_section = None;
            for (selectable_idx, entry) in matches.iter().enumerate() {
                let section = palette_entry_section(entry);
                let section_header = (previous_section != Some(section)).then(|| section.title());
                previous_section = Some(section);
                let lines = palette_item_lines(
                    entry,
                    &theme,
                    selectable_idx == self.selected_index,
                    section_header,
                );
                item_heights.push(lines.len());
                items.push(ListItem::new(lines));
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
        let viewport_height = chunks[2].height.saturating_sub(2) as usize;
        let selected = (!matches.is_empty()).then_some(self.selected_index.min(matches.len() - 1));
        let offset = selected
            .map(|selected_index| {
                compute_palette_offset(&item_heights, selected_index, viewport_height)
            })
            .unwrap_or_default();
        let (scroll_total, scroll_position) = palette_scroll_metrics(&item_heights, offset);
        let mut state = ListState::default()
            .with_selected(selected)
            .with_offset(offset);
        frame.render_stateful_widget(List::new(items).block(list_block), chunks[2], &mut state);
        render_vertical_scrollbar(frame, chunks[2], scroll_total, scroll_position);

        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.statusbar_bg));
        frame.render_widget(
            Paragraph::new(footer_lines)
                .wrap(Wrap { trim: false })
                .block(footer_block),
            chunks[3],
        );
    }
}

fn query_indicates_template_intent(query: &str) -> bool {
    !query.is_empty()
        && TEMPLATE_INTENT_ALIASES
            .iter()
            .any(|alias| fuzzy_match(query, alias) || fuzzy_match(alias, query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_history::ActionKind;
    use crate::app::AppState;
    use crate::policy::ResourceActionContext;
    use crate::workbench::{PodLogsTabState, WorkbenchTabState};
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered_text(palette: &CommandPalette, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| palette.render(frame, frame.area()))
            .expect("command palette should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn ctx(resource: ResourceRef, node_unschedulable: Option<bool>) -> ResourceActionContext {
        ResourceActionContext {
            resource,
            node_unschedulable,
            cronjob_suspended: None,
            cronjob_history_logs_available: false,
            effective_logs_resource: None,
            effective_logs_authorization: None,
            action_authorizations: Default::default(),
        }
    }

    fn palette_resource_entry(resource: ResourceRef) -> PaletteResourceEntry {
        PaletteResourceEntry {
            resource,
            title: "api-0".into(),
            subtitle: "Pod · prod".into(),
            aliases: vec!["api-0".into(), "prod/api-0".into(), "pod".into()],
            badge_label: "Pods".into(),
        }
    }

    fn extension_action(id: &str, title: &str, alias: &str) -> LoadedExtensionAction {
        LoadedExtensionAction {
            id: id.into(),
            title: title.into(),
            description: None,
            aliases: vec![alias.into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            mode: crate::extensions::ExtensionExecutionMode::Background,
            command: crate::extensions::ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["get".into(), "pod".into()],
                cwd: None,
                env: Default::default(),
            },
        }
    }

    fn ai_action(id: &str, title: &str, alias: &str) -> LoadedAiAction {
        LoadedAiAction {
            id: id.into(),
            title: title.into(),
            description: None,
            aliases: vec![alias.into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            provider: crate::ai_actions::AiProviderConfig {
                provider: crate::ai_actions::AiProviderKind::ClaudeCli,
                model: "claude-cli".into(),
                api_key_env: String::new(),
                endpoint: None,
                timeout_secs: 15,
                max_output_tokens: 512,
                temperature: Some(0.1),
                command: None,
                args: Vec::new(),
                action: None,
            },
            workflow: crate::ai_actions::AiWorkflowKind::ExplainFailure,
            system_prompt: None,
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
        assert_eq!(p.filtered().len(), COMMANDS.len() + 1);
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
    fn filtered_gateway_queries_match_gateway_views() {
        let mut p = CommandPalette::default();
        p.open();
        for c in "gateway".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let results = p.filtered();
        assert!(results.contains(&PaletteEntry::Navigate(AppView::Gateways)));
        assert!(results.contains(&PaletteEntry::Navigate(AppView::GatewayClasses)));
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
        assert_eq!(p.selected_index, p.filtered().len() - 1);
    }

    #[test]
    fn arrow_navigation_moves_selection() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(p.selected_index, 1);
        p.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(p.selected_index, 0);
    }

    #[test]
    fn modified_enter_and_arrows_do_not_execute_or_navigate() {
        let mut p = CommandPalette::default();
        p.open();

        for (code, modifiers) in [
            (KeyCode::Enter, KeyModifiers::CONTROL),
            (KeyCode::Down, KeyModifiers::CONTROL),
            (KeyCode::Up, KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::ALT),
            (KeyCode::Down, KeyModifiers::ALT),
            (KeyCode::Up, KeyModifiers::ALT),
        ] {
            assert_eq!(
                p.handle_key(KeyEvent::new(code, modifiers)),
                CommandPaletteAction::None,
                "{code:?} {modifiers:?}"
            );
            assert_eq!(p.selected_index, 0);
        }
    }

    #[test]
    fn modified_escape_does_not_close_palette() {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::META,
            KeyModifiers::SUPER,
            KeyModifiers::CONTROL | KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::SUPER,
        ] {
            let mut p = CommandPalette::default();
            p.open();

            assert_eq!(
                p.handle_key(KeyEvent::new(KeyCode::Esc, modifiers)),
                CommandPaletteAction::None,
                "{modifiers:?}"
            );
            assert!(p.is_open(), "{modifiers:?}");
        }
    }

    #[test]
    fn typing_j_k_appends_to_query() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Char('j')));
        p.handle_key(KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(p.query, "jk");
        assert_eq!(p.selected_index, 0);
    }

    #[test]
    fn footer_does_not_advertise_jk_navigation_because_jk_filter_query() {
        let mut p = CommandPalette::default();
        p.open();

        let rendered = rendered_text(&p, 120, 40);

        assert!(rendered.contains("↑↓"));
        assert!(!rendered.contains("↑↓/jk"));
    }

    #[test]
    fn modified_chars_do_not_edit_query() {
        let mut p = CommandPalette::default();
        p.open();
        p.query = "seed".to_string();
        p.query_cursor = 2;
        p.handle_key(KeyEvent::new(
            KeyCode::Char('u'),
            crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT,
        ));
        p.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        p.handle_key(KeyEvent::new(
            KeyCode::Char('k'),
            crossterm::event::KeyModifiers::ALT,
        ));
        p.handle_key(KeyEvent::new(
            KeyCode::Char('m'),
            crossterm::event::KeyModifiers::META,
        ));
        for code in [
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
        ] {
            p.handle_key(KeyEvent::new(code, KeyModifiers::ALT));
            p.handle_key(KeyEvent::new(code, KeyModifiers::CONTROL));
        }
        assert_eq!(p.query, "seed");
        assert_eq!(p.query_cursor, 2);
    }

    #[test]
    fn query_supports_cursor_editing() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Char('a')));
        p.handle_key(KeyEvent::from(KeyCode::Char('c')));
        p.handle_key(KeyEvent::from(KeyCode::Left));
        p.handle_key(KeyEvent::from(KeyCode::Char('b')));

        assert_eq!(p.query, "abc");
    }

    #[test]
    fn query_supports_unicode_cursor_editing() {
        let mut p = CommandPalette::default();
        p.open();
        p.handle_key(KeyEvent::from(KeyCode::Char('a')));
        p.handle_key(KeyEvent::from(KeyCode::Char('å')));
        p.handle_key(KeyEvent::from(KeyCode::Char('b')));
        p.handle_key(KeyEvent::from(KeyCode::Left));
        p.handle_key(KeyEvent::from(KeyCode::Left));
        p.handle_key(KeyEvent::from(KeyCode::Char('β')));
        p.handle_key(KeyEvent::from(KeyCode::Delete));
        p.handle_key(KeyEvent::from(KeyCode::Backspace));

        assert_eq!(p.query, "ab");
        assert_eq!(p.query_cursor, 1);
    }

    #[test]
    fn filtered_empty_query_keeps_activity_recency_order() {
        let mut palette = CommandPalette::default();
        palette.set_activity_entries(vec![
            PaletteActivityEntry {
                title: "Newest".into(),
                subtitle: "recent".into(),
                aliases: vec!["newest".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Pods),
            },
            PaletteActivityEntry {
                title: "Older".into(),
                subtitle: "older".into(),
                aliases: vec!["older".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Services),
            },
        ]);

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.first(),
            Some(PaletteEntry::Activity(PaletteActivityEntry { title, .. })) if title == "Newest"
        ));
        assert!(matches!(
            filtered.get(1),
            Some(PaletteEntry::Activity(PaletteActivityEntry { title, .. })) if title == "Older"
        ));
    }

    #[test]
    fn activity_refresh_preserves_selected_entry_identity() {
        let mut palette = CommandPalette::default();
        palette.set_activity_entries(vec![
            PaletteActivityEntry {
                title: "Pods".into(),
                subtitle: "recent".into(),
                aliases: vec!["pods".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Pods),
            },
            PaletteActivityEntry {
                title: "Services".into(),
                subtitle: "recent".into(),
                aliases: vec!["services".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Services),
            },
        ]);
        palette.open();
        palette.handle_key(KeyEvent::from(KeyCode::Down));

        palette.set_activity_entries(vec![
            PaletteActivityEntry {
                title: "Dashboard".into(),
                subtitle: "recent".into(),
                aliases: vec!["dashboard".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Dashboard),
            },
            PaletteActivityEntry {
                title: "Pods".into(),
                subtitle: "recent".into(),
                aliases: vec!["pods".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Pods),
            },
            PaletteActivityEntry {
                title: "Services".into(),
                subtitle: "recent".into(),
                aliases: vec!["services".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Services),
            },
        ]);

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::Activity(PaletteActivityEntry { title, .. })) if title == "Services"
        ));
    }

    #[test]
    fn extension_reload_preserves_selected_action_by_id() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource, None)));
        palette.set_extension_actions(vec![extension_action("describe", "Describe", "diag")]);
        for ch in "diag".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }

        palette.set_extension_actions(vec![extension_action("describe", "Describe v2", "diag")]);

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::ExtensionAction(PaletteExtensionAction { id, title, .. }))
                if id == "describe" && title == "Describe v2"
        ));
    }

    #[test]
    fn ai_reload_preserves_selected_action_by_id() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource, None)));
        palette.set_ai_actions(vec![ai_action("ai_explain_failure", "Explain", "ai diag")]);
        for ch in "ai diag".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }

        palette.set_ai_actions(vec![ai_action(
            "ai_explain_failure",
            "Explain v2",
            "ai diag",
        )]);

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::AiAction(PaletteAiAction { id, title, .. }))
                if id == "ai_explain_failure" && title == "Explain v2"
        ));
    }

    #[test]
    fn reload_drops_stale_anchor_when_selected_action_disappears() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource, None)));
        palette.set_ai_actions(vec![
            ai_action("ai_describe", "Describe", "provider-action"),
            ai_action("ai_debug", "Debug", "provider-action"),
        ]);
        for ch in "provider-action".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }
        let filtered = palette.filtered();
        palette.selected_index = filtered
            .iter()
            .position(|entry| {
                matches!(entry, PaletteEntry::AiAction(PaletteAiAction { id, .. }) if id == "ai_debug")
            })
            .expect("debug action");
        palette.selection_anchor = palette.selected_entry_snapshot();

        palette.set_ai_actions(vec![ai_action(
            "ai_describe",
            "Describe",
            "provider-action",
        )]);
        palette.set_ai_actions(vec![
            ai_action("ai_describe", "Describe", "provider-action"),
            ai_action("ai_debug", "Debug", "provider-action"),
        ]);

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::AiAction(PaletteAiAction { id, .. })) if id == "ai_describe"
        ));
    }

    #[test]
    fn runbook_reload_preserves_selected_entry_by_id_and_resource() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource.clone(), None)));
        palette.set_runbooks(
            vec![LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure".into(),
                description: None,
                aliases: vec!["incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: None,
                steps: Vec::new(),
            }],
            Some(resource.clone()),
        );
        for ch in "incident".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }

        palette.set_runbooks(
            vec![LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure v2".into(),
                description: None,
                aliases: vec!["incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: None,
                steps: Vec::new(),
            }],
            Some(resource),
        );

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::Runbook(PaletteRunbookAction { id, title, .. }))
                if id == "pod_failure" && title == "Pod Failure v2"
        ));
    }

    #[test]
    fn query_roundtrip_preserves_selected_entry_across_zero_matches() {
        let mut palette = CommandPalette::default();
        palette.set_activity_entries(vec![
            PaletteActivityEntry {
                title: "Pods".into(),
                subtitle: "recent".into(),
                aliases: vec!["pods".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Pods),
            },
            PaletteActivityEntry {
                title: "Services".into(),
                subtitle: "recent".into(),
                aliases: vec!["services".into()],
                badge_label: "Recent".into(),
                target: PaletteActivityTarget::Navigate(AppView::Services),
            },
        ]);
        palette.open();
        palette.selected_index = palette
            .filtered()
            .iter()
            .position(|entry| {
                matches!(
                    entry,
                    PaletteEntry::Activity(PaletteActivityEntry { title, .. }) if title == "Services"
                )
            })
            .expect("services entry");
        palette.selection_anchor = palette.selected_entry_snapshot();

        for ch in ['z', 'z', 'z', 'z'] {
            palette.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }
        assert!(palette.filtered().is_empty());

        palette.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        let filtered = palette.filtered();
        assert!(matches!(
            filtered.get(palette.selected_index),
            Some(PaletteEntry::Activity(PaletteActivityEntry { title, .. })) if title == "Services"
        ));
    }

    #[test]
    fn compute_palette_offset_keeps_selected_multiline_item_visible() {
        let heights = vec![3, 2, 2, 2];

        assert_eq!(compute_palette_offset(&heights, 0, 5), 0);
        assert_eq!(compute_palette_offset(&heights, 2, 5), 1);
        assert_eq!(compute_palette_offset(&heights, 3, 5), 2);
    }

    #[test]
    fn palette_scroll_metrics_use_visual_row_offsets() {
        let heights = vec![2, 3, 1, 4];

        assert_eq!(palette_scroll_metrics(&heights, 0), (10, 0));
        assert_eq!(palette_scroll_metrics(&heights, 2), (10, 5));
        assert_eq!(palette_scroll_metrics(&heights, 99), (10, 6));
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
        assert!(
            entries
                .iter()
                .any(|e| e.action == DetailAction::DebugContainer)
        );
        assert!(
            entries
                .iter()
                .any(|e| e.action == DetailAction::CheckNetworkConnectivity)
        );
        assert!(!entries.iter().any(|e| e.action == DetailAction::Scale));
    }

    #[test]
    fn palette_entry_action_aliases_deployment() {
        let resource = ctx(
            ResourceRef::Deployment("api".into(), "default".into()),
            None,
        );
        let entries = action_entries_for_resource(Some(&resource));
        let rollout_entry = entries
            .iter()
            .find(|e| e.action == DetailAction::ViewRollout)
            .expect("rollout action");
        let restart_entry = entries
            .iter()
            .find(|e| e.action == DetailAction::Restart)
            .expect("restart action");

        assert!(rollout_entry.aliases.contains(&"rollout"));
        assert!(entries.iter().any(|e| e.action == DetailAction::Scale));
        assert!(entries.iter().any(|e| e.action == DetailAction::Restart));
        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Exec));
        assert!(!restart_entry.aliases.contains(&"rollout"));
    }

    #[test]
    fn palette_entry_action_aliases_helm_release() {
        let resource = ctx(
            ResourceRef::HelmRelease("web".into(), "default".into()),
            None,
        );
        let entries = action_entries_for_resource(Some(&resource));

        assert!(
            entries
                .iter()
                .any(|entry| entry.action == DetailAction::ViewHelmHistory)
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.action == DetailAction::EditYaml)
        );
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
    fn palette_entry_cronjob_actions_follow_suspend_state() {
        let mut schedulable = ctx(ResourceRef::CronJob("nightly".into(), "ops".into()), None);
        schedulable.cronjob_suspended = Some(false);
        let entries = action_entries_for_resource(Some(&schedulable));
        assert!(entries.iter().any(|e| e.action == DetailAction::Trigger));
        assert!(
            entries
                .iter()
                .any(|e| e.action == DetailAction::SuspendCronJob)
        );
        assert!(
            !entries
                .iter()
                .any(|e| e.action == DetailAction::ResumeCronJob)
        );

        let mut suspended = ctx(ResourceRef::CronJob("nightly".into(), "ops".into()), None);
        suspended.cronjob_suspended = Some(true);
        let entries = action_entries_for_resource(Some(&suspended));
        assert!(
            entries
                .iter()
                .any(|e| e.action == DetailAction::ResumeCronJob)
        );
        assert!(
            !entries
                .iter()
                .any(|e| e.action == DetailAction::SuspendCronJob)
        );
    }

    #[test]
    fn palette_hides_denied_permission_actions() {
        let mut resource = ctx(ResourceRef::Pod("test".into(), "default".into()), None);
        resource.action_authorizations.insert(
            DetailAction::Exec,
            crate::authorization::DetailActionAuthorization::Denied,
        );
        let entries = action_entries_for_resource(Some(&resource));

        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Exec));
    }

    #[test]
    fn palette_hides_unknown_strict_actions_but_keeps_reads() {
        let mut resource = ctx(ResourceRef::Pod("test".into(), "default".into()), None);
        resource.action_authorizations.insert(
            DetailAction::Exec,
            crate::authorization::DetailActionAuthorization::Unknown,
        );
        resource.action_authorizations.insert(
            DetailAction::Logs,
            crate::authorization::DetailActionAuthorization::Unknown,
        );
        let entries = action_entries_for_resource(Some(&resource));

        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
        assert!(!entries.iter().any(|e| e.action == DetailAction::Exec));
    }

    #[test]
    fn palette_offers_cronjob_logs_when_selected_run_has_access() {
        let mut resource = ctx(ResourceRef::CronJob("nightly".into(), "ops".into()), None);
        resource.cronjob_history_logs_available = true;
        let entries = action_entries_for_resource(Some(&resource));

        assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
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
    fn filtered_secret_query_matches_decoded_action() {
        let mut palette = CommandPalette::default();
        let resource = ctx(ResourceRef::Secret("app".into(), "default".into()), None);
        palette.open_with_context(Some(resource));
        for c in "decode".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let entries = palette.filtered();
        assert!(entries.iter().any(|entry| {
            matches!(entry, PaletteEntry::Action(DetailAction::ViewDecodedSecret))
        }));
    }

    #[test]
    fn filtered_helm_query_matches_history_action() {
        let mut palette = CommandPalette::default();
        let resource = ctx(
            ResourceRef::HelmRelease("web".into(), "default".into()),
            None,
        );
        palette.open_with_context(Some(resource));
        for c in "rollback".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let entries = palette.filtered();
        assert!(
            entries.iter().any(|entry| {
                matches!(entry, PaletteEntry::Action(DetailAction::ViewHelmHistory))
            })
        );
    }

    #[test]
    fn filtered_bookmark_query_matches_bookmark_action() {
        let mut palette = CommandPalette::default();
        let resource = ctx(ResourceRef::Pod("api".into(), "default".into()), None);
        palette.open_with_context(Some(resource));
        for c in "bookmark".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let entries = palette.filtered();
        assert!(
            entries.iter().any(|entry| {
                matches!(entry, PaletteEntry::Action(DetailAction::ToggleBookmark))
            })
        );
    }

    #[test]
    fn filtered_traffic_query_matches_traffic_debug_action() {
        let mut palette = CommandPalette::default();
        let resource = ctx(ResourceRef::Service("api".into(), "default".into()), None);
        palette.open_with_context(Some(resource));
        for c in "traffic".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let entries = palette.filtered();
        assert!(entries.iter().any(|entry| {
            matches!(entry, PaletteEntry::Action(DetailAction::ViewTrafficDebug))
        }));
    }

    #[test]
    fn filtered_no_context_has_no_actions() {
        let mut palette = CommandPalette::default();
        palette.open_with_context(None);
        let entries = palette.filtered();
        assert!(
            entries
                .iter()
                .all(|e| matches!(e, PaletteEntry::Navigate(_) | PaletteEntry::SaveWorkspace))
        );
    }

    #[test]
    fn create_query_exposes_template_entry() {
        let mut palette = CommandPalette::default();
        palette.open();
        for c in "create deployment".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let entries = palette.filtered();
        assert!(entries.contains(&PaletteEntry::Template(ResourceTemplateKind::Deployment)));
    }

    #[test]
    fn partial_template_intent_query_exposes_template_entries() {
        let mut palette = CommandPalette::default();
        palette.open();
        for c in "templ".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let entries = palette.filtered();
        assert!(entries.contains(&PaletteEntry::Template(ResourceTemplateKind::Deployment)));
        assert!(entries.contains(&PaletteEntry::Template(ResourceTemplateKind::ConfigMap)));
    }

    #[test]
    fn partial_create_query_exposes_template_entries() {
        let mut palette = CommandPalette::default();
        palette.open();
        for c in "cre".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let entries = palette.filtered();
        assert!(entries.contains(&PaletteEntry::Template(ResourceTemplateKind::Deployment)));
        assert!(entries.contains(&PaletteEntry::Template(ResourceTemplateKind::ConfigMap)));
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

    #[test]
    fn palette_includes_workspace_entries() {
        let mut palette = CommandPalette::default();
        palette.set_workspace_info(
            vec!["prod pods".into()],
            vec![("prod bank".into(), Some("alt+1".into()))],
        );
        palette.open();

        let entries = palette.filtered();
        assert!(entries.contains(&PaletteEntry::SaveWorkspace));
        assert!(entries.contains(&PaletteEntry::Workspace("prod pods".into())));
        assert!(entries.contains(&PaletteEntry::WorkspaceBank {
            name: "prod bank".into(),
            hotkey: Some("alt+1".into()),
        }));
    }

    #[test]
    fn palette_enter_can_apply_workspace() {
        let mut palette = CommandPalette::default();
        palette.set_workspace_info(vec!["incident".into()], Vec::new());
        palette.open();
        for c in "incident".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::ApplyWorkspace("incident".into())
        );
    }

    #[test]
    fn palette_enter_can_execute_extension() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource.clone(), None)));
        palette.set_extension_actions(vec![LoadedExtensionAction {
            id: "describe".into(),
            title: "Describe Pod".into(),
            description: None,
            aliases: vec!["describe pod".into(), "diag".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: Some("gp".into()),
            mode: crate::extensions::ExtensionExecutionMode::Background,
            command: crate::extensions::ExtensionCommandConfig {
                program: "kubectl".into(),
                args: vec!["describe".into(), "pod".into()],
                cwd: None,
                env: Default::default(),
            },
        }]);
        for c in "diag".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::ExecuteExtension("describe".into(), resource)
        );
    }

    #[test]
    fn palette_enter_can_execute_native_ai_action() {
        let resource = ResourceRef::Pod("api".into(), "default".into());
        let mut palette = CommandPalette::default();
        palette.open_with_context(Some(ctx(resource.clone(), None)));
        palette.set_ai_actions(vec![LoadedAiAction {
            id: "ai_explain_failure".into(),
            title: "Explain Failure".into(),
            description: None,
            aliases: vec!["explain failure".into(), "why failing".into()],
            resource_kinds: vec!["Pod".into()],
            shortcut: None,
            provider: crate::ai_actions::AiProviderConfig {
                provider: crate::ai_actions::AiProviderKind::ClaudeCli,
                model: "claude-cli".into(),
                api_key_env: String::new(),
                endpoint: None,
                timeout_secs: 15,
                max_output_tokens: 512,
                temperature: Some(0.1),
                command: None,
                args: Vec::new(),
                action: None,
            },
            workflow: crate::ai_actions::AiWorkflowKind::ExplainFailure,
            system_prompt: None,
        }]);
        for c in "explain failure".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::ExecuteAi("ai_explain_failure".into(), resource)
        );
    }

    #[test]
    fn palette_includes_runbook_entries() {
        let mut palette = CommandPalette::default();
        palette.set_runbooks(
            vec![LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure Triage".into(),
                description: Some("Investigate failing pods".into()),
                aliases: vec!["pod failure".into(), "incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: Some("rp".into()),
                steps: Vec::new(),
            }],
            Some(ResourceRef::Pod("api".into(), "prod".into())),
        );
        palette.open_with_context(Some(ctx(
            ResourceRef::Pod("api".into(), "prod".into()),
            None,
        )));

        let entries = palette.filtered();
        assert!(entries.iter().any(|entry| matches!(
            entry,
            PaletteEntry::Runbook(runbook) if runbook.id == "pod_failure"
        )));
    }

    #[test]
    fn enter_opens_selected_runbook() {
        let mut palette = CommandPalette::default();
        let resource = ResourceRef::Pod("api".into(), "prod".into());
        palette.set_runbooks(
            vec![LoadedRunbook {
                id: "pod_failure".into(),
                title: "Pod Failure Triage".into(),
                description: Some("Investigate failing pods".into()),
                aliases: vec!["pod failure".into(), "incident".into()],
                resource_kinds: vec!["Pod".into()],
                shortcut: None,
                steps: Vec::new(),
            }],
            Some(resource.clone()),
        );
        palette.open_with_context(Some(ctx(resource.clone(), None)));
        for c in "incident".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::OpenRunbook("pod_failure".into(), Some(resource))
        );
    }

    #[test]
    fn palette_includes_global_resource_entries() {
        let mut palette = CommandPalette::default();
        palette.set_resource_entries(vec![PaletteResourceEntry {
            resource: ResourceRef::Deployment("api".into(), "prod".into()),
            title: "api".into(),
            subtitle: "Deployment · prod".into(),
            aliases: vec![
                "api".into(),
                "deployment".into(),
                "prod/api".into(),
                "team=platform".into(),
            ],
            badge_label: "Deployments".into(),
        }]);
        palette.open();
        for c in "platform".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert!(palette.filtered().iter().any(|entry| matches!(
            entry,
            PaletteEntry::Resource(resource)
                if resource.resource == ResourceRef::Deployment("api".into(), "prod".into())
        )));
    }

    #[test]
    fn resource_search_alias_matching_stays_case_insensitive() {
        let mut palette = CommandPalette::default();
        palette.set_resource_entries(vec![PaletteResourceEntry {
            resource: ResourceRef::Deployment("api".into(), "prod".into()),
            title: "api".into(),
            subtitle: "Deployment · prod".into(),
            aliases: vec![
                "Deployment".into(),
                "Prod/API".into(),
                "Team=Platform".into(),
            ],
            badge_label: "Deployments".into(),
        }]);
        palette.open();
        for c in "platform".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert!(palette.filtered().iter().any(|entry| matches!(
        entry,
        PaletteEntry::Resource(resource)
            if resource.resource == ResourceRef::Deployment("api".into(), "prod".into())
        )));
    }

    #[test]
    fn resource_search_top_k_keeps_late_exact_match_before_early_fuzzy_matches() {
        let mut entries = (0..80)
            .map(|index| PaletteResourceEntry {
                resource: ResourceRef::Pod(format!("fuzzy-{index:02}"), "prod".into()),
                title: format!("fuzzy-{index:02}"),
                subtitle: "Pod · prod".into(),
                aliases: vec![format!("alpha platform item {index:02}")],
                badge_label: "Pods".into(),
            })
            .collect::<Vec<_>>();
        entries.push(PaletteResourceEntry {
            resource: ResourceRef::Deployment("api".into(), "prod".into()),
            title: "api".into(),
            subtitle: "Deployment · prod".into(),
            aliases: vec!["api".into()],
            badge_label: "Deployments".into(),
        });

        let mut palette = CommandPalette::default();
        palette.set_resource_entries(entries);
        palette.open();
        for c in "api".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let first_resource = palette
            .filtered()
            .into_iter()
            .find_map(|entry| match entry {
                PaletteEntry::Resource(resource) => Some(resource),
                _ => None,
            })
            .expect("resource match should exist");

        assert_eq!(
            first_resource.resource,
            ResourceRef::Deployment("api".into(), "prod".into())
        );
    }

    #[test]
    fn same_resource_entries_arc_preserves_cached_filtered_entries() {
        let mut palette = CommandPalette::default();
        let entries = Arc::new(vec![palette_resource_entry(ResourceRef::Pod(
            "api-0".into(),
            "prod".into(),
        ))]);
        palette.set_resource_entries(Arc::clone(&entries));
        palette.open();
        for c in "api-0".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        let first = palette.filtered_entries();
        palette.set_resource_entries(Arc::clone(&entries));
        let second = palette.filtered_entries();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn noop_query_edit_keys_preserve_cached_filtered_entries() {
        let mut palette = CommandPalette::default();
        palette.open();

        let first = palette.filtered_entries();
        palette.handle_key(KeyEvent::from(KeyCode::Backspace));
        let after_empty_backspace = palette.filtered_entries();
        palette.handle_key(KeyEvent::from(KeyCode::Delete));
        let after_empty_delete = palette.filtered_entries();
        palette.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        let after_empty_ctrl_u = palette.filtered_entries();

        assert!(Arc::ptr_eq(&first, &after_empty_backspace));
        assert!(Arc::ptr_eq(&first, &after_empty_delete));
        assert!(Arc::ptr_eq(&first, &after_empty_ctrl_u));

        for c in "api".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let query_results = palette.filtered_entries();
        palette.handle_key(KeyEvent::from(KeyCode::Delete));
        let after_end_delete = palette.filtered_entries();

        assert!(Arc::ptr_eq(&query_results, &after_end_delete));
    }

    #[test]
    fn close_drops_cached_filtered_entries() {
        let mut palette = CommandPalette::default();
        palette.set_resource_entries(vec![palette_resource_entry(ResourceRef::Pod(
            "api-0".into(),
            "prod".into(),
        ))]);
        palette.open();
        for c in "api-0".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert!(palette.cached_filtered.borrow().is_some());
        palette.close();

        assert!(palette.cached_filtered.borrow().is_none());
    }

    #[test]
    fn enter_can_jump_to_global_resource_result() {
        let mut palette = CommandPalette::default();
        let resource = ResourceRef::Pod("api-0".into(), "prod".into());
        palette.set_resource_entries(vec![PaletteResourceEntry {
            resource: resource.clone(),
            title: "api-0".into(),
            subtitle: "Pod · prod".into(),
            aliases: vec!["api-0".into(), "prod/api-0".into(), "pod".into()],
            badge_label: "Pods".into(),
        }]);
        palette.open();
        for c in "api-0".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::JumpToResource(resource)
        );
    }

    #[test]
    fn enter_can_activate_workbench_activity() {
        let mut palette = CommandPalette::default();
        let key = WorkbenchTabKey::ResourceEvents(ResourceRef::Pod("api-0".into(), "prod".into()));
        palette.set_activity_entries(vec![PaletteActivityEntry {
            title: "Events prod/api-0".into(),
            subtitle: "Open workbench tab".into(),
            aliases: vec!["timeline".into(), "events api-0".into()],
            badge_label: "Tab".into(),
            target: PaletteActivityTarget::WorkbenchTab(key.clone()),
        }]);
        palette.open();
        for c in "timeline".chars() {
            palette.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(
            palette.handle_key(KeyEvent::from(KeyCode::Enter)),
            CommandPaletteAction::ActivateWorkbenchTab(key)
        );
    }

    #[test]
    fn command_palette_popup_stays_within_small_terminal() {
        let popup = command_palette_popup(Rect::new(0, 0, 40, 10));
        assert!(popup.width <= 40);
        assert!(popup.height <= 10);
    }

    #[test]
    fn compact_command_palette_layout_activates_on_small_terminal() {
        assert!(use_compact_command_palette_layout(command_palette_popup(
            Rect::new(0, 0, 40, 10),
        )));
        assert!(!use_compact_command_palette_layout(command_palette_popup(
            Rect::new(0, 0, 120, 40),
        )));
    }

    #[test]
    fn collect_activity_entries_includes_recent_view_jumps() {
        let mut app = AppState::default();
        app.record_recent_view_jump(AppView::Pods);
        app.record_recent_view_jump(AppView::Services);

        let entries = collect_activity_entries(&app);

        assert!(entries.iter().any(|entry| {
            matches!(
                entry.target,
                PaletteActivityTarget::Navigate(AppView::Services)
            ) && entry.subtitle == "Recent view jump"
        }));
        assert!(entries.iter().any(|entry| {
            matches!(entry.target, PaletteActivityTarget::Navigate(AppView::Pods))
        }));
    }

    #[test]
    fn collect_activity_entries_filters_recent_items_to_current_scope() {
        let mut app = AppState::default();
        app.current_context_name = Some("prod".into());
        app.set_namespace("payments".into());
        app.record_recent_resource_jump(ResourceRef::Pod("api-0".into(), "payments".into()));
        app.record_action_pending(
            ActionKind::Restart,
            AppView::Pods,
            Some(ResourceRef::Pod("api-0".into(), "payments".into())),
            "Pod api-0",
            "Restart requested",
        );

        app.current_context_name = Some("staging".into());
        app.set_namespace("default".into());
        app.record_recent_resource_jump(ResourceRef::Pod("web-0".into(), "default".into()));
        app.record_action_pending(
            ActionKind::Restart,
            AppView::Pods,
            Some(ResourceRef::Pod("web-0".into(), "default".into())),
            "Pod web-0",
            "Restart requested",
        );

        let entries = collect_activity_entries(&app);

        assert!(entries.iter().any(|entry| {
            matches!(
                &entry.target,
                PaletteActivityTarget::Resource(resource)
                    if resource == &ResourceRef::Pod("web-0".into(), "default".into())
            )
        }));
        assert!(!entries.iter().any(|entry| {
            matches!(
                &entry.target,
                PaletteActivityTarget::Resource(resource)
                    if resource == &ResourceRef::Pod("api-0".into(), "payments".into())
            )
        }));
    }

    #[test]
    fn collect_activity_entries_prefers_action_history_target_over_duplicate_recent_jump() {
        let mut app = AppState::default();
        let resource = ResourceRef::Pod("api-0".into(), "prod".into());
        app.record_action_pending(
            ActionKind::Restart,
            AppView::Pods,
            Some(resource.clone()),
            "Pod api-0",
            "Restart requested",
        );
        app.record_recent_resource_jump(resource.clone());

        let entries = collect_activity_entries(&app);
        let matching: Vec<_> = entries
            .iter()
            .filter(|entry| {
                matches!(
                    &entry.target,
                    PaletteActivityTarget::Resource(candidate) if candidate == &resource
                )
            })
            .collect();

        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].badge_label, "Pending");
        assert_eq!(matching[0].subtitle, "Restart requested");
    }

    #[test]
    fn collect_activity_entries_action_history_aliases_include_namespace() {
        let mut app = AppState::default();
        let resource = ResourceRef::Pod("api-0".into(), "prod".into());
        app.record_action_pending(
            ActionKind::Restart,
            AppView::Pods,
            Some(resource),
            "Pod api-0",
            "Restart requested",
        );

        let entries = collect_activity_entries(&app);
        let entry = entries
            .iter()
            .find(|entry| matches!(entry.target, PaletteActivityTarget::Resource(_)))
            .expect("resource activity entry");

        assert!(entry.aliases.iter().any(|alias| alias == "prod"));
        assert!(entry.aliases.iter().any(|alias| alias == "pod/prod/api-0"));
    }

    #[test]
    fn collect_activity_entries_includes_open_workbench_tabs() {
        let mut app = AppState::default();
        let resource = ResourceRef::Pod("api-0".into(), "prod".into());
        let key = WorkbenchTabKey::PodLogs(resource.clone());
        app.workbench_mut()
            .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));

        let entries = collect_activity_entries(&app);

        assert!(entries.iter().any(|entry| {
            matches!(
                &entry.target,
                PaletteActivityTarget::WorkbenchTab(candidate) if candidate == &key
            ) && entry.badge_label == "Tab"
        }));
    }

    #[test]
    fn empty_query_activity_prioritizes_history_before_open_tabs() {
        let mut app = AppState::default();
        for idx in 0..(MAX_ACTIVITY_RESULTS + 4) {
            let resource = ResourceRef::Pod(format!("pod-{idx}"), "prod".into());
            app.workbench_mut()
                .open_tab(WorkbenchTabState::PodLogs(PodLogsTabState::new(resource)));
        }
        app.record_action_pending(
            ActionKind::Restart,
            AppView::Deployments,
            Some(ResourceRef::Deployment("api".into(), "prod".into())),
            "Deployment api",
            "Restart requested",
        );

        let mut palette = CommandPalette::default();
        palette.set_activity_entries(collect_activity_entries(&app));

        let filtered = palette.filtered();
        assert!(filtered.iter().take(MAX_ACTIVITY_RESULTS).any(|entry| {
            matches!(
                entry,
                PaletteEntry::Activity(PaletteActivityEntry { subtitle, badge_label, .. })
                    if subtitle == "Restart requested" && badge_label == "Pending"
            )
        }));
    }
}
