//! Shared label-selector matching helpers used by diagnostics and policy analysis.

use std::collections::BTreeMap;

use crate::k8s::dtos::LabelSelectorInfo;

pub fn selector_is_empty(selector: &LabelSelectorInfo) -> bool {
    selector.match_labels.is_empty() && selector.match_expressions.is_empty()
}

pub fn selector_matches_map(
    selector: &LabelSelectorInfo,
    labels: &BTreeMap<String, String>,
) -> bool {
    selector
        .match_labels
        .iter()
        .all(|(key, expected)| labels.get(key).is_some_and(|actual| actual == expected))
        && selector.match_expressions.iter().all(|expr| {
            let actual = labels.get(&expr.key);
            match expr.operator.as_str() {
                "In" => actual
                    .is_some_and(|value| expr.values.iter().any(|candidate| candidate == value)),
                "NotIn" => actual
                    .is_some_and(|value| expr.values.iter().all(|candidate| candidate != value)),
                "Exists" => actual.is_some(),
                "DoesNotExist" => actual.is_none(),
                _ => false,
            }
        })
}

pub fn selector_matches_pairs(selector: &LabelSelectorInfo, labels: &[(String, String)]) -> bool {
    let label_map = labels
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    selector_matches_map(selector, &label_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::{LabelSelectorInfo, LabelSelectorRequirementInfo};

    #[test]
    fn selector_matches_labels_and_expressions() {
        let selector = LabelSelectorInfo {
            match_labels: BTreeMap::from([("app".to_string(), "demo".to_string())]),
            match_expressions: vec![
                LabelSelectorRequirementInfo {
                    key: "tier".to_string(),
                    operator: "In".to_string(),
                    values: vec!["frontend".to_string(), "edge".to_string()],
                },
                LabelSelectorRequirementInfo {
                    key: "managed-by".to_string(),
                    operator: "DoesNotExist".to_string(),
                    values: Vec::new(),
                },
            ],
        };
        let labels = BTreeMap::from([
            ("app".to_string(), "demo".to_string()),
            ("tier".to_string(), "frontend".to_string()),
        ]);

        assert!(selector_matches_map(&selector, &labels));
    }

    #[test]
    fn selector_rejects_unsupported_operator() {
        let selector = LabelSelectorInfo {
            match_labels: BTreeMap::new(),
            match_expressions: vec![LabelSelectorRequirementInfo {
                key: "tier".to_string(),
                operator: "Gt".to_string(),
                values: vec!["3".to_string()],
            }],
        };
        let labels = BTreeMap::from([("tier".to_string(), "4".to_string())]);

        assert!(!selector_matches_map(&selector, &labels));
    }
}
