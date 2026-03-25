//! Helm client-side helpers (repositories, local config, and CLI-backed history/rollback flows).

use std::{path::PathBuf, process::Command, sync::OnceLock};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::{
    k8s::dtos::{HelmReleaseRevisionInfo, HelmRepoInfo},
    resource_diff::YamlDocumentDiffResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmCliInfo {
    pub version: String,
    pub major: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmHistoryResult {
    pub cli_version: String,
    pub revisions: Vec<HelmReleaseRevisionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmValuesDiffResult {
    pub current_revision: i32,
    pub target_revision: i32,
    pub diff: YamlDocumentDiffResult,
}

static HELM_CLI_INFO: OnceLock<Result<HelmCliInfo, String>> = OnceLock::new();

/// Reads configured Helm repositories from the local filesystem.
///
/// Helm 3 stores repository config in `$HELM_REPOSITORY_CONFIG` or
/// `~/.config/helm/repositories.yaml` (XDG) / `~/Library/Preferences/helm/repositories.yaml` (macOS).
pub fn read_helm_repositories() -> Vec<HelmRepoInfo> {
    let candidates = helm_repo_paths();
    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Some(repos) = parse_helm_repositories(&content)
        {
            return repos;
        }
    }
    Vec::new()
}

pub fn helm_cli_info() -> Result<HelmCliInfo, String> {
    HELM_CLI_INFO.get_or_init(detect_helm_cli).clone()
}

pub async fn fetch_release_history(
    release_name: &str,
    namespace: &str,
    kube_context: Option<String>,
) -> Result<HelmHistoryResult> {
    let release_name = release_name.to_string();
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        fetch_release_history_blocking(&release_name, &namespace, kube_context)
    })
    .await
    .map_err(|err| anyhow!("Helm history task failed: {err}"))?
}

pub async fn fetch_release_values_diff(
    release_name: &str,
    namespace: &str,
    kube_context: Option<String>,
    current_revision: i32,
    target_revision: i32,
) -> Result<HelmValuesDiffResult> {
    let release_name = release_name.to_string();
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        let current_values = fetch_release_values_blocking(
            &release_name,
            &namespace,
            kube_context.clone(),
            current_revision,
        )?;
        let target_values = fetch_release_values_blocking(
            &release_name,
            &namespace,
            kube_context,
            target_revision,
        )?;
        let diff = crate::resource_diff::build_yaml_document_diff(
            &current_values,
            &target_values,
            &format!("current-rev-{current_revision}"),
            &format!("target-rev-{target_revision}"),
        )?;
        Ok(HelmValuesDiffResult {
            current_revision,
            target_revision,
            diff,
        })
    })
    .await
    .map_err(|err| anyhow!("Helm values diff task failed: {err}"))?
}

pub async fn rollback_release(
    release_name: &str,
    namespace: &str,
    kube_context: Option<String>,
    revision: i32,
) -> Result<String> {
    let release_name = release_name.to_string();
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        let mut args = base_command_args(kube_context.as_deref(), &namespace);
        args.extend([
            "rollback".to_string(),
            release_name.clone(),
            revision.to_string(),
            "--wait".to_string(),
            "--wait-for-jobs".to_string(),
            "--cleanup-on-fail".to_string(),
        ]);
        let stdout = run_helm_command(&args)?;
        Ok(if stdout.trim().is_empty() {
            format!("Rolled back release '{release_name}' to revision {revision}.")
        } else {
            stdout.trim().to_string()
        })
    })
    .await
    .map_err(|err| anyhow!("Helm rollback task failed: {err}"))?
}

fn helm_repo_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // $HELM_REPOSITORY_CONFIG takes precedence
    if let Ok(p) = std::env::var("HELM_REPOSITORY_CONFIG") {
        paths.push(std::path::PathBuf::from(p));
    }

    if let Some(home) = dirs::home_dir() {
        // macOS: ~/Library/Preferences/helm/repositories.yaml
        #[cfg(target_os = "macos")]
        paths.push(home.join("Library/Preferences/helm/repositories.yaml"));

        // XDG: ~/.config/helm/repositories.yaml
        paths.push(home.join(".config/helm/repositories.yaml"));
    }

    paths
}

fn detect_helm_cli() -> Result<HelmCliInfo, String> {
    let output = Command::new("helm")
        .args(["version", "--template", "{{ .Version }}"])
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                "Helm CLI is not available on PATH.".to_string()
            } else {
                format!("Failed to execute 'helm version': {err}")
            }
        })?;
    if !output.status.success() {
        return Err(format!(
            "Helm CLI is unavailable: {}",
            stderr_or_stdout(&output.stdout, &output.stderr)
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let major = parse_major_version(&version)
        .ok_or_else(|| format!("Could not parse Helm version '{version}'."))?;
    if major < 3 {
        return Err(format!(
            "Helm {version} is unsupported. Kubectui requires Helm 3 or newer."
        ));
    }

    Ok(HelmCliInfo { version, major })
}

fn parse_helm_repositories(yaml_content: &str) -> Option<Vec<HelmRepoInfo>> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_content).ok()?;
    let repos = doc.get("repositories")?.as_sequence()?;

    let mut result = Vec::new();
    for entry in repos {
        let name = entry.get("name")?.as_str()?.to_string();
        let url = entry.get("url")?.as_str()?.to_string();
        result.push(HelmRepoInfo { name, url });
    }
    result.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Some(result)
}

