//! Built-in resource templates rendered onto the canonical YAML apply path.

use anyhow::{Result, anyhow};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceTemplateKind {
    Deployment,
    DeploymentService,
    ConfigMap,
}

impl ResourceTemplateKind {
    pub const ALL: [Self; 3] = [Self::Deployment, Self::DeploymentService, Self::ConfigMap];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::DeploymentService => "Deployment + Service",
            Self::ConfigMap => "ConfigMap",
        }
    }

    pub const fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Deployment => &[
                "new deployment",
                "create deployment",
                "deployment template",
                "workload template",
            ],
            Self::DeploymentService => &[
                "new service",
                "create service",
                "deployment service template",
                "web service template",
            ],
            Self::ConfigMap => &[
                "new configmap",
                "create configmap",
                "config template",
                "configmap template",
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTemplateValues {
    pub kind: ResourceTemplateKind,
    pub name: String,
    pub namespace: String,
    pub image: String,
    pub replicas: String,
    pub container_port: String,
    pub service_port: String,
    pub config_key: String,
    pub config_value: String,
}

impl ResourceTemplateValues {
    pub fn validate(&self) -> Result<ValidatedResourceTemplate> {
        match self.kind {
            ResourceTemplateKind::Deployment | ResourceTemplateKind::ConfigMap => {
                validate_dns_subdomain("name", &self.name)?
            }
            ResourceTemplateKind::DeploymentService => validate_dns_label("name", &self.name)?,
        }
        validate_dns_label("namespace", &self.namespace)?;

        match self.kind {
            ResourceTemplateKind::Deployment => {
                if self.image.trim().is_empty() {
                    return Err(anyhow!("image is required"));
                }
                Ok(ValidatedResourceTemplate {
                    kind: self.kind,
                    name: self.name.trim().to_string(),
                    namespace: self.namespace.trim().to_string(),
                    image: Some(self.image.trim().to_string()),
                    replicas: Some(parse_bounded_u16("replicas", &self.replicas, 0, 100)?),
                    container_port: Some(parse_port("container port", &self.container_port)?),
                    service_port: None,
                    config_key: None,
                    config_value: None,
                })
            }
            ResourceTemplateKind::DeploymentService => {
                if self.image.trim().is_empty() {
                    return Err(anyhow!("image is required"));
                }
                Ok(ValidatedResourceTemplate {
                    kind: self.kind,
                    name: self.name.trim().to_string(),
                    namespace: self.namespace.trim().to_string(),
                    image: Some(self.image.trim().to_string()),
                    replicas: Some(parse_bounded_u16("replicas", &self.replicas, 0, 100)?),
                    container_port: Some(parse_port("container port", &self.container_port)?),
                    service_port: Some(parse_port("service port", &self.service_port)?),
                    config_key: None,
                    config_value: None,
                })
            }
            ResourceTemplateKind::ConfigMap => {
                validate_config_key(&self.config_key)?;
                Ok(ValidatedResourceTemplate {
                    kind: self.kind,
                    name: self.name.trim().to_string(),
                    namespace: self.namespace.trim().to_string(),
                    image: None,
                    replicas: None,
                    container_port: None,
                    service_port: None,
                    config_key: Some(self.config_key.trim().to_string()),
                    config_value: Some(self.config_value.clone()),
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedResourceTemplate {
    pub kind: ResourceTemplateKind,
    pub name: String,
    pub namespace: String,
    pub image: Option<String>,
    pub replicas: Option<u16>,
    pub container_port: Option<u16>,
    pub service_port: Option<u16>,
    pub config_key: Option<String>,
    pub config_value: Option<String>,
}

impl ValidatedResourceTemplate {
    pub fn render_yaml(&self) -> Result<String> {
        let deployment_labels = json!({ "app": self.name });
        match self.kind {
            ResourceTemplateKind::Deployment => serde_yaml::to_string(&json!({
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": {
                    "name": self.name,
                    "namespace": self.namespace,
                },
                "spec": {
                    "replicas": self.replicas.expect("validated replicas"),
                    "selector": { "matchLabels": deployment_labels.clone() },
                    "template": {
                        "metadata": { "labels": deployment_labels.clone() },
                        "spec": {
                            "containers": [{
                                "name": self.name,
                                "image": self.image.as_deref().expect("validated image"),
                                "ports": [{
                                    "containerPort": self.container_port.expect("validated port")
                                }]
                            }]
                        }
                    }
                }
            }))
            .map_err(Into::into),
            ResourceTemplateKind::DeploymentService => {
                let deployment = serde_yaml::to_string(&json!({
                    "apiVersion": "apps/v1",
                    "kind": "Deployment",
                    "metadata": {
                        "name": self.name,
                        "namespace": self.namespace,
                    },
                    "spec": {
                        "replicas": self.replicas.expect("validated replicas"),
                        "selector": { "matchLabels": deployment_labels.clone() },
                        "template": {
                            "metadata": { "labels": deployment_labels.clone() },
                            "spec": {
                                "containers": [{
                                    "name": self.name,
                                    "image": self.image.as_deref().expect("validated image"),
                                    "ports": [{
                                        "containerPort": self.container_port.expect("validated port")
                                    }]
                                }]
                            }
                        }
                    }
                }))?;
                let service = serde_yaml::to_string(&json!({
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": {
                        "name": self.name,
                        "namespace": self.namespace,
                    },
                    "spec": {
                        "selector": deployment_labels,
                        "ports": [{
                            "port": self.service_port.expect("validated service port"),
                            "targetPort": self.container_port.expect("validated container port")
                        }]
                    }
                }))?;
                Ok(format!("{deployment}---\n{service}"))
            }
            ResourceTemplateKind::ConfigMap => serde_yaml::to_string(&json!({
                "apiVersion": "v1",
                "kind": "ConfigMap",
                "metadata": {
                    "name": self.name,
                    "namespace": self.namespace,
                },
                "data": {
                    self.config_key.as_deref().expect("validated key"): self.config_value.as_deref().expect("validated value")
                }
            }))
            .map_err(Into::into),
        }
    }
}

fn validate_dns_label(field: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("{field} is required"));
    }
    if trimmed.len() > 63 {
        return Err(anyhow!("{field} must be 63 characters or fewer"));
    }
    let bytes = trimmed.as_bytes();
    let valid = bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-');
    if !valid {
        return Err(anyhow!(
            "{field} must use lowercase letters, digits, or '-'"
        ));
    }
    if trimmed.starts_with('-') || trimmed.ends_with('-') {
        return Err(anyhow!("{field} cannot start or end with '-'"));
    }
    Ok(())
}

fn validate_dns_subdomain(field: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("{field} is required"));
    }
    if trimmed.len() > 253 {
        return Err(anyhow!("{field} must be 253 characters or fewer"));
    }

    for segment in trimmed.split('.') {
        validate_dns_label(field, segment)?;
    }
    Ok(())
}

fn validate_config_key(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("config key is required"));
    }
    if trimmed.len() > 253 {
        return Err(anyhow!("config key must be 253 characters or fewer"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(anyhow!(
            "config key may only contain letters, digits, '.', '-', or '_'"
        ));
    }
    Ok(())
}

fn parse_bounded_u16(field: &str, value: &str, min: u16, max: u16) -> Result<u16> {
    let parsed: u16 = value
        .trim()
        .parse()
        .map_err(|_| anyhow!("{field} must be a number"))?;
    if !(min..=max).contains(&parsed) {
        return Err(anyhow!("{field} must be between {min} and {max}"));
    }
    Ok(parsed)
}

fn parse_port(field: &str, value: &str) -> Result<u16> {
    parse_bounded_u16(field, value, 1, 65535)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_service_renders_two_documents() {
        let yaml = ResourceTemplateValues {
            kind: ResourceTemplateKind::DeploymentService,
            name: "api".into(),
            namespace: "default".into(),
            image: "nginx:1.27".into(),
            replicas: "2".into(),
            container_port: "8080".into(),
            service_port: "80".into(),
            config_key: String::new(),
            config_value: String::new(),
        }
        .validate()
        .expect("valid")
        .render_yaml()
        .expect("yaml");

        assert!(yaml.contains("kind: Deployment"));
        assert!(yaml.contains("kind: Service"));
        assert!(yaml.contains("namespace: default"));
    }

    #[test]
    fn invalid_names_are_rejected() {
        let err = ResourceTemplateValues {
            kind: ResourceTemplateKind::Deployment,
            name: "Bad_Name".into(),
            namespace: "default".into(),
            image: "nginx".into(),
            replicas: "1".into(),
            container_port: "80".into(),
            service_port: String::new(),
            config_key: String::new(),
            config_value: String::new(),
        }
        .validate()
        .expect_err("invalid");

        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn deployment_name_accepts_dns_subdomain() {
        let validated = ResourceTemplateValues {
            kind: ResourceTemplateKind::Deployment,
            name: "api.v2".into(),
            namespace: "default".into(),
            image: "nginx".into(),
            replicas: "1".into(),
            container_port: "80".into(),
            service_port: String::new(),
            config_key: String::new(),
            config_value: String::new(),
        }
        .validate()
        .expect("deployment names support dns subdomains");

        assert_eq!(validated.name, "api.v2");
    }

    #[test]
    fn deployment_service_name_rejects_dots_for_service_compatibility() {
        let err = ResourceTemplateValues {
            kind: ResourceTemplateKind::DeploymentService,
            name: "api.v2".into(),
            namespace: "default".into(),
            image: "nginx".into(),
            replicas: "1".into(),
            container_port: "80".into(),
            service_port: "80".into(),
            config_key: String::new(),
            config_value: String::new(),
        }
        .validate()
        .expect_err("service template uses label-compatible names");

        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn config_key_rejects_invalid_characters() {
        let err = ResourceTemplateValues {
            kind: ResourceTemplateKind::ConfigMap,
            name: "api-config".into(),
            namespace: "default".into(),
            image: String::new(),
            replicas: String::new(),
            container_port: String::new(),
            service_port: String::new(),
            config_key: "invalid/key".into(),
            config_value: "value".into(),
        }
        .validate()
        .expect_err("config key should fail");

        assert!(err.to_string().contains("config key"));
    }
}
