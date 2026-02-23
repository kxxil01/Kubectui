//! Port forwarding error types and handling

use std::fmt;

/// Comprehensive port forwarding error types
#[derive(Debug, Clone)]
pub enum PortForwardError {
    /// Pod not found in namespace
    PodNotFound {
        namespace: String,
        pod_name: String,
    },

    /// Port not exposed in pod spec
    PortNotExposed {
        pod_name: String,
        port: u16,
        available_ports: Vec<u16>,
    },

    /// Local port already in use
    PortInUse { port: u16, process_name: Option<String> },

    /// Permission denied (typically for privileged ports < 1024)
    PermissionDenied { port: u16, reason: String },

    /// Connection to pod failed
    ConnectionFailed { pod_name: String, retryable: bool, message: String },

    /// Tunnel terminated unexpectedly
    TunnelClosed { tunnel_id: String, reason: String },

    /// Timeout waiting for tunnel
    Timeout { operation: String, duration_ms: u64 },

    /// Invalid port number
    InvalidPort { port: u16, reason: String },

    /// Kubernetes API error
    ApiError { code: String, message: String },

    /// Operation cancelled
    Cancelled { reason: String },
}

impl PortForwardError {
    /// Convert to user-friendly message for TUI display
    pub fn to_user_message(&self) -> String {
        match self {
            Self::PodNotFound { namespace, pod_name } => {
                format!("Pod '{}' not found in namespace '{}'", pod_name, namespace)
            }
            Self::PortNotExposed {
                pod_name,
                port,
                available_ports,
            } => {
                if available_ports.is_empty() {
                    format!(
                        "Port {} not exposed by pod '{}' (no ports available)",
                        port, pod_name
                    )
                } else {
                    format!(
                        "Port {} not exposed by pod '{}'\nAvailable ports: {:?}",
                        port, pod_name, available_ports
                    )
                }
            }
            Self::PortInUse { port, process_name } => {
                if let Some(name) = process_name {
                    format!("Local port {} is already in use (by {})", port, name)
                } else {
                    format!("Local port {} is already in use", port)
                }
            }
            Self::PermissionDenied { port, reason } => {
                format!("Cannot bind to port {}: {} (try port > 1024)", port, reason)
            }
            Self::ConnectionFailed {
                pod_name,
                retryable,
                message,
            } => {
                let retry_hint = if *retryable { " (will retry)" } else { "" };
                format!("Connection to '{}' failed{}: {}", pod_name, retry_hint, message)
            }
            Self::TunnelClosed { tunnel_id, reason } => {
                format!("Tunnel {} closed: {}", tunnel_id, reason)
            }
            Self::Timeout {
                operation,
                duration_ms,
            } => {
                format!("{} timed out after {}ms", operation, duration_ms)
            }
            Self::InvalidPort { port, reason } => {
                format!("Invalid port {}: {}", port, reason)
            }
            Self::ApiError { code, message } => {
                format!("Kubernetes API error {}: {}", code, message)
            }
            Self::Cancelled { reason } => {
                format!("Operation cancelled: {}", reason)
            }
        }
    }

    /// Determine if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ConnectionFailed {
                retryable: true,
                ..
            } | Self::Timeout { .. }
        )
    }

    /// Get suggested action for user
    pub fn suggested_action(&self) -> Option<String> {
        match self {
            Self::PodNotFound { .. } => Some("Check pod name or namespace".to_string()),
            Self::PortNotExposed { available_ports, .. } => {
                available_ports
                    .first()
                    .map(|p| format!("Try port {} instead", p))
            }
            Self::PortInUse { .. } => Some("Use port 0 for auto-assignment".to_string()),
            Self::PermissionDenied { .. } => Some("Use a port > 1024".to_string()),
            _ => None,
        }
    }

    /// Get severity level for error display
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::PodNotFound { .. } | Self::PermissionDenied { .. } => ErrorSeverity::Error,
            Self::PortNotExposed { .. } | Self::PortInUse { .. } => ErrorSeverity::Warning,
            Self::ConnectionFailed { .. } => ErrorSeverity::Warning,
            Self::Timeout { .. } => ErrorSeverity::Warning,
            Self::TunnelClosed { .. } => ErrorSeverity::Info,
            _ => ErrorSeverity::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for PortForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_user_message())
    }
}

impl std::error::Error for PortForwardError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pod_not_found_message() {
        let err = PortForwardError::PodNotFound {
            namespace: "default".to_string(),
            pod_name: "test-pod".to_string(),
        };
        let msg = err.to_user_message();
        assert!(msg.contains("test-pod"));
        assert!(msg.contains("default"));
    }

    #[test]
    fn test_port_in_use_retryable() {
        let err = PortForwardError::PortInUse {
            port: 8080,
            process_name: None,
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_connection_failed_retryable() {
        let err = PortForwardError::ConnectionFailed {
            pod_name: "test".to_string(),
            retryable: true,
            message: "test".to_string(),
        };
        assert!(err.is_retryable());
    }
}
