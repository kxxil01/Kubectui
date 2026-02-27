//! Helm client-side helpers (repositories, local config).

use crate::k8s::dtos::HelmRepoInfo;

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

fn helm_repo_paths() -> Vec<std::path::PathBuf> {
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

fn parse_helm_repositories(yaml_content: &str) -> Option<Vec<HelmRepoInfo>> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_content).ok()?;
    let repos = doc.get("repositories")?.as_sequence()?;

    let mut result = Vec::new();
    for entry in repos {
        let name = entry.get("name")?.as_str()?.to_string();
        let url = entry.get("url")?.as_str()?.to_string();
        result.push(HelmRepoInfo { name, url });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Some(result)
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
}
