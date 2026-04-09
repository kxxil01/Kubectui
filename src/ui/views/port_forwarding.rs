//! Port Forwarding list view — shows active tunnels.

use ratatui::{
    layout::{Constraint, Rect},
    prelude::{Frame, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row},
};

use crate::{
    app::AppView,
    icons::view_icon,
    k8s::portforward::TunnelState,
    state::port_forward::TunnelRegistry,
    ui::{
        TableFrame, components::default_theme, contains_ci, render_table_frame, striped_row_style,
        table_viewport_rows, table_window,
    },
};

const NARROW_PORT_FORWARD_WIDTH: u16 = 96;

fn port_forward_widths(area: Rect) -> [Constraint; 5] {
    if area.width < NARROW_PORT_FORWARD_WIDTH {
        [
            Constraint::Min(18),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Length(8),
        ]
    } else {
        [
            Constraint::Min(22),
            Constraint::Length(16),
            Constraint::Length(15),
            Constraint::Length(8),
            Constraint::Length(10),
        ]
    }
}

pub fn render_port_forwarding(
    frame: &mut Frame,
    area: Rect,
    registry: &TunnelRegistry,
    selected: usize,
    search: &str,
    focused: bool,
) {
    let theme = default_theme();
    let tunnels = registry.ordered_tunnels();

    let items: Vec<_> = tunnels
        .iter()
        .filter(|t| {
            search.is_empty()
                || contains_ci(&t.target.pod_name, search)
                || contains_ci(&t.target.namespace, search)
        })
        .collect();

    let clamped_selected = if items.is_empty() {
        0
    } else {
        selected.min(items.len() - 1)
    };

    if items.is_empty() {
        let (icon, icon_color, msg) = if search.is_empty() {
            (
                "○ ",
                theme.fg_dim,
                "No active port forwards. Open a Pod detail and press [f] to create one.",
            )
        } else {
            ("⊘ ", theme.warning, "No matching tunnels.")
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::styled(msg, theme.inactive_style()),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .block(crate::ui::components::content_block(
                &format!(
                    "{}Port Forwarding",
                    view_icon(AppView::PortForwarding).active()
                ),
                focused,
            )),
            area,
        );
        return;
    }

    let total = items.len();
    let window = table_window(total, clamped_selected, table_viewport_rows(area));
    let rows: Vec<Row> = items[window.start..window.end]
        .iter()
        .enumerate()
        .map(|(offset, t)| {
            let idx = window.start + offset;
            let state_str = match t.state {
                TunnelState::Starting => "Starting",
                TunnelState::Active => "Active",
                TunnelState::Error => "Error",
                TunnelState::Closing => "Closing",
                TunnelState::Closed => "Closed",
            };
            Row::new(vec![
                Cell::from(t.target.pod_name.clone()),
                Cell::from(t.target.namespace.clone()),
                Cell::from(t.local_addr.to_string()),
                Cell::from(t.target.remote_port.to_string()),
                Cell::from(state_str),
            ])
            .style(striped_row_style(idx, &theme))
        })
        .collect();

    let header = Row::new(vec!["POD", "NAMESPACE", "LOCAL", "REMOTE", "STATUS"])
        .style(theme.header_style())
        .height(1);

    let title = if search.is_empty() {
        format!(
            " {}Port Forwarding ({}) ",
            view_icon(AppView::PortForwarding).active(),
            items.len()
        )
    } else {
        format!(
            " {}Port Forwarding ({} of {}) [/{search}] ",
            view_icon(AppView::PortForwarding).active(),
            items.len(),
            tunnels.len()
        )
    };
    let widths = port_forward_widths(area);
    render_table_frame(
        frame,
        area,
        TableFrame {
            rows,
            header,
            widths: &widths,
            title: &title,
            focused,
            window,
            total,
            selected: clamped_selected,
        },
        &theme,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::{net::SocketAddr, str::FromStr};

    fn tunnel(
        id: &str,
        pod: &str,
        namespace: &str,
        port: u16,
    ) -> crate::k8s::portforward::PortForwardTunnelInfo {
        crate::k8s::portforward::PortForwardTunnelInfo {
            id: id.to_string(),
            target: crate::k8s::portforward::PortForwardTarget::new(namespace, pod, port),
            local_addr: SocketAddr::from_str(&format!("127.0.0.1:{port}")).expect("socket"),
            state: TunnelState::Active,
        }
    }

    #[test]
    fn render_port_forwarding_windows_selected_row_into_view() {
        let mut registry = TunnelRegistry::new();
        for idx in 0..24 {
            registry.add_tunnel(tunnel(
                &format!("tunnel-{idx}"),
                &format!("pod-{idx}"),
                "default",
                3000 + idx as u16,
            ));
        }
        let backend = TestBackend::new(72, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| {
                render_port_forwarding(frame, frame.area(), &registry, 18, "", true);
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }

        assert!(out.contains("pod-18"));
        assert!(!out.contains("pod-0"));
    }

    #[test]
    fn port_forward_widths_switch_to_compact_profile() {
        let widths = port_forward_widths(Rect::new(0, 0, 88, 20));
        assert_eq!(widths[0], Constraint::Min(18));
        assert_eq!(widths[1], Constraint::Length(12));
        assert_eq!(widths[4], Constraint::Length(8));
    }

    #[test]
    fn port_forward_widths_keep_wide_profile() {
        let widths = port_forward_widths(Rect::new(0, 0, 120, 20));
        assert_eq!(widths[0], Constraint::Min(22));
        assert_eq!(widths[1], Constraint::Length(16));
        assert_eq!(widths[4], Constraint::Length(10));
    }
}
