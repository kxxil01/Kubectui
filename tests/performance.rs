//! Optional performance checks for core pure functions.

mod common;

use std::time::Instant;

use common::{make_node, make_pod, make_service};
use kubectui::{
    app::AppState,
    state::{
        ClusterSnapshot,
        alerts::compute_alerts,
        filters::{NodeRoleFilter, NodeStatusFilter, filter_nodes, filter_services},
    },
};

/// Verifies filtering 10k nodes stays under 100ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_filter_10k_nodes_under_100ms() {
    let nodes = (0..10_000)
        .map(|i| make_node(&format!("worker-{i}"), i % 2 == 0, "worker"))
        .collect::<Vec<_>>();

    let start = Instant::now();
    let _ = filter_nodes(
        &nodes,
        "worker-99",
        Some(NodeStatusFilter::Ready),
        Some(NodeRoleFilter::Worker),
    );
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 100, "{}ms", elapsed.as_millis());
}

/// Verifies filtering 1k services stays under 50ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_filter_1k_services_under_50ms() {
    let svcs = (0..1_000)
        .map(|i| {
            make_service(
                &format!("svc-{i}"),
                if i % 2 == 0 { "prod" } else { "dev" },
                "ClusterIP",
            )
        })
        .collect::<Vec<_>>();

    let start = Instant::now();
    let _ = filter_services(&svcs, "svc-9", Some("prod"), Some("ClusterIP"));
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 50, "{}ms", elapsed.as_millis());
}

/// Verifies computing alerts from 1k pods stays under 50ms.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_compute_alerts_1k_pods_under_50ms() {
    let mut snapshot = ClusterSnapshot::default();
    for i in 0..1_000 {
        snapshot.pods.push(make_pod(
            &format!("pod-{i}"),
            "default",
            if i % 3 == 0 { "Failed" } else { "Running" },
        ));
    }

    let start = Instant::now();
    let _ = compute_alerts(&snapshot);
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 50, "{}ms", elapsed.as_millis());
}

/// Verifies tab switching on AppState is very fast.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_tab_switch_under_10ms() {
    let mut app = AppState::default();

    let start = Instant::now();
    for _ in 0..1_000 {
        app.handle_key_event(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Tab,
        ));
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 10, "{}ms", elapsed.as_millis());
}

/// Verifies search keystroke routing is under 5ms for 1k chars.
#[test]
#[ignore = "Optional performance run"]
fn benchmark_search_keystroke_under_5ms() {
    let mut app = AppState::default();
    app.handle_key_event(crossterm::event::KeyEvent::from(
        crossterm::event::KeyCode::Char('/'),
    ));

    let start = Instant::now();
    for _ in 0..1_000 {
        app.handle_key_event(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('a'),
        ));
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 5, "{}ms", elapsed.as_millis());
}
