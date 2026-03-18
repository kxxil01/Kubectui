//! Log export to local files.

use std::path::PathBuf;

use crate::time::format_local;

/// Writes `content` to a log file and returns the path.
///
/// Default location: `$TMPDIR/kubectui-logs-{label}-{timestamp}.log`
pub fn save_logs_to_file(label: &str, content: &str) -> std::io::Result<PathBuf> {
    let timestamp = format_local(crate::time::now(), "%Y%m%d-%H%M%S");
    let safe_label: String = label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("kubectui-logs-{safe_label}-{timestamp}.log");
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, content)?;
    Ok(path)
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
}
