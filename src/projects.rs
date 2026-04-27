//! Snapshot-cached application/project scope inference built from native labels and ownership.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    app::ResourceRef,
    k8s::{
        dtos::{
            AlertSeverity, GatewayBackendRefInfo, IngressInfo, JobInfo, LabelSelectorInfo, PodInfo,
            ServiceInfo,
        },
        selectors::selector_matches_pairs,
    },
    state::{
        ClusterSnapshot, issues::compute_issues, vulnerabilities::compute_vulnerability_findings,
    },
    ui::contains_ci,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub key: String,
    pub name: String,
    pub source_label: String,
    pub cluster_scoped: bool,
    pub namespaces: Vec<String>,
    pub namespaces_label: String,
    pub deployments: usize,
    pub statefulsets: usize,
    pub daemonsets: usize,
    pub jobs: usize,
    pub cronjobs: usize,
    pub pods: usize,
    pub services: usize,
    pub ingresses: usize,
    pub http_routes: usize,
    pub grpc_routes: usize,
    pub issue_count: usize,
    pub workload_count_label: String,
    pub services_label: String,
    pub pods_label: String,
    pub issue_count_label: String,
    pub highest_severity: AlertSeverity,
    pub representative: Option<ResourceRef>,
    pub recent_issues: Vec<String>,
    pub sample_workloads: Vec<String>,
    pub sample_services: Vec<String>,
    pub sample_ingresses: Vec<String>,
    pub sample_routes: Vec<String>,
}

impl ProjectSummary {
    pub fn matches_query(&self, query: &str) -> bool {
        contains_ci(&self.name, query)
            || contains_ci(&self.source_label, query)
            || self
                .namespaces
                .iter()
                .any(|namespace| contains_ci(namespace, query))
            || self
                .sample_workloads
                .iter()
                .any(|name| contains_ci(name, query))
            || self
                .sample_services
                .iter()
                .any(|name| contains_ci(name, query))
            || self
                .sample_ingresses
                .iter()
                .any(|name| contains_ci(name, query))
            || self
                .sample_routes
                .iter()
                .any(|name| contains_ci(name, query))
    }

    pub const fn workload_count(&self) -> usize {
        self.deployments + self.statefulsets + self.daemonsets + self.jobs + self.cronjobs
    }
}

type ProjectCache = Arc<Vec<ProjectSummary>>;
type ProjectCacheKey = (u64, usize);

static PROJECT_CACHE: LazyLock<Mutex<Option<(ProjectCacheKey, ProjectCache)>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn compute_projects(snapshot: &ClusterSnapshot) -> ProjectCache {
    let key = (
        snapshot.snapshot_version,
        std::ptr::from_ref(snapshot) as usize,
    );
    {
        let guard = PROJECT_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_key, projects)) = guard.as_ref()
            && *cached_key == key
        {
            return Arc::clone(projects);
        }
    }

    let projects = Arc::new(build_projects(snapshot));
    {
        let mut guard = PROJECT_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *guard = Some((key, Arc::clone(&projects)));
    }
    projects
}

pub fn filtered_project_indices(projects: &[ProjectSummary], query: &str) -> Vec<usize> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (0..projects.len()).collect();
    }

    projects
        .iter()
        .enumerate()
        .filter_map(|(idx, project)| project.matches_query(trimmed).then_some(idx))
        .collect()
}

#[derive(Debug, Clone)]
struct ProjectIdentity {
    key: String,
    name: String,
    source_label: &'static str,
    cluster_scoped: bool,
}

#[derive(Debug)]
struct ProjectAccumulator {
    name: String,
    source_label: String,
    cluster_scoped: bool,
    namespaces: BTreeSet<String>,
    deployments: BTreeSet<String>,
    statefulsets: BTreeSet<String>,
    daemonsets: BTreeSet<String>,
    jobs: BTreeSet<String>,
    cronjobs: BTreeSet<String>,
    services: BTreeSet<String>,
    ingresses: BTreeSet<String>,
    http_routes: BTreeSet<String>,
    grpc_routes: BTreeSet<String>,
    sample_workloads: BTreeSet<String>,
    sample_services: BTreeSet<String>,
    sample_ingresses: BTreeSet<String>,
    sample_routes: BTreeSet<String>,
    pod_count: usize,
    issue_count: usize,
    highest_severity: AlertSeverity,
    representative: Option<ResourceRef>,
    recent_issues: Vec<String>,
}

impl ProjectAccumulator {
    fn new(identity: &ProjectIdentity) -> Self {
        Self {
            name: identity.name.clone(),
            source_label: identity.source_label.to_string(),
            cluster_scoped: identity.cluster_scoped,
            highest_severity: AlertSeverity::Info,
            ..Self::default()
        }
    }

    fn add_namespace(&mut self, namespace: &str) {
        self.namespaces.insert(namespace.to_string());
    }

