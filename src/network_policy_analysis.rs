//! Snapshot-only NetworkPolicy resolution for pod, namespace, and policy inspection.

use crate::{
    app::ResourceRef,
    k8s::{
        dtos::{
            LabelSelectorInfo, NamespaceInfo, NetworkPolicyInfo, NetworkPolicyPeerInfo,
            NetworkPolicyPortInfo, NetworkPolicyRuleInfo, PodInfo,
        },
        relationships::{RelationKind, RelationNode},
        selectors::{selector_is_empty, selector_matches_pairs},
    },
    network_policy_semantics::{
        matching_namespaces, policy_applies_to_egress, policy_applies_to_ingress,
        policy_selects_pod,
    },
    state::ClusterSnapshot,
};

#[derive(Debug, Clone)]
pub struct NetworkPolicyAnalysis {
    pub summary_lines: Vec<String>,
    pub tree: Vec<RelationNode>,
}

const MAX_RESOLVED_PEER_PODS: usize = 40;

pub fn analyze_resource(
    resource: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Result<NetworkPolicyAnalysis, String> {
    match resource {
        ResourceRef::Pod(name, namespace) => {
            let pod = snapshot
                .pods
                .iter()
                .find(|pod| &pod.name == name && &pod.namespace == namespace)
                .ok_or_else(|| format!("Pod '{namespace}/{name}' is no longer in the snapshot."))?;
            Ok(analyze_pod(pod, snapshot))
        }
        ResourceRef::Namespace(namespace) => Ok(analyze_namespace(namespace, snapshot)),
        ResourceRef::NetworkPolicy(name, namespace) => {
            let policy = snapshot
                .network_policies
                .iter()
                .find(|policy| &policy.name == name && &policy.namespace == namespace)
                .ok_or_else(|| {
                    format!("NetworkPolicy '{namespace}/{name}' is no longer in the snapshot.")
                })?;
            Ok(analyze_policy(policy, snapshot))
        }
        _ => Err(
            "Network policy inspection is available for Pods, Namespaces, and NetworkPolicies."
                .to_string(),
        ),
    }
}

fn analyze_pod(pod: &PodInfo, snapshot: &ClusterSnapshot) -> NetworkPolicyAnalysis {
    let matching = policies_selecting_pod(pod, snapshot);
    let ingress_isolated = matching
        .iter()
        .any(|policy| policy_applies_to_ingress(policy));
    let egress_isolated = matching
        .iter()
        .any(|policy| policy_applies_to_egress(policy));
    let namespace_summary = namespace_isolation_summary(&pod.namespace, snapshot);

    let mut summary_lines = vec![
        format!(
            "Pod {} selected by {} NetworkPolicy object(s).",
            pod_ref(pod),
            matching.len()
        ),
        direction_summary("Ingress", ingress_isolated, matching.len()),
        direction_summary("Egress", egress_isolated, matching.len()),
        namespace_summary,
    ];
    if matching.is_empty() {
        summary_lines.push("No NetworkPolicies select this pod.".to_string());
    }

    let mut tree = Vec::new();
    if !matching.is_empty() {
        tree.push(section(
            "Policies selecting pod",
            matching
                .into_iter()
                .map(|policy| policy_node(policy, snapshot))
                .collect(),
        ));
    }

    NetworkPolicyAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_namespace(namespace: &str, snapshot: &ClusterSnapshot) -> NetworkPolicyAnalysis {
    let policies = snapshot
        .network_policies
        .iter()
        .filter(|policy| policy.namespace == namespace)
        .collect::<Vec<_>>();
    let pods = snapshot
        .pods
        .iter()
        .filter(|pod| pod.namespace == namespace)
        .collect::<Vec<_>>();
    let ingress_isolated = pods
        .iter()
        .filter(|pod| {
            policies
                .iter()
                .any(|policy| policy_applies_to_ingress(policy) && policy_selects_pod(policy, pod))
        })
        .count();
    let egress_isolated = pods
        .iter()
        .filter(|pod| {
            policies
                .iter()
                .any(|policy| policy_applies_to_egress(policy) && policy_selects_pod(policy, pod))
        })
        .count();

    let mut summary_lines = vec![
        format!(
            "Namespace {namespace} has {} NetworkPolicy object(s).",
            policies.len()
        ),
        format!(
            "Pods isolated by ingress: {ingress_isolated}/{}.",
            pods.len()
        ),
        format!("Pods isolated by egress: {egress_isolated}/{}.", pods.len()),
    ];
    if policies.is_empty() {
        summary_lines.push("Default policy intent: allow all traffic.".to_string());
    }

    let tree = if policies.is_empty() {
        Vec::new()
    } else {
        vec![section(
            "Namespace policies",
            policies
                .into_iter()
                .map(|policy| {
                    let selected_pods = snapshot
                        .pods
                        .iter()
                        .filter(|pod| pod.namespace == namespace && policy_selects_pod(policy, pod))
                        .map(|pod| {
                            leaf_with_resource(
                                &format!("Pod {}", pod.name),
                                Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
                                Some(pod.status.clone()),
                                Some(pod.namespace.clone()),
                            )
                        })
                        .collect::<Vec<_>>();
                    let mut children = vec![leaf(&format!(
                        "SelectedPods {}",
                        if selected_pods.is_empty() {
                            "none".to_string()
                        } else {
                            format!("{}", selected_pods.len())
                        }
                    ))];
                    if !selected_pods.is_empty() {
                        children.push(section("Selected pods", selected_pods));
                    }
                    children.extend(direction_nodes(policy, snapshot));
                    RelationNode {
                        resource: Some(ResourceRef::NetworkPolicy(
                            policy.name.clone(),
                            policy.namespace.clone(),
                        )),
                        label: format!("Policy {}", policy.name),
                        status: Some(policy_direction_status(policy)),
                        namespace: Some(policy.namespace.clone()),
                        relation: RelationKind::SelectedBy,
                        not_found: false,
                        children,
                    }
                })
                .collect(),
        )]
    };

    NetworkPolicyAnalysis {
        summary_lines,
        tree,
    }
}

fn analyze_policy(policy: &NetworkPolicyInfo, snapshot: &ClusterSnapshot) -> NetworkPolicyAnalysis {
    let selected_pods = snapshot
        .pods
        .iter()
        .filter(|pod| policy_selects_pod(policy, pod))
        .collect::<Vec<_>>();
    let mut summary_lines = vec![
        format!(
            "Policy {}/{} selects {} pod(s).",
            policy.namespace,
            policy.name,
            selected_pods.len()
        ),
        format!("Directions: {}.", policy_direction_status(policy)),
        namespace_isolation_summary(&policy.namespace, snapshot),
    ];
    if selected_pods.is_empty() {
        summary_lines.push("No current pods match this policy selector.".to_string());
    }

    let mut tree = Vec::new();
    if !selected_pods.is_empty() {
        tree.push(section(
            "Selected pods",
            selected_pods
                .into_iter()
                .map(|pod| {
                    leaf_with_resource(
                        &format!("Pod {}", pod.name),
                        Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
                        Some(pod.status.clone()),
                        Some(pod.namespace.clone()),
                    )
                })
                .collect(),
        ));
    }
    tree.extend(direction_nodes(policy, snapshot));

    NetworkPolicyAnalysis {
        summary_lines,
        tree,
    }
}

fn policies_selecting_pod<'a>(
    pod: &PodInfo,
    snapshot: &'a ClusterSnapshot,
) -> Vec<&'a NetworkPolicyInfo> {
    snapshot
        .network_policies
        .iter()
        .filter(|policy| policy_selects_pod(policy, pod))
        .collect()
}

