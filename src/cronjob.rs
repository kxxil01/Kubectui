//! Canonical CronJob scheduling and execution-history helpers.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use chrono::{DateTime, Utc};

use crate::k8s::dtos::{CronJobInfo, JobInfo, PodInfo};

pub const CRONJOB_HISTORY_LIMIT: usize = 20;
pub const CRONJOB_NEXT_RUN_TIMEZONE_FALLBACK: &str = "UTC";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobHistoryEntry {
    pub job_name: String,
    pub namespace: String,
    pub status: String,
    pub completions: String,
    pub duration: Option<String>,
    pub pod_count: i32,
    pub live_pod_count: i32,
    pub completion_pct: Option<u8>,
    pub active_pods: i32,
    pub failed_pods: i32,
    pub age: Option<Duration>,
    pub created_at: Option<DateTime<Utc>>,
    pub logs_authorized: Option<bool>,
}

impl CronJobHistoryEntry {
    pub fn has_log_target(&self) -> bool {
        self.live_pod_count > 0 && self.logs_authorized.unwrap_or(true)
    }
}

pub fn compute_next_schedule_time(
    schedule: &str,
    timezone: Option<&str>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if schedule.split_whitespace().count() != 5 {
        return None;
    }

    let timezone = timezone
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(CRONJOB_NEXT_RUN_TIMEZONE_FALLBACK);
    let crontab = cronexpr::parse_crontab(&format!("{schedule} {timezone}")).ok()?;
    let now_rfc3339 = now.to_rfc3339();
    let next = crontab.find_next(now_rfc3339.as_str()).ok()?;

    parse_zoned_timestamp(&next.to_string())
}

pub fn cronjob_next_schedule_time(
    schedule: &str,
    timezone: Option<&str>,
    suspend: bool,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if suspend {
        None
    } else {
        compute_next_schedule_time(schedule, timezone, now)
    }
}

pub fn cronjob_history_entries(
    cronjob: &CronJobInfo,
    jobs: &[JobInfo],
    pods: &[PodInfo],
) -> Vec<CronJobHistoryEntry> {
    let relevant_jobs = jobs
        .iter()
        .filter(|job| {
            job.namespace == cronjob.namespace
                && job
                    .owner_references
                    .iter()
                    .any(|owner| owner.kind == "CronJob" && owner.name == cronjob.name)
        })
        .collect::<Vec<_>>();

    if relevant_jobs.is_empty() {
        return Vec::new();
    }

    let relevant_job_names = relevant_jobs
        .iter()
        .map(|job| job.name.as_str())
        .collect::<HashSet<_>>();
    let mut live_pod_counts = HashMap::<String, i32>::new();
    for pod in pods.iter().filter(|pod| pod.namespace == cronjob.namespace) {
        for owner in &pod.owner_references {
            if owner.kind == "Job" && relevant_job_names.contains(owner.name.as_str()) {
                *live_pod_counts.entry(owner.name.clone()).or_default() += 1;
                break;
            }
        }
    }

    let mut entries = relevant_jobs
        .into_iter()
        .map(|job| CronJobHistoryEntry {
            job_name: job.name.clone(),
            namespace: job.namespace.clone(),
            status: job.status.clone(),
            completions: job.completions.clone(),
            duration: job.duration.clone(),
            pod_count: (job.succeeded_pods + job.failed_pods + job.active_pods).max(0),
            live_pod_count: *live_pod_counts.get(&job.name).unwrap_or(&0),
            completion_pct: job_completion_percentage(job),
            active_pods: job.active_pods,
            failed_pods: job.failed_pods,
            age: job.age,
            created_at: job.created_at,
            logs_authorized: None,
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.job_name.cmp(&right.job_name))
    });
    entries.truncate(CRONJOB_HISTORY_LIMIT);
    entries
}

pub fn preferred_history_index(entries: &[CronJobHistoryEntry]) -> usize {
    entries
        .iter()
        .position(|entry| entry.status.eq_ignore_ascii_case("failed") || entry.active_pods > 0)
        .unwrap_or(0)
}

fn job_completion_percentage(job: &JobInfo) -> Option<u8> {
    (job.desired_completions > 0).then_some(())?;

    let succeeded = job.succeeded_pods.max(0);
    let desired = job.desired_completions.max(1);
    let pct = ((succeeded * 100) / desired).clamp(0, 100);
    Some(pct as u8)
}

