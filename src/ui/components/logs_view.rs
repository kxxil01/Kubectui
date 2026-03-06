//! Logs Viewer Component for KubecTUI

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Wrap,
    },
    Frame,
};
use crossterm::event::{KeyCode, KeyEvent};

use crate::ui::theme::Theme;
use crate::ui::contains_ci;
use crate::k8s::logs::PodRef;

const MAX_LOG_LINES: usize = 50_000;
const MAX_LINE_LENGTH: usize = 10_000;

/// Action emitted by logs view based on keyboard input
#[derive(Debug, Clone, PartialEq)]
pub enum LogsViewAction {
    None,
    Close,
    ScrollUp,
    ScrollDown,
    ScrollStart,
    ScrollEnd,
    ToggleFollow,
    Search,
    ClearSearch,
    UpdateSearch(String),
}

/// Represents a log line with metadata
#[derive(Debug, Clone)]
pub struct LogLine {
    pub number: usize,
    pub content: String,
    pub level: LogLevel,
}

/// Log level classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

impl LogLevel {
    pub fn from_line(content: &str) -> Self {
        if contains_ci(content, "ERROR") || contains_ci(content, "ERR") {
            LogLevel::Error
        } else if contains_ci(content, "WARN") {
            LogLevel::Warn
        } else if contains_ci(content, "INFO") {
            LogLevel::Info
        } else if contains_ci(content, "DEBUG") {
            LogLevel::Debug
        } else if contains_ci(content, "TRACE") {
            LogLevel::Trace
        } else {
            LogLevel::Unknown
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
            LogLevel::Unknown => "LOG",
        }
    }
}

/// State for the logs viewer component
#[derive(Debug, Clone)]
pub struct LogsViewState {
    /// Reference to the pod
    pub pod_ref: Option<PodRef>,
    /// Log lines
    pub logs: Vec<LogLine>,
    /// Current scroll position (top line index)
    pub scroll_pos: usize,
    /// Whether following the end of logs
    pub follow_mode: bool,
    /// Search query
    pub search_query: String,
    /// Whether in search mode
    pub in_search_mode: bool,
    /// Filtered log line indices (when searching)
    pub filtered_indices: Option<Vec<usize>>,
    /// Error message
    pub error: Option<String>,
    /// Whether loading logs
    pub loading: bool,
}

impl LogsViewState {
    pub fn new() -> Self {
        Self {
            pod_ref: None,
            logs: Vec::new(),
            scroll_pos: 0,
            follow_mode: true,
            search_query: String::new(),
            in_search_mode: false,
            filtered_indices: None,
            error: None,
            loading: false,
        }
    }

    pub fn with_pod(pod_ref: PodRef) -> Self {
        Self {
            pod_ref: Some(pod_ref),
            follow_mode: true,
            ..Default::default()
        }
    }

    /// Add a log line
    pub fn add_log(&mut self, content: String) {
        let content = if content.len() > MAX_LINE_LENGTH {
            let mut truncated = content;
            truncated.truncate(MAX_LINE_LENGTH);
            truncated.push_str("…[truncated]");
            truncated
        } else {
            content
        };
        let number = self.logs.len() + 1;
        let level = LogLevel::from_line(&content);
        self.logs.push(LogLine {
            number,
            content,
            level,
        });

        if self.logs.len() > MAX_LOG_LINES {
            let excess = self.logs.len() - MAX_LOG_LINES;
            self.logs.drain(..excess);
        }

        if self.follow_mode {
            self.scroll_to_end();
        }
    }

    /// Add multiple log lines at once
    pub fn add_logs(&mut self, lines: Vec<String>) {
        for line in lines {
            self.add_log(line);
        }
    }

    /// Clear all logs
    pub fn clear(&mut self) {
        self.logs.clear();
        self.scroll_pos = 0;
        self.search_query.clear();
        self.filtered_indices = None;
    }

    /// Scroll up by one line
    pub fn scroll_up(&mut self) {
        self.scroll_pos = self.scroll_pos.saturating_sub(1);
        self.follow_mode = false;
    }

    /// Scroll down by one line
    pub fn scroll_down(&mut self, visible_lines: usize) {
        let max_scroll = self.logs.len().saturating_sub(visible_lines);
        self.scroll_pos = (self.scroll_pos + 1).min(max_scroll);
    }

