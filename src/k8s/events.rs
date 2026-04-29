//! Pod events data transfer object and fetching helpers.

use crate::time::{AppTimestamp, now};
use anyhow::{Context, Result};
use k8s_openapi::api::core::v1::Event;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
};

use crate::k8s::conversions::app_timestamp_from_k8s_timestamp;

/// A simplified Kubernetes Event record used by the detail modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventInfo {
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub first_timestamp: AppTimestamp,
    pub last_timestamp: AppTimestamp,
    pub count: i32,
}

/// Fetches events for any resource kind in `namespace` by `involvedObject.kind` and `involvedObject.name`.
///
/// Degrades gracefully on RBAC-forbidden access, returning a synthetic info row.
pub async fn fetch_resource_events(
    client: &Client,
    kind: &str,
    name: &str,
    namespace: &str,
) -> Result<Vec<EventInfo>> {
    let events_api: Api<Event> = Api::namespaced(client.clone(), namespace);
    let selector = format!("involvedObject.kind={kind},involvedObject.name={name}");
    let params = ListParams::default().fields(&selector);

    let list = match events_api.list(&params).await {
        Ok(items) => items,
        Err(err) if is_forbidden_error(&err) => {
            return Ok(vec![EventInfo {
                event_type: "Info".to_string(),
                reason: "RBAC".to_string(),
                message: "Events unavailable (RBAC)".to_string(),
                first_timestamp: now(),
                last_timestamp: now(),
                count: 1,
            }]);
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed fetching events for {kind} '{name}' in namespace '{namespace}'")
            });
        }
    };

    Ok(map_events(list))
}

/// Fetches Pod-scoped events in `namespace` for Pod `name`.
///
/// This helper never fails for RBAC-forbidden access. Instead, it returns a
/// synthetic informational row so the UI can show a graceful message.
pub async fn fetch_pod_events(
    client: &Client,
    name: &str,
    namespace: &str,
) -> Result<Vec<EventInfo>> {
    let events_api: Api<Event> = Api::namespaced(client.clone(), namespace);
    let selector = format!("involvedObject.kind=Pod,involvedObject.name={name}");
    let params = ListParams::default().fields(&selector);

    let list = match events_api.list(&params).await {
        Ok(items) => items,
        Err(err) if is_forbidden_error(&err) => {
            return Ok(vec![EventInfo {
                event_type: "Info".to_string(),
                reason: "RBAC".to_string(),
                message: "Events unavailable (RBAC)".to_string(),
                first_timestamp: now(),
                last_timestamp: now(),
                count: 1,
            }]);
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed fetching events for pod '{name}' in namespace '{namespace}'")
            });
        }
    };

    Ok(map_events(list))
}

fn map_events(list: ObjectList<Event>) -> Vec<EventInfo> {
    let mut mapped: Vec<EventInfo> = list
        .into_iter()
        .map(|event| {
            let fallback_ts = event
                .metadata
                .creation_timestamp
                .as_ref()
                .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0))
                .unwrap_or_else(now);

            let first_timestamp = event
                .first_timestamp
                .as_ref()
                .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0))
                .unwrap_or(fallback_ts);

            let last_timestamp = event
                .last_timestamp
                .as_ref()
                .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0))
                .or_else(|| {
                    event
                        .event_time
                        .as_ref()
                        .and_then(|ts| app_timestamp_from_k8s_timestamp(&ts.0))
                })
                .unwrap_or(first_timestamp);

            EventInfo {
                event_type: event.type_.unwrap_or_else(|| "Normal".to_string()),
                reason: event.reason.unwrap_or_else(|| "Unknown".to_string()),
                message: event.message.unwrap_or_else(|| "No message".to_string()),
                first_timestamp,
                last_timestamp,
                count: event.count.unwrap_or(1),
            }
        })
        .collect();

    mapped.sort_unstable_by(|a, b| {
        a.event_type
            .cmp(&b.event_type)
            .then_with(|| a.reason.cmp(&b.reason))
            .then_with(|| a.message.cmp(&b.message))
    });
    mapped.dedup_by(|b, a| {
        if a.event_type == b.event_type && a.reason == b.reason && a.message == b.message {
            a.count = a.count.saturating_add(b.count);
            if b.last_timestamp > a.last_timestamp {
                a.last_timestamp = b.last_timestamp;
            }
            if b.first_timestamp < a.first_timestamp {
                a.first_timestamp = b.first_timestamp;
            }
            true
        } else {
            false
        }
    });
    mapped.sort_unstable_by(|a, b| {
        a.last_timestamp
            .cmp(&b.last_timestamp)
            .then_with(|| a.event_type.cmp(&b.event_type))
            .then_with(|| {
                a.reason
                    .cmp(&b.reason)
                    .then_with(|| a.message.cmp(&b.message))
                    .then_with(|| a.first_timestamp.cmp(&b.first_timestamp))
                    .then_with(|| a.count.cmp(&b.count))
            })
    });
    mapped
}

