//! Shared Gateway API policy semantics used by traffic debug and relationships.

use crate::k8s::dtos::{
    GatewayBackendRefInfo, GatewayInfo, GatewayListenerInfo, GatewayParentRefInfo,
    ReferenceGrantInfo,
};

pub fn gateway_listener_allows_cross_namespace(listener: &GatewayListenerInfo) -> bool {
    listener
        .allowed_routes_from
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("all"))
}

pub fn gateway_parent_attachment_allowed(
    gateway: &GatewayInfo,
    route_namespace: &str,
    parent_ref: &GatewayParentRefInfo,
) -> bool {
    if route_namespace == gateway.namespace {
        return true;
    }

    let listener = parent_ref.section_name.as_deref().and_then(|section_name| {
        gateway
            .listeners
            .iter()
            .find(|listener| listener.name == section_name)
    });
    match listener {
        Some(listener) => gateway_listener_allows_cross_namespace(listener),
        None => gateway
            .listeners
            .iter()
            .any(gateway_listener_allows_cross_namespace),
    }
}

pub fn reference_grant_allows_backend(
    grants: &[ReferenceGrantInfo],
    route_namespace: &str,
    route_kind: &str,
    backend_ref: &GatewayBackendRefInfo,
) -> bool {
    let target_namespace = backend_ref.namespace.as_deref().unwrap_or(route_namespace);
    grants.iter().any(|grant| {
        grant.namespace == target_namespace
            && grant.from.iter().any(|from| {
                from.group == "gateway.networking.k8s.io"
                    && from.kind == route_kind
                    && from.namespace == route_namespace
            })
            && grant.to.iter().any(|to| {
                to.kind == backend_ref.kind
                    && to.group == backend_ref.group
                    && to
                        .name
                        .as_deref()
                        .is_none_or(|name| name == backend_ref.name)
            })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::k8s::dtos::{
        GatewayListenerInfo, LabelSelectorInfo, LabelSelectorRequirementInfo,
        ReferenceGrantFromInfo, ReferenceGrantToInfo,
    };

    #[test]
    fn listener_allows_only_explicit_all_cross_namespace_policy() {
        assert!(gateway_listener_allows_cross_namespace(
            &GatewayListenerInfo {
                allowed_routes_from: Some("All".into()),
                ..GatewayListenerInfo::default()
            }
        ));
        assert!(!gateway_listener_allows_cross_namespace(
            &GatewayListenerInfo {
                allowed_routes_from: Some("Selector".into()),
                allowed_routes_selector: Some(LabelSelectorInfo {
                    match_labels: BTreeMap::from([("team".into(), "edge".into())]),
                    match_expressions: vec![LabelSelectorRequirementInfo {
                        key: "env".into(),
                        operator: "In".into(),
                        values: vec!["prod".into()],
                    }],
                }),
                ..GatewayListenerInfo::default()
            }
        ));
    }

    #[test]
    fn parent_attachment_uses_section_name_when_present() {
        let gateway = GatewayInfo {
            namespace: "shared".into(),
            listeners: vec![
                GatewayListenerInfo {
                    name: "private".into(),
                    allowed_routes_from: Some("Same".into()),
                    ..GatewayListenerInfo::default()
                },
                GatewayListenerInfo {
                    name: "public".into(),
                    allowed_routes_from: Some("All".into()),
                    ..GatewayListenerInfo::default()
                },
            ],
            ..GatewayInfo::default()
        };
        let parent_ref = GatewayParentRefInfo {
            kind: "Gateway".into(),
            name: "edge".into(),
            namespace: Some("shared".into()),
            section_name: Some("private".into()),
            ..GatewayParentRefInfo::default()
        };

        assert!(!gateway_parent_attachment_allowed(
            &gateway,
            "apps",
            &parent_ref,
        ));
    }

    #[test]
    fn reference_grant_matching_respects_route_kind_and_backend_name() {
        let grants = vec![ReferenceGrantInfo {
            namespace: "backend".into(),
            from: vec![ReferenceGrantFromInfo {
                group: "gateway.networking.k8s.io".into(),
                kind: "HTTPRoute".into(),
                namespace: "apps".into(),
            }],
            to: vec![ReferenceGrantToInfo {
                group: "".into(),
                kind: "Service".into(),
                name: Some("api".into()),
            }],
            ..ReferenceGrantInfo::default()
        }];
        let backend = GatewayBackendRefInfo {
            group: "".into(),
            kind: "Service".into(),
            name: "api".into(),
            namespace: Some("backend".into()),
            port: Some(80),
        };

        assert!(reference_grant_allows_backend(
            &grants,
            "apps",
            "HTTPRoute",
            &backend,
        ));
        assert!(!reference_grant_allows_backend(
            &grants,
            "apps",
            "GRPCRoute",
            &backend,
        ));
    }
}