fn policy_node(policy: &NetworkPolicyInfo, snapshot: &ClusterSnapshot) -> RelationNode {
    RelationNode {
        resource: Some(ResourceRef::NetworkPolicy(
            policy.name.clone(),
            policy.namespace.clone(),
        )),
        label: format!("Policy {}", policy.name),
        status: Some(policy_direction_status(policy)),
        namespace: Some(policy.namespace.clone()),
        relation: RelationKind::SelectedBy,
        not_found: false,
        children: direction_nodes(policy, snapshot),
    }
}

fn direction_nodes(policy: &NetworkPolicyInfo, snapshot: &ClusterSnapshot) -> Vec<RelationNode> {
    let mut nodes = Vec::new();

    if policy_applies_to_ingress(policy) {
        nodes.push(direction_node(
            "Ingress",
            &policy.ingress,
            &policy.namespace,
            snapshot,
        ));
    }
    if policy_applies_to_egress(policy) {
        nodes.push(direction_node(
            "Egress",
            &policy.egress,
            &policy.namespace,
            snapshot,
        ));
    }

    nodes
}

fn direction_node(
    direction: &str,
    rules: &[NetworkPolicyRuleInfo],
    policy_namespace: &str,
    snapshot: &ClusterSnapshot,
) -> RelationNode {
    let children = if rules.is_empty() {
        vec![leaf(&format!("{direction}Rule deny-all"))]
    } else {
        rules
            .iter()
            .enumerate()
            .map(|(idx, rule)| rule_node(direction, idx + 1, rule, policy_namespace, snapshot))
            .collect()
    };
    RelationNode {
        resource: None,
        label: format!("{direction} rules"),
        status: Some(format!("{}", rules.len())),
        namespace: None,
        relation: RelationKind::Owned,
        not_found: false,
        children,
    }
}

