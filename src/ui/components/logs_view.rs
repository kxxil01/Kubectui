//! Logs Viewer Component for KubecTUI

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crossterm::event::{KeyCode, KeyEvent};

use crate::ui::theme::Theme;
use crate::k8s::logs::PodRef;

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
        let upper = content.to_uppercase();
        if upper.contains("ERROR") || upper.contains("ERR") {
            LogLevel::Error
        } else if upper.contains("WARN") {
            LogLevel::Warn
        } else if upper.contains("INFO") {
            LogLevel::Info
        } else if upper.contains("DEBUG") {
            LogLevel::Debug
        } else if upper.contains("TRACE") {
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
        let number = self.logs.len() + 1;
        let level = LogLevel::from_line(&content);
        self.logs.push(LogLine {
            number,
            content,
            level,
        });

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
            let query_lower = query.to_lowercase();
            let indices: Vec<usize> = self
                .logs
                .iter()
                .enumerate()
                .filter(|(_, log)| log.content.to_lowercase().contains(&query_lower))
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
        .title(" Logs Viewer ")
        .borders(Borders::BOTTOM)
        .style(theme.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![];

    if let Some(ref pod_ref) = state.pod_ref {
        let mut title_spans = vec![
            Span::styled(&pod_ref.name, theme.title_style()),
            Span::raw(" in "),
            Span::styled(&pod_ref.namespace, theme.get_style("accent")),
        ];

        if state.follow_mode {
            title_spans.push(Span::raw(" "));
            title_spans.push(Span::styled("[FOLLOW]", theme.get_style("success")));
        }

        lines.push(Line::from(title_spans));
    }

    let total = state.visible_line_count();
    let status = format!(
        "Lines: {} | Position: {}/{} | Mode: {}",
        total,
        state.scroll_pos + 1,
        total.max(1),
        if state.loading { "Loading..." } else { "Ready" }
    );
    lines.push(Line::from(Span::styled(status, theme.inactive_style())));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_log_area(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(theme.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.logs.is_empty() {
        let msg = Paragraph::new("No logs available")
            .style(theme.inactive_style());
        frame.render_widget(msg, inner);
        return;
    }

    let visible_lines = (inner.height as usize).saturating_sub(1);
    let logs = state.get_visible_logs(state.scroll_pos, visible_lines);

    let mut lines = vec![];
    let line_num_width = state.visible_line_count().to_string().len();

    for log in logs {
        let line_num = format!("{:>width$}", log.number, width = line_num_width);
        let level_style = theme.get_log_level_style(log.level.label());

        let spans = vec![
            Span::styled(format!("{}│ ", line_num), theme.get_style("muted")),
            Span::styled(format!("[{}]", log.level.label()), level_style),
            Span::raw(" "),
            Span::styled(&log.content, level_style),
        ];

        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_search_box(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let block = Block::default()
        .title(" Search ")
        .borders(Borders::ALL)
        .style(theme.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let search_text = if state.search_query.is_empty() {
        "Search: █".to_string()
    } else {
        format!("Search: {} █", state.search_query)
    };

    let paragraph = Paragraph::new(search_text)
        .style(Style::default().fg(theme.accent));
    frame.render_widget(paragraph, inner);
}

fn render_footer(
    frame: &mut Frame,
    theme: &Theme,
    state: &LogsViewState,
    area: Rect,
) {
    let help_text = vec![
        Span::styled("j", theme.get_style("accent")),
        Span::raw("/k: scroll "),
        Span::styled("g", theme.get_style("accent")),
        Span::raw("/G: top/bottom "),
        Span::styled("f", theme.get_style("accent")),
        Span::raw(": follow "),
        Span::styled("/", theme.get_style("accent")),
        Span::raw(": search "),
        Span::styled("c", theme.get_style("accent")),
        Span::raw(": clear "),
        Span::styled("q", theme.get_style("accent")),
        Span::raw(": close"),
    ];

    if let Some(ref error) = state.error {
        let error_line = Line::from(vec![
            Span::styled("ERROR: ", theme.get_style("error")),
            Span::raw(error),
        ]);

        let paragraph = Paragraph::new(error_line);
        frame.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new(Line::from(help_text));
        frame.render_widget(paragraph, area);
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