    /// Scroll to start (top)
    pub fn scroll_to_start(&mut self) {
        self.scroll_pos = 0;
        self.follow_mode = false;
    }

    /// Scroll to end (bottom)
    pub fn scroll_to_end(&mut self) {
        if self.logs.is_empty() {
            self.scroll_pos = 0;
        } else {
            self.scroll_pos = self.logs.len().saturating_sub(1);
        }
    }

    /// Toggle follow mode
    pub fn toggle_follow(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode {
            self.scroll_to_end();
        }
    }

    /// Update search query and apply filter
    pub fn update_search(&mut self, query: String) {
        self.search_query = query.clone();
        
        if query.is_empty() {
            self.filtered_indices = None;
            self.search_query.clear();
        } else {
            let indices: Vec<usize> = self
                .logs
                .iter()
                .enumerate()
                .filter(|(_, log)| contains_ci(&log.content, &query))
                .map(|(i, _)| i)
                .collect();

            if !indices.is_empty() {
                self.filtered_indices = Some(indices);
            }
        }
    }

    /// Get visible logs (applying search filter if active)
    pub fn get_visible_logs(&self, start: usize, count: usize) -> Vec<&LogLine> {
        if let Some(ref indices) = self.filtered_indices {
            indices
                .iter()
                .skip(start)
                .take(count)
                .filter_map(|&idx| self.logs.get(idx))
                .collect()
        } else {
            self.logs.iter().skip(start).take(count).collect()
        }
    }

    /// Get total visible line count
    pub fn visible_line_count(&self) -> usize {
        if let Some(ref indices) = self.filtered_indices {
            indices.len()
        } else {
            self.logs.len()
        }
    }

    /// Handle keyboard input
    pub fn handle_key(&mut self, key: KeyEvent, visible_lines: usize) -> LogsViewAction {
        if self.in_search_mode {
            return self.handle_search_input(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => LogsViewAction::Close,
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_down(visible_lines);
                LogsViewAction::ScrollDown
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_up();
                LogsViewAction::ScrollUp
            }
            KeyCode::Char('G') => {
                self.scroll_to_end();
                LogsViewAction::ScrollEnd
            }
            KeyCode::Char('g') => {
                self.scroll_to_start();
                LogsViewAction::ScrollStart
            }
            KeyCode::Char('f') => {
                self.toggle_follow();
                LogsViewAction::ToggleFollow
            }
            KeyCode::Char('/') => {
                self.in_search_mode = true;
                LogsViewAction::Search
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.clear();
                LogsViewAction::ClearSearch
            }
            _ => LogsViewAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> LogsViewAction {
        match key.code {
            KeyCode::Esc => {
                self.in_search_mode = false;
                self.search_query.clear();
                self.filtered_indices = None;
                LogsViewAction::ClearSearch
            }
            KeyCode::Enter => {
                self.in_search_mode = false;
                LogsViewAction::None
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.update_search(self.search_query.clone());
                LogsViewAction::UpdateSearch(self.search_query.clone())
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.update_search(self.search_query.clone());
                LogsViewAction::UpdateSearch(self.search_query.clone())
            }
            _ => LogsViewAction::None,
        }
    }
}

impl Default for LogsViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the logs view component
pub fn render_logs_view(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),           // Title bar
            Constraint::Min(10),              // Log area
            Constraint::Length(if state.in_search_mode { 3 } else { 0 }), // Search box
            Constraint::Length(2),            // Status/help footer
        ])
        .split(area);

    render_title_bar(frame, theme, state, chunks[0]);
    render_log_area(frame, theme, state, chunks[1]);

    if state.in_search_mode && chunks[2].height > 0 {
        render_search_box(frame, theme, state, chunks[2]);
    }

    render_footer(frame, theme, state, chunks[3]);
}

