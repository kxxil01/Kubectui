//! Sort types and filtering functions for resource list views.

use std::cmp::Ordering;

use crate::k8s::dtos::PodInfo;
use crate::ui::contains_ci;

/// Shared sortable columns for cross-view list sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadSortColumn {
    Name,
    Age,
}

impl WorkloadSortColumn {
    pub(crate) const fn default_descending(self) -> bool {
        match self {
            WorkloadSortColumn::Name => false,
            WorkloadSortColumn::Age => true,
        }
    }
}

/// Active shared sort configuration for cross-view list sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkloadSortState {
    pub column: WorkloadSortColumn,
    pub descending: bool,
}

impl WorkloadSortState {
    pub const fn new(column: WorkloadSortColumn, descending: bool) -> Self {
        Self { column, descending }
    }

    pub const fn cache_variant(self) -> u64 {
        let column = match self.column {
            WorkloadSortColumn::Name => 1_u64,
            WorkloadSortColumn::Age => 2_u64,
        };
        let direction = if self.descending { 1_u64 } else { 0_u64 };
        (column << 1) | direction
    }

    pub const fn short_label(self) -> &'static str {
        match (self.column, self.descending) {
            (WorkloadSortColumn::Name, true) => "name desc",
            (WorkloadSortColumn::Name, false) => "name asc",
            (WorkloadSortColumn::Age, true) => "age desc",
            (WorkloadSortColumn::Age, false) => "age asc",
        }
    }
}

/// Sortable columns for Pods view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PodSortColumn {
    Name,
    Age,
    Status,
    Restarts,
}

impl PodSortColumn {
    pub(crate) const fn default_descending(self) -> bool {
        match self {
            PodSortColumn::Name | PodSortColumn::Status => false,
            PodSortColumn::Age | PodSortColumn::Restarts => true,
        }
    }
}

/// Active Pods sort configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PodSortState {
    pub column: PodSortColumn,
    pub descending: bool,
}

impl PodSortState {
    pub const fn new(column: PodSortColumn, descending: bool) -> Self {
        Self { column, descending }
    }

    pub const fn cache_variant(self) -> u64 {
        let column = match self.column {
            PodSortColumn::Name => 1_u64,
            PodSortColumn::Age => 2_u64,
            PodSortColumn::Status => 3_u64,
            PodSortColumn::Restarts => 4_u64,
        };
        let direction = if self.descending { 1_u64 } else { 0_u64 };
        (column << 1) | direction
    }

    pub const fn short_label(self) -> &'static str {
        match (self.column, self.descending) {
            (PodSortColumn::Name, true) => "name desc",
            (PodSortColumn::Name, false) => "name asc",
            (PodSortColumn::Age, true) => "age desc",
            (PodSortColumn::Age, false) => "age asc",
            (PodSortColumn::Status, true) => "status desc",
            (PodSortColumn::Status, false) => "status asc",
            (PodSortColumn::Restarts, true) => "restarts desc",
            (PodSortColumn::Restarts, false) => "restarts asc",
        }
    }
}

#[inline]
pub(crate) fn cmp_ci_ascii(left: &str, right: &str) -> Ordering {
    let mut l = left.bytes();
    let mut r = right.bytes();
    loop {
        match (l.next(), r.next()) {
            (Some(lb), Some(rb)) => {
                let lc = lb.to_ascii_lowercase();
                let rc = rb.to_ascii_lowercase();
                if lc != rc {
                    return lc.cmp(&rc);
                }
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

/// Builds filtered pod indices and applies optional sort.
///
/// This function is the canonical pods list ordering path used by both rendering
/// and selected-row resource resolution, so table selection and Enter-open stay aligned.
pub fn filtered_pod_indices(
    pods: &[PodInfo],
    query: &str,
    sort: Option<PodSortState>,
) -> Vec<usize> {
    let query = query.trim();
    let mut out: Vec<usize> = if query.is_empty() {
        (0..pods.len()).collect()
    } else {
        pods.iter()
            .enumerate()
            .filter_map(|(idx, pod)| {
                if contains_ci(&pod.name, query)
                    || contains_ci(&pod.namespace, query)
                    || contains_ci(&pod.status, query)
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    };

    if let Some(sort) = sort {
        out.sort_unstable_by(|left_idx, right_idx| {
            let left = &pods[*left_idx];
            let right = &pods[*right_idx];
            let ordered = match sort.column {
                PodSortColumn::Name => {
                    let base = cmp_ci_ascii(&left.name, &right.name);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                PodSortColumn::Age => {
                    // Sort None (unknown created_at) to the end regardless of direction.
                    match (left.created_at, right.created_at) {
                        (None, None) => Ordering::Equal,
                        (None, Some(_)) => Ordering::Greater,
                        (Some(_), None) => Ordering::Less,
                        (Some(l), Some(r)) => {
                            let base = l.cmp(&r);
                            if sort.descending {
                                base.reverse()
                            } else {
                                base
                            }
                        }
                    }
                }
                PodSortColumn::Status => {
                    let base = cmp_ci_ascii(&left.status, &right.status);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                PodSortColumn::Restarts => {
                    let base = left.restarts.cmp(&right.restarts);
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
            };
            if ordered != Ordering::Equal {
                return ordered;
            }
            let ns = cmp_ci_ascii(&left.namespace, &right.namespace);
            if ns != Ordering::Equal {
                return ns;
            }
            let name = cmp_ci_ascii(&left.name, &right.name);
            if name != Ordering::Equal {
                return name;
            }
            left_idx.cmp(right_idx)
        });
    }

    out
}

/// Builds filtered workload indices and applies shared name/age sorting.
pub fn filtered_workload_indices<T, Match, Name, Namespace, Age>(
    items: &[T],
    query: &str,
    sort: Option<WorkloadSortState>,
    matches_query: Match,
    name: Name,
    namespace: Namespace,
    age: Age,
) -> Vec<usize>
where
    Match: Fn(&T, &str) -> bool,
    Name: Fn(&T) -> &str,
    Namespace: Fn(&T) -> &str,
    Age: Fn(&T) -> Option<std::time::Duration>,
{
    let query = query.trim();
    let mut out: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| matches_query(item, query).then_some(idx))
        .collect();

    if let Some(sort) = sort {
        out.sort_unstable_by(|left_idx, right_idx| {
            let left = &items[*left_idx];
            let right = &items[*right_idx];
            let ordered = match sort.column {
                WorkloadSortColumn::Name => {
                    let base = cmp_ci_ascii(name(left), name(right));
                    if sort.descending {
                        base.reverse()
                    } else {
                        base
                    }
                }
                WorkloadSortColumn::Age => {
                    // Sort None (unknown age) to the end regardless of direction.
                    match (age(left), age(right)) {
                        (None, None) => Ordering::Equal,
                        (None, Some(_)) => Ordering::Greater,
                        (Some(_), None) => Ordering::Less,
                        (Some(l), Some(r)) => {
                            let base = l.cmp(&r);
                            if sort.descending {
                                base.reverse()
                            } else {
                                base
                            }
                        }
                    }
                }
            };
            if ordered != Ordering::Equal {
                return ordered;
            }
            let ns = cmp_ci_ascii(namespace(left), namespace(right));
            if ns != Ordering::Equal {
                return ns;
            }
            let item_name = cmp_ci_ascii(name(left), name(right));
            if item_name != Ordering::Equal {
                return item_name;
            }
            left_idx.cmp(right_idx)
        });
    }

    out
}
