//! Dynamic Gateway API fetch and parsing helpers.

use anyhow::{Context, Result};
use kube::{
    Api, Client,
    api::{ApiResource, DynamicObject, GroupVersionKind, ListParams},
};
use serde_json::Value;

use crate::k8s::{
    client::{is_forbidden_error, is_missing_api_error},
    conversions::extract_common_metadata,
    dtos::{
        GatewayBackendRefInfo, GatewayClassInfo, GatewayInfo, GatewayListenerInfo,
        GatewayParentRefInfo, GrpcRouteInfo, HttpRouteInfo, LabelSelectorInfo,
        LabelSelectorRequirementInfo, ReferenceGrantFromInfo, ReferenceGrantInfo,
        ReferenceGrantToInfo,
    },
};

const GATEWAY_API_GROUP: &str = "gateway.networking.k8s.io";

#[derive(Clone, Copy)]
pub struct GatewayApiKindSpec {
    pub kind: &'static str,
    pub plural: &'static str,
    pub versions: &'static [&'static str],
    pub namespaced: bool,
}

pub const GATEWAY_CLASS_SPEC: GatewayApiKindSpec = GatewayApiKindSpec {
    kind: "GatewayClass",
    plural: "gatewayclasses",
    versions: &["v1", "v1beta1"],
    namespaced: false,
};

pub const GATEWAY_SPEC: GatewayApiKindSpec = GatewayApiKindSpec {
    kind: "Gateway",
    plural: "gateways",
    versions: &["v1", "v1beta1"],
    namespaced: true,
};

pub const HTTP_ROUTE_SPEC: GatewayApiKindSpec = GatewayApiKindSpec {
    kind: "HTTPRoute",
    plural: "httproutes",
    versions: &["v1", "v1beta1"],
    namespaced: true,
};

pub const GRPC_ROUTE_SPEC: GatewayApiKindSpec = GatewayApiKindSpec {
    kind: "GRPCRoute",
    plural: "grpcroutes",
    versions: &["v1", "v1alpha2"],
    namespaced: true,
};

pub const REFERENCE_GRANT_SPEC: GatewayApiKindSpec = GatewayApiKindSpec {
    kind: "ReferenceGrant",
    plural: "referencegrants",
    versions: &["v1beta1", "v1alpha2"],
    namespaced: true,
};

pub async fn fetch_gateway_classes(client: &Client) -> Result<Vec<GatewayClassInfo>> {
    let (version, items) = list_gateway_api_resources(client, None, GATEWAY_CLASS_SPEC).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| parse_gateway_class(version, item))
        .collect())
}

pub async fn fetch_gateways(client: &Client, namespace: Option<&str>) -> Result<Vec<GatewayInfo>> {
    let (version, items) = list_gateway_api_resources(client, namespace, GATEWAY_SPEC).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| parse_gateway(version, item))
        .collect())
}

pub async fn fetch_http_routes(
    client: &Client,
    namespace: Option<&str>,
) -> Result<Vec<HttpRouteInfo>> {
    let (version, items) = list_gateway_api_resources(client, namespace, HTTP_ROUTE_SPEC).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| parse_http_route(version, item))
        .collect())
}

pub async fn fetch_grpc_routes(
    client: &Client,
    namespace: Option<&str>,
) -> Result<Vec<GrpcRouteInfo>> {
    let (version, items) = list_gateway_api_resources(client, namespace, GRPC_ROUTE_SPEC).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| parse_grpc_route(version, item))
        .collect())
}

pub async fn fetch_reference_grants(
    client: &Client,
    namespace: Option<&str>,
) -> Result<Vec<ReferenceGrantInfo>> {
    let (version, items) =
        list_gateway_api_resources(client, namespace, REFERENCE_GRANT_SPEC).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| parse_reference_grant(version, item))
        .collect())
}