fn render_title_bar(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.header_bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let total = state.visible_line_count();

    let mut spans = vec![
        Span::styled(" 📋 ", theme.title_style()),
    ];

    if let Some(ref pod_ref) = state.pod_ref {
        spans.push(Span::styled(pod_ref.name.clone(), theme.title_style()));
        spans.push(Span::styled(" · ", theme.inactive_style()));
        spans.push(Span::styled(pod_ref.namespace.clone(), Style::default().fg(theme.accent2)));
        spans.push(Span::styled("  ", theme.inactive_style()));
    }

    if state.follow_mode {
        spans.push(Span::styled(" ⟳ FOLLOW ", theme.badge_success_style()));
        spans.push(Span::raw("  "));
    }

    if !state.search_query.is_empty() {
        spans.push(Span::styled(" / ", theme.keybind_key_style()));
        spans.push(Span::styled(state.search_query.clone(), Style::default().fg(theme.fg)));
        spans.push(Span::styled(
            format!(" ({} matches) ", total),
            theme.inactive_style(),
        ));
    } else {
        spans.push(Span::styled(
            format!(" {} lines ", total),
            theme.inactive_style(),
        ));
    }

    if state.loading {
        spans.push(Span::styled(" ⟳ Loading… ", Style::default().fg(theme.warning)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_log_area(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.logs.is_empty() {
        let msg = if state.loading {
            Paragraph::new(Span::styled("  ⟳ Loading logs…", Style::default().fg(theme.warning)))
        } else if let Some(ref err) = state.error {
            Paragraph::new(Span::styled(format!("  ✗ {err}"), theme.badge_error_style()))
        } else {
            Paragraph::new(Span::styled("  No logs available", theme.inactive_style()))
        };
        frame.render_widget(msg, inner);
        return;
    }

    let visible_lines = (inner.height as usize).max(1);
    let logs = state.get_visible_logs(state.scroll_pos, visible_lines);
    let total = state.visible_line_count();
    let line_num_width = total.to_string().len().max(3);

    let lines: Vec<Line> = logs
        .iter()
        .map(|log| {
            let line_num = format!("{:>width$}", log.number, width = line_num_width);
            let level_style = theme.get_log_level_style(log.level.label());
            let content_style = match log.level {
                LogLevel::Error => Style::default().fg(theme.error),
                LogLevel::Warn => Style::default().fg(theme.warning),
                LogLevel::Info => Style::default().fg(theme.fg),
                LogLevel::Debug => Style::default().fg(theme.fg_dim),
                LogLevel::Trace => Style::default().fg(theme.muted),
                LogLevel::Unknown => Style::default().fg(theme.fg_dim),
            };

            let mut spans = vec![
                Span::styled(format!("{line_num} "), theme.inactive_style()),
                Span::styled("│ ", theme.inactive_style()),
                Span::styled(
                    format!("{:<5}", log.level.label()),
                    level_style,
                ),
                Span::styled(" ", Style::default()),
            ];

            let has_match = !state.search_query.is_empty()
                && contains_ci(&log.content, &state.search_query);

            if has_match {
                let highlight_style = Style::default()
                    .fg(theme.bg)
                    .bg(theme.warning);
                let lower_content = log.content.to_ascii_lowercase();
                let lower_needle = state.search_query.to_ascii_lowercase();
                let mut last_end = 0usize;
                for (start, _) in lower_content.match_indices(&lower_needle) {
                    if start < last_end {
                        continue;
                    }
                    if start > last_end {
                        spans.push(Span::styled(
                            log.content[last_end..start].to_string(),
                            content_style,
                        ));
                    }
                    spans.push(Span::styled(
                        log.content[start..start + lower_needle.len()].to_string(),
                        highlight_style,
                    ));
                    last_end = start + lower_needle.len();
                }
                if last_end < log.content.len() {
                    spans.push(Span::styled(
                        log.content[last_end..].to_string(),
                        content_style,
                    ));
                }
            } else {
                spans.push(Span::styled(log.content.clone(), content_style));
            }

            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("│"))
        .thumb_symbol("█");
    let mut scrollbar_state = ScrollbarState::new(total).position(state.scroll_pos);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut scrollbar_state,
    );
}

fn render_search_box(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_active_style())
        .style(Style::default().bg(theme.bg_surface));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let search_line = if state.search_query.is_empty() {
        Line::from(vec![
            Span::styled(" / ", theme.keybind_key_style()),
            Span::styled("Type to search…", theme.inactive_style()),
        ])
    } else {
        Line::from(vec![
            Span::styled(" / ", theme.keybind_key_style()),
            Span::styled(state.search_query.clone(), Style::default().fg(theme.fg)),
            Span::styled("█", theme.keybind_key_style()),
        ])
    };

    frame.render_widget(Paragraph::new(search_line), inner);
}

fn render_footer(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme.border_style())
        .style(Style::default().bg(theme.statusbar_bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ref error) = state.error {
        let error_line = Line::from(vec![
            Span::styled(" ✗ ", theme.badge_error_style()),
            Span::styled(error.clone(), Style::default().fg(theme.error)),
        ]);
        frame.render_widget(Paragraph::new(error_line), inner);
    } else {
        let help_line = Line::from(vec![
            Span::styled(" [j/k] ", theme.keybind_key_style()),
            Span::styled("scroll  ", theme.keybind_desc_style()),
            Span::styled("[g/G] ", theme.keybind_key_style()),
            Span::styled("top/bottom  ", theme.keybind_desc_style()),
            Span::styled("[f] ", theme.keybind_key_style()),
            Span::styled("follow  ", theme.keybind_desc_style()),
            Span::styled("[/] ", theme.keybind_key_style()),
            Span::styled("search  ", theme.keybind_desc_style()),
            Span::styled("[c] ", theme.keybind_key_style()),
            Span::styled("clear  ", theme.keybind_desc_style()),
            Span::styled("[q] ", theme.keybind_key_style()),
            Span::styled("close", theme.keybind_desc_style()),
        ]);
        frame.render_widget(Paragraph::new(help_line), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_detection() {
        assert_eq!(LogLevel::from_line("ERROR: something"), LogLevel::Error);
        assert_eq!(LogLevel::from_line("WARN: warning"), LogLevel::Warn);
        assert_eq!(LogLevel::from_line("INFO: info message"), LogLevel::Info);
        assert_eq!(LogLevel::from_line("DEBUG: debug info"), LogLevel::Debug);
        assert_eq!(LogLevel::from_line("plain text"), LogLevel::Unknown);
    }

    #[test]
    fn test_logs_view_state_creation() {
        let state = LogsViewState::new();
        assert!(state.pod_ref.is_none());
        assert_eq!(state.scroll_pos, 0);
        assert!(state.follow_mode);
        assert!(!state.in_search_mode);
    }

    #[test]
    fn test_add_log_increments_line_number() {
        let mut state = LogsViewState::new();
        state.add_log("First log".to_string());
        state.add_log("Second log".to_string());

        assert_eq!(state.logs.len(), 2);
        assert_eq!(state.logs[0].number, 1);
        assert_eq!(state.logs[1].number, 2);
    }

    #[test]
    fn test_scroll_to_end() {
        let mut state = LogsViewState::new();
        for i in 0..10 {
            state.add_log(format!("log line {}", i));
        }

        state.scroll_to_end();

        assert_eq!(state.scroll_pos, 9);
    }

    #[test]
    fn test_toggle_follow_mode() {
        let mut state = LogsViewState::new();
        state.add_log("log".to_string());

        assert!(state.follow_mode);
        state.toggle_follow();
        assert!(!state.follow_mode);
        state.toggle_follow();
        assert!(state.follow_mode);
    }

    #[test]
    fn test_search_filtering() {
        let mut state = LogsViewState::new();
        state.add_log("ERROR: database connection failed".to_string());
        state.add_log("INFO: service started".to_string());
        state.add_log("ERROR: timeout occurred".to_string());

        state.update_search("ERROR".to_string());

        assert_eq!(state.filtered_indices, Some(vec![0, 2]));
        assert_eq!(state.visible_line_count(), 2);
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut state = LogsViewState::new();
        state.add_log("Warning: low memory".to_string());
        state.add_log("INFO: running".to_string());

        state.update_search("warning".to_string());

        assert_eq!(state.visible_line_count(), 1);
    }

    #[test]
    fn test_clear_all_logs() {
        let mut state = LogsViewState::new();
        state.add_log("log 1".to_string());
        state.add_log("log 2".to_string());
        state.scroll_pos = 5;

        state.clear();

        assert!(state.logs.is_empty());
        assert_eq!(state.scroll_pos, 0);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn test_follow_mode_on_new_log() {
        let mut state = LogsViewState::new();
        for _ in 0..5 {
            state.add_log("log".to_string());
        }

        state.scroll_to_start();
        assert_eq!(state.scroll_pos, 0);

        state.add_log("new log".to_string());
        assert_eq!(state.scroll_pos, 5);
    }
}
