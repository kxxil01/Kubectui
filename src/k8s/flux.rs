//! Flux-specific reconcile helpers.

/// Annotation used by Flux controllers to handle an explicit reconcile request.
pub const RECONCILE_REQUEST_ANNOTATION: &str = "reconcile.fluxcd.io/requestedAt";

/// Whether a Flux resource kind supports the direct reconcile action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluxReconcileSupport {
    Supported,
    Unsupported(&'static str),
}

impl FluxReconcileSupport {
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub const fn unsupported_reason(self) -> Option<&'static str> {
        match self {
            Self::Supported => None,
            Self::Unsupported(reason) => Some(reason),
        }
    }
}

/// Returns true when the API group belongs to Flux toolkit controllers.
pub fn is_flux_group(group: &str) -> bool {
    group.ends_with(".toolkit.fluxcd.io")
}

/// Returns whether the selected Flux kind supports a direct reconcile request.
///
/// KubecTUI mirrors the Flux CLI capability surface here: Flux toolkit resources
/// are generally reconcilable, except alerting objects that do not expose a
/// `flux reconcile ...` subcommand.
pub fn flux_reconcile_support(group: &str, kind: &str) -> FluxReconcileSupport {
    if !is_flux_group(group) {
        return FluxReconcileSupport::Unsupported(
            "Flux reconcile is only available for Flux toolkit resources.",
        );
    }

    if group == "notification.toolkit.fluxcd.io"
        && matches!(kind, "Alert" | "AlertProvider" | "Provider")
    {
        return FluxReconcileSupport::Unsupported(
            "Flux reconcile is not supported for Alert or AlertProvider resources.",
        );
    }

    FluxReconcileSupport::Supported
}

#[cfg(test)]
mod tests {
    use super::{FluxReconcileSupport, flux_reconcile_support, is_flux_group};

    #[test]
    fn recognizes_flux_groups() {
        assert!(is_flux_group("helm.toolkit.fluxcd.io"));
        assert!(!is_flux_group("apps"));
    }

    #[test]
    fn supports_flux_resource_kinds_reconciled_by_flux_cli() {
        assert_eq!(
            flux_reconcile_support("kustomize.toolkit.fluxcd.io", "Kustomization"),
            FluxReconcileSupport::Supported
        );
        assert_eq!(
            flux_reconcile_support("image.toolkit.fluxcd.io", "ImagePolicy"),
            FluxReconcileSupport::Supported
        );
        assert_eq!(
            flux_reconcile_support("notification.toolkit.fluxcd.io", "Receiver"),
            FluxReconcileSupport::Supported
        );
    }

    #[test]
    fn rejects_unsupported_or_non_flux_resources() {
        assert_eq!(
            flux_reconcile_support("notification.toolkit.fluxcd.io", "Alert").unsupported_reason(),
            Some("Flux reconcile is not supported for Alert or AlertProvider resources.")
        );
        assert_eq!(
            flux_reconcile_support("apps", "Deployment").unsupported_reason(),
            Some("Flux reconcile is only available for Flux toolkit resources.")
        );
    }
}