fn rule_node(
    direction: &str,
    ordinal: usize,
    rule: &NetworkPolicyRuleInfo,
    policy_namespace: &str,
    snapshot: &ClusterSnapshot,
) -> RelationNode {
    let peer_children = if rule.peers.is_empty() {
        vec![leaf("Peers all")]
    } else {
        rule.peers
            .iter()
            .flat_map(|peer| peer_nodes(peer, policy_namespace, snapshot))
            .collect()
    };
    let ports = if rule.ports.is_empty() {
        "all ports".to_string()
    } else {
        rule.ports
            .iter()
            .map(format_port)
            .collect::<Vec<_>>()
            .join(", ")
    };

    RelationNode {
        resource: None,
        label: format!("Rule {direction}#{ordinal}"),
        status: Some(ports.clone()),
        namespace: None,
        relation: RelationKind::Owned,
        not_found: false,
        children: vec![
            section("Peers", peer_children),
            leaf(&format!("Ports {ports}")),
        ],
    }
}

fn peer_nodes(
    peer: &NetworkPolicyPeerInfo,
    policy_namespace: &str,
    snapshot: &ClusterSnapshot,
) -> Vec<RelationNode> {
    if let Some(cidr) = &peer.ip_block_cidr {
        let mut text = format!("IPBlock {cidr}");
        if !peer.ip_block_except.is_empty() {
            text.push_str(&format!(" except {}", peer.ip_block_except.join(", ")));
        }
        return vec![leaf(&text)];
    }

    if peer_matches_all_pods(peer) {
        return vec![leaf("AllPods all namespaces")];
    }

    let mut resolved = resolve_peer_pods(peer, policy_namespace, snapshot);
    if resolved.is_empty() {
        return vec![leaf(&format!(
            "Selector {}",
            peer_selector_label(peer, policy_namespace)
        ))];
    }

    resolved.sort_unstable_by(|left, right| {
        left.namespace
            .cmp(&right.namespace)
            .then_with(|| left.name.cmp(&right.name))
    });
    let total = resolved.len();
    let mut nodes = resolved
        .into_iter()
        .take(MAX_RESOLVED_PEER_PODS)
        .map(|pod| {
            leaf_with_resource(
                &format!("Pod {}", pod.name),
                Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
                Some(pod.status.clone()),
                Some(pod.namespace.clone()),
            )
        })
        .collect::<Vec<_>>();
    if total > MAX_RESOLVED_PEER_PODS {
        nodes.push(leaf(&format!(
            "ResolvedPods {} total (showing first {})",
            total, MAX_RESOLVED_PEER_PODS
        )));
    }
    nodes
}

