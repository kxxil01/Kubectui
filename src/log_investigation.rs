//! Shared log analysis and query helpers for pod/workload log tabs.

use crate::time::{AppTimestamp, format_rfc3339, parse_timestamp};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_LOG_ENTRY_BYTES: usize = 10_000;

fn default_structured_view() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogTimeWindow {
    #[default]
    All,
    Last5Minutes,
    Last15Minutes,
    Last1Hour,
    Last6Hours,
}

impl LogTimeWindow {
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Last5Minutes,
            Self::Last5Minutes => Self::Last15Minutes,
            Self::Last15Minutes => Self::Last1Hour,
            Self::Last1Hour => Self::Last6Hours,
            Self::Last6Hours => Self::All,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Last5Minutes => "5m",
            Self::Last15Minutes => "15m",
            Self::Last1Hour => "1h",
            Self::Last6Hours => "6h",
        }
    }

    pub const fn max_age_seconds(self) -> Option<i64> {
        match self {
            Self::All => None,
            Self::Last5Minutes => Some(5 * 60),
            Self::Last15Minutes => Some(15 * 60),
            Self::Last1Hour => Some(60 * 60),
            Self::Last6Hours => Some(6 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogQueryMode {
    #[default]
    Substring,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodLogPreset {
    pub name: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub mode: LogQueryMode,
    #[serde(default)]
    pub time_window: LogTimeWindow,
    #[serde(default = "default_structured_view")]
    pub structured_view: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadLogPreset {
    pub name: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub mode: LogQueryMode,
    #[serde(default)]
    pub time_window: LogTimeWindow,
    #[serde(default = "default_structured_view")]
    pub structured_view: bool,
    #[serde(default)]
    pub label_filter: Option<String>,
    #[serde(default)]
    pub pod_filter: Option<String>,
    #[serde(default)]
    pub container_filter: Option<String>,
}

impl PodLogPreset {
    pub fn summary_label(&self) -> String {
        summarize_preset_label(&self.name, &self.query, self.mode)
    }
}

impl WorkloadLogPreset {
    pub fn summary_label(&self) -> String {
        summarize_preset_label(&self.name, &self.query, self.mode)
    }
}

impl LogQueryMode {
    pub const fn toggle(self) -> Self {
        match self {
            Self::Substring => Self::Regex,
            Self::Regex => Self::Substring,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Substring => "text",
            Self::Regex => "regex",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSeverity {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogSeverity {
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::Error => "ERR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DBG",
            Self::Trace => "TRC",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    raw: String,
    rendered: String,
    timestamp: Option<AppTimestamp>,
    severity: Option<LogSeverity>,
    request_id: Option<String>,
}

impl LogEntry {
    pub fn from_raw(raw: impl Into<String>) -> Self {
        let raw = cap_log_entry(raw.into());
        let trimmed = raw.trim();
        let parsed_json = if trimmed.starts_with('{') {
            serde_json::from_str::<Value>(trimmed).ok()
        } else {
            None
        };

        if let Some(Value::Object(map)) = parsed_json {
            let timestamp = value_text(
                [
                    map.get("ts"),
                    map.get("time"),
                    map.get("timestamp"),
                    map.get("@timestamp"),
                ]
                .into_iter(),
            );
            let level_text = value_text(
                [
                    map.get("level"),
                    map.get("lvl"),
                    map.get("severity"),
                    map.get("log.level"),
                ]
                .into_iter(),
            );
            let message = value_text(
                [
                    map.get("message"),
                    map.get("msg"),
                    map.get("error"),
                    map.get("err"),
                ]
                .into_iter(),
            );
            let request_id = value_text(
                [
                    map.get("request_id"),
                    map.get("requestId"),
                    map.get("req_id"),
                    map.get("trace_id"),
                    map.get("traceId"),
                    map.get("correlation_id"),
                    map.get("correlationId"),
                    map.get("span_id"),
                    map.get("spanId"),
                ]
                .into_iter(),
            );

            let severity = level_text
                .as_deref()
                .and_then(severity_from_text)
                .or_else(|| severity_from_text(&raw));
            let parsed_timestamp = timestamp.as_deref().and_then(parse_timestamp);

            let extras = map
                .iter()
                .filter_map(|(key, value)| {
                    (!matches!(
                        key.as_str(),
                        "ts" | "time"
                            | "timestamp"
                            | "@timestamp"
                            | "level"
                            | "lvl"
                            | "severity"
                            | "log.level"
                            | "message"
                            | "msg"
                            | "error"
                            | "err"
                            | "request_id"
                            | "requestId"
                            | "req_id"
                            | "trace_id"
                            | "traceId"
                            | "correlation_id"
                            | "correlationId"
                            | "span_id"
                            | "spanId"
                    ))
                    .then_some(format!("{key}={}", scalar_value_text(value)?))
                })
                .take(3)
                .collect::<Vec<_>>();

            let mut parts = Vec::with_capacity(5);
            if let Some(timestamp) = timestamp.filter(|value| !value.is_empty()) {
                parts.push(timestamp);
            }
            if let Some(level_text) = level_text
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                parts.push(level_text.to_ascii_uppercase());
            }
            if let Some(request_id) = request_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                parts.push(format!("req={request_id}"));
            }
            if let Some(message) = message.filter(|value| !value.is_empty()) {
                parts.push(message);
            }
            parts.extend(extras);
            let rendered = if parts.is_empty() {
                raw.clone()
            } else {
                parts.join(" ")
            };
            return Self {
                raw,
                rendered,
                timestamp: parsed_timestamp,
                severity,
                request_id,
            };
        }

        Self {
            timestamp: extract_timestamp(&raw),
            severity: severity_from_text(&raw),
            request_id: extract_request_id(&raw),
            rendered: raw.clone(),
            raw,
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn display_text(&self, structured: bool) -> &str {
        if structured {
            &self.rendered
        } else {
            &self.raw
        }
    }

    pub fn timestamp(&self) -> Option<AppTimestamp> {
        self.timestamp
    }

    pub fn severity(&self) -> Option<LogSeverity> {
        self.severity
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

fn cap_log_entry(mut raw: String) -> String {
    if raw.len() <= MAX_LOG_ENTRY_BYTES {
        return raw;
    }

    let end = raw.floor_char_boundary(MAX_LOG_ENTRY_BYTES);
    raw.truncate(end);
    raw.push_str("…[truncated]");
    raw
}

pub fn compile_query(query: &str, mode: LogQueryMode) -> Result<Option<Regex>, String> {
    if query.trim().is_empty() || mode == LogQueryMode::Substring {
        return Ok(None);
    }

    RegexBuilder::new(query)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map(Some)
        .map_err(|err| format!("invalid regex: {err}"))
}

pub fn entry_matches_query(
    entry: &LogEntry,
    query: &str,
    mode: LogQueryMode,
    compiled: Option<&Regex>,
    structured: bool,
) -> bool {
    if query.is_empty() {
        return true;
    }

    match mode {
        LogQueryMode::Substring => {
            contains_ci_ascii(entry.raw(), query)
                || (structured
                    && entry.display_text(true) != entry.raw()
                    && contains_ci_ascii(entry.display_text(true), query))
        }
        LogQueryMode::Regex => compiled.is_some_and(|regex| {
            regex.is_match(entry.raw())
                || (structured
                    && entry.display_text(true) != entry.raw()
                    && regex.is_match(entry.display_text(true)))
        }),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LogFilterSpec<'a> {
    pub query: &'a str,
    pub mode: LogQueryMode,
    pub compiled: Option<&'a Regex>,
    pub structured: bool,
    pub time_window: LogTimeWindow,
    pub correlation_request_id: Option<&'a str>,
}

pub fn entry_matches_filters(
    entry: &LogEntry,
    filter: LogFilterSpec<'_>,
    now: AppTimestamp,
) -> bool {
    entry_matches_time_window(entry, filter.time_window, now)
        && entry_matches_correlation(entry, filter.correlation_request_id)
        && entry_matches_query(
            entry,
            filter.query,
            filter.mode,
            filter.compiled,
            filter.structured,
        )
}

pub fn entry_matches_time_window(
    entry: &LogEntry,
    time_window: LogTimeWindow,
    now: AppTimestamp,
) -> bool {
    let Some(limit) = time_window.max_age_seconds() else {
        return true;
    };
    entry
        .timestamp()
        .map(|timestamp| now.as_second().saturating_sub(timestamp.as_second()).max(0) <= limit)
        .unwrap_or(false)
}

pub fn entry_matches_correlation(entry: &LogEntry, correlation_request_id: Option<&str>) -> bool {
    correlation_request_id.is_none_or(|request_id| entry.request_id() == Some(request_id))
}

pub fn parse_jump_target(input: &str) -> Result<AppTimestamp, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter an RFC3339 timestamp like 2026-03-26T10:00:00Z.".to_string());
    }
    parse_timestamp(trimmed)
        .ok_or_else(|| "Invalid timestamp. Use RFC3339 like 2026-03-26T10:00:00Z.".to_string())
}

pub fn nearest_timestamp_index<'a, I>(entries: I, target: AppTimestamp) -> Option<usize>
where
    I: Iterator<Item = (usize, &'a LogEntry)>,
{
    entries
        .filter_map(|(index, entry)| {
            entry.timestamp().map(|timestamp| {
                (
                    index,
                    timestamp
                        .as_second()
                        .saturating_sub(target.as_second())
                        .unsigned_abs(),
                )
            })
        })
        .min_by_key(|(index, delta)| (*delta, *index))
        .map(|(index, _)| index)
}

pub fn format_jump_target(timestamp: AppTimestamp) -> String {
    format_rfc3339(timestamp)
}

pub fn highlight_ranges(
    text: &str,
    query: &str,
    mode: LogQueryMode,
    compiled: Option<&Regex>,
) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    match mode {
        LogQueryMode::Substring => substring_match_ranges(text, query),
        LogQueryMode::Regex => compiled
            .map(|regex| {
                regex
                    .find_iter(text)
                    .take(32)
                    .map(|m| (m.start(), m.end()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn value_text<'a>(mut values: impl Iterator<Item = Option<&'a Value>>) -> Option<String> {
    values.find_map(|value| scalar_value_text(value?))
}

fn scalar_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn severity_from_text(text: &str) -> Option<LogSeverity> {
    if contains_token_ci(text, "error") || contains_token_ci(text, "fatal") {
        Some(LogSeverity::Error)
    } else if contains_token_ci(text, "warn") || contains_token_ci(text, "warning") {
        Some(LogSeverity::Warn)
    } else if contains_token_ci(text, "info") {
        Some(LogSeverity::Info)
    } else if contains_token_ci(text, "debug") {
        Some(LogSeverity::Debug)
    } else if contains_token_ci(text, "trace") {
        Some(LogSeverity::Trace)
    } else {
        None
    }
}

fn extract_request_id(text: &str) -> Option<String> {
    const KEYS: &[&str] = &[
        "request_id",
        "request-id",
        "requestid",
        "req_id",
        "trace_id",
        "trace-id",
        "traceid",
        "correlation_id",
        "correlation-id",
        "span_id",
        "span-id",
    ];

    for key in KEYS {
        let mut search_start = 0usize;
        while let Some(relative) = find_ascii_ci(&text[search_start..], key) {
            let start = search_start + relative;
            let end = start + key.len();
            if !is_left_token_boundary(text.as_bytes(), start)
                || !is_token_boundary(text.as_bytes(), end)
            {
                search_start = end;
                continue;
            }
            let remainder = &text[end..];
            let trimmed = remainder.trim_start_matches([' ', '\t']);
            let Some(value) = trimmed
                .strip_prefix('=')
                .or_else(|| trimmed.strip_prefix(':'))
                .map(str::trim_start)
            else {
                search_start = end;
                continue;
            };
            let token = value
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .next()
                .unwrap_or("")
                .trim_matches(|c| matches!(c, '"' | '\'' | '[' | ']' | '{' | '}'));
            if !token.is_empty() {
                return Some(token.to_string());
            }
            search_start = end;
        }
    }
    None
}

fn extract_timestamp(text: &str) -> Option<AppTimestamp> {
    text.split_whitespace().next().and_then(parse_timestamp)
}

fn substring_match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    ascii_ci_match_indices(text, query)
        .take(32)
        .map(|(start, _)| (start, start + query.len()))
        .collect()
}

fn contains_ci_ascii(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn contains_token_ci(text: &str, token: &str) -> bool {
    ascii_ci_match_indices(text, token).any(|(start, _)| {
        is_left_token_boundary(text.as_bytes(), start)
            && is_token_boundary(text.as_bytes(), start + token.len())
    })
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    ascii_ci_match_indices(haystack, needle)
        .next()
        .map(|(idx, _)| idx)
}

fn ascii_ci_match_indices<'a>(
    haystack: &'a str,
    needle: &'a str,
) -> impl Iterator<Item = (usize, &'a str)> + 'a {
    haystack.char_indices().filter_map(move |(idx, _)| {
        if needle.is_empty() {
            return None;
        }
        let end = idx.checked_add(needle.len())?;
        haystack
            .get(idx..end)
            .filter(|window| window.eq_ignore_ascii_case(needle))
            .map(|window| (idx, window))
    })
}

fn is_left_token_boundary(bytes: &[u8], start: usize) -> bool {
    start == 0 || is_token_boundary(bytes, start - 1)
}

fn is_token_boundary(bytes: &[u8], index: usize) -> bool {
    bytes
        .get(index)
        .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_' && *byte != b'-')
}

fn summarize_preset_label(name: &str, query: &str, mode: LogQueryMode) -> String {
    if !name.trim().is_empty() {
        name.to_string()
    } else if query.trim().is_empty() {
        "all lines".to_string()
    } else {
        let compact = query.trim().chars().take(24).collect::<String>();
        let suffix = if query.trim().chars().count() > 24 {
            format!("{compact}…")
        } else {
            compact
        };
        match mode {
            LogQueryMode::Substring => format!("text: {suffix}"),
            LogQueryMode::Regex => format!("regex: {suffix}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_entry_caps_long_raw_lines_once_for_all_log_views() {
        let entry = LogEntry::from_raw("x".repeat(MAX_LOG_ENTRY_BYTES + 256));

        assert_eq!(
            entry.raw().len(),
            MAX_LOG_ENTRY_BYTES + "…[truncated]".len()
        );
        assert!(entry.raw().ends_with("…[truncated]"));
        assert_eq!(entry.display_text(false), entry.raw());
    }

    #[test]
    fn log_entry_cap_preserves_utf8_boundary() {
        let entry = LogEntry::from_raw("🚀".repeat((MAX_LOG_ENTRY_BYTES / 4) + 8));

        assert!(entry.raw().is_char_boundary(MAX_LOG_ENTRY_BYTES));
        assert!(entry.raw().ends_with("…[truncated]"));
    }

    #[test]
    fn structured_json_line_extracts_summary_fields() {
        let entry = LogEntry::from_raw(
            r#"{"time":"2026-03-26T10:00:00Z","level":"warn","message":"retrying","request_id":"abc-123","attempt":2}"#,
        );

        assert_eq!(entry.severity(), Some(LogSeverity::Warn));
        assert_eq!(entry.request_id(), Some("abc-123"));
        assert_eq!(
            entry.display_text(true),
            "2026-03-26T10:00:00Z WARN req=abc-123 retrying attempt=2"
        );
    }

    #[test]
    fn plain_line_extracts_request_id_and_severity() {
        let entry = LogEntry::from_raw("ERROR trace_id=xyz-9 request failed");

        assert_eq!(entry.severity(), Some(LogSeverity::Error));
        assert_eq!(entry.request_id(), Some("xyz-9"));
        assert_eq!(
            entry.display_text(true),
            "ERROR trace_id=xyz-9 request failed"
        );
    }

    #[test]
    fn plain_line_extracts_mixed_case_request_id_without_key_allocation() {
        let entry = LogEntry::from_raw("warn Trace_ID: abc-9 request delayed");

        assert_eq!(entry.severity(), Some(LogSeverity::Warn));
        assert_eq!(entry.request_id(), Some("abc-9"));
    }

    #[test]
    fn substring_highlight_ranges_match_ascii_case_insensitively() {
        let ranges = highlight_ranges(
            "INFO request failed; info retry",
            "info",
            LogQueryMode::Substring,
            None,
        );

        assert_eq!(ranges, vec![(0, 4), (21, 25)]);
    }

    #[test]
    fn regex_query_is_compiled_case_insensitively() {
        let compiled = compile_query("warn|error", LogQueryMode::Regex)
            .expect("regex should compile")
            .expect("regex present");

        assert!(compiled.is_match("WARN"));
    }

    #[test]
    fn entry_matches_query_checks_structured_summary_when_enabled() {
        let entry = LogEntry::from_raw(r#"{"message":"startup complete","level":"info"}"#);

        assert!(entry_matches_query(
            &entry,
            "INFO startup",
            LogQueryMode::Substring,
            None,
            true,
        ));
        assert!(!entry_matches_query(
            &entry,
            "INFO startup",
            LogQueryMode::Substring,
            None,
            false,
        ));
    }

    #[test]
    fn highlight_ranges_support_regex() {
        let compiled = compile_query("req=\\w+", LogQueryMode::Regex)
            .expect("regex should compile")
            .expect("regex present");
        let ranges = highlight_ranges(
            "INFO req=abc startup",
            "req=\\w+",
            LogQueryMode::Regex,
            Some(&compiled),
        );
        assert_eq!(ranges, vec![(5, 12)]);
    }

    #[test]
    fn preset_summary_uses_generated_label_when_name_empty() {
        let preset = PodLogPreset {
            name: String::new(),
            query: "request_id=abc".into(),
            mode: LogQueryMode::Regex,
            time_window: LogTimeWindow::All,
            structured_view: true,
        };

        assert_eq!(preset.summary_label(), "regex: request_id=abc");
    }

    #[test]
    fn preset_query_mode_round_trips_through_json() {
        let encoded = serde_json::to_string(&WorkloadLogPreset {
            name: "errors".into(),
            query: "error|fatal".into(),
            mode: LogQueryMode::Regex,
            time_window: LogTimeWindow::Last15Minutes,
            structured_view: false,
            label_filter: Some("app=api".into()),
            pod_filter: Some("api-0".into()),
            container_filter: Some("main".into()),
        })
        .expect("serialize preset");
        let decoded: WorkloadLogPreset =
            serde_json::from_str(&encoded).expect("deserialize preset");

        assert_eq!(decoded.mode, LogQueryMode::Regex);
        assert_eq!(decoded.time_window, LogTimeWindow::Last15Minutes);
        assert_eq!(decoded.label_filter.as_deref(), Some("app=api"));
        assert_eq!(decoded.pod_filter.as_deref(), Some("api-0"));
        assert!(!decoded.structured_view);
    }

    #[test]
    fn structured_json_entry_preserves_parsed_timestamp() {
        let entry = LogEntry::from_raw(
            r#"{"timestamp":"2026-03-26T10:00:00Z","level":"info","message":"ok"}"#,
        );

        assert_eq!(
            entry.timestamp().map(|timestamp| timestamp.as_second()),
            parse_timestamp("2026-03-26T10:00:00Z").map(|timestamp| timestamp.as_second())
        );
    }

    #[test]
    fn plain_timestamp_prefixed_line_extracts_timestamp() {
        let entry = LogEntry::from_raw("2026-03-26T10:00:00Z request started");
        assert!(entry.timestamp().is_some());
    }

    #[test]
    fn time_window_rejects_lines_without_timestamp() {
        let entry = LogEntry::from_raw("plain line without timestamp");
        let now = parse_timestamp("2026-03-26T10:05:00Z").expect("now");

        assert!(!entry_matches_time_window(
            &entry,
            LogTimeWindow::Last5Minutes,
            now,
        ));
    }

    #[test]
    fn parse_jump_target_accepts_rfc3339() {
        let target = parse_jump_target("2026-03-26T10:05:00Z").expect("timestamp");
        assert_eq!(format_jump_target(target), "2026-03-26T10:05:00Z");
    }

    #[test]
    fn nearest_timestamp_index_returns_closest_entry() {
        let entries = [
            LogEntry::from_raw("2026-03-26T10:00:00Z first"),
            LogEntry::from_raw("2026-03-26T10:05:30Z second"),
            LogEntry::from_raw("2026-03-26T10:10:00Z third"),
        ];
        let target = parse_timestamp("2026-03-26T10:06:00Z").expect("target");

        let index = nearest_timestamp_index(entries.iter().enumerate(), target);
        assert_eq!(index, Some(1));
    }
}
