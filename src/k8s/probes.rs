//! Health probe extraction and status tracking for Kubernetes pods.

use anyhow::Result;
use k8s_openapi::api::core::v1::{Pod, Probe};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Type of health probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeType {
    Liveness,
    Readiness,
}

impl fmt::Display for ProbeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeType::Liveness => write!(f, "Liveness"),
            ProbeType::Readiness => write!(f, "Readiness"),
        }
    }
}

/// Probe handler type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeHandler {
    Http {
        path: String,
        port: i32,
        scheme: String,
    },
    Exec {
        command: Vec<String>,
    },
    Tcp {
        port: i32,
    },
}

impl fmt::Display for ProbeHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeHandler::Http { path, port, scheme } => {
                write!(f, "{} {}:{}{}", scheme, port, path, port)
            }
            ProbeHandler::Exec { command } => {
                write!(f, "exec: {}", command.join(" "))
            }
            ProbeHandler::Tcp { port } => {
                write!(f, "TCP :{}", port)
            }
        }
    }
}

/// Probe status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeStatus {
    Pending,
    Success,
    Failure,
    Error,
}

impl fmt::Display for ProbeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeStatus::Pending => write!(f, "⏳"),
            ProbeStatus::Success => write!(f, "✓"),
            ProbeStatus::Failure => write!(f, "✗"),
            ProbeStatus::Error => write!(f, "?"),
        }
    }
}

/// Configuration for a health probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeConfig {
    pub probe_type: ProbeType,
    pub handler: ProbeHandler,
    pub initial_delay_seconds: i32,
    pub period_seconds: i32,
    pub timeout_seconds: i32,
    pub success_threshold: i32,
    pub failure_threshold: i32,
}

impl ProbeConfig {
    /// Format probe config for display.
    pub fn format_display(&self) -> String {
        format!(
            "{} ({}), delay: {}s, period: {}s, timeout: {}s",
            self.probe_type,
            self.handler,
            self.initial_delay_seconds,
            self.period_seconds,
            self.timeout_seconds
        )
    }
}

/// Container probes (readiness and/or liveness).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerProbes {
    pub liveness: Option<ProbeConfig>,
    pub readiness: Option<ProbeConfig>,
}

impl ContainerProbes {
    /// Check if container has any probes configured.
    pub fn has_probes(&self) -> bool {
        self.liveness.is_some() || self.readiness.is_some()
    }

    /// Count healthy probes (assuming success).
    pub fn healthy_count(&self) -> usize {
        let mut count = 0;
        if self.liveness.is_some() {
            count += 1;
        }
        if self.readiness.is_some() {
            count += 1;
        }
        count
    }
}

/// Extract probes from a pod specification.
pub fn extract_probes_from_pod(pod: &Pod) -> Result<Vec<(String, ContainerProbes)>> {
    let mut probes = Vec::new();

    if let Some(spec) = &pod.spec {
        for container in &spec.containers {
            let container_name = container.name.clone();
            let mut container_probes = ContainerProbes::default();

            // Extract liveness probe
            if let Some(probe) = &container.liveness_probe
                && let Some(config) = parse_probe(probe, ProbeType::Liveness) {
                    container_probes.liveness = Some(config);
                }

            // Extract readiness probe
            if let Some(probe) = &container.readiness_probe
                && let Some(config) = parse_probe(probe, ProbeType::Readiness) {
                    container_probes.readiness = Some(config);
                }

            if container_probes.has_probes() {
                probes.push((container_name, container_probes));
            }
        }
    }

    Ok(probes)
}

/// Parse a K8s probe into ProbeConfig.
fn parse_probe(probe: &Probe, probe_type: ProbeType) -> Option<ProbeConfig> {
    let handler = if let Some(http_get) = &probe.http_get {
        let port = match &http_get.port {
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(p) => *p,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(_) => 8080,
        };
        ProbeHandler::Http {
            path: http_get.path.clone().unwrap_or_default(),
            port,
            scheme: http_get
                .scheme
                .clone()
                .unwrap_or_else(|| "HTTP".to_string()),
        }
    } else if let Some(exec) = &probe.exec {
        ProbeHandler::Exec {
            command: exec.command.clone().unwrap_or_default(),
        }
    } else if let Some(tcp_socket) = &probe.tcp_socket {
        let port = match &tcp_socket.port {
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(p) => *p,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(_) => 8080,
        };
        ProbeHandler::Tcp { port }
    } else {
        return None;
    };

    Some(ProbeConfig {
        probe_type,
        handler,
        initial_delay_seconds: probe.initial_delay_seconds.unwrap_or(0),
        period_seconds: probe.period_seconds.unwrap_or(10),
        timeout_seconds: probe.timeout_seconds.unwrap_or(1),
        success_threshold: probe.success_threshold.unwrap_or(1),
        failure_threshold: probe.failure_threshold.unwrap_or(3),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_type_display() {
        assert_eq!(ProbeType::Liveness.to_string(), "Liveness");
        assert_eq!(ProbeType::Readiness.to_string(), "Readiness");
    }

    #[test]
    fn test_probe_status_display() {
        assert_eq!(ProbeStatus::Pending.to_string(), "⏳");
        assert_eq!(ProbeStatus::Success.to_string(), "✓");
        assert_eq!(ProbeStatus::Failure.to_string(), "✗");
        assert_eq!(ProbeStatus::Error.to_string(), "?");
    }

    #[test]
    fn test_http_probe_handler_display() {
        let handler = ProbeHandler::Http {
            path: "/healthz".to_string(),
            port: 8080,
            scheme: "HTTP".to_string(),
        };
        assert_eq!(handler.to_string(), "HTTP 8080:/healthz8080");
    }

    #[test]
    fn test_container_probes_healthy_count() {
        let mut probes = ContainerProbes::default();
        assert_eq!(probes.healthy_count(), 0);

        probes.liveness = Some(ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        });
        assert_eq!(probes.healthy_count(), 1);

        probes.readiness = Some(ProbeConfig {
            probe_type: ProbeType::Readiness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        });
        assert_eq!(probes.healthy_count(), 2);
    }
}