    fn add_pod(&mut self, pod: &PodInfo) {
        self.add_namespace(&pod.namespace);
        self.pod_count += 1;
        self.representative
            .get_or_insert_with(|| ResourceRef::Pod(pod.name.clone(), pod.namespace.clone()));
    }

    fn add_deployment(&mut self, resource: &ResourceRef) {
        let ResourceRef::Deployment(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.deployments.insert(name.clone());
        self.sample_workloads.insert(format!("Deployment/{name}"));
        self.representative.get_or_insert_with(|| resource.clone());
    }

    fn add_statefulset(&mut self, resource: &ResourceRef) {
        let ResourceRef::StatefulSet(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.statefulsets.insert(name.clone());
        self.sample_workloads.insert(format!("StatefulSet/{name}"));
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_daemonset(&mut self, resource: &ResourceRef) {
        let ResourceRef::DaemonSet(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.daemonsets.insert(name.clone());
        self.sample_workloads.insert(format!("DaemonSet/{name}"));
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_job(&mut self, resource: &ResourceRef) {
        let ResourceRef::Job(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.jobs.insert(name.clone());
        self.sample_workloads.insert(format!("Job/{name}"));
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_cronjob(&mut self, resource: &ResourceRef) {
        let ResourceRef::CronJob(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.cronjobs.insert(name.clone());
        self.sample_workloads.insert(format!("CronJob/{name}"));
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_service(&mut self, resource: &ResourceRef) {
        let ResourceRef::Service(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.services.insert(name.clone());
        self.sample_services.insert(name.clone());
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_ingress(&mut self, resource: &ResourceRef) {
        let ResourceRef::Ingress(name, namespace) = resource else {
            return;
        };
        self.add_namespace(namespace);
        self.ingresses.insert(name.clone());
        self.sample_ingresses.insert(name.clone());
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_gateway_route(&mut self, resource: &ResourceRef) {
        let ResourceRef::CustomResource {
            name,
            namespace: Some(namespace),
            group,
            kind,
            ..
        } = resource
        else {
            return;
        };
        if group != "gateway.networking.k8s.io" {
            return;
        }
        self.add_namespace(namespace);
        match kind.as_str() {
            "HTTPRoute" => {
                self.http_routes.insert(name.clone());
                self.sample_routes.insert(format!("HTTPRoute/{name}"));
            }
            "GRPCRoute" => {
                self.grpc_routes.insert(name.clone());
                self.sample_routes.insert(format!("GRPCRoute/{name}"));
            }
            _ => return,
        }
        if self.representative.is_none() {
            self.representative = Some(resource.clone());
        }
    }

    fn add_issue(&mut self, severity: AlertSeverity, message: String) {
        self.issue_count += 1;
        if severity_rank(severity) > severity_rank(self.highest_severity) {
            self.highest_severity = severity;
        }
        if self.recent_issues.len() < 3 && !self.recent_issues.contains(&message) {
            self.recent_issues.push(message);
        }
    }

    fn finish(self, key: String) -> ProjectSummary {
        let namespaces: Vec<_> = self.namespaces.into_iter().collect();
        let namespaces_label = namespaces.join(", ");
        let deployments = self.deployments.len();
        let statefulsets = self.statefulsets.len();
        let daemonsets = self.daemonsets.len();
        let jobs = self.jobs.len();
        let cronjobs = self.cronjobs.len();
        let services = self.services.len();
        let issue_count = self.issue_count;
        let workload_count = deployments + statefulsets + daemonsets + jobs + cronjobs;
        ProjectSummary {
            key,
            name: self.name,
            source_label: self.source_label,
            cluster_scoped: self.cluster_scoped,
            namespaces,
            namespaces_label,
            deployments,
            statefulsets,
            daemonsets,
            jobs,
            cronjobs,
            pods: self.pod_count,
            services,
            ingresses: self.ingresses.len(),
            http_routes: self.http_routes.len(),
            grpc_routes: self.grpc_routes.len(),
            issue_count,
            workload_count_label: workload_count.to_string(),
            services_label: services.to_string(),
            pods_label: self.pod_count.to_string(),
            issue_count_label: issue_count.to_string(),
            highest_severity: self.highest_severity,
            representative: self.representative,
            recent_issues: self.recent_issues,
            sample_workloads: self.sample_workloads.into_iter().take(4).collect(),
            sample_services: self.sample_services.into_iter().take(4).collect(),
            sample_ingresses: self.sample_ingresses.into_iter().take(4).collect(),
            sample_routes: self.sample_routes.into_iter().take(4).collect(),
        }
    }
}

impl Default for ProjectAccumulator {
    fn default() -> Self {
        Self {
            name: String::new(),
            source_label: String::new(),
            cluster_scoped: false,
            namespaces: BTreeSet::new(),
            deployments: BTreeSet::new(),
            statefulsets: BTreeSet::new(),
            daemonsets: BTreeSet::new(),
            jobs: BTreeSet::new(),
            cronjobs: BTreeSet::new(),
            services: BTreeSet::new(),
            ingresses: BTreeSet::new(),
            http_routes: BTreeSet::new(),
            grpc_routes: BTreeSet::new(),
            sample_workloads: BTreeSet::new(),
            sample_services: BTreeSet::new(),
            sample_ingresses: BTreeSet::new(),
            sample_routes: BTreeSet::new(),
            pod_count: 0,
            issue_count: 0,
            highest_severity: AlertSeverity::Info,
            representative: None,
            recent_issues: Vec::new(),
        }
    }
}

fn build_projects(snapshot: &ClusterSnapshot) -> Vec<ProjectSummary> {
    let mut projects = BTreeMap::<String, ProjectAccumulator>::new();
    let mut resource_projects = HashMap::<String, String>::new();
    let mut pod_projects = HashMap::<(String, String), String>::new();
    let mut service_projects = HashMap::<(String, String), String>::new();
    let mut owner_projects = HashMap::<(String, String, String), BTreeSet<String>>::new();
    let mut job_to_cronjobs = HashMap::<(String, String), BTreeSet<String>>::new();

    for deployment in &snapshot.deployments {
        let labels = if deployment.pod_template_labels.is_empty() {
            &deployment.selector.match_labels
        } else {
            &deployment.pod_template_labels
        };
        if let Some(identity) = identity_from_map(&deployment.namespace, labels) {
            let resource =
                ResourceRef::Deployment(deployment.name.clone(), deployment.namespace.clone());
            let project = project_mut(&mut projects, &identity);
            project.add_deployment(&resource);
            resource_projects.insert(resource_key(&resource), identity.key.clone());
        }
    }

    for daemonset in &snapshot.daemonsets {
        let labels = if daemonset.pod_template_labels.is_empty() {
            &daemonset.labels
        } else {
            &daemonset.pod_template_labels
        };
        if let Some(identity) = identity_from_map(&daemonset.namespace, labels) {
            let resource =
                ResourceRef::DaemonSet(daemonset.name.clone(), daemonset.namespace.clone());
            let project = project_mut(&mut projects, &identity);
            project.add_daemonset(&resource);
            resource_projects.insert(resource_key(&resource), identity.key.clone());
        }
    }

    for pod in &snapshot.pods {
        if let Some(identity) = identity_from_pairs(&pod.namespace, &pod.labels) {
            let project = project_mut(&mut projects, &identity);
            project.add_pod(pod);
            pod_projects.insert(
                (pod.namespace.clone(), pod.name.clone()),
                identity.key.clone(),
            );
            resource_projects.insert(
                resource_key(&ResourceRef::Pod(pod.name.clone(), pod.namespace.clone())),
                identity.key.clone(),
            );
            for owner in &pod.owner_references {
                owner_projects
                    .entry((
                        owner.kind.clone(),
                        pod.namespace.clone(),
                        owner.name.clone(),
                    ))
                    .or_default()
                    .insert(identity.key.clone());
            }
        }
    }

    for statefulset in &snapshot.statefulsets {
        let project_key =
            identity_from_map(&statefulset.namespace, &statefulset.pod_template_labels)
                .map(|identity| {
                    let resource = ResourceRef::StatefulSet(
                        statefulset.name.clone(),
                        statefulset.namespace.clone(),
                    );
                    let project = project_mut(&mut projects, &identity);
                    project.add_statefulset(&resource);
                    resource_projects.insert(resource_key(&resource), identity.key.clone());
                    identity.key
                })
                .or_else(|| {
                    single_owner_project(
                        &owner_projects,
                        "StatefulSet",
                        &statefulset.namespace,
                        &statefulset.name,
                    )
                });
        if let Some(project_key) = project_key {
            let resource =
                ResourceRef::StatefulSet(statefulset.name.clone(), statefulset.namespace.clone());
            if !resource_projects.contains_key(&resource_key(&resource))
                && let Some(project) = projects.get_mut(&project_key)
            {
                project.add_statefulset(&resource);
                resource_projects.insert(resource_key(&resource), project_key.clone());
            }
        }
    }

    for job in &snapshot.jobs {
        let project_key = identity_from_map(&job.namespace, &job.pod_template_labels)
            .map(|identity| {
                let resource = ResourceRef::Job(job.name.clone(), job.namespace.clone());
                let project = project_mut(&mut projects, &identity);
                project.add_job(&resource);
                resource_projects.insert(resource_key(&resource), identity.key.clone());
                remember_cronjob_parents(&mut job_to_cronjobs, job, &identity.key);
                identity.key
            })
            .or_else(|| single_owner_project(&owner_projects, "Job", &job.namespace, &job.name));
        if let Some(project_key) = project_key {
            let resource = ResourceRef::Job(job.name.clone(), job.namespace.clone());
            if !resource_projects.contains_key(&resource_key(&resource))
                && let Some(project) = projects.get_mut(&project_key)
            {
                project.add_job(&resource);
                resource_projects.insert(resource_key(&resource), project_key.clone());
                remember_cronjob_parents(&mut job_to_cronjobs, job, &project_key);
            }
        }
    }

    for cronjob in &snapshot.cronjobs {
        let project_key = identity_from_map(&cronjob.namespace, &cronjob.pod_template_labels)
            .map(|identity| {
                let resource =
                    ResourceRef::CronJob(cronjob.name.clone(), cronjob.namespace.clone());
                let project = project_mut(&mut projects, &identity);
                project.add_cronjob(&resource);
                resource_projects.insert(resource_key(&resource), identity.key.clone());
                identity.key
            })
            .or_else(|| {
                let lookup = (cronjob.namespace.clone(), cronjob.name.clone());
                job_to_cronjobs.get(&lookup).and_then(|project_keys| {
                    (project_keys.len() == 1)
                        .then(|| project_keys.iter().next().cloned())
                        .flatten()
                })
            });
        if let Some(project_key) = project_key {
            let resource = ResourceRef::CronJob(cronjob.name.clone(), cronjob.namespace.clone());
            if !resource_projects.contains_key(&resource_key(&resource))
                && let Some(project) = projects.get_mut(&project_key)
            {
                project.add_cronjob(&resource);
                resource_projects.insert(resource_key(&resource), project_key.clone());
            }
        }
    }

    for service in &snapshot.services {
        let service_labels = if service.selector.is_empty() {
            &service.labels
        } else {
            &service.selector
        };
        let project_key = identity_from_map(&service.namespace, service_labels)
            .map(|identity| {
                let resource =
                    ResourceRef::Service(service.name.clone(), service.namespace.clone());
                let project = project_mut(&mut projects, &identity);
                project.add_service(&resource);
                resource_projects.insert(resource_key(&resource), identity.key.clone());
                service_projects.insert(
                    (service.namespace.clone(), service.name.clone()),
                    identity.key.clone(),
                );
                identity.key
            })
            .or_else(|| service_project(service, snapshot, &pod_projects));
        if let Some(project_key) = project_key {
            let resource = ResourceRef::Service(service.name.clone(), service.namespace.clone());
            if !resource_projects.contains_key(&resource_key(&resource))
                && let Some(project) = projects.get_mut(&project_key)
            {
                project.add_service(&resource);
                resource_projects.insert(resource_key(&resource), project_key.clone());
                service_projects.insert(
                    (service.namespace.clone(), service.name.clone()),
                    project_key.clone(),
                );
            }
        }
    }

    for ingress in &snapshot.ingresses {
        if let Some(identity) = identity_from_map(&ingress.namespace, &ingress.labels) {
            let resource = ResourceRef::Ingress(ingress.name.clone(), ingress.namespace.clone());
            let project = project_mut(&mut projects, &identity);
            project.add_ingress(&resource);
            resource_projects.insert(resource_key(&resource), identity.key.clone());
        } else if let Some(project_key) = ingress_project(ingress, &service_projects) {
            let resource = ResourceRef::Ingress(ingress.name.clone(), ingress.namespace.clone());
            if let Some(project) = projects.get_mut(&project_key) {
                project.add_ingress(&resource);
            }
            resource_projects.insert(resource_key(&resource), project_key);
        } else {
            continue;
        };
    }

    for route in &snapshot.http_routes {
        let resource = ResourceRef::CustomResource {
            name: route.name.clone(),
            namespace: Some(route.namespace.clone()),
            group: "gateway.networking.k8s.io".to_string(),
            version: route.version.clone(),
            kind: "HTTPRoute".to_string(),
            plural: "httproutes".to_string(),
        };
        if let Some(identity) = identity_from_map(&route.namespace, &route.labels) {
            let project = project_mut(&mut projects, &identity);
            project.add_gateway_route(&resource);
            resource_projects.insert(resource_key(&resource), identity.key);
        } else if let Some(project_key) = route_backend_project(
            &route.namespace,
            route.backend_refs.iter(),
            &service_projects,
        ) {
            if let Some(project) = projects.get_mut(&project_key) {
                project.add_gateway_route(&resource);
            }
            resource_projects.insert(resource_key(&resource), project_key);
        }
    }

    for route in &snapshot.grpc_routes {
        let resource = ResourceRef::CustomResource {
            name: route.name.clone(),
            namespace: Some(route.namespace.clone()),
            group: "gateway.networking.k8s.io".to_string(),
            version: route.version.clone(),
            kind: "GRPCRoute".to_string(),
            plural: "grpcroutes".to_string(),
        };
        if let Some(identity) = identity_from_map(&route.namespace, &route.labels) {
            let project = project_mut(&mut projects, &identity);
            project.add_gateway_route(&resource);
            resource_projects.insert(resource_key(&resource), identity.key);
        } else if let Some(project_key) = route_backend_project(
            &route.namespace,
            route.backend_refs.iter(),
            &service_projects,
        ) {
            if let Some(project) = projects.get_mut(&project_key) {
                project.add_gateway_route(&resource);
            }
            resource_projects.insert(resource_key(&resource), project_key);
        }
    }

    for issue in compute_issues(snapshot).iter() {
        let key = resource_key(&issue.resource_ref);
        if let Some(project_key) = resource_projects.get(&key)
            && let Some(project) = projects.get_mut(project_key)
        {
            project.add_issue(
                issue.severity,
                format!(
                    "{} {}: {}",
                    issue.resource_kind,
                    issue.resource_name,
                    truncate_issue(&issue.message)
                ),
            );
        }
    }

    for finding in compute_vulnerability_findings(snapshot).iter() {
        let Some(resource_ref) = &finding.resource_ref else {
            continue;
        };
        let key = resource_key(resource_ref);
        if let Some(project_key) = resource_projects.get(&key)
            && let Some(project) = projects.get_mut(project_key)
        {
            project.add_issue(
                finding.severity,
                format!(
                    "{} {}: {} total vulnerability findings ({} fixable)",
                    finding.resource_kind,
                    finding.resource_name,
                    finding.counts.total(),
                    finding.fixable_count
                ),
            );
        }
    }

    let mut summaries = projects
        .into_iter()
        .map(|(key, project)| project.finish(key))
        .collect::<Vec<_>>();
    summaries.sort_unstable_by(|left, right| {
        severity_rank(right.highest_severity)
            .cmp(&severity_rank(left.highest_severity))
            .then_with(|| right.issue_count.cmp(&left.issue_count))
            .then_with(|| right.workload_count().cmp(&left.workload_count()))
            .then_with(|| right.services.cmp(&left.services))
            .then_with(|| left.name.cmp(&right.name))
    });
    summaries
}

fn project_mut<'a>(
    projects: &'a mut BTreeMap<String, ProjectAccumulator>,
    identity: &ProjectIdentity,
) -> &'a mut ProjectAccumulator {
    projects
        .entry(identity.key.clone())
        .or_insert_with(|| ProjectAccumulator::new(identity))
}

fn identity_from_pairs(namespace: &str, labels: &[(String, String)]) -> Option<ProjectIdentity> {
    if labels.is_empty() {
        return None;
    }
    let mut part_of = None;
    let mut instance = None;
    let mut name = None;
    let mut app = None;
    let mut k8s_app = None;
    let mut release = None;

    for (key, value) in labels {
        match key.as_str() {
            "app.kubernetes.io/part-of" => part_of = Some(value.as_str()),
            "app.kubernetes.io/instance" => instance = Some(value.as_str()),
            "app.kubernetes.io/name" => name = Some(value.as_str()),
            "app" => app = Some(value.as_str()),
            "k8s-app" => k8s_app = Some(value.as_str()),
            "release" => release = Some(value.as_str()),
            _ => {}
        }
        if part_of.is_some()
            && instance.is_some()
            && name.is_some()
            && app.is_some()
            && k8s_app.is_some()
            && release.is_some()
        {
            break;
        }
    }
    identity_from_values(namespace, part_of, instance, name, app, k8s_app, release)
}

fn identity_from_map(
    namespace: &str,
    labels: &BTreeMap<String, String>,
) -> Option<ProjectIdentity> {
    if labels.is_empty() {
        return None;
    }
    identity_from_values(
        namespace,
        labels.get("app.kubernetes.io/part-of").map(String::as_str),
        labels.get("app.kubernetes.io/instance").map(String::as_str),
        labels.get("app.kubernetes.io/name").map(String::as_str),
        labels.get("app").map(String::as_str),
        labels.get("k8s-app").map(String::as_str),
        labels.get("release").map(String::as_str),
    )
}

fn identity_from_values(
    namespace: &str,
    part_of: Option<&str>,
    instance: Option<&str>,
    name: Option<&str>,
    app: Option<&str>,
    k8s_app: Option<&str>,
    release: Option<&str>,
) -> Option<ProjectIdentity> {
    let (label, value, cluster_scoped) = if let Some(value) = part_of {
        ("app.kubernetes.io/part-of", value, true)
    } else if let Some(value) = instance {
        ("app.kubernetes.io/instance", value, true)
    } else if let Some(value) = name {
        ("app.kubernetes.io/name", value, false)
    } else if let Some(value) = app {
        ("app", value, false)
    } else if let Some(value) = k8s_app {
        ("k8s-app", value, false)
    } else if let Some(value) = release {
        ("release", value, false)
    } else {
        return None;
    };

    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let scope = if cluster_scoped {
        format!("cluster:{value}")
    } else {
        format!("ns:{namespace}:{value}")
    };
    Some(ProjectIdentity {
        key: scope,
        name: value.to_string(),
        source_label: label,
        cluster_scoped,
    })
}

fn single_owner_project(
    owners: &HashMap<(String, String, String), BTreeSet<String>>,
    kind: &str,
    namespace: &str,
    name: &str,
) -> Option<String> {
    owners
        .get(&(kind.to_string(), namespace.to_string(), name.to_string()))
        .filter(|keys| keys.len() == 1)
        .and_then(|keys| keys.iter().next().cloned())
}

fn remember_cronjob_parents(
    mapping: &mut HashMap<(String, String), BTreeSet<String>>,
    job: &JobInfo,
    project_key: &str,
) {
    for owner in &job.owner_references {
        if owner.kind != "CronJob" {
            continue;
        }
        mapping
            .entry((job.namespace.clone(), owner.name.clone()))
            .or_default()
            .insert(project_key.to_string());
    }
}

fn service_project(
    service: &ServiceInfo,
    snapshot: &ClusterSnapshot,
    pod_projects: &HashMap<(String, String), String>,
) -> Option<String> {
    if service.selector.is_empty() {
        return None;
    }

    let selector = LabelSelectorInfo {
        match_labels: service.selector.clone(),
        match_expressions: Vec::new(),
    };
    let matching = snapshot
        .pods
        .iter()
        .filter(|pod| {
            pod.namespace == service.namespace && selector_matches_pairs(&selector, &pod.labels)
        })
        .filter_map(|pod| {
            pod_projects
                .get(&(pod.namespace.clone(), pod.name.clone()))
                .cloned()
        })
        .collect::<BTreeSet<_>>();

    (matching.len() == 1)
        .then(|| matching.iter().next().cloned())
        .flatten()
}

fn ingress_project(
    ingress: &IngressInfo,
    service_projects: &HashMap<(String, String), String>,
) -> Option<String> {
    let matching = ingress
        .backend_services
        .iter()
        .filter_map(|(service, _port)| {
            service_projects
                .get(&(ingress.namespace.clone(), service.clone()))
                .cloned()
        })
        .collect::<BTreeSet<_>>();

    (matching.len() == 1)
        .then(|| matching.iter().next().cloned())
        .flatten()
}

fn route_backend_project<'a>(
    namespace: &str,
    backend_refs: impl Iterator<Item = &'a GatewayBackendRefInfo>,
    service_projects: &HashMap<(String, String), String>,
) -> Option<String> {
    let matching = backend_refs
        .filter_map(|backend| {
            if backend.kind != "Service" {
                return None;
            }
            let service_namespace = backend.namespace.as_deref().unwrap_or(namespace);
            service_projects
                .get(&(service_namespace.to_string(), backend.name.clone()))
                .cloned()
        })
        .collect::<BTreeSet<_>>();

    (matching.len() == 1)
        .then(|| matching.iter().next().cloned())
        .flatten()
}

fn resource_key(resource: &ResourceRef) -> String {
    match resource.namespace() {
        Some(namespace) => format!("{}:{namespace}:{}", resource.kind(), resource.name()),
        None => format!("{}::{}", resource.kind(), resource.name()),
    }
}

const fn severity_rank(severity: AlertSeverity) -> u8 {
    match severity {
        AlertSeverity::Info => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Error => 2,
    }
}

fn truncate_issue(message: &str) -> String {
    if message.len() <= 96 {
        return message.to_string();
    }
    let mut end = 96;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::{
        CronJobInfo, DaemonSetInfo, DeploymentInfo, GatewayBackendRefInfo, GrpcRouteInfo,
        HttpRouteInfo, IngressInfo, JobInfo, OwnerRefInfo, PodInfo, ServiceInfo, StatefulSetInfo,
    };

    #[test]
    fn computes_project_summary_from_labels_and_service_selector() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 1,
            ..ClusterSnapshot::default()
        };
        snapshot.deployments.push(DeploymentInfo {
            name: "api".into(),
            namespace: "payments".into(),
            pod_template_labels: BTreeMap::from([(
                "app.kubernetes.io/part-of".into(),
                "checkout".into(),
            )]),
            ..DeploymentInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "api-123".into(),
            namespace: "payments".into(),
            labels: vec![("app.kubernetes.io/part-of".into(), "checkout".into())],
            owner_references: vec![OwnerRefInfo {
                kind: "Deployment".into(),
                name: "api".into(),
                uid: "1".into(),
            }],
            ..PodInfo::default()
        });
        snapshot.services.push(ServiceInfo {
            name: "api".into(),
            namespace: "payments".into(),
            selector: BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]),
            ..ServiceInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.deployments, 1);
        assert_eq!(project.pods, 1);
        assert_eq!(project.services, 1);
        assert!(project.cluster_scoped);
    }

    #[test]
    fn namespace_scopes_name_label_to_avoid_cross_namespace_collisions() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 2,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "a".into(),
            namespace: "payments".into(),
            labels: vec![("app.kubernetes.io/name".into(), "api".into())],
            ..PodInfo::default()
        });
        snapshot.pods.push(PodInfo {
            name: "b".into(),
            namespace: "ops".into(),
            labels: vec![("app.kubernetes.io/name".into(), "api".into())],
            ..PodInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 2);
        assert!(
            projects
                .iter()
                .any(|project| project.namespaces == vec!["payments"])
        );
        assert!(
            projects
                .iter()
                .any(|project| project.namespaces == vec!["ops"])
        );
    }

    #[test]
    fn project_filter_matches_namespaces_and_related_resources() {
        let project = ProjectSummary {
            key: "cluster:checkout".into(),
            name: "checkout".into(),
            source_label: "app.kubernetes.io/part-of".into(),
            cluster_scoped: true,
            namespaces: vec!["payments".into()],
            namespaces_label: "payments".into(),
            deployments: 1,
            statefulsets: 0,
            daemonsets: 0,
            jobs: 0,
            cronjobs: 0,
            pods: 2,
            services: 1,
            ingresses: 0,
            http_routes: 0,
            grpc_routes: 0,
            issue_count: 0,
            workload_count_label: "1".into(),
            services_label: "1".into(),
            pods_label: "2".into(),
            issue_count_label: "0".into(),
            highest_severity: AlertSeverity::Info,
            representative: None,
            recent_issues: Vec::new(),
            sample_workloads: vec!["Deployment/api".into()],
            sample_services: vec!["api".into()],
            sample_ingresses: Vec::new(),
            sample_routes: Vec::new(),
        };

        assert!(project.matches_query("payments"));
        assert!(project.matches_query("api"));
        assert!(project.matches_query("checkout"));
        assert!(!project.matches_query("inventory"));
    }

    #[test]
    fn project_summary_collects_recent_runtime_issue() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 3,
            ..ClusterSnapshot::default()
        };
        snapshot.pods.push(PodInfo {
            name: "api-123".into(),
            namespace: "payments".into(),
            status: "Failed".into(),
            labels: vec![("app.kubernetes.io/part-of".into(), "checkout".into())],
            ..PodInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert!(project.issue_count >= 1);
        assert_eq!(project.highest_severity, AlertSeverity::Error);
        assert!(
            project
                .recent_issues
                .iter()
                .any(|issue| issue.contains("Pod api-123"))
        );
    }

    #[test]
    fn infers_zero_pod_workloads_from_template_labels() {
        let labels = BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]);
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 4,
            ..ClusterSnapshot::default()
        };
        snapshot.statefulsets.push(StatefulSetInfo {
            name: "db".into(),
            namespace: "payments".into(),
            pod_template_labels: labels.clone(),
            ..StatefulSetInfo::default()
        });
        snapshot.daemonsets.push(DaemonSetInfo {
            name: "agent".into(),
            namespace: "payments".into(),
            pod_template_labels: labels.clone(),
            ..DaemonSetInfo::default()
        });
        snapshot.jobs.push(JobInfo {
            name: "seed".into(),
            namespace: "payments".into(),
            pod_template_labels: labels.clone(),
            ..JobInfo::default()
        });
        snapshot.cronjobs.push(CronJobInfo {
            name: "nightly".into(),
            namespace: "payments".into(),
            pod_template_labels: labels,
            ..CronJobInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.statefulsets, 1);
        assert_eq!(project.daemonsets, 1);
        assert_eq!(project.jobs, 1);
        assert_eq!(project.cronjobs, 1);
        assert_eq!(project.pods, 0);
    }

    #[test]
    fn infers_service_and_ingress_projects_without_live_pods() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 5,
            ..ClusterSnapshot::default()
        };
        snapshot.services.push(ServiceInfo {
            name: "api".into(),
            namespace: "payments".into(),
            selector: BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]),
            ..ServiceInfo::default()
        });
        snapshot.ingresses.push(IngressInfo {
            name: "api".into(),
            namespace: "payments".into(),
            backend_services: vec![("api".into(), "80".into())],
            ..IngressInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.services, 1);
        assert_eq!(project.ingresses, 1);
        assert_eq!(project.pods, 0);
    }

    #[test]
    fn infers_gateway_route_projects_from_backend_service() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 51,
            ..ClusterSnapshot::default()
        };
        snapshot.services.push(ServiceInfo {
            name: "api".into(),
            namespace: "payments".into(),
            selector: BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]),
            ..ServiceInfo::default()
        });
        snapshot.http_routes.push(HttpRouteInfo {
            name: "frontend".into(),
            namespace: "payments".into(),
            version: "v1".into(),
            backend_refs: vec![GatewayBackendRefInfo {
                group: "".into(),
                kind: "Service".into(),
                namespace: None,
                name: "api".into(),
                port: Some(80),
            }],
            ..HttpRouteInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.http_routes, 1);
        assert!(
            project
                .sample_routes
                .iter()
                .any(|entry| entry == "HTTPRoute/frontend")
        );
    }

    #[test]
    fn infers_labeled_grpc_route_projects_without_service_ownership() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 52,
            ..ClusterSnapshot::default()
        };
        snapshot.grpc_routes.push(GrpcRouteInfo {
            name: "grpc-api".into(),
            namespace: "payments".into(),
            version: "v1".into(),
            labels: BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]),
            ..GrpcRouteInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.grpc_routes, 1);
        assert!(
            project
                .sample_routes
                .iter()
                .any(|entry| entry == "GRPCRoute/grpc-api")
        );
    }

    #[test]
    fn ignores_non_service_gateway_backend_refs_for_project_inference() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 53,
            ..ClusterSnapshot::default()
        };
        snapshot.services.push(ServiceInfo {
            name: "api".into(),
            namespace: "payments".into(),
            selector: BTreeMap::from([("app.kubernetes.io/part-of".into(), "checkout".into())]),
            ..ServiceInfo::default()
        });
        snapshot.http_routes.push(HttpRouteInfo {
            name: "frontend".into(),
            namespace: "payments".into(),
            version: "v1".into(),
            backend_refs: vec![GatewayBackendRefInfo {
                group: "example.com".into(),
                kind: "BackendPolicy".into(),
                namespace: None,
                name: "api".into(),
                port: None,
            }],
            ..HttpRouteInfo::default()
        });

        let projects = compute_projects(&snapshot);
        let project = projects
            .iter()
            .find(|project| project.name == "checkout")
            .expect("service-backed project");
        assert_eq!(project.http_routes, 0);
    }

    #[test]
    fn infers_selectorless_services_and_labeled_ingresses() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 6,
            ..ClusterSnapshot::default()
        };
        snapshot.services.push(ServiceInfo {
            name: "api-external".into(),
            namespace: "payments".into(),
            labels: BTreeMap::from([("app.kubernetes.io/name".into(), "checkout".into())]),
            type_: "ExternalName".into(),
            external_name: Some("api.example.com".into()),
            ..ServiceInfo::default()
        });
        snapshot.ingresses.push(IngressInfo {
            name: "api-edge".into(),
            namespace: "payments".into(),
            labels: BTreeMap::from([("app.kubernetes.io/name".into(), "checkout".into())]),
            ..IngressInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.services, 1);
        assert_eq!(project.ingresses, 1);
        assert_eq!(project.pods, 0);
    }

    #[test]
    fn infers_project_from_labeled_ingress_without_other_resources() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 7,
            ..ClusterSnapshot::default()
        };
        snapshot.ingresses.push(IngressInfo {
            name: "api-edge".into(),
            namespace: "payments".into(),
            labels: BTreeMap::from([("app.kubernetes.io/name".into(), "checkout".into())]),
            ..IngressInfo::default()
        });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.name, "checkout");
        assert_eq!(project.ingresses, 1);
        assert_eq!(
            project.representative,
            Some(ResourceRef::Ingress("api-edge".into(), "payments".into()))
        );
    }

    #[test]
    fn project_summary_includes_vulnerability_findings() {
        let mut snapshot = ClusterSnapshot {
            snapshot_version: 8,
            ..ClusterSnapshot::default()
        };
        snapshot.deployments.push(DeploymentInfo {
            name: "api".into(),
            namespace: "payments".into(),
            pod_template_labels: BTreeMap::from([(
                "app.kubernetes.io/part-of".into(),
                "checkout".into(),
            )]),
            ..DeploymentInfo::default()
        });
        snapshot
            .vulnerability_reports
            .push(crate::k8s::dtos::VulnerabilityReportInfo {
                namespace: "payments".into(),
                resource_kind: "Deployment".into(),
                resource_name: "api".into(),
                resource_namespace: "payments".into(),
                counts: crate::k8s::dtos::VulnerabilitySummaryCounts {
                    critical: 1,
                    high: 0,
                    medium: 0,
                    low: 0,
                    unknown: 0,
                },
                fixable_count: 1,
                ..crate::k8s::dtos::VulnerabilityReportInfo::default()
            });

        let projects = compute_projects(&snapshot);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.highest_severity, AlertSeverity::Error);
        assert!(project.issue_count >= 1);
        assert!(
            project
                .recent_issues
                .iter()
                .any(|issue| issue.contains("vulnerability findings"))
        );
    }
}
