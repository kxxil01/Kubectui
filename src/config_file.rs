//! Bounded local config file reads.

use std::{
    fs::File,
    io::{Read, Take},
    path::Path,
};

pub const APP_CONFIG_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const EDITED_YAML_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const EXTENSIONS_CONFIG_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const HELM_REPOSITORY_CONFIG_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const RUNBOOKS_CONFIG_MAX_BYTES: u64 = 4 * 1024 * 1024;

pub fn read_bounded_config_file(
    path: &Path,
    label: &str,
    max_bytes: u64,
) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("failed to read {label} '{}': {err}", path.display()))?;
    let mut reader: Take<&mut File> = file.by_ref().take(max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read {label} '{}': {err}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "{label} '{}' is larger than {max_bytes} bytes",
            path.display()
        ));
    }
    String::from_utf8(bytes)
        .map_err(|err| format!("failed to read {label} '{}': {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "kubectui-config-file-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn read_bounded_config_file_rejects_oversized_file() {
        let path = temp_path("oversized");
        std::fs::write(&path, b"abcdef").expect("write config");

        let err = read_bounded_config_file(&path, "test config", 5)
            .expect_err("oversized file should fail");

        assert!(err.contains("larger than 5 bytes"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_bounded_config_file_preserves_valid_utf8() {
        let path = temp_path("valid");
        std::fs::write(&path, "ready").expect("write config");

        let content =
            read_bounded_config_file(&path, "test config", 5).expect("config should read");

        assert_eq!(content, "ready");
        let _ = std::fs::remove_file(path);
    }
}