fn parse_zoned_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let timestamp = value.split_once('[').map_or(value, |(ts, _)| ts);
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::dtos::OwnerRefInfo;

    fn owner_ref() -> OwnerRefInfo {
        OwnerRefInfo {
            kind: "CronJob".to_string(),
            name: "nightly".to_string(),
            uid: "uid-1".to_string(),
        }
    }

    #[test]
    fn computes_next_schedule_time_with_explicit_timezone() {
        let now = DateTime::parse_from_rfc3339("2026-03-12T10:30:00+00:00")
            .expect("timestamp")
            .with_timezone(&Utc);

        let next = compute_next_schedule_time("0 9 * * *", Some("Asia/Jakarta"), now)
            .expect("next schedule");

        assert_eq!(next.to_rfc3339(), "2026-03-13T02:00:00+00:00");
    }

    #[test]
    fn computes_next_schedule_time_with_utc_fallback() {
        let now = DateTime::parse_from_rfc3339("2026-03-12T10:30:00+00:00")
            .expect("timestamp")
            .with_timezone(&Utc);

        let next = compute_next_schedule_time("*/15 * * * *", None, now).expect("next schedule");

        assert_eq!(next.to_rfc3339(), "2026-03-12T10:45:00+00:00");
    }

    #[test]
    fn suspended_cronjobs_do_not_expose_next_schedule_time() {
        let now = DateTime::parse_from_rfc3339("2026-03-12T10:30:00+00:00")
            .expect("timestamp")
            .with_timezone(&Utc);

        assert_eq!(
            cronjob_next_schedule_time("*/15 * * * *", Some("UTC"), true, now),
            None
        );
    }

    #[test]
    fn rejects_non_standard_field_counts() {
        let now = Utc::now();
        assert!(compute_next_schedule_time("0 9 * * * *", Some("UTC"), now).is_none());
    }

    #[test]
    fn builds_recent_cronjob_history_from_snapshot_jobs() {
        let cronjob = CronJobInfo {
            name: "nightly".to_string(),
            namespace: "ops".to_string(),
            ..CronJobInfo::default()
        };
        let older = DateTime::parse_from_rfc3339("2026-03-11T10:00:00+00:00")
            .expect("older")
            .with_timezone(&Utc);
        let newer = DateTime::parse_from_rfc3339("2026-03-12T10:00:00+00:00")
            .expect("newer")
            .with_timezone(&Utc);

        let entries = cronjob_history_entries(
            &cronjob,
            &[
                JobInfo {
                    name: "nightly-002".to_string(),
                    namespace: "ops".to_string(),
                    status: "Failed".to_string(),
                    completions: "0/1".to_string(),
                    failed_pods: 1,
                    desired_completions: 1,
                    created_at: Some(newer),
                    owner_references: vec![owner_ref()],
                    ..JobInfo::default()
                },
                JobInfo {
                    name: "nightly-001".to_string(),
                    namespace: "ops".to_string(),
                    status: "Succeeded".to_string(),
                    completions: "1/1".to_string(),
                    succeeded_pods: 1,
                    desired_completions: 1,
                    created_at: Some(older),
                    owner_references: vec![owner_ref()],
                    ..JobInfo::default()
                },
                JobInfo {
                    name: "other".to_string(),
                    namespace: "ops".to_string(),
                    owner_references: vec![OwnerRefInfo {
                        name: "other".to_string(),
                        ..owner_ref()
                    }],
                    ..JobInfo::default()
                },
            ],
            &[PodInfo {
                name: "nightly-002-pod".to_string(),
                namespace: "ops".to_string(),
                owner_references: vec![OwnerRefInfo {
                    kind: "Job".to_string(),
                    name: "nightly-002".to_string(),
                    uid: "job-uid-1".to_string(),
                }],
                ..PodInfo::default()
            }],
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].job_name, "nightly-002");
        assert_eq!(entries[0].pod_count, 1);
        assert_eq!(entries[0].live_pod_count, 1);
        assert_eq!(entries[0].completion_pct, Some(0));
        assert_eq!(entries[1].completion_pct, Some(100));
        assert_eq!(entries[1].live_pod_count, 0);
    }

    #[test]
    fn history_logs_require_live_job_owned_pods() {
        let cronjob = CronJobInfo {
            name: "nightly".to_string(),
            namespace: "ops".to_string(),
            ..CronJobInfo::default()
        };

        let entries = cronjob_history_entries(
            &cronjob,
            &[JobInfo {
                name: "nightly-001".to_string(),
                namespace: "ops".to_string(),
                status: "Failed".to_string(),
                completions: "0/1".to_string(),
                failed_pods: 1,
                desired_completions: 1,
                owner_references: vec![owner_ref()],
                ..JobInfo::default()
            }],
            &[],
        );

        assert_eq!(entries[0].pod_count, 1);
        assert_eq!(entries[0].live_pod_count, 0);
        assert!(!entries[0].has_log_target());
    }

    #[test]
    fn preferred_history_index_prioritizes_failed_entries() {
        let entries = vec![
            CronJobHistoryEntry {
                job_name: "nightly-001".to_string(),
                namespace: "ops".to_string(),
                status: "Succeeded".to_string(),
                completions: "1/1".to_string(),
                duration: Some("12s".to_string()),
                pod_count: 1,
                live_pod_count: 0,
                completion_pct: Some(100),
                active_pods: 0,
                failed_pods: 0,
                age: None,
                created_at: None,
                logs_authorized: None,
            },
            CronJobHistoryEntry {
                job_name: "nightly-002".to_string(),
                namespace: "ops".to_string(),
                status: "Failed".to_string(),
                completions: "0/1".to_string(),
                duration: Some("3s".to_string()),
                pod_count: 1,
                live_pod_count: 0,
                completion_pct: Some(0),
                active_pods: 0,
                failed_pods: 1,
                age: None,
                created_at: None,
                logs_authorized: None,
            },
        ];

        assert_eq!(preferred_history_index(&entries), 1);
    }

    #[test]
    fn log_target_requires_live_pods_and_authorized_access() {
        let mut entry = CronJobHistoryEntry {
            job_name: "nightly-001".to_string(),
            namespace: "ops".to_string(),
            status: "Running".to_string(),
            completions: "0/1".to_string(),
            duration: None,
            pod_count: 1,
            live_pod_count: 1,
            completion_pct: Some(0),
            active_pods: 1,
            failed_pods: 0,
            age: None,
            created_at: None,
            logs_authorized: None,
        };

        assert!(entry.has_log_target());

        entry.logs_authorized = Some(false);
        assert!(!entry.has_log_target());
    }
}
