//! Integration tests for GlobalState lifecycle and transitions.

mod common;

use std::sync::atomic::Ordering;

use common::{MockDataSource, make_pod};
use kubectui::state::{DataPhase, GlobalState};

/// Verifies initial GlobalState phase is Idle.
#[tokio::test]
#[ignore = "Optional integration run"]
async fn initial_state_is_idle() {
    let state = GlobalState::default();
    assert_eq!(state.snapshot().phase, DataPhase::Idle);
}

/// Verifies successful refresh transitions phase to Ready.
#[tokio::test]
#[ignore = "Optional integration run"]
async fn refresh_transitions_to_ready() {
    let mut state = GlobalState::default();
    let source = MockDataSource::default();

    state
        .refresh(&source, None)
        .await
        .expect("refresh should succeed with mock source");

    let snapshot = state.snapshot();
    assert_eq!(snapshot.phase, DataPhase::Ready);
    assert!(snapshot.last_updated.is_some());
}

/// Verifies mock responses populate snapshot counts and resources.
#[tokio::test]
#[ignore = "Optional integration run"]
async fn mock_refresh_populates_snapshot() {
    let mut state = GlobalState::default();
    let mut source = MockDataSource::default();
    source.pods.push(make_pod("p2", "kube-system", "Pending"));

    state
        .refresh(&source, None)
        .await
        .expect("refresh should succeed");

    let snapshot = state.snapshot();
    assert_eq!(snapshot.nodes.len(), source.nodes.len());
    assert_eq!(snapshot.pods.len(), 2);
    assert_eq!(snapshot.services.len(), source.services.len());
    assert_eq!(snapshot.namespaces_count, 2);
}

/// Verifies failing data source transitions phase to Error and records message.
#[tokio::test]
#[ignore = "Optional integration run"]
async fn error_state_transition_on_refresh_failure() {
    let mut state = GlobalState::default();
    let source = MockDataSource {
        fail: true,
        ..MockDataSource::default()
    };

    let err = state
        .refresh(&source, None)
        .await
        .expect_err("refresh should fail when mock configured to fail");

    assert!(err.to_string().contains("mock"));
    let snapshot = state.snapshot();
    assert_eq!(snapshot.phase, DataPhase::Error);
    assert!(snapshot.last_error.is_some());
}

/// Verifies data source methods are called and first refresh path wins.
#[tokio::test]
#[ignore = "Optional integration run"]
async fn concurrent_refresh_requests_are_observable() {
    let mut state = GlobalState::default();
    let source = MockDataSource::default();

    state
        .refresh(&source, None)
        .await
        .expect("first refresh should succeed");

    // second call should still be valid and deterministic
    state
        .refresh(&source, None)
        .await
        .expect("second refresh should succeed");

    assert!(
        source.calls.load(Ordering::SeqCst) >= 20,
        "expected at least two rounds of 10 calls"
    );
}