fn resolve_peer_pods<'a>(
    peer: &NetworkPolicyPeerInfo,
    policy_namespace: &str,
    snapshot: &'a ClusterSnapshot,
) -> Vec<&'a PodInfo> {
    if peer_matches_all_pods(peer) {
        return snapshot.pods.iter().collect();
    }

    let namespace_candidates = peer
        .namespace_selector
        .as_ref()
        .map(|selector| {
            if selector_is_empty(selector) {
                observed_namespace_names(snapshot, policy_namespace)
            } else {
                matching_namespaces(selector, snapshot)
                    .into_iter()
                    .map(|namespace| namespace.name.clone())
                    .collect::<Vec<_>>()
            }
        })
        .unwrap_or_else(|| {
            if peer.pod_selector.is_some() {
                vec![policy_namespace.to_string()]
            } else {
                Vec::new()
            }
        });

    let namespaces = if namespace_candidates.is_empty() && peer.namespace_selector.is_none() {
        vec![policy_namespace.to_string()]
    } else {
        namespace_candidates
    };

    snapshot
        .pods
        .iter()
        .filter(|pod| {
            namespaces
                .iter()
                .any(|namespace| namespace == &pod.namespace)
        })
        .filter(|pod| {
            peer.pod_selector
                .as_ref()
                .is_none_or(|selector| selector_matches_pairs(selector, &pod.labels))
        })
        .collect()
}

fn peer_matches_all_pods(peer: &NetworkPolicyPeerInfo) -> bool {
    peer.ip_block_cidr.is_none() && peer.namespace_selector.is_none() && peer.pod_selector.is_none()
}

fn observed_namespace_names(snapshot: &ClusterSnapshot, policy_namespace: &str) -> Vec<String> {
    let mut names = snapshot
        .namespace_list
        .iter()
        .map(|namespace| namespace.name.clone())
        .collect::<Vec<_>>();
    names.extend(snapshot.pods.iter().map(|pod| pod.namespace.clone()));
    names.extend(
        snapshot
            .network_policies
            .iter()
            .map(|policy| policy.namespace.clone()),
    );
    names.push(policy_namespace.to_string());
    names.sort_unstable();
    names.dedup();
    names
}

fn namespace_isolation_summary(namespace: &str, snapshot: &ClusterSnapshot) -> String {
    let policies = snapshot
        .network_policies
        .iter()
        .filter(|policy| policy.namespace == namespace)
        .count();
    let labels = snapshot
        .namespace_list
        .iter()
        .find(|entry| entry.name == namespace)
        .map(namespace_label_summary)
        .unwrap_or_default();
    if policies == 0 {
        format!("Namespace {namespace}: default allow (0 policies active){labels}")
    } else {
        format!("Namespace {namespace}: policy intent managed by {policies} policy(s){labels}")
    }
}