async fn list_gateway_api_resources(
    client: &Client,
    namespace: Option<&str>,
    spec: GatewayApiKindSpec,
) -> Result<(&'static str, Vec<DynamicObject>)> {
    for version in spec.versions {
        let gvk = GroupVersionKind::gvk(GATEWAY_API_GROUP, version, spec.kind);
        let mut ar = ApiResource::from_gvk(&gvk);
        ar.plural = spec.plural.to_string();
        let api: Api<DynamicObject> = if spec.namespaced {
            match namespace {
                Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
                None => Api::all_with(client.clone(), &ar),
            }
        } else {
            Api::all_with(client.clone(), &ar)
        };

        match api.list(&ListParams::default()).await {
            Ok(list) => return Ok((version, list.items)),
            Err(err) if is_missing_api_error(&err) => continue,
            Err(err) if is_forbidden_error(&err) => return Ok((version, Vec::new())),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed fetching Gateway API resource {} {}",
                        version, spec.plural
                    )
                });
            }
        }
    }

    Ok((spec.versions[0], Vec::new()))
}

fn parse_gateway_class(version: &str, item: DynamicObject) -> Option<GatewayClassInfo> {
    let metadata = extract_common_metadata(&item.metadata);
    Some(GatewayClassInfo {
        name: metadata.name,
        version: version.to_string(),
        controller_name: item
            .data
            .pointer("/spec/controllerName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        accepted: condition_status_bool(item.data.pointer("/status/conditions"), "Accepted"),
        age: metadata.age,
        created_at: metadata.created_at,
    })
}

fn parse_gateway(version: &str, item: DynamicObject) -> Option<GatewayInfo> {
    let metadata = extract_common_metadata(&item.metadata);
    let listeners = listener_specs(&item.data);
    Some(GatewayInfo {
        name: metadata.name,
        namespace: metadata.namespace,
        version: version.to_string(),
        gateway_class_name: item
            .data
            .pointer("/spec/gatewayClassName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        addresses: item
            .data
            .pointer("/status/addresses")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| entry.get("value").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        listeners,
        labels: item.metadata.labels.unwrap_or_default(),
        age: metadata.age,
        created_at: metadata.created_at,
    })
}

fn parse_http_route(version: &str, item: DynamicObject) -> Option<HttpRouteInfo> {
    parse_route_common(version, item).map(
        |(
            name,
            namespace,
            hostnames,
            parent_refs,
            backend_refs,
            rule_count,
            labels,
            age,
            created_at,
        )| HttpRouteInfo {
            name,
            namespace,
            version: version.to_string(),
            hostnames,
            parent_refs,
            backend_refs,
            rule_count,
            labels,
            age,
            created_at,
        },
    )
}

fn parse_grpc_route(version: &str, item: DynamicObject) -> Option<GrpcRouteInfo> {
    parse_route_common(version, item).map(
        |(
            name,
            namespace,
            hostnames,
            parent_refs,
            backend_refs,
            rule_count,
            labels,
            age,
            created_at,
        )| GrpcRouteInfo {
            name,
            namespace,
            version: version.to_string(),
            hostnames,
            parent_refs,
            backend_refs,
            rule_count,
            labels,
            age,
            created_at,
        },
    )
}

type ParsedRouteCommon = (
    String,
    String,
    Vec<String>,
    Vec<GatewayParentRefInfo>,
    Vec<GatewayBackendRefInfo>,
    usize,
    std::collections::BTreeMap<String, String>,
    Option<std::time::Duration>,
    Option<crate::time::AppTimestamp>,
);

fn parse_route_common(version: &str, item: DynamicObject) -> Option<ParsedRouteCommon> {
    let metadata = extract_common_metadata(&item.metadata);
    let _ = version;
    let rule_path = "/spec/rules";
    let rule_count = item
        .data
        .pointer(rule_path)
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Some((
        metadata.name,
        metadata.namespace,
        string_array(item.data.pointer("/spec/hostnames")),
        parent_refs(item.data.pointer("/spec/parentRefs")),
        backend_refs(item.data.pointer(rule_path)),
        rule_count,
        item.metadata.labels.unwrap_or_default(),
        metadata.age,
        metadata.created_at,
    ))
}

fn parse_reference_grant(version: &str, item: DynamicObject) -> Option<ReferenceGrantInfo> {
    let metadata = extract_common_metadata(&item.metadata);
    Some(ReferenceGrantInfo {
        name: metadata.name,
        namespace: metadata.namespace,
        version: version.to_string(),
        from: item
            .data
            .pointer("/spec/from")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| {
                        Some(ReferenceGrantFromInfo {
                            group: entry.get("group")?.as_str()?.to_string(),
                            kind: entry.get("kind")?.as_str()?.to_string(),
                            namespace: entry.get("namespace")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        to: item
            .data
            .pointer("/spec/to")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| {
                        Some(ReferenceGrantToInfo {
                            group: entry.get("group")?.as_str()?.to_string(),
                            kind: entry.get("kind")?.as_str()?.to_string(),
                            name: entry
                                .get("name")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        age: metadata.age,
        created_at: metadata.created_at,
    })
}

fn listener_specs(data: &Value) -> Vec<GatewayListenerInfo> {
    let status_listeners = data
        .pointer("/status/listeners")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    data.pointer("/spec/listeners")
        .and_then(Value::as_array)
        .map(|listeners| {
            listeners
                .iter()
                .map(|entry| {
                    let name = entry
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let status = status_listeners.iter().find(|status_entry| {
                        status_entry.get("name").and_then(Value::as_str) == Some(name.as_str())
                    });
                    GatewayListenerInfo {
                        name,
                        protocol: entry
                            .get("protocol")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        port: entry
                            .get("port")
                            .and_then(Value::as_i64)
                            .map(|value| value as i32)
                            .unwrap_or_default(),
                        hostname: entry
                            .get("hostname")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        allowed_routes_from: entry
                            .pointer("/allowedRoutes/namespaces/from")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        allowed_routes_selector: label_selector(
                            entry.pointer("/allowedRoutes/namespaces/selector"),
                        ),
                        attached_routes: status
                            .and_then(|value| value.get("attachedRoutes"))
                            .and_then(Value::as_u64)
                            .map_or(0, |value| value as usize),
                        ready: status.and_then(|value| {
                            condition_status_bool(value.get("conditions"), "Programmed")
                                .or_else(|| condition_status_bool(value.get("conditions"), "Ready"))
                        }),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parent_refs(value: Option<&Value>) -> Vec<GatewayParentRefInfo> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|entry| {
                    Some(GatewayParentRefInfo {
                        group: entry
                            .get("group")
                            .and_then(Value::as_str)
                            .unwrap_or(GATEWAY_API_GROUP)
                            .to_string(),
                        kind: entry
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("Gateway")
                            .to_string(),
                        name: entry.get("name")?.as_str()?.to_string(),
                        namespace: entry
                            .get("namespace")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        section_name: entry
                            .get("sectionName")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn backend_refs(rules: Option<&Value>) -> Vec<GatewayBackendRefInfo> {
    let mut refs = Vec::new();
    for rule in rules.and_then(Value::as_array).into_iter().flatten() {
        for candidate in rule
            .get("backendRefs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(reference) = parse_backend_ref(candidate) {
                refs.push(reference);
            }
        }
    }
    refs.sort_unstable_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.namespace.cmp(&right.namespace))
            .then_with(|| left.port.cmp(&right.port))
    });
    refs.dedup();
    refs
}

fn parse_backend_ref(value: &Value) -> Option<GatewayBackendRefInfo> {
    Some(GatewayBackendRefInfo {
        group: value
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("Service")
            .to_string(),
        name: value.get("name")?.as_str()?.to_string(),
        namespace: value
            .get("namespace")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        port: value
            .get("port")
            .and_then(Value::as_i64)
            .map(|value| value as i32),
    })
}

fn label_selector(value: Option<&Value>) -> Option<LabelSelectorInfo> {
    let selector = value?;
    let match_labels = selector
        .get("matchLabels")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let match_expressions = selector
        .get("matchExpressions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|entry| {
                    Some(LabelSelectorRequirementInfo {
                        key: entry.get("key")?.as_str()?.to_string(),
                        operator: entry.get("operator")?.as_str()?.to_string(),
                        values: entry
                            .get("values")
                            .and_then(Value::as_array)
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(ToOwned::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(LabelSelectorInfo {
        match_labels,
        match_expressions,
    })
}

fn condition_status_bool(value: Option<&Value>, condition_type: &str) -> Option<bool> {
    value
        .and_then(Value::as_array)
        .and_then(|conditions| {
            conditions.iter().find(|condition| {
                condition.get("type").and_then(Value::as_str) == Some(condition_type)
            })
        })
        .and_then(|condition| condition.get("status").and_then(Value::as_str))
        .map(|status| status.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use kube::api::ObjectMeta;

    use super::*;

    fn object(name: &str, namespace: Option<&str>, data: Value) -> DynamicObject {
        DynamicObject {
            types: None,
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: namespace.map(ToOwned::to_owned),
                ..Default::default()
            },
            data,
        }
    }

    #[test]
    fn parses_gateway_listener_details() {
        let item = object(
            "edge",
            Some("default"),
            serde_json::json!({
                "spec": {
                    "gatewayClassName": "istio",
                    "listeners": [{
                        "name": "http",
                        "protocol": "HTTP",
                        "port": 80,
                        "hostname": "app.example.test",
                        "allowedRoutes": {"namespaces": {"from": "All"}}
                    }]
                },
                "status": {
                    "addresses": [{"value": "10.0.0.12"}],
                    "listeners": [{
                        "name": "http",
                        "attachedRoutes": 2,
                        "conditions": [{"type": "Programmed", "status": "True"}]
                    }]
                }
            }),
        );

        let parsed = parse_gateway("v1", item).expect("gateway");
        assert_eq!(parsed.gateway_class_name, "istio");
        assert_eq!(parsed.addresses, vec!["10.0.0.12"]);
        assert_eq!(parsed.listeners.len(), 1);
        assert_eq!(
            parsed.listeners[0].allowed_routes_from.as_deref(),
            Some("All")
        );
        assert!(parsed.listeners[0].allowed_routes_selector.is_none());
        assert_eq!(parsed.listeners[0].attached_routes, 2);
        assert_eq!(parsed.listeners[0].ready, Some(true));
    }

    #[test]
    fn parses_gateway_listener_namespace_selector() {
        let item = object(
            "edge",
            Some("default"),
            serde_json::json!({
                "spec": {
                    "gatewayClassName": "istio",
                    "listeners": [{
                        "name": "http",
                        "protocol": "HTTP",
                        "port": 80,
                        "allowedRoutes": {
                            "namespaces": {
                                "from": "Selector",
                                "selector": {
                                    "matchLabels": {"team": "edge"}
                                }
                            }
                        }
                    }]
                }
            }),
        );

        let parsed = parse_gateway("v1", item).expect("gateway");
        let selector = parsed.listeners[0]
            .allowed_routes_selector
            .as_ref()
            .expect("selector");
        assert_eq!(
            selector.match_labels.get("team").map(String::as_str),
            Some("edge")
        );
    }

    #[test]
    fn parses_http_route_parent_and_backend_refs() {
        let item = object(
            "frontend",
            Some("default"),
            serde_json::json!({
                "spec": {
                    "hostnames": ["app.example.test"],
                    "parentRefs": [{"name": "edge", "sectionName": "http"}],
                    "rules": [{
                        "backendRefs": [
                            {"name": "frontend-svc", "port": 8080},
                            {"name": "frontend-svc", "port": 8080}
                        ]
                    }]
                }
            }),
        );

        let parsed = parse_http_route("v1", item).expect("http route");
        assert_eq!(parsed.hostnames, vec!["app.example.test"]);
        assert_eq!(parsed.parent_refs.len(), 1);
        assert_eq!(parsed.backend_refs.len(), 1);
        assert_eq!(parsed.backend_refs[0].name, "frontend-svc");
        assert_eq!(parsed.backend_refs[0].port, Some(8080));
    }
}
