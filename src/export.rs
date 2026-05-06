//! Text export to local files.

use std::{fs::OpenOptions, io::Write, path::PathBuf};

use crate::time::format_local;

const MAX_FILENAME_SEGMENT_BYTES: usize = 96;

/// Writes `content` to a text file and returns the path.
///
/// Default location: `$TMPDIR/kubectui-{kind}-{label}-{timestamp}.log`
pub fn save_text_to_file(kind: &str, label: &str, content: &str) -> std::io::Result<PathBuf> {
    let timestamp = format_local(crate::time::now(), "%Y%m%d-%H%M%S");
    save_text_to_file_with_timestamp(kind, label, content, &timestamp)
}

fn save_text_to_file_with_timestamp(
    kind: &str,
    label: &str,
    content: &str,
    timestamp: &str,
) -> std::io::Result<PathBuf> {
    let safe_kind = sanitize_filename_segment(kind);
    let safe_label = sanitize_filename_segment(label);
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let filename = format!("kubectui-{safe_kind}-{safe_label}-{timestamp}{suffix}.log");
        let path = std::env::temp_dir().join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate unique export filename",
    ))
}

/// Writes `content` to a log file and returns the path.
pub fn save_logs_to_file(label: &str, content: &str) -> std::io::Result<PathBuf> {
    save_text_to_file("logs", label, content)
}

fn sanitize_filename_segment(value: &str) -> String {
    let mut safe_label = String::new();
    for c in value.chars() {
        let safe = if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            c
        } else {
            '_'
        };
        if safe_label.len() + safe.len_utf8() > MAX_FILENAME_SEGMENT_BYTES {
            break;
        }
        safe_label.push(safe);
    }

    if safe_label.is_empty() {
        "output".to_string()
    } else {
        safe_label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_logs_creates_file() {
        let path = save_logs_to_file("test-pod", "line 1\nline 2\n").unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line 1"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn label_sanitization() {
        let path = save_logs_to_file("ns/pod:container", "data").unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_text_uses_kind_prefix() {
        let path = save_text_to_file("exec", "ns/pod:container", "data").unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();

        assert!(filename.starts_with("kubectui-exec-ns_pod_container-"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn repeated_exports_do_not_overwrite_same_second_file() {
        let first =
            save_text_to_file_with_timestamp("logs", "same-pod", "first", "20260506-120000")
                .unwrap();
        let second =
            save_text_to_file_with_timestamp("logs", "same-pod", "second", "20260506-120000")
                .unwrap();

        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
        assert!(
            second
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("-1.log")
        );

        std::fs::remove_file(first).ok();
        std::fs::remove_file(second).ok();
    }

    #[test]
    fn long_labels_are_capped_to_filesystem_safe_names() {
        let long_kind = "k".repeat(200);
        let long_label = "ns/".to_string() + &"pod".repeat(120) + ":container";
        let path =
            save_text_to_file_with_timestamp(&long_kind, &long_label, "data", "20260506-120000")
                .unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();

        assert!(filename.len() <= 255, "{filename}");
        assert!(filename.starts_with("kubectui-"));
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));

        std::fs::remove_file(path).ok();
    }
}
