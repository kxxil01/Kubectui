//! Background task for polling pod probes at regular intervals.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::UpdateMessage;
use crate::k8s::client::K8sClient;
use crate::k8s::probes::extract_probes_from_pod;

const BASE_INTERVAL_SECS: u64 = 2;
const MAX_INTERVAL_SECS: u64 = 30;
const NO_CHANGE_THRESHOLD: u32 = 3;

/// Poll probes for a pod with adaptive backoff.
///
/// Starts at 2s intervals. After 3 consecutive no-change polls, doubles the
/// interval up to 30s. Any change or error resets to 2s.
pub async fn poll_probes_loop(
    client: Arc<K8sClient>,
    pod_name: String,
    namespace: String,
    update_tx: mpsc::Sender<UpdateMessage>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut last_probes: Option<Vec<(String, crate::k8s::probes::ContainerProbes)>> = None;
    let mut interval_secs = BASE_INTERVAL_SECS;
    let mut no_change_count: u32 = 0;

    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                match fetch_and_compare_probes(&client, &pod_name, &namespace, &last_probes).await {
                    Ok((probes, changed)) => {
                        if changed {
                            let msg = UpdateMessage::ProbeUpdate {
                                pod_name: pod_name.clone(),
                                namespace: namespace.clone(),
                                probes: probes.clone(),
                            };
                            if update_tx.send(msg).await.is_err() {
                                break;
                            }
                            last_probes = Some(probes);
                            interval_secs = BASE_INTERVAL_SECS;
                            no_change_count = 0;
                        } else {
                            no_change_count += 1;
                            if no_change_count >= NO_CHANGE_THRESHOLD {
                                interval_secs = (interval_secs * 2).min(MAX_INTERVAL_SECS);
                            }
                        }
                    }
                    Err(e) => {
                        let msg = UpdateMessage::ProbeError {
                            pod_name: pod_name.clone(),
                            namespace: namespace.clone(),
                            error: e.to_string(),
                        };
                        if update_tx.send(msg).await.is_err() {
                            break;
                        }
                        interval_secs = BASE_INTERVAL_SECS;
                        no_change_count = 0;
                    }
                }
            }
            _ = &mut cancel_rx => {
                break;
            }
        }
    }
}

/// Fetch probes and compare with previous state.
///
/// Returns Ok((probes, changed)) where changed indicates if probes differ from last_probes.
async fn fetch_and_compare_probes(
    client: &Arc<K8sClient>,
    pod_name: &str,
    namespace: &str,
    last_probes: &Option<Vec<(String, crate::k8s::probes::ContainerProbes)>>,
) -> anyhow::Result<(Vec<(String, crate::k8s::probes::ContainerProbes)>, bool)> {
    use k8s_openapi::api::core::v1::Pod;
    use kube::Api;

    let pods_api: Api<Pod> = Api::namespaced(client.get_client(), namespace);
    let pod = pods_api.get(pod_name).await?;

    let probes = extract_probes_from_pod(&pod)?;

    // Determine if probes have changed
    let changed = match last_probes {
        None => true,
        Some(last) => {
            // Simple comparison: check if probes differ
            probes.len() != last.len()
                || !probes.iter().all(|(name, probe)| {
                    last.iter()
                        .any(|(n, p)| n == name && probes_equal(p, probe))
                })
        }
    };

    Ok((probes, changed))
}

/// Compare two ContainerProbes for equality.
fn probes_equal(
    a: &crate::k8s::probes::ContainerProbes,
    b: &crate::k8s::probes::ContainerProbes,
) -> bool {
    match (&a.liveness, &b.liveness) {
        (None, None) => {}
        (Some(l1), Some(l2)) => {
            if !probe_config_equal(l1, l2) {
                return false;
            }
        }
        _ => return false,
    }

    match (&a.readiness, &b.readiness) {
        (None, None) => {}
        (Some(r1), Some(r2)) => {
            if !probe_config_equal(r1, r2) {
                return false;
            }
        }
        _ => return false,
    }

    match (&a.startup, &b.startup) {
        (None, None) => {}
        (Some(s1), Some(s2)) => {
            if !probe_config_equal(s1, s2) {
                return false;
            }
        }
        _ => return false,
    }

    true
}

/// Compare two ProbeConfigs for equality.
fn probe_config_equal(
    a: &crate::k8s::probes::ProbeConfig,
    b: &crate::k8s::probes::ProbeConfig,
) -> bool {
    use crate::k8s::probes::ProbeHandler;

    if a.probe_type != b.probe_type {
        return false;
    }

    if a.initial_delay_seconds != b.initial_delay_seconds
        || a.period_seconds != b.period_seconds
        || a.timeout_seconds != b.timeout_seconds
        || a.success_threshold != b.success_threshold
        || a.failure_threshold != b.failure_threshold
    {
        return false;
    }

    match (&a.handler, &b.handler) {
        (
            ProbeHandler::Http {
                path: p1,
                port: po1,
                scheme: s1,
            },
            ProbeHandler::Http {
                path: p2,
                port: po2,
                scheme: s2,
            },
        ) => p1 == p2 && po1 == po2 && s1 == s2,
        (ProbeHandler::Exec { command: c1 }, ProbeHandler::Exec { command: c2 }) => c1 == c2,
        (ProbeHandler::Tcp { port: p1 }, ProbeHandler::Tcp { port: p2 }) => p1 == p2,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::probes::{ContainerProbes, ProbeConfig, ProbeHandler, ProbeType};

    #[test]
    fn test_probes_equal_identical() {
        let probe = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Http {
                path: "/health".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut cp1 = ContainerProbes::default();
        cp1.liveness = Some(probe.clone());

        let mut cp2 = ContainerProbes::default();
        cp2.liveness = Some(probe);

        assert!(probes_equal(&cp1, &cp2));
    }

    #[test]
    fn test_probes_equal_different_handler() {
        let probe1 = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Http {
                path: "/health".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let probe2 = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut cp1 = ContainerProbes::default();
        cp1.liveness = Some(probe1);

        let mut cp2 = ContainerProbes::default();
        cp2.liveness = Some(probe2);

        assert!(!probes_equal(&cp1, &cp2));
    }

    #[test]
    fn test_probes_equal_different_timing() {
        let probe1 = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut probe2 = probe1.clone();
        probe2.initial_delay_seconds = 10;

        let mut cp1 = ContainerProbes::default();
        cp1.liveness = Some(probe1);

        let mut cp2 = ContainerProbes::default();
        cp2.liveness = Some(probe2);

        assert!(!probes_equal(&cp1, &cp2));
    }

    #[test]
    fn test_probes_equal_both_empty() {
        let cp1 = ContainerProbes::default();
        let cp2 = ContainerProbes::default();

        assert!(probes_equal(&cp1, &cp2));
    }

    #[test]
    fn test_probes_equal_one_empty() {
        let probe = ProbeConfig {
            probe_type: ProbeType::Liveness,
            handler: ProbeHandler::Tcp { port: 8080 },
            initial_delay_seconds: 0,
            period_seconds: 10,
            timeout_seconds: 1,
            success_threshold: 1,
            failure_threshold: 3,
        };

        let mut cp1 = ContainerProbes::default();
        cp1.liveness = Some(probe);

        let cp2 = ContainerProbes::default();

        assert!(!probes_equal(&cp1, &cp2));
    }
}
