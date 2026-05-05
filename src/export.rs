//! Text export to local files.

use std::path::PathBuf;

use crate::time::format_local;

/// Writes `content` to a text file and returns the path.
///
/// Default location: `$TMPDIR/kubectui-{kind}-{label}-{timestamp}.log`
pub fn save_text_to_file(kind: &str, label: &str, content: &str) -> std::io::Result<PathBuf> {
    let timestamp = format_local(crate::time::now(), "%Y%m%d-%H%M%S");
    let safe_kind = sanitize_filename_segment(kind);
    let safe_label = sanitize_filename_segment(label);
    let filename = format!("kubectui-{safe_kind}-{safe_label}-{timestamp}.log");
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Writes `content` to a log file and returns the path.
pub fn save_logs_to_file(label: &str, content: &str) -> std::io::Result<PathBuf> {
    save_text_to_file("logs", label, content)
}

fn sanitize_filename_segment(value: &str) -> String {
    let safe_label: String = value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
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
}
