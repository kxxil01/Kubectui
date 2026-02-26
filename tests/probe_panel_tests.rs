#![allow(clippy::field_reassign_with_default)]
//! Integration tests for the Probe Panel component.

use kubectui::k8s::probes::{ContainerProbes, ProbeConfig, ProbeHandler, ProbeType};
use kubectui::ui::components::probe_panel::ProbePanelState;

/// Test container navigation with up/down keys.
#[test]
fn test_container_navigation_down() {
    let probes = vec![
        ("nginx".to_string(), ContainerProbes::default()),
        ("sidecar".to_string(), ContainerProbes::default()),
        ("init".to_string(), ContainerProbes::default()),
    ];

    let mut state = ProbePanelState::new("test-pod".to_string(), "default".to_string(), probes);

    // Initial selection
    assert_eq!(state.selected_index, 0);

    // Navigate down
    state.select_next();
    assert_eq!(state.selected_index, 1);

    state.select_next();
    assert_eq!(state.selected_index, 2);

    // Wrap around
    state.select_next();
    assert_eq!(state.selected_index, 0);
}

/// Test container navigation with up keys.
#[test]
fn test_container_navigation_up() {
    let probes = vec![
        ("nginx".to_string(), ContainerProbes::default()),
        ("sidecar".to_string(), ContainerProbes::default()),
        ("init".to_string(), ContainerProbes::default()),
    ];

    let mut state = ProbePanelState::new("test-pod".to_string(), "default".to_string(), probes);

    // Navigate up from 0
    state.select_prev();
    assert_eq!(state.selected_index, 2);

    // Navigate up further
    state.select_prev();
    assert_eq!(state.selected_index, 1);

    state.select_prev();
    assert_eq!(state.selected_index, 0);

    // Wrap around
    state.select_prev();
    assert_eq!(state.selected_index, 2);
}

/// Test expanding and collapsing containers.
#[test]
fn test_expand_collapse_toggle() {
    let probes = vec![
        ("app".to_string(), ContainerProbes::default()),
        ("sidecar".to_string(), ContainerProbes::default()),
    ];

    let mut state = ProbePanelState::new("test-pod".to_string(), "default".to_string(), probes);

    // Initially not expanded
    assert!(!state.expanded_containers.contains("app"));

    // Toggle expand
    state.toggle_expand();
    assert!(state.expanded_containers.contains("app"));

    // Toggle collapse
    state.toggle_expand();
    assert!(!state.expanded_containers.contains("app"));

    // Navigate and expand different container
    state.select_next();
    assert_eq!(state.selected_index, 1);

    state.toggle_expand();
    assert!(state.expanded_containers.contains("sidecar"));
    assert!(!state.expanded_containers.contains("app"));
}

/// Test with no probes (graceful display).
#[test]
fn test_no_probes_graceful_display() {
    let state = ProbePanelState::new("no-probe-pod".to_string(), "default".to_string(), vec![]);

    assert_eq!(state.healthy_count(), 0);
    assert_eq!(state.selected_index, 0);
    assert!(state.container_probes.is_empty());
    assert!(state.expanded_containers.is_empty());

    // Verify navigation doesn't break
    state.clone().select_next();
    state.clone().select_prev();
    state.clone().toggle_expand();
}

/// Test with multi-container pods (3+ containers).
#[test]
fn test_multi_container_probes() {
    let liveness_probe = ProbeConfig {
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

    let readiness_probe = ProbeConfig {
        probe_type: ProbeType::Readiness,
        handler: ProbeHandler::Tcp { port: 9000 },
        initial_delay_seconds: 2,
        period_seconds: 5,
        timeout_seconds: 1,
        success_threshold: 1,
        failure_threshold: 3,
    };

    let mut probes1 = ContainerProbes::default();
    probes1.liveness = Some(liveness_probe.clone());
    probes1.readiness = Some(readiness_probe.clone());

    let mut probes2 = ContainerProbes::default();
    probes2.liveness = Some(liveness_probe);

    let mut probes3 = ContainerProbes::default();
    probes3.readiness = Some(readiness_probe);

    let container_list = vec![
        ("web".to_string(), probes1),
        ("cache".to_string(), probes2),
        ("monitor".to_string(), probes3),
    ];

    let mut state = ProbePanelState::new(
        "complex-pod".to_string(),
        "production".to_string(),
        container_list,
    );

    // Verify counts
    assert_eq!(state.healthy_count(), 3);
    assert_eq!(state.selected_index, 0);

    // Navigate through containers
    state.select_next();
    assert_eq!(state.selected_index, 1);

    state.select_next();
    assert_eq!(state.selected_index, 2);

    // Expand final container
    state.toggle_expand();
    assert!(state.expanded_containers.contains("monitor"));

    // Verify state consistency
    assert_eq!(state.pod_name, "complex-pod");
    assert_eq!(state.namespace, "production");
}
