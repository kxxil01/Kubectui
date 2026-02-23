//! Reusable UI widgets and building blocks.

pub mod input_field;
pub mod namespace_picker;
pub mod port_forward_dialog;
pub mod probe_panel;

pub mod scale_dialog;
pub use input_field::InputFieldWidget;
pub use namespace_picker::{NamespacePicker, NamespacePickerAction};
pub use port_forward_dialog::{FormField, PortForwardAction, PortForwardDialog, PortForwardMode};
pub use probe_panel::ProbePanelState;
pub use scale_dialog::{ScaleAction, ScaleDialogState, ScaleField, render_scale_dialog};

use ratatui::{
    layout::Rect,
    prelude::{Color, Frame, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::app::AppView;

/// Renders the top header containing title and cluster metadata.
pub fn render_header(frame: &mut Frame, area: Rect, title: &str, cluster_meta: &str) {
    let text = Line::from(vec![
        Span::styled(title, Style::default().fg(Color::Cyan)),
        Span::raw("  |  "),
        Span::styled(cluster_meta, Style::default().fg(Color::Gray)),
    ]);

    let widget = Paragraph::new(text).block(default_block("Cluster"));
    frame.render_widget(widget, area);
}

/// Renders tab navigation for all primary app views.
pub fn render_tabs(frame: &mut Frame, area: Rect, views: &[AppView], active: AppView) {
    let titles: Vec<Line> = views
        .iter()
        .map(|view| Line::from(Span::raw(format!(" {} ", view.label()))))
        .collect();

    let selected = views.iter().position(|view| *view == active).unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(default_block("Navigation"))
        .select(selected)
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Cyan));

    frame.render_widget(tabs, area);
}

/// Renders a bottom status bar. Turns red when `is_error` is true.
pub fn render_status_bar(frame: &mut Frame, area: Rect, message: &str, is_error: bool) {
    let style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };

    let widget = Paragraph::new(message)
        .style(style)
        .block(default_block("Status"));

    frame.render_widget(widget, area);
}

/// Shared default bordered block style used across widgets.
pub fn default_block(title: &str) -> Block<'_> {
    Block::default().title(title).borders(Borders::ALL)
}
