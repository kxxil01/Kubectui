//! Pod events data transfer object and fetching helpers.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Event;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
    error::ErrorResponse,
};

/// A simplified Kubernetes Event record used by the detail modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventInfo {
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub first_timestamp: DateTime<Utc>,
    pub last_timestamp: DateTime<Utc>,
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
                first_timestamp: Utc::now(),
                last_timestamp: Utc::now(),
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
                first_timestamp: Utc::now(),
                last_timestamp: Utc::now(),
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
                .map(|ts| ts.0)
                .unwrap_or_else(Utc::now);

            let first_timestamp = event
                .first_timestamp
                .as_ref()
                .map(|ts| ts.0)
                .unwrap_or(fallback_ts);

            let last_timestamp = event
                .last_timestamp
                .as_ref()
                .map(|ts| ts.0)
                .or_else(|| event.event_time.as_ref().map(|mt| mt.0))
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

    mapped.sort_by(|a, b| a.reason.cmp(&b.reason).then_with(|| a.message.cmp(&b.message)));
    mapped.dedup_by(|b, a| {
        if a.reason == b.reason && a.message == b.message {
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
    mapped.sort_by_key(|evt| evt.last_timestamp);
    mapped
}

fn is_forbidden_error(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(ErrorResponse { code, .. }) => *code == 403,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use k8s_openapi::{
        api::core::v1::{Event, ObjectReference},
        apimachinery::pkg::apis::meta::v1::{ListMeta, MicroTime, Time},
    };
    use kube::{api::ObjectList, error::ErrorResponse};

    use super::*;

    fn event(reason: &str, msg: &str, last_offset_sec: i64) -> Event {
        let now = Utc::now();
        let mut e = Event::default();
        e.reason = Some(reason.to_string());
        e.message = Some(msg.to_string());
        e.type_ = Some("Warning".to_string());
        e.event_time = Some(MicroTime(now + Duration::seconds(last_offset_sec)));
        e.first_timestamp = Some(Time(now - Duration::minutes(1)));
        e.last_timestamp = Some(Time(now + Duration::seconds(last_offset_sec)));
        e.involved_object = ObjectReference {
            kind: Some("Pod".to_string()),
            name: Some("pod-a".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        };
        e.count = Some(2);
        e
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

    /// Verifies forbidden error detection returns true only for 403 API responses.
    #[test]
    fn is_forbidden_error_only_403() {
        let forbidden = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "forbidden".to_string(),
            reason: "Forbidden".to_string(),
            code: 403,
        });
        let timeout = kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: "timeout".to_string(),
            reason: "Timeout".to_string(),
            code: 504,
        });

        assert!(is_forbidden_error(&forbidden));
        assert!(!is_forbidden_error(&timeout));
    }
}
