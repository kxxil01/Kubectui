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

    mapped.sort_by_key(|evt| evt.last_timestamp);
    mapped
}

fn is_forbidden_error(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(ErrorResponse { code, .. }) => *code == 403,
        _ => false,
    }
}