fn is_forbidden_error(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.is_forbidden())
}

#[cfg(test)]
mod tests {
    use jiff::ToSpan;
    use k8s_openapi::{
        api::core::v1::{Event, ObjectReference},
        apimachinery::pkg::apis::meta::v1::{ListMeta, MicroTime, Time},
    };
    use kube::{api::ObjectList, core::Status};

    use super::*;

    fn event(reason: &str, msg: &str, last_offset_sec: i64) -> Event {
        let now = now();
        let mut e = Event::default();
        e.reason = Some(reason.to_string());
        e.message = Some(msg.to_string());
        e.type_ = Some("Warning".to_string());
        let last_timestamp = now
            .checked_add(last_offset_sec.seconds())
            .expect("timestamp in range");
        e.event_time = Some(MicroTime(to_k8s_timestamp(last_timestamp)));
        e.first_timestamp = Some(Time(to_k8s_timestamp(
            now.checked_sub(1.minute()).expect("timestamp in range"),
        )));
        e.last_timestamp = Some(Time(to_k8s_timestamp(last_timestamp)));
        e.involved_object = ObjectReference {
            kind: Some("Pod".to_string()),
            name: Some("pod-a".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        };
        e.count = Some(2);
        e
    }

    fn event_with_type(event_type: &str, reason: &str, msg: &str, last_offset_sec: i64) -> Event {
        let mut e = event(reason, msg, last_offset_sec);
        e.type_ = Some(event_type.to_string());
        e
    }

    fn api_error(code: u16, reason: &str, message: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    fn to_k8s_timestamp(value: AppTimestamp) -> k8s_openapi::jiff::Timestamp {
        value
    }

    /// Verifies empty event list maps to empty view model list.
    #[test]
    fn map_events_empty_list() {
        let list: ObjectList<Event> = ObjectList {
            metadata: ListMeta::default(),
            items: vec![],
            types: Default::default(),
        };

        let mapped = map_events(list);
        assert!(mapped.is_empty());
    }

    /// Verifies event mapping preserves reason/message and sorts by last timestamp.
    #[test]
    fn map_events_sorts_by_last_timestamp() {
        let newer = event("Newer", "second", 10);
        let older = event("Older", "first", 1);

        let list: ObjectList<Event> = ObjectList {
            metadata: ListMeta::default(),
            items: vec![newer, older],
            types: Default::default(),
        };

        let mapped = map_events(list);

        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].reason, "Older");
        assert_eq!(mapped[1].reason, "Newer");
    }

    #[test]
    fn map_events_keeps_distinct_event_types_for_same_reason_message() {
        let normal = event_with_type("Normal", "Pulled", "image ready", 1);
        let warning = event_with_type("Warning", "Pulled", "image ready", 1);

        let list: ObjectList<Event> = ObjectList {
            metadata: ListMeta::default(),
            items: vec![warning, normal],
            types: Default::default(),
        };

        let mapped = map_events(list);

        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].event_type, "Normal");
        assert_eq!(mapped[1].event_type, "Warning");
        assert_eq!(mapped[0].count, 2);
        assert_eq!(mapped[1].count, 2);
    }

    /// Verifies forbidden error detection returns true only for 403 API responses.
    #[test]
    fn is_forbidden_error_only_403() {
        let forbidden = api_error(403, "Forbidden", "forbidden");
        let timeout = api_error(504, "Timeout", "timeout");

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&timeout));
    }
}
