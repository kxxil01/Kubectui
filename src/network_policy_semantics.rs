//! Shared NetworkPolicy selector and policy-type semantics.

use crate::{
    k8s::{
        dtos::{LabelSelectorInfo, NamespaceInfo, NetworkPolicyInfo, PodInfo},
        selectors::{selector_matches_map, selector_matches_pairs},
    },
    state::ClusterSnapshot,
};

pub(crate) fn policy_selects_pod(policy: &NetworkPolicyInfo, pod: &PodInfo) -> bool {
    policy.namespace == pod.namespace
        && selector_matches_pairs(&policy.pod_selector_spec, &pod.labels)
}

pub(crate) fn policy_applies_to_ingress(policy: &NetworkPolicyInfo) -> bool {
    effective_policy_types(policy).0
}

pub(crate) fn policy_applies_to_egress(policy: &NetworkPolicyInfo) -> bool {
    effective_policy_types(policy).1
}

pub(crate) fn matching_namespaces<'a>(
    selector: &LabelSelectorInfo,
    snapshot: &'a ClusterSnapshot,
) -> Vec<&'a NamespaceInfo> {
    snapshot
        .namespace_list
        .iter()
        .filter(|namespace| selector_matches_map(selector, &namespace.labels))
        .collect()
}

pub(crate) fn effective_policy_types(policy: &NetworkPolicyInfo) -> (bool, bool) {
    if policy.policy_types.is_empty() {
        return (true, !policy.egress.is_empty());
    }
    (
        policy
            .policy_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case("Ingress")),
        policy
            .policy_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case("Egress")),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::k8s::dtos::{LabelSelectorRequirementInfo, NetworkPolicyRuleInfo};

    #[test]
    fn empty_policy_types_default_to_ingress_only_without_egress_rules() {
        let policy = NetworkPolicyInfo::default();
        assert!(policy_applies_to_ingress(&policy));
        assert!(!policy_applies_to_egress(&policy));
    }

    #[test]
    fn empty_policy_types_enable_egress_when_rules_exist() {
        let policy = NetworkPolicyInfo {
            egress: vec![NetworkPolicyRuleInfo::default()],
            ..Default::default()
        };
        assert!(policy_applies_to_ingress(&policy));
        assert!(policy_applies_to_egress(&policy));
    }

    #[test]
    fn policy_selector_matches_only_same_namespace_pods() {
        let policy = NetworkPolicyInfo {
            namespace: "default".into(),
            pod_selector_spec: LabelSelectorInfo {
                match_labels: BTreeMap::from([("app".into(), "api".into())]),
                match_expressions: vec![LabelSelectorRequirementInfo {
                    key: "tier".into(),
                    operator: "In".into(),
                    values: vec!["backend".into()],
                }],
            },
            ..Default::default()
        };
        let matching_pod = PodInfo {
            namespace: "default".into(),
            labels: vec![
                ("app".into(), "api".into()),
                ("tier".into(), "backend".into()),
            ],
            ..Default::default()
        };
        let other_namespace = PodInfo {
            namespace: "other".into(),
            labels: matching_pod.labels.clone(),
            ..Default::default()
        };

        assert!(policy_selects_pod(&policy, &matching_pod));
        assert!(!policy_selects_pod(&policy, &other_namespace));
    }
}
