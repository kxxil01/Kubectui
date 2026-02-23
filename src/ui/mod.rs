//! User interface composition and rendering utilities.

pub mod components;
pub mod theme;
pub mod views;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    prelude::Frame,
    text::Span,
    widgets::{
        Cell, HighlightSpacing, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
};

use crate::{
    app::{AppState, AppView},
    state::ClusterSnapshot,
    ui::components::{active_block, default_block, default_theme},
};

/// Renders a full frame for the current app and cluster state.
pub fn render(frame: &mut Frame, app: &AppState, cluster: &ClusterSnapshot) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(2)])
        .split(frame.area());

    components::render_header(
        frame,
        root[0],
        "KubecTUI v0.1.0",
        cluster.cluster_summary(),
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(root[1]);

    components::render_sidebar(frame, body[0], app.view(), app.sidebar_cursor, &app.collapsed_groups, app.focus);

    let content = body[1];

    match app.view() {
        AppView::Dashboard => views::dashboard::render_dashboard(frame, content, cluster),
        AppView::Nodes => views::nodes::render_nodes(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Pods => {
            render_pods_widget(frame, content, cluster, app.selected_idx(), app.search_query());
        }
        AppView::ReplicaSets => views::replicasets::render_replicasets(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::ReplicationControllers => {
            views::replication_controllers::render_replication_controllers(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            )
        }
        AppView::HelmCharts
        | AppView::HelmReleases
        | AppView::Endpoints
        | AppView::Ingresses
        | AppView::IngressClasses
        | AppView::NetworkPolicies
        | AppView::ConfigMaps
        | AppView::Secrets
        | AppView::HPAs
        | AppView::PriorityClasses
        | AppView::PersistentVolumeClaims
        | AppView::PersistentVolumes
        | AppView::StorageClasses
        | AppView::Namespaces
        | AppView::Events => render_placeholder(frame, content, app.view().label()),
        AppView::Services => views::services::render_services(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Deployments => views::deployments::render_deployments(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::StatefulSets => views::statefulsets::render_statefulsets(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::DaemonSets => views::daemonsets::render_daemonsets(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Jobs => views::jobs::render_jobs(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::CronJobs => views::cronjobs::render_cronjobs(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::ServiceAccounts => views::security::service_accounts::render_service_accounts(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Roles => views::security::roles::render_roles(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::RoleBindings => views::security::role_bindings::render_role_bindings(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::ClusterRoles => views::security::cluster_roles::render_cluster_roles(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::ClusterRoleBindings => {
            views::security::cluster_role_bindings::render_cluster_role_bindings(
                frame,
                content,
                cluster,
                app.selected_idx(),
                app.search_query(),
            )
        }
        AppView::ResourceQuotas => views::governance::quotas::render_resource_quotas(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::LimitRanges => views::governance::limits::render_limit_ranges(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::PodDisruptionBudgets => views::governance::pdbs::render_pdbs(
            frame,
            content,
            cluster,
            app.selected_idx(),
            app.search_query(),
        ),
        AppView::Extensions => views::extensions::render_extensions(frame, content, cluster, app),
    }

    let status = if let Some(err) = app.error_message() {
        format!("[{}] ERROR: {err}", app.get_namespace())
    } else if app.is_search_mode() {
        format!("[{}] Search: {}", app.get_namespace(), app.search_query())
    } else {
        format!(
            "[{}]  [j/k] navigate • [/] search • [~] namespace • [c] context • [Enter] detail • [r] refresh • [q] quit",
            app.get_namespace()
        )
    };

    components::render_status_bar(frame, root[2], &status, app.error_message().is_some());

    if let Some(detail_state) = app.detail_view.as_ref() {
        views::detail::render_detail(frame, frame.area(), detail_state);
    }

    if app.is_namespace_picker_open() {
        app.namespace_picker().render(frame, frame.area());
    }

    if app.is_context_picker_open() {
        app.context_picker.render(frame, frame.area());
    }

    if app.command_palette.is_open() {
        app.command_palette.render(frame, frame.area());
    }

    if app.confirm_quit {
        render_quit_confirm(frame, frame.area());
    }
}

fn render_quit_confirm(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::{
        style::Modifier,
        text::Line,
        widgets::{Block, BorderType, Borders, Clear},
    };

    let theme = default_theme();

    let w = 36u16;
    let h = 5u16;
    let popup = ratatui::layout::Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.badge_error_style())
        .style(ratatui::style::Style::default().bg(theme.bg));
    frame.render_widget(block, popup);

    let inner = ratatui::layout::Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Quit KubecTUI? ", theme.title_style()),
        ])),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  [y/q/Enter] ",
                ratatui::style::Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("yes  ", theme.inactive_style()),
            Span::styled("[any] ", theme.keybind_key_style()),
            Span::styled("cancel", theme.keybind_desc_style()),
        ])),
        chunks[1],
    );
}

fn render_placeholder(frame: &mut Frame, area: ratatui::layout::Rect, label: &str) {
    use ratatui::{
        style::Modifier,
        text::Line,
        widgets::{Block, BorderType, Borders, Paragraph},
    };
    let theme = default_theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(ratatui::style::Style::default().bg(theme.bg));
    let text = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            ratatui::text::Span::styled(
                format!("  {label}"),
                ratatui::style::Style::default()
                    .fg(theme.fg_dim)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![ratatui::text::Span::styled(
            "  Coming soon — not yet implemented",
            ratatui::style::Style::default().fg(theme.fg_dim),
        )]),
    ])
    .block(block);
    frame.render_widget(text, area);
}

fn render_pods_widget(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    cluster: &ClusterSnapshot,
    selected_idx: usize,
    query: &str,
) {
    let theme = default_theme();

    let filtered: Vec<_> = cluster
        .pods
        .iter()
        .filter(|p| {
            if query.is_empty() {
                true
            } else {
                let q = query.to_lowercase();
                p.name.to_lowercase().contains(&q)
                    || p.namespace.to_lowercase().contains(&q)
                    || p.status.to_lowercase().contains(&q)
            }
        })
        .collect();

    if filtered.is_empty() {
        let msg = if cluster.pods.is_empty() {
            "  No pods available"
        } else {
            "  No pods match the search query"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, theme.inactive_style()))
                .block(default_block("Pods")),
            area,
        );
        return;
    }

    let total = filtered.len();
    let selected = selected_idx.min(total.saturating_sub(1));

    let header = Row::new([
        Cell::from(Span::styled("  Name", theme.header_style())),
        Cell::from(Span::styled("Namespace", theme.header_style())),
        Cell::from(Span::styled("Status", theme.header_style())),
        Cell::from(Span::styled("Node", theme.header_style())),
        Cell::from(Span::styled("Restarts", theme.header_style())),
        Cell::from(Span::styled("Age", theme.header_style())),
    ])
    .height(1)
    .style(theme.header_style());

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(idx, pod)| {
            let status_style = theme.get_status_style(&pod.status);
            let restart_style = if pod.restarts > 5 {
                theme.badge_error_style()
            } else if pod.restarts > 0 {
                theme.badge_warning_style()
            } else {
                theme.inactive_style()
            };
            let row_style = if idx % 2 == 0 {
                ratatui::prelude::Style::default().bg(theme.bg)
            } else {
                theme.row_alt_style()
            };

            let age = pod
                .created_at
                .map(|ts| {
                    let delta = chrono::Utc::now().signed_duration_since(ts);
                    let days = delta.num_days();
                    let hours = delta.num_hours() % 24;
                    let mins = delta.num_minutes() % 60;
                    if days > 0 {
                        format!("{days}d{hours}h")
                    } else if hours > 0 {
                        format!("{hours}h{mins}m")
                    } else {
                        format!("{mins}m")
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {}", pod.name),
                    ratatui::prelude::Style::default().fg(theme.fg),
                )),
                Cell::from(Span::styled(
                    pod.namespace.clone(),
                    ratatui::prelude::Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(pod.status.clone(), status_style)),
                Cell::from(Span::styled(
                    pod.node.clone().unwrap_or_else(|| "n/a".to_string()),
                    ratatui::prelude::Style::default().fg(theme.fg_dim),
                )),
                Cell::from(Span::styled(pod.restarts.to_string(), restart_style)),
                Cell::from(Span::styled(age, theme.inactive_style())),
            ])
            .style(row_style)
        })
        .collect();

    let mut table_state = TableState::default().with_selected(Some(selected));

    let title = format!(" 🐳 Pods ({total}) ");
    let block = if query.is_empty() {
        active_block(&title)
    } else {
        active_block(&format!("{title} [/{query}]"))
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(28),
            Constraint::Length(18),
            Constraint::Length(20),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme.selection_style())
    .highlight_symbol(theme.highlight_symbol())
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut table_state);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");

    let mut scrollbar_state = ScrollbarState::new(total).position(selected);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut scrollbar_state,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        app::{AppState, AppView, DetailMetadata, DetailViewState, ResourceRef},
        k8s::dtos::{
            ClusterRoleBindingInfo, ClusterRoleInfo, CronJobInfo, CustomResourceDefinitionInfo,
            CustomResourceInfo, DaemonSetInfo, DeploymentInfo, JobInfo, LimitRangeInfo, NodeInfo,
            PodDisruptionBudgetInfo, PodInfo, ResourceQuotaInfo, RoleBindingInfo, RoleInfo,
            ServiceAccountInfo, ServiceInfo, StatefulSetInfo,
        },
        state::ClusterSnapshot,
    };

    use super::*;

    fn draw(app: &AppState, snapshot: &ClusterSnapshot) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, app, snapshot))
            .expect("render should not panic");
    }

    fn app_with_view(view: AppView) -> AppState {
        let mut app = AppState::default();
        while app.view() != view {
            app.handle_key_event(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Tab,
            ));
        }
        app
    }

    /// Verifies dashboard renders without panic for empty snapshot.
    #[test]
    fn render_dashboard_empty_snapshot_smoke() {
        let app = app_with_view(AppView::Dashboard);
        draw(&app, &ClusterSnapshot::default());
    }

    /// Verifies dashboard renders without panic for populated snapshot.
    #[test]
    fn render_dashboard_full_snapshot_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.nodes.push(NodeInfo {
            name: "n1".to_string(),
            ready: true,
            ..NodeInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });
        snapshot.services.push(ServiceInfo {
            name: "svc".to_string(),
            namespace: "default".to_string(),
            type_: "ClusterIP".to_string(),
            service_type: "ClusterIP".to_string(),
            ..ServiceInfo::default()
        });
        snapshot.deployments.push(DeploymentInfo {
            name: "dep".to_string(),
            namespace: "default".to_string(),
            ready: "1/1".to_string(),
            ..DeploymentInfo::default()
        });

        let app = app_with_view(AppView::Dashboard);
        draw(&app, &snapshot);
    }

    /// Verifies nodes view renders without panic for multiple list sizes.
    #[test]
    fn render_nodes_various_sizes_smoke() {
        let app = app_with_view(AppView::Nodes);

        for size in [0, 1, 100, 1000] {
            let mut snapshot = ClusterSnapshot::default();
            for i in 0..size {
                snapshot.nodes.push(NodeInfo {
                    name: format!("node-{i}"),
                    ready: i % 2 == 0,
                    role: if i % 3 == 0 { "master" } else { "worker" }.to_string(),
                    ..NodeInfo::default()
                });
            }
            draw(&app, &snapshot);
        }
    }

    /// Verifies services view renders without panic for mixed service types.
    #[test]
    fn render_services_mixed_types_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for t in ["ClusterIP", "NodePort", "LoadBalancer", "ExternalName"] {
            snapshot.services.push(ServiceInfo {
                name: format!("svc-{t}"),
                namespace: "default".to_string(),
                type_: t.to_string(),
                service_type: t.to_string(),
                ports: vec!["80/TCP".to_string(), "443/TCP".to_string()],
                ..ServiceInfo::default()
            });
        }

        let app = app_with_view(AppView::Services);
        draw(&app, &snapshot);
    }

    /// Verifies deployments view renders without panic for mixed health values.
    #[test]
    fn render_deployments_mixed_health_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        for ready in ["3/3", "1/3", "0/3"] {
            snapshot.deployments.push(DeploymentInfo {
                name: format!("dep-{ready}"),
                namespace: "default".to_string(),
                ready: ready.to_string(),
                ..DeploymentInfo::default()
            });
        }

        let app = app_with_view(AppView::Deployments);
        draw(&app, &snapshot);
    }

    /// Verifies StatefulSets view renders without panic for mixed readiness states.
    #[test]
    fn render_statefulsets_mixed_readiness_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-ready".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 3,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db-partial".to_string(),
            namespace: "default".to_string(),
            desired_replicas: 3,
            ready_replicas: 1,
            service_name: "db-headless".to_string(),
            image: Some("postgres:16".to_string()),
            ..StatefulSetInfo::default()
        });

        let app = app_with_view(AppView::StatefulSets);
        draw(&app, &snapshot);
    }

    /// Verifies DaemonSets view renders without panic for mixed desired/ready/unavailable counts.
    #[test]
    fn render_daemonsets_mixed_counts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-ok".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 10,
            unavailable_count: 0,
            image: Some("agent:1".to_string()),
            ..DaemonSetInfo::default()
        });
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent-warn".to_string(),
            namespace: "kube-system".to_string(),
            desired_count: 10,
            ready_count: 8,
            unavailable_count: 2,
            image: Some("agent:2".to_string()),
            ..DaemonSetInfo::default()
        });

        let app = app_with_view(AppView::DaemonSets);
        draw(&app, &snapshot);
    }

    /// Verifies Jobs view renders without panic for mixed status values.
    #[test]
    fn render_jobs_mixed_status_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.jobs.push(JobInfo {
            name: "batch-success".to_string(),
            namespace: "default".to_string(),
            status: "Succeeded".to_string(),
            completions: "1/1".to_string(),
            ..JobInfo::default()
        });
        snapshot.jobs.push(JobInfo {
            name: "batch-running".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            completions: "0/1".to_string(),
            ..JobInfo::default()
        });

        let app = app_with_view(AppView::Jobs);
        draw(&app, &snapshot);
    }

    /// Verifies CronJobs view renders without panic for suspended and active rows.
    #[test]
    fn render_cronjobs_suspend_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cronjobs.push(CronJobInfo {
            name: "nightly".to_string(),
            namespace: "default".to_string(),
            schedule: "0 0 * * *".to_string(),
            suspend: false,
            ..CronJobInfo::default()
        });
        snapshot.cronjobs.push(CronJobInfo {
            name: "paused".to_string(),
            namespace: "default".to_string(),
            schedule: "*/15 * * * *".to_string(),
            suspend: true,
            ..CronJobInfo::default()
        });

        let app = app_with_view(AppView::CronJobs);
        draw(&app, &snapshot);
    }

    /// Verifies ResourceQuotas governance view renders usage bands without panic.
    #[test]
    fn render_resource_quotas_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.resource_quotas.push(ResourceQuotaInfo {
            name: "rq-default".to_string(),
            namespace: "default".to_string(),
            percent_used: [("pods".to_string(), 85.0)].into_iter().collect(),
            ..ResourceQuotaInfo::default()
        });

        let app = app_with_view(AppView::ResourceQuotas);
        draw(&app, &snapshot);
    }

    /// Verifies LimitRanges governance view renders limits summary without panic.
    #[test]
    fn render_limit_ranges_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.limit_ranges.push(LimitRangeInfo {
            name: "limits-default".to_string(),
            namespace: "default".to_string(),
            ..LimitRangeInfo::default()
        });

        let app = app_with_view(AppView::LimitRanges);
        draw(&app, &snapshot);
    }

    /// Verifies PDB governance view renders disruption stats without panic.
    #[test]
    fn render_pdbs_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .pod_disruption_budgets
            .push(PodDisruptionBudgetInfo {
                name: "web-pdb".to_string(),
                namespace: "default".to_string(),
                min_available: Some("1".to_string()),
                disruptions_allowed: 1,
                ..PodDisruptionBudgetInfo::default()
            });

        let app = app_with_view(AppView::PodDisruptionBudgets);
        draw(&app, &snapshot);
    }

    /// Verifies ServiceAccounts view renders without panic.
    #[test]
    fn render_service_accounts_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.service_accounts.push(ServiceAccountInfo {
            name: "default".to_string(),
            namespace: "default".to_string(),
            secrets_count: 1,
            image_pull_secrets_count: 0,
            automount_service_account_token: Some(true),
            ..ServiceAccountInfo::default()
        });

        let app = app_with_view(AppView::ServiceAccounts);
        draw(&app, &snapshot);
    }

    /// Verifies Roles view renders rule details without panic.
    #[test]
    fn render_roles_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.roles.push(RoleInfo {
            name: "reader".to_string(),
            namespace: "default".to_string(),
            ..RoleInfo::default()
        });

        let app = app_with_view(AppView::Roles);
        draw(&app, &snapshot);
    }

    /// Verifies RoleBindings view renders subject details without panic.
    #[test]
    fn render_role_bindings_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.role_bindings.push(RoleBindingInfo {
            name: "reader-binding".to_string(),
            namespace: "default".to_string(),
            role_ref_kind: "Role".to_string(),
            role_ref_name: "reader".to_string(),
            ..RoleBindingInfo::default()
        });

        let app = app_with_view(AppView::RoleBindings);
        draw(&app, &snapshot);
    }

    /// Verifies ClusterRoles and ClusterRoleBindings views render without panic.
    #[test]
    fn render_cluster_rbac_views_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_roles.push(ClusterRoleInfo {
            name: "cluster-admin".to_string(),
            ..ClusterRoleInfo::default()
        });
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "cluster-admin-binding".to_string(),
            role_ref_kind: "ClusterRole".to_string(),
            role_ref_name: "cluster-admin".to_string(),
            ..ClusterRoleBindingInfo::default()
        });

        let app_roles = app_with_view(AppView::ClusterRoles);
        draw(&app_roles, &snapshot);

        let app_bindings = app_with_view(AppView::ClusterRoleBindings);
        draw(&app_bindings, &snapshot);
    }

    #[test]
    fn render_extensions_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot
            .custom_resource_definitions
            .push(CustomResourceDefinitionInfo {
                name: "widgets.demo.io".to_string(),
                group: "demo.io".to_string(),
                version: "v1".to_string(),
                kind: "Widget".to_string(),
                plural: "widgets".to_string(),
                scope: "Namespaced".to_string(),
                instances: 1,
            });

        let mut app = app_with_view(AppView::Extensions);
        app.extension_instances = vec![CustomResourceInfo {
            name: "sample".to_string(),
            namespace: Some("default".to_string()),
            ..CustomResourceInfo::default()
        }];

        draw(&app, &snapshot);
    }

    /// Verifies detail modal overlay renders on top of list view without panic.
    #[test]
    fn render_detail_overlay_smoke() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.pods.push(PodInfo {
            name: "p1".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            ..PodInfo::default()
        });

        let mut app = app_with_view(AppView::Pods);
        app.detail_view = Some(DetailViewState {
            resource: Some(ResourceRef::Pod("p1".to_string(), "default".to_string())),
            metadata: DetailMetadata {
                name: "p1".to_string(),
                namespace: Some("default".to_string()),
                ..DetailMetadata::default()
            },
            yaml: Some("kind: Pod\nmetadata:\n  name: p1\n".to_string()),
            ..DetailViewState::default()
        });

        draw(&app, &snapshot);
    }
}
