//! Snapshot-backed RBAC subject reverse lookup for access review surfaces.

use crate::{
    app::ResourceRef,
    k8s::dtos::{RbacRule, RoleBindingSubject},
    state::ClusterSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessReviewSubject {
    ServiceAccount { name: String, namespace: String },
    User { name: String },
    Group { name: String },
}

impl AccessReviewSubject {
    pub fn from_resource(resource: &ResourceRef) -> Option<Self> {
        match resource {
            ResourceRef::ServiceAccount(name, namespace) => Some(Self::ServiceAccount {
                name: name.clone(),
                namespace: namespace.clone(),
            }),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::ServiceAccount { name, namespace } => {
                format!("ServiceAccount {namespace}/{name}")
            }
            Self::User { name } => format!("User {name}"),
            Self::Group { name } => format!("Group {name}"),
        }
    }

    pub fn spec(&self) -> String {
        match self {
            Self::ServiceAccount { name, namespace } => {
                format!("ServiceAccount/{namespace}/{name}")
            }
            Self::User { name } => format!("User/{name}"),
            Self::Group { name } => format!("Group/{name}"),
        }
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(
                "Enter ServiceAccount/<namespace>/<name>, User/<name>, or Group/<name>."
                    .to_string(),
            );
        }

        let Some((kind, remainder)) = trimmed.split_once('/') else {
            return Err("Unknown subject kind. Use ServiceAccount, User, or Group.".to_string());
        };
        if kind.eq_ignore_ascii_case("serviceaccount") || kind.eq_ignore_ascii_case("sa") {
            let parts = remainder.split('/').collect::<Vec<_>>();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(
                    "ServiceAccount subjects must use ServiceAccount/<namespace>/<name>."
                        .to_string(),
                );
            }
            return Ok(Self::ServiceAccount {
                namespace: parts[0].to_string(),
                name: parts[1].to_string(),
            });
        }
        if kind.eq_ignore_ascii_case("user") {
            if remainder.is_empty() {
                return Err("User subjects must use User/<name>.".to_string());
            }
            return Ok(Self::User {
                name: remainder.to_string(),
            });
        }
        if kind.eq_ignore_ascii_case("group") {
            if remainder.is_empty() {
                return Err("Group subjects must use Group/<name>.".to_string());
            }
            return Ok(Self::Group {
                name: remainder.to_string(),
            });
        }

        Err("Unknown subject kind. Use ServiceAccount, User, or Group.".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectRoleResolution {
    pub resource: Option<ResourceRef>,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub rules: Vec<RbacRule>,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectBindingResolution {
    pub binding: ResourceRef,
    pub role: SubjectRoleResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectAccessReview {
    pub subject: AccessReviewSubject,
    pub bindings: Vec<SubjectBindingResolution>,
}

pub fn resolve_subject_access_review(
    snapshot: &ClusterSnapshot,
    subject: AccessReviewSubject,
) -> SubjectAccessReview {
    let mut bindings = Vec::new();

    for binding in &snapshot.role_bindings {
        if !role_binding_subject_matches(
            &subject,
            &binding.subjects,
            Some(binding.namespace.as_str()),
        ) {
            continue;
        }
        bindings.push(SubjectBindingResolution {
            binding: ResourceRef::RoleBinding(binding.name.clone(), binding.namespace.clone()),
            role: resolve_role_reference(
                snapshot,
                &binding.role_ref_kind,
                &binding.role_ref_name,
                Some(binding.namespace.as_str()),
            ),
        });
    }

    for binding in &snapshot.cluster_role_bindings {
        if !role_binding_subject_matches(&subject, &binding.subjects, None) {
            continue;
        }
        bindings.push(SubjectBindingResolution {
            binding: ResourceRef::ClusterRoleBinding(binding.name.clone()),
            role: resolve_role_reference(
                snapshot,
                &binding.role_ref_kind,
                &binding.role_ref_name,
                None,
            ),
        });
    }

    bindings.sort_by(|left, right| {
        binding_sort_key(&left.binding).cmp(&binding_sort_key(&right.binding))
    });

    SubjectAccessReview { subject, bindings }
}

fn role_binding_subject_matches(
    subject: &AccessReviewSubject,
    candidates: &[RoleBindingSubject],
    binding_namespace: Option<&str>,
) -> bool {
    candidates.iter().any(|candidate| match subject {
        AccessReviewSubject::ServiceAccount { name, namespace } => {
            candidate.kind == "ServiceAccount"
                && candidate.name == *name
                && candidate
                    .namespace
                    .as_deref()
                    .unwrap_or(binding_namespace.unwrap_or(""))
                    == namespace
        }
        AccessReviewSubject::User { name } => candidate.kind == "User" && candidate.name == *name,
        AccessReviewSubject::Group { name } => candidate.kind == "Group" && candidate.name == *name,
    })
}

fn resolve_role_reference(
    snapshot: &ClusterSnapshot,
    role_ref_kind: &str,
    role_ref_name: &str,
    binding_namespace: Option<&str>,
) -> SubjectRoleResolution {
    match role_ref_kind {
        "Role" => {
            let namespace = binding_namespace.map(str::to_string);
            if let Some(role) = binding_namespace.and_then(|namespace| {
                snapshot
                    .roles
                    .iter()
                    .find(|role| role.name == role_ref_name && role.namespace == namespace)
            }) {
                SubjectRoleResolution {
                    resource: Some(ResourceRef::Role(role.name.clone(), role.namespace.clone())),
                    kind: "Role".to_string(),
                    name: role.name.clone(),
                    namespace: Some(role.namespace.clone()),
                    rules: role.rules.clone(),
                    missing: false,
                }
            } else {
                SubjectRoleResolution {
                    resource: None,
                    kind: "Role".to_string(),
                    name: role_ref_name.to_string(),
                    namespace,
                    rules: Vec::new(),
                    missing: true,
                }
            }
        }
        "ClusterRole" => {
            if let Some(role) = snapshot
                .cluster_roles
                .iter()
                .find(|role| role.name == role_ref_name)
            {
                SubjectRoleResolution {
                    resource: Some(ResourceRef::ClusterRole(role.name.clone())),
                    kind: "ClusterRole".to_string(),
                    name: role.name.clone(),
                    namespace: None,
                    rules: role.rules.clone(),
                    missing: false,
                }
            } else {
                SubjectRoleResolution {
                    resource: None,
                    kind: "ClusterRole".to_string(),
                    name: role_ref_name.to_string(),
                    namespace: None,
                    rules: Vec::new(),
                    missing: true,
                }
            }
        }
        _ => SubjectRoleResolution {
            resource: None,
            kind: role_ref_kind.to_string(),
            name: role_ref_name.to_string(),
            namespace: binding_namespace.map(str::to_string),
            rules: Vec::new(),
            missing: true,
        },
    }
}

fn binding_sort_key(binding: &ResourceRef) -> (u8, String, String) {
    match binding {
        ResourceRef::RoleBinding(name, namespace) => (0, namespace.clone(), name.clone()),
        ResourceRef::ClusterRoleBinding(name) => (1, String::new(), name.clone()),
        _ => (2, String::new(), binding.summary_label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        k8s::dtos::{ClusterRoleBindingInfo, ClusterRoleInfo, RoleBindingInfo, RoleInfo},
        state::ClusterSnapshot,
    };

    #[test]
    fn service_account_reverse_lookup_finds_namespaced_and_cluster_bindings() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.roles.push(RoleInfo {
            name: "payments-reader".into(),
            namespace: "payments".into(),
            rules: vec![RbacRule {
                verbs: vec!["get".into(), "list".into()],
                resources: vec!["pods".into()],
                ..RbacRule::default()
            }],
            ..RoleInfo::default()
        });
        snapshot.cluster_roles.push(ClusterRoleInfo {
            name: "ops-admin".into(),
            rules: vec![RbacRule {
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                ..RbacRule::default()
            }],
            ..ClusterRoleInfo::default()
        });
        snapshot.role_bindings.push(RoleBindingInfo {
            name: "payments-view".into(),
            namespace: "payments".into(),
            role_ref_kind: "Role".into(),
            role_ref_name: "payments-reader".into(),
            subjects: vec![RoleBindingSubject {
                kind: "ServiceAccount".into(),
                name: "api".into(),
                namespace: None,
                api_group: None,
            }],
            ..RoleBindingInfo::default()
        });
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "api-admin".into(),
            role_ref_kind: "ClusterRole".into(),
            role_ref_name: "ops-admin".into(),
            subjects: vec![RoleBindingSubject {
                kind: "ServiceAccount".into(),
                name: "api".into(),
                namespace: Some("payments".into()),
                api_group: None,
            }],
            ..ClusterRoleBindingInfo::default()
        });

        let review = resolve_subject_access_review(
            &snapshot,
            AccessReviewSubject::ServiceAccount {
                name: "api".into(),
                namespace: "payments".into(),
            },
        );

        assert_eq!(review.bindings.len(), 2);
        assert_eq!(
            review.bindings[0].binding,
            ResourceRef::RoleBinding("payments-view".into(), "payments".into())
        );
        assert_eq!(
            review.bindings[0].role.resource,
            Some(ResourceRef::Role(
                "payments-reader".into(),
                "payments".into()
            ))
        );
        assert_eq!(
            review.bindings[1].binding,
            ResourceRef::ClusterRoleBinding("api-admin".into())
        );
        assert_eq!(
            review.bindings[1].role.resource,
            Some(ResourceRef::ClusterRole("ops-admin".into()))
        );
    }

    #[test]
    fn user_reverse_lookup_matches_cluster_role_bindings() {
        let mut snapshot = ClusterSnapshot::default();
        snapshot.cluster_role_bindings.push(ClusterRoleBindingInfo {
            name: "alice-admin".into(),
            role_ref_kind: "ClusterRole".into(),
            role_ref_name: "admin".into(),
            subjects: vec![RoleBindingSubject {
                kind: "User".into(),
                name: "alice@example.com".into(),
                namespace: None,
                api_group: Some("rbac.authorization.k8s.io".into()),
            }],
            ..ClusterRoleBindingInfo::default()
        });

        let review = resolve_subject_access_review(
            &snapshot,
            AccessReviewSubject::User {
                name: "alice@example.com".into(),
            },
        );

        assert_eq!(review.bindings.len(), 1);
        assert_eq!(
            review.bindings[0].binding,
            ResourceRef::ClusterRoleBinding("alice-admin".into())
        );
        assert!(review.bindings[0].role.missing);
        assert_eq!(review.bindings[0].role.kind, "ClusterRole");
        assert_eq!(review.bindings[0].role.name, "admin");
    }

    #[test]
    fn parse_service_account_subject_spec() {
        assert_eq!(
            AccessReviewSubject::parse("sa/payments/api").unwrap(),
            AccessReviewSubject::ServiceAccount {
                name: "api".into(),
                namespace: "payments".into(),
            }
        );
    }

    #[test]
    fn parse_user_and_group_subject_specs() {
        assert_eq!(
            AccessReviewSubject::parse("User/alice@example.com").unwrap(),
            AccessReviewSubject::User {
                name: "alice@example.com".into(),
            }
        );
        assert_eq!(
            AccessReviewSubject::parse("Group/system:masters").unwrap(),
            AccessReviewSubject::Group {
                name: "system:masters".into(),
            }
        );
        assert_eq!(
            AccessReviewSubject::parse("User/spiffe://cluster.local/ns/default/sa/api").unwrap(),
            AccessReviewSubject::User {
                name: "spiffe://cluster.local/ns/default/sa/api".into(),
            }
        );
        assert_eq!(
            AccessReviewSubject::parse("Group/oidc/dev/platform/admins").unwrap(),
            AccessReviewSubject::Group {
                name: "oidc/dev/platform/admins".into(),
            }
        );
    }

    #[test]
    fn parse_rejects_invalid_subject_specs() {
        assert!(AccessReviewSubject::parse("").is_err());
        assert!(AccessReviewSubject::parse("ServiceAccount/payments").is_err());
        assert!(AccessReviewSubject::parse("Robot/api").is_err());
    }
}