fn fetch_release_history_blocking(
    release_name: &str,
    namespace: &str,
    kube_context: Option<String>,
) -> Result<HelmHistoryResult> {
    let cli = helm_cli_info().map_err(anyhow::Error::msg)?;
    let mut args = base_command_args(kube_context.as_deref(), namespace);
    args.extend([
        "history".to_string(),
        release_name.to_string(),
        "--max".to_string(),
        "256".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]);
    let stdout = run_helm_command(&args)?;
    let mut revisions = parse_history_json(&stdout)?;
    revisions.sort_unstable_by(|left, right| {
        right
            .revision
            .cmp(&left.revision)
            .then_with(|| left.updated.cmp(&right.updated))
    });
    if revisions.is_empty() {
        return Err(anyhow!(
            "Helm history returned no revisions for release '{release_name}'."
        ));
    }

    Ok(HelmHistoryResult {
        cli_version: cli.version,
        revisions,
    })
}

fn fetch_release_values_blocking(
    release_name: &str,
    namespace: &str,
    kube_context: Option<String>,
    revision: i32,
) -> Result<String> {
    helm_cli_info().map_err(anyhow::Error::msg)?;
    let mut args = base_command_args(kube_context.as_deref(), namespace);
    args.extend([
        "get".to_string(),
        "values".to_string(),
        release_name.to_string(),
        "--all".to_string(),
        "--revision".to_string(),
        revision.to_string(),
        "--output".to_string(),
        "yaml".to_string(),
    ]);
    run_helm_command(&args)
}

fn run_helm_command(args: &[String]) -> Result<String> {
    let output = Command::new("helm")
        .args(args)
        .output()
        .with_context(|| format!("failed to execute helm {}", args.join(" ")))?;
    if !output.status.success() {
        return Err(anyhow!(stderr_or_stdout(&output.stdout, &output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn base_command_args(kube_context: Option<&str>, namespace: &str) -> Vec<String> {
    let mut args = Vec::with_capacity(4);
    if let Some(context) = kube_context
        && !context.is_empty()
    {
        args.push("--kube-context".to_string());
        args.push(context.to_string());
    }
    args.push("--namespace".to_string());
    args.push(namespace.to_string());
    args
}

fn stderr_or_stdout(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    "helm command failed without output".to_string()
}

fn parse_major_version(version: &str) -> Option<u64> {
    version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse::<u64>().ok())
}

#[derive(Debug, Deserialize)]
struct HistoryJsonEntry {
    #[serde(deserialize_with = "deserialize_i32")]
    revision: i32,
    updated: String,
    status: String,
    chart: String,
    #[serde(default, alias = "appVersion")]
    app_version: String,
    #[serde(default)]
    description: String,
}

fn deserialize_i32<'de, D>(deserializer: D) -> std::result::Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| D::Error::custom("invalid revision number")),
        serde_json::Value::String(text) => text
            .parse::<i32>()
            .map_err(|_| D::Error::custom("invalid revision string")),
        other => Err(D::Error::custom(format!(
            "unexpected revision type: {other}"
        ))),
    }
}

fn parse_history_json(content: &str) -> Result<Vec<HelmReleaseRevisionInfo>> {
    let entries: Vec<HistoryJsonEntry> =
        serde_json::from_str(content).context("failed to parse helm history JSON")?;
    Ok(entries
        .into_iter()
        .map(|entry| HelmReleaseRevisionInfo {
            revision: entry.revision,
            updated: entry.updated,
            status: entry.status,
            chart: entry.chart,
            app_version: entry.app_version,
            description: entry.description,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_repositories_yaml() {
        let yaml = r#"
apiVersion: ""
generated: "2024-01-01T00:00:00Z"
repositories:
  - name: bitnami
    url: https://charts.bitnami.com/bitnami
  - name: stable
    url: https://charts.helm.sh/stable
"#;
        let repos = parse_helm_repositories(yaml).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "bitnami");
        assert_eq!(repos[1].name, "stable");
    }

    #[test]
    fn parse_empty_repositories() {
        let yaml = "repositories: []\n";
        let repos = parse_helm_repositories(yaml).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn parse_invalid_yaml_returns_none() {
        let repos = parse_helm_repositories("not yaml at all {{{");
        assert!(repos.is_none());
    }

    #[test]
    fn parse_major_version_extracts_supported_major() {
        assert_eq!(parse_major_version("v4.1.3"), Some(4));
        assert_eq!(parse_major_version("3.14.0"), Some(3));
        assert_eq!(parse_major_version("garbage"), None);
    }

    #[test]
    fn parse_history_json_accepts_revision_numbers_and_strings() {
        let json = r#"
[
  {
    "revision": 5,
    "updated": "2026-03-25 20:13:00 +0700",
    "status": "deployed",
    "chart": "web-1.2.3",
    "app_version": "2.0.0",
    "description": "Upgrade complete"
  },
  {
    "revision": "4",
    "updated": "2026-03-25 19:10:00 +0700",
    "status": "superseded",
    "chart": "web-1.2.2",
    "appVersion": "1.9.0",
    "description": "Rollback complete"
  }
]
"#;

        let revisions = parse_history_json(json).expect("history should parse");
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].revision, 5);
        assert_eq!(revisions[1].revision, 4);
        assert_eq!(revisions[1].app_version, "1.9.0");
    }

    #[test]
    fn base_command_args_include_context_when_present() {
        let args = base_command_args(Some("staging"), "demo");
        assert_eq!(
            args,
            vec![
                "--kube-context".to_string(),
                "staging".to_string(),
                "--namespace".to_string(),
                "demo".to_string()
            ]
        );
    }
}
