//! Snapshot-only pod-to-pod NetworkPolicy intent evaluation.

use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use crate::{
    app::ResourceRef,
    k8s::{
        dtos::{
            NetworkPolicyInfo, NetworkPolicyPeerInfo, NetworkPolicyPortInfo, NetworkPolicyRuleInfo,
            PodInfo,
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
pub struct ConnectivityAnalysis {
    pub summary_lines: Vec<String>,
    pub tree: Vec<RelationNode>,
}

pub fn analyze_connectivity(
    source: &ResourceRef,
    target: &ResourceRef,
    snapshot: &ClusterSnapshot,
) -> Result<ConnectivityAnalysis, String> {
    let source_pod = resolve_pod(source, snapshot)?;
    let target_pod = resolve_pod(target, snapshot)?;
    let egress = evaluate_egress(source_pod, target_pod, snapshot);
    let ingress = evaluate_ingress(source_pod, target_pod, snapshot);
    let verdict = egress.allowed && ingress.allowed;
    let mut summary_lines = vec![
        format!(
            "Connectivity intent {} -> {}: {}.",
            pod_ref(source_pod),
            pod_ref(target_pod),
            if verdict {
                "ALLOW for at least one matching port/protocol"
            } else {
                "DENY for all evaluated ports/protocols"
            }
        ),
        "Kubernetes policy intent requires both source egress and destination ingress to allow the traffic."
            .to_string(),
        egress.summary,
        ingress.summary,
    ];
    if egress.uses_ip_block || ingress.uses_ip_block {
        summary_lines.push(
            "ipBlock-based pod matching is heuristic here. Kubernetes documents ipBlock for cluster-external CIDRs because Pod IPs are ephemeral and unpredictable."
                .to_string(),
        );
    }

    Ok(ConnectivityAnalysis {
        summary_lines,
        tree: vec![
            section(
                "Verdict",
                vec![
                    resource_leaf(source_pod, "Source pod"),
                    resource_leaf(target_pod, "Target pod"),
                    leaf(if verdict {
                        "Decision allow (at least one matching port/protocol)"
                    } else {
                        "Decision deny (no matching policy path)"
                    }),
                ],
            ),
            egress.node,
            ingress.node,
        ],
    })
}

#[derive(Debug)]
struct DirectionEvaluation {
    allowed: bool,
    summary: String,
    node: RelationNode,
    uses_ip_block: bool,
}

fn resolve_pod<'a>(
    resource: &ResourceRef,
    snapshot: &'a ClusterSnapshot,
) -> Result<&'a PodInfo, String> {
    let ResourceRef::Pod(name, namespace) = resource else {
        return Err("Connectivity check is available for Pod resources only.".to_string());
    };
    snapshot
        .pods
        .iter()
        .find(|pod| pod.name == *name && pod.namespace == *namespace)
        .ok_or_else(|| format!("Pod '{namespace}/{name}' is no longer in the snapshot."))
}

fn evaluate_egress(
    source: &PodInfo,
    target: &PodInfo,
    snapshot: &ClusterSnapshot,
) -> DirectionEvaluation {
    evaluate_direction(
        "Source egress",
        source,
        target,
        snapshot,
        policy_applies_to_egress,
        |policy| &policy.egress,
    )
}

fn evaluate_ingress(
    source: &PodInfo,
    target: &PodInfo,
    snapshot: &ClusterSnapshot,
) -> DirectionEvaluation {
    evaluate_direction(
        "Target ingress",
        target,
        source,
        snapshot,
        policy_applies_to_ingress,
        |policy| &policy.ingress,
    )
}

fn evaluate_direction(
    section_label: &str,
    selected_pod: &PodInfo,
    peer_pod: &PodInfo,
    snapshot: &ClusterSnapshot,
    applies: fn(&NetworkPolicyInfo) -> bool,
    rules: fn(&NetworkPolicyInfo) -> &[NetworkPolicyRuleInfo],
) -> DirectionEvaluation {
    let policies = snapshot
        .network_policies
        .iter()
        .filter(|policy| applies(policy) && policy_selects_pod(policy, selected_pod))
        .collect::<Vec<_>>();

    if policies.is_empty() {
        return DirectionEvaluation {
            allowed: true,
            summary: format!(
                "{section_label}: allow-all because no isolating NetworkPolicy selects {}.",
                pod_ref(selected_pod)
            ),
            node: section(
                section_label,
                vec![leaf(&format!(
                    "No isolating policies select {}",
                    pod_ref(selected_pod)
                ))],
            ),
            uses_ip_block: false,
        };
    }

    let mut matched_policies = 0usize;
    let mut matched_rules = 0usize;
    let mut matched_ports = BTreeSet::new();
    let mut children = Vec::with_capacity(policies.len());
    let mut uses_ip_block = false;

    for policy in policies {
        let mut matched_rule_nodes = Vec::new();
        let mut local_rule_count = 0usize;
        for (idx, rule) in rules(policy).iter().enumerate() {
            if rule
                .peers
                .iter()
                .any(|peer| peer.ip_block_cidr.as_ref().is_some())
            {
                uses_ip_block = true;
            }
            if rule_matches_pod(rule, peer_pod, &policy.namespace, snapshot) {
                local_rule_count += 1;
                matched_rules += 1;
                let ports = ports_label(&rule.ports);
                matched_ports.insert(ports.clone());
                matched_rule_nodes.push(leaf(&format!("Rule #{} allows {}", idx + 1, ports)));
            }
        }

        if local_rule_count > 0 {
            matched_policies += 1;
        }

        children.push(RelationNode {
            resource: Some(ResourceRef::NetworkPolicy(
                policy.name.clone(),
                policy.namespace.clone(),
            )),
            label: format!("Policy {}", policy.name),
            status: Some(if local_rule_count > 0 {
                format!("matched {local_rule_count} rule(s)")
            } else if rules(policy).is_empty() {
                "deny-all".to_string()
            } else {
                "no match".to_string()
            }),
            namespace: Some(policy.namespace.clone()),
            relation: RelationKind::SelectedBy,
            not_found: false,
            children: if matched_rule_nodes.is_empty() {
                vec![leaf(&format!("No rule matches {}", pod_ref(peer_pod)))]
            } else {
                matched_rule_nodes
            },
        });
    }

    let allowed = matched_rules > 0;
    let ports = if matched_ports.is_empty() {
        "no matching ports".to_string()
    } else {
        matched_ports.into_iter().collect::<Vec<_>>().join(", ")
    };
    let summary = if allowed {
        format!(
            "{section_label}: allowed by {matched_rules} matching rule(s) across {matched_policies} policy(s) [{ports}]."
        )
    } else {
        format!(
            "{section_label}: denied because {} isolating policy(s) select {} and none match {}.",
            children.len(),
            pod_ref(selected_pod),
            pod_ref(peer_pod)
        )
    };

    DirectionEvaluation {
        allowed,
        summary,
        node: section(section_label, children),
        uses_ip_block,
    }
}

fn rule_matches_pod(
    rule: &NetworkPolicyRuleInfo,
    pod: &PodInfo,
    policy_namespace: &str,
    snapshot: &ClusterSnapshot,
) -> bool {
    if rule.peers.is_empty() {
        return true;
    }
    rule.peers
        .iter()
        .any(|peer| peer_matches_pod(peer, pod, policy_namespace, snapshot))
}

fn peer_matches_pod(
    peer: &NetworkPolicyPeerInfo,
    pod: &PodInfo,
    policy_namespace: &str,
    snapshot: &ClusterSnapshot,
) -> bool {
    if let Some(cidr) = &peer.ip_block_cidr {
        return pod
            .pod_ip
            .as_deref()
            .and_then(|ip| ip.parse::<IpAddr>().ok())
            .is_some_and(|ip| ip_in_cidr(ip, cidr, &peer.ip_block_except));
    }

    let namespace_match = match (&peer.namespace_selector, &peer.pod_selector) {
        (Some(selector), _) if selector_is_empty(selector) => true,
        (Some(selector), _) => matching_namespaces(selector, snapshot)
            .into_iter()
            .any(|namespace| namespace.name == pod.namespace),
        (None, Some(_)) => pod.namespace == policy_namespace,
        (None, None) => true,
    };
    if !namespace_match {
        return false;
    }

    peer.pod_selector
        .as_ref()
        .is_none_or(|selector| selector_matches_pairs(selector, &pod.labels))
}

fn ip_in_cidr(ip: IpAddr, cidr: &str, except: &[String]) -> bool {
    cidr_contains(ip, cidr) && !except.iter().any(|exception| cidr_contains(ip, exception))
}

fn cidr_contains(ip: IpAddr, cidr: &str) -> bool {
    let Some((network, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match (ip, network.parse::<IpAddr>()) {
        (IpAddr::V4(ip), Ok(IpAddr::V4(network))) if prefix <= 32 => {
            masked_v4(ip, prefix) == masked_v4(network, prefix)
        }
        (IpAddr::V6(ip), Ok(IpAddr::V6(network))) if prefix <= 128 => {
            masked_v6(ip, prefix) == masked_v6(network, prefix)
        }
        _ => false,
    }
}

fn masked_v4(ip: Ipv4Addr, prefix: u8) -> u32 {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    u32::from(ip) & mask
}

fn masked_v6(ip: Ipv6Addr, prefix: u8) -> u128 {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    u128::from_be_bytes(ip.octets()) & mask
}

fn ports_label(ports: &[NetworkPolicyPortInfo]) -> String {
    if ports.is_empty() {
        return "all ports".to_string();
    }
    ports.iter().map(format_port).collect::<Vec<_>>().join(", ")
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
    RelationNode {
        resource: None,
        label: label.to_string(),
        status: None,
        namespace: None,
        relation: RelationKind::Owned,
        not_found: false,
        children: Vec::new(),
    }
}

fn resource_leaf(pod: &PodInfo, prefix: &str) -> RelationNode {
    RelationNode {
        resource: Some(ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
        label: format!("{prefix} {}", pod.name),
        status: Some(pod.status.clone()),
        namespace: Some(pod.namespace.clone()),
        relation: RelationKind::Owned,
        not_found: false,
        children: Vec::new(),
    }
}

fn pod_ref(pod: &PodInfo) -> String {
    format!("{}/{}", pod.namespace, pod.name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::k8s::dtos::{LabelSelectorInfo, NamespaceInfo, NetworkPolicyPeerInfo, OwnerRefInfo};

    #[test]
    fn connectivity_requires_both_egress_and_ingress_allow() {
        let source = pod("client", "default", "10.0.0.2", &[("app", "client")]);
        let target = pod("db", "default", "10.0.0.3", &[("app", "db")]);
        let snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "default".into(),
                ..Default::default()
            }],
            pods: vec![source.clone(), target.clone()],
            network_policies: vec![
                NetworkPolicyInfo {
                    name: "client-egress".into(),
                    namespace: "default".into(),
                    pod_selector_spec: selector("app", "client"),
                    policy_types: vec!["Egress".into()],
                    egress: vec![NetworkPolicyRuleInfo {
                        peers: vec![NetworkPolicyPeerInfo {
                            pod_selector: Some(selector("app", "db")),
                            ..Default::default()
                        }],
                        ports: Vec::new(),
                    }],
                    ..Default::default()
                },
                NetworkPolicyInfo {
                    name: "db-ingress".into(),
                    namespace: "default".into(),
                    pod_selector_spec: selector("app", "db"),
                    policy_types: vec!["Ingress".into()],
                    ingress: vec![NetworkPolicyRuleInfo {
                        peers: vec![NetworkPolicyPeerInfo {
                            pod_selector: Some(selector("app", "client")),
                            ..Default::default()
                        }],
                        ports: Vec::new(),
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let analysis = analyze_connectivity(
            &ResourceRef::Pod(source.name.clone(), source.namespace.clone()),
            &ResourceRef::Pod(target.name.clone(), target.namespace.clone()),
            &snapshot,
        )
        .expect("connectivity");
        assert!(analysis.summary_lines[0].contains("ALLOW"));
    }

    #[test]
    fn ingress_deny_blocks_even_when_source_egress_allows() {
        let source = pod("client", "default", "10.0.0.2", &[("app", "client")]);
        let target = pod("db", "default", "10.0.0.3", &[("app", "db")]);
        let snapshot = ClusterSnapshot {
            namespace_list: vec![NamespaceInfo {
                name: "default".into(),
                ..Default::default()
            }],
            pods: vec![source.clone(), target.clone()],
            network_policies: vec![NetworkPolicyInfo {
                name: "db-deny".into(),
                namespace: "default".into(),
                pod_selector_spec: selector("app", "db"),
                policy_types: vec!["Ingress".into()],
                ingress: Vec::new(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let analysis = analyze_connectivity(
            &ResourceRef::Pod(source.name.clone(), source.namespace.clone()),
            &ResourceRef::Pod(target.name.clone(), target.namespace.clone()),
            &snapshot,
        )
        .expect("connectivity");
        assert!(analysis.summary_lines[0].contains("DENY"));
        assert!(analysis.summary_lines[3].contains("denied"));
    }

    #[test]
    fn ip_block_can_match_target_pod_ip() {
        let source = pod("client", "default", "10.0.0.2", &[("app", "client")]);
        let target = pod("db", "other", "10.2.3.4", &[("app", "db")]);
        let snapshot = ClusterSnapshot {
            namespace_list: vec![
                NamespaceInfo {
                    name: "default".into(),
                    ..Default::default()
                },
                NamespaceInfo {
                    name: "other".into(),
                    ..Default::default()
                },
            ],
            pods: vec![source.clone(), target.clone()],
            network_policies: vec![NetworkPolicyInfo {
                name: "client-egress".into(),
                namespace: "default".into(),
                pod_selector_spec: selector("app", "client"),
                policy_types: vec!["Egress".into()],
                egress: vec![NetworkPolicyRuleInfo {
                    peers: vec![NetworkPolicyPeerInfo {
                        ip_block_cidr: Some("10.2.0.0/16".into()),
                        ip_block_except: vec!["10.2.3.0/24".into()],
                        ..Default::default()
                    }],
                    ports: Vec::new(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let denied = analyze_connectivity(
            &ResourceRef::Pod(source.name.clone(), source.namespace.clone()),
            &ResourceRef::Pod(target.name.clone(), target.namespace.clone()),
            &snapshot,
        )
        .expect("connectivity");
        assert!(denied.summary_lines[0].contains("DENY"));
        assert!(
            denied
                .summary_lines
                .iter()
                .any(|line| line.contains("cluster-external CIDRs"))
        );
    }

    fn selector(key: &str, value: &str) -> LabelSelectorInfo {
        LabelSelectorInfo {
            match_labels: BTreeMap::from([(key.to_string(), value.to_string())]),
            match_expressions: Vec::new(),
        }
    }

    fn pod(name: &str, namespace: &str, ip: &str, labels: &[(&str, &str)]) -> PodInfo {
        PodInfo {
            name: name.into(),
            namespace: namespace.into(),
            status: "Running".into(),
            pod_ip: Some(ip.into()),
            labels: labels
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            owner_references: vec![OwnerRefInfo {
                kind: "ReplicaSet".into(),
                name: format!("{name}-rs"),
                uid: format!("{name}-uid"),
            }],
            ..Default::default()
        }
    }
}