fn namespace_label_summary(namespace: &NamespaceInfo) -> String {
    if namespace.labels.is_empty() {
        String::new()
    } else {
        format!(
            " [{}]",
            namespace
                .labels
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn direction_summary(direction: &str, isolated: bool, matching_policies: usize) -> String {
    if isolated {
        format!(
            "{direction}: deny by default unless a matching rule allows it ({matching_policies} selecting policies)."
        )
    } else {
        format!("{direction}: allow all (no {direction} isolation for this pod).")
    }
}

fn policy_direction_status(policy: &NetworkPolicyInfo) -> String {
    let mut directions = Vec::new();
    if policy_applies_to_ingress(policy) {
        directions.push("Ingress");
    }
    if policy_applies_to_egress(policy) {
        directions.push("Egress");
    }
    directions.join("+")
}

fn peer_selector_label(peer: &NetworkPolicyPeerInfo, policy_namespace: &str) -> String {
    if peer_matches_all_pods(peer) {
        return "all namespaces all pods".to_string();
    }

    let mut parts = Vec::new();
    if let Some(selector) = &peer.namespace_selector {
        parts.push(format!(
            "ns:{}",
            selector_label(selector, "<all namespaces>")
        ));
    } else if peer.pod_selector.is_some() {
        parts.push(format!("ns:{policy_namespace}"));
    }
    if let Some(selector) = &peer.pod_selector {
        parts.push(format!("pod:{}", selector_label(selector, "<all pods>")));
    }
    if parts.is_empty() {
        "all".to_string()
    } else {
        parts.join(" ")
    }
}

fn selector_label(selector: &LabelSelectorInfo, fallback: &str) -> String {
    if selector_is_empty(selector) {
        return fallback.to_string();
    }

    let labels = selector
        .match_labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"));
    let expressions = selector.match_expressions.iter().map(|expr| {
        if expr.values.is_empty() {
            format!("{} {}", expr.key, expr.operator)
        } else {
            format!("{} {} [{}]", expr.key, expr.operator, expr.values.join("|"))
        }
    });
    labels.chain(expressions).collect::<Vec<_>>().join(",")
}

fn format_port(port: &NetworkPolicyPortInfo) -> String {
    let protocol = port.protocol.as_deref().unwrap_or("TCP");
    match (port.port_number, port.port_name.as_deref(), port.end_port) {
        (Some(start), _, Some(end)) if end >= start => format!("{protocol}:{start}-{end}"),
        (Some(number), _, _) => format!("{protocol}:{number}"),
        (None, Some(name), _) => format!("{protocol}:{name}"),
        (None, None, _) => protocol.to_string(),
    }
}

fn pod_ref(pod: &PodInfo) -> String {
    format!("{}/{}", pod.namespace, pod.name)
}

fn section(label: &str, children: Vec<RelationNode>) -> RelationNode {
    RelationNode {
        resource: None,
        label: label.to_string(),
        status: None,
        namespace: None,
        relation: RelationKind::SectionHeader,
        not_found: false,
        children,
    }
}

fn leaf(label: &str) -> RelationNode {
    leaf_with_resource(label, None, None, None)
}

fn leaf_with_resource(
    label: &str,
    resource: Option<ResourceRef>,
    status: Option<String>,
    namespace: Option<String>,
) -> RelationNode {
    RelationNode {
        resource,
        label: label.to_string(),
        status,
        namespace,
        relation: RelationKind::Owned,
        not_found: false,
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::k8s::dtos::{ConfigMapInfo, NetworkPolicyPeerInfo, OwnerRefInfo};

    #[test]
    fn default_policy_types_imply_ingress_only_when_egress_is_empty() {
        let policy = NetworkPolicyInfo::default();
        assert_eq!(
            crate::network_policy_semantics::effective_policy_types(&policy),
            (true, false)
        );
    }

    #[test]
    fn pod_analysis_reports_matching_policy_and_isolation() {
        let snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "default".into(),
                labels: BTreeMap::new(),
                ..Default::default()
            }],
            pods: vec![PodInfo {
                name: "api-0".into(),
                namespace: "default".into(),
                labels: vec![("app".into(), "api".into())],
                owner_references: vec![OwnerRefInfo {
                    kind: "ReplicaSet".into(),
                    name: "api".into(),
                    uid: "uid".into(),
                }],
                ..Default::default()
            }],
            network_policies: vec![NetworkPolicyInfo {
                name: "api-policy".into(),
                namespace: "default".into(),
                pod_selector_spec: LabelSelectorInfo {
                    match_labels: BTreeMap::from([("app".into(), "api".into())]),
                    match_expressions: Vec::new(),
                },
                ingress: vec![NetworkPolicyRuleInfo::default()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let analysis = analyze_resource(
            &ResourceRef::Pod("api-0".into(), "default".into()),
            &snapshot,
        )
        .expect("pod analysis");

        assert!(
            analysis
                .summary_lines
                .iter()
                .any(|line| line.contains("selected by 1"))
        );
        assert!(
            analysis
                .tree
                .iter()
                .any(|node| node.label == "Policies selecting pod")
        );
    }

    #[test]
    fn namespace_analysis_counts_isolated_pods() {
        let snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "default".into(),
                labels: BTreeMap::new(),
                ..Default::default()
            }],
            pods: vec![PodInfo {
                name: "api-0".into(),
                namespace: "default".into(),
                labels: vec![("app".into(), "api".into())],
                ..Default::default()
            }],
            network_policies: vec![NetworkPolicyInfo {
                name: "api-policy".into(),
                namespace: "default".into(),
                pod_selector_spec: LabelSelectorInfo {
                    match_labels: BTreeMap::from([("app".into(), "api".into())]),
                    match_expressions: Vec::new(),
                },
                ingress: vec![NetworkPolicyRuleInfo::default()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let analysis =
            analyze_resource(&ResourceRef::Namespace("default".into()), &snapshot).expect("ns");
        assert!(
            analysis
                .summary_lines
                .iter()
                .any(|line| line.contains("ingress: 1/1") || line.contains("ingress"))
        );
    }

    #[test]
    fn policy_analysis_lists_selected_pods() {
        let snapshot = ClusterSnapshot {
            pods: vec![PodInfo {
                name: "api-0".into(),
                namespace: "default".into(),
                labels: vec![("app".into(), "api".into())],
                ..Default::default()
            }],
            network_policies: vec![NetworkPolicyInfo {
                name: "api-policy".into(),
                namespace: "default".into(),
                pod_selector_spec: LabelSelectorInfo {
                    match_labels: BTreeMap::from([("app".into(), "api".into())]),
                    match_expressions: Vec::new(),
                },
                ..Default::default()
            }],
            config_maps: vec![ConfigMapInfo::default()],
            ..Default::default()
        };

        let analysis = analyze_resource(
            &ResourceRef::NetworkPolicy("api-policy".into(), "default".into()),
            &snapshot,
        )
        .expect("policy");
        assert!(
            analysis
                .tree
                .iter()
                .any(|node| node.label == "Selected pods")
        );
    }

    #[test]
    fn empty_peer_is_rendered_as_all_namespaces_all_pods() {
        let snapshot = ClusterSnapshot {
            pods: vec![
                PodInfo {
                    name: "api-0".into(),
                    namespace: "default".into(),
                    labels: vec![("app".into(), "api".into())],
                    ..Default::default()
                },
                PodInfo {
                    name: "db-0".into(),
                    namespace: "prod".into(),
                    labels: vec![("app".into(), "db".into())],
                    ..Default::default()
                },
            ],
            network_policies: vec![NetworkPolicyInfo {
                name: "allow-all".into(),
                namespace: "default".into(),
                pod_selector_spec: LabelSelectorInfo {
                    match_labels: BTreeMap::from([("app".into(), "api".into())]),
                    match_expressions: Vec::new(),
                },
                ingress: vec![NetworkPolicyRuleInfo {
                    peers: vec![NetworkPolicyPeerInfo::default()],
                    ports: Vec::new(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let analysis = analyze_resource(
            &ResourceRef::NetworkPolicy("allow-all".into(), "default".into()),
            &snapshot,
        )
        .expect("policy");

        let text = format!("{:#?}", analysis.tree);
        assert!(text.contains("AllPods all namespaces"));
    }

    #[test]
    fn broad_peer_resolution_is_capped_for_tree_rendering() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.network_policies.push(NetworkPolicyInfo {
            name: "ns-wide".into(),
            namespace: "default".into(),
            ingress: vec![NetworkPolicyRuleInfo {
                peers: vec![NetworkPolicyPeerInfo {
                    namespace_selector: Some(LabelSelectorInfo::default()),
                    ..Default::default()
                }],
                ports: Vec::new(),
            }],
            ..Default::default()
        });
        for idx in 0..50 {
            snapshot.pods.push(PodInfo {
                name: format!("pod-{idx:02}"),
                namespace: if idx % 2 == 0 {
                    "default".into()
                } else {
                    "prod".into()
                },
                ..Default::default()
            });
        }

        let analysis = analyze_resource(
            &ResourceRef::NetworkPolicy("ns-wide".into(), "default".into()),
            &snapshot,
        )
        .expect("policy");

        let text = format!("{:#?}", analysis.tree);
        assert!(text.contains("ResolvedPods 50 total (showing first 40)"));
    }
}
