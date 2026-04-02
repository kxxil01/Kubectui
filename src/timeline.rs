//! Unified timeline merging K8s events with action history for a resource.

use std::collections::VecDeque;

use jiff::ToSpan;

use crate::time::AppTimestamp;

use crate::action_history::{ActionHistoryEntry, ActionKind, ActionStatus};
use crate::app::ResourceRef;
use crate::k8s::client::EventInfo;

/// K8s events within this duration after a user action are marked as correlated.
pub const CORRELATION_WINDOW_SECS: i64 = 300; // 5 minutes

/// A single entry in the unified per-resource timeline.
#[derive(Debug, Clone)]
pub enum TimelineEntry {
    /// A Kubernetes event from the API.
    Event {
        event: EventInfo,
        /// Index of the correlated `Action` in the sorted timeline, if any.
        correlated_action_idx: Option<usize>,
    },
    /// A user-initiated mutation from ActionHistory.
    Action {
        kind: ActionKind,
        status: ActionStatus,
        message: String,
        started_at: AppTimestamp,
        finished_at: Option<AppTimestamp>,
    },
}

impl TimelineEntry {
    /// The primary timestamp used for sorting.
    pub fn sort_timestamp(&self) -> AppTimestamp {
        match self {
            TimelineEntry::Event { event, .. } => event.last_timestamp,
            TimelineEntry::Action { started_at, .. } => *started_at,
        }
    }
}

/// Builds a chronologically sorted timeline from K8s events and
/// action history entries relevant to `resource`.
///
/// Events are already pre-filtered to this resource by the K8s field selector.
/// Action history entries are filtered by `target.resource == resource`.
pub fn build_timeline(
    events: &[EventInfo],
    history: &VecDeque<ActionHistoryEntry>,
    resource: &ResourceRef,
) -> Vec<TimelineEntry> {
    let mut timeline: Vec<TimelineEntry> = Vec::new();

    // 1. Add all K8s events (already filtered to this resource by the API).
    for event in events {
        timeline.push(TimelineEntry::Event {
            event: event.clone(),
            correlated_action_idx: None,
        });
    }

    // 2. Add matching action history entries.
    for entry in history.iter() {
        let matches = entry
            .target
            .as_ref()
            .is_some_and(|t| &t.resource == resource);
        if matches {
            timeline.push(TimelineEntry::Action {
                kind: entry.kind,
                status: entry.status,
                message: entry.message.clone(),
                started_at: entry.started_at,
                finished_at: entry.finished_at,
            });
        }
    }

    // 3. Sort by timestamp ascending (oldest first = natural timeline reading).
    //    Tiebreaker: Actions sort before Events at the same timestamp so that the
    //    forward-scan correlation pass always sees same-timestamp events after their action.
    timeline.sort_unstable_by_key(|e| {
        let order = match e {
            TimelineEntry::Action { .. } => 0u8,
            TimelineEntry::Event { .. } => 1u8,
        };
        (e.sort_timestamp(), order)
    });

    // 4. Correlation pass: for each Action, mark subsequent Events within the
    //    correlation window as related.
    let window = CORRELATION_WINDOW_SECS.seconds();

    let action_indices: Vec<(usize, AppTimestamp)> = timeline
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            if let TimelineEntry::Action { started_at, .. } = entry {
                Some((idx, *started_at))
            } else {
                None
            }
        })
        .collect();

    for &(action_idx, action_ts) in &action_indices {
        let window_end = action_ts
            .checked_add(window)
            .expect("correlation window should stay in range");
        for entry in timeline.iter_mut().skip(action_idx + 1) {
            if entry.sort_timestamp() > window_end {
                break;
            }
            if let TimelineEntry::Event {
                correlated_action_idx,
                ..
            } = entry
                && correlated_action_idx.is_none()
            {
                *correlated_action_idx = Some(action_idx);
            }
        }
    }

    timeline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_history::ActionHistoryTarget;
    use crate::app::AppView;

    fn ts(minutes: i64) -> AppTimestamp {
        let base = "2026-01-01T12:00:00Z"
            .parse::<AppTimestamp>()
            .expect("valid timestamp");
        if minutes >= 0 {
            base.checked_add(minutes.minutes())
                .expect("timestamp in range")
        } else {
            base.checked_sub((-minutes).minutes())
                .expect("timestamp in range")
        }
    }

    fn make_event(reason: &str, last_ts_minutes: i64) -> EventInfo {
        EventInfo {
            event_type: "Normal".to_string(),
            reason: reason.to_string(),
            message: format!("{reason} happened"),
            first_timestamp: ts(last_ts_minutes),
            last_timestamp: ts(last_ts_minutes),
            count: 1,
        }
    }

    fn pod_ref() -> ResourceRef {
        ResourceRef::Pod("api-0".to_string(), "default".to_string())
    }

    fn other_ref() -> ResourceRef {
        ResourceRef::Pod("web-0".to_string(), "default".to_string())
    }

    fn make_action_entry(
        kind: ActionKind,
        resource: &ResourceRef,
        started_minutes: i64,
        finished_minutes: Option<i64>,
    ) -> ActionHistoryEntry {
        ActionHistoryEntry {
            id: 1,
            kind,
            status: ActionStatus::Succeeded,
            resource_label: "test".to_string(),
            message: format!("{} completed", kind.label()),
            target: Some(ActionHistoryTarget {
                view: AppView::Pods,
                resource: resource.clone(),
                scope: crate::app::ActivityScope {
                    context: Some("test-context".to_string()),
                    namespace: "default".to_string(),
                },
            }),
            started_at: ts(started_minutes),
            finished_at: finished_minutes.map(ts),
        }
    }

    fn make_action_entry_no_target(kind: ActionKind, started_minutes: i64) -> ActionHistoryEntry {
        ActionHistoryEntry {
            id: 2,
            kind,
            status: ActionStatus::Succeeded,
            resource_label: "test".to_string(),
            message: format!("{} completed", kind.label()),
            target: None,
            started_at: ts(started_minutes),
            finished_at: Some(ts(started_minutes + 1)),
        }
    }

    #[test]
    fn empty_inputs_empty_timeline() {
        let result = build_timeline(&[], &VecDeque::new(), &pod_ref());
        assert!(result.is_empty());
    }

    #[test]
    fn events_only_sorted() {
        let events = vec![make_event("Pulling", 10), make_event("Started", 5)];
        let result = build_timeline(&events, &VecDeque::new(), &pod_ref());

        assert_eq!(result.len(), 2);
        // Should be sorted: Started (5) before Pulling (10)
        assert!(result[0].sort_timestamp() <= result[1].sort_timestamp());
        match &result[0] {
            TimelineEntry::Event { event, .. } => assert_eq!(event.reason, "Started"),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn actions_only_matching_resource() {
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(ActionKind::Scale, &pod_ref(), 0, Some(1)));

        let result = build_timeline(&[], &history, &pod_ref());
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], TimelineEntry::Action { .. }));
    }

    #[test]
    fn actions_for_different_resource_excluded() {
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &other_ref(),
            0,
            Some(1),
        ));

        let result = build_timeline(&[], &history, &pod_ref());
        assert!(result.is_empty());
    }

    #[test]
    fn actions_without_target_excluded() {
        let mut history = VecDeque::new();
        history.push_back(make_action_entry_no_target(ActionKind::Delete, 0));

        let result = build_timeline(&[], &history, &pod_ref());
        assert!(result.is_empty());
    }

    #[test]
    fn mixed_chronological_order() {
        let events = vec![make_event("Pulling", 5), make_event("Started", 15)];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Restart,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 3);
        // Pulling(5), Restart(10), Started(15)
        assert!(result[0].sort_timestamp() <= result[1].sort_timestamp());
        assert!(result[1].sort_timestamp() <= result[2].sort_timestamp());
        assert!(matches!(result[0], TimelineEntry::Event { .. }));
        assert!(matches!(result[1], TimelineEntry::Action { .. }));
        assert!(matches!(result[2], TimelineEntry::Event { .. }));
    }

    #[test]
    fn correlation_within_window() {
        let events = vec![make_event("Pulling", 12)]; // 2 min after action
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);

        // Action at 10, Event at 12 → correlated
        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(0)),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn correlation_outside_window() {
        let events = vec![make_event("BackOff", 16)]; // 6 min after action (> 5 min window)
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);

        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, None),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn multiple_actions_nearest_wins() {
        // Action1 at 0, Action2 at 10, Event at 12
        let events = vec![make_event("Pulling", 12)];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(ActionKind::Scale, &pod_ref(), 0, Some(1)));
        history.push_back(make_action_entry(
            ActionKind::Restart,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 3);
        // Action1(0), Action2(10), Event(12)
        // Event at 12 is within window of Action1(0+5=5 → no, 12>5) and Action2(10+5=15 → yes)
        // But Action1 processes first — 12 > 5, so it skips.
        // Action2 processes second — 12 < 15, so it correlates.
        match &result[2] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(1)), // Action2 index
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn correlation_does_not_apply_to_events_before_action() {
        let events = vec![make_event("Started", 5)]; // before action at 10
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);

        // Event at 5 is before action at 10 → not correlated
        match &result[0] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, None),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn correlation_at_exact_boundary_is_included() {
        // Event at exactly action_ts + 5 minutes should be correlated (inclusive boundary)
        let events = vec![make_event("Scheduled", 15)]; // exactly 5 min after action at 10
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);

        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(0)),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn multiple_events_correlated_to_same_action() {
        // Two events within window of a single action should both correlate
        let events = vec![make_event("Pulling", 12), make_event("Started", 13)];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Restart,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 3);

        // Both events should correlate to the action at index 0
        for (i, entry) in result.iter().enumerate().skip(1) {
            match entry {
                TimelineEntry::Event {
                    correlated_action_idx,
                    ..
                } => assert_eq!(*correlated_action_idx, Some(0), "event at index {i}"),
                _ => panic!("expected Event at index {i}"),
            }
        }
    }

    #[test]
    fn event_in_overlapping_windows_correlates_with_first_action() {
        // Action1 at 0, Action2 at 3. Event at 4 is within window of both.
        // Since Action1 processes first and 4 < 5, Action1 claims it.
        let events = vec![make_event("Pulling", 4)];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(ActionKind::Scale, &pod_ref(), 0, Some(1)));
        history.push_back(make_action_entry(
            ActionKind::Restart,
            &pod_ref(),
            3,
            Some(4),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 3);
        // Action1(0), Action2(3), Event(4)
        // Action1 window: 0..5 → 4 is within → claims it
        match &result[2] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(0)), // First action wins
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn same_timestamp_action_sorts_before_event() {
        // Action and Event at the same timestamp — Action must sort first
        // so that the correlation pass sees the Event after the Action.
        let events = vec![make_event("Scheduled", 10)];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], TimelineEntry::Action { .. }));
        assert!(matches!(result[1], TimelineEntry::Event { .. }));
        // Event at same timestamp as action should be correlated
        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(0)),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn pending_action_appears_in_timeline() {
        let mut history = VecDeque::new();
        let mut entry = make_action_entry(ActionKind::Scale, &pod_ref(), 5, None);
        entry.status = ActionStatus::Pending;
        history.push_back(entry);

        let result = build_timeline(&[], &history, &pod_ref());
        assert_eq!(result.len(), 1);
        match &result[0] {
            TimelineEntry::Action { status, .. } => assert_eq!(*status, ActionStatus::Pending),
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn failed_action_correlates_with_events() {
        let events = vec![make_event("BackOff", 12)];
        let mut history = VecDeque::new();
        let mut entry = make_action_entry(ActionKind::Delete, &pod_ref(), 10, Some(11));
        entry.status = ActionStatus::Failed;
        history.push_back(entry);

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);
        match &result[0] {
            TimelineEntry::Action { status, .. } => assert_eq!(*status, ActionStatus::Failed),
            _ => panic!("expected Action"),
        }
        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, Some(0)),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn in_progress_action_with_no_finished_at() {
        let mut history = VecDeque::new();
        // finished_at: None simulates an in-progress action
        history.push_back(make_action_entry(ActionKind::Drain, &pod_ref(), 5, None));

        let result = build_timeline(&[], &history, &pod_ref());
        assert_eq!(result.len(), 1);
        match &result[0] {
            TimelineEntry::Action { finished_at, .. } => assert!(finished_at.is_none()),
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn mixed_resources_in_history_filtered_correctly() {
        let events = vec![make_event("Pulled", 5)];
        let mut history = VecDeque::new();
        // 3 entries: matching, non-matching, matching
        history.push_back(make_action_entry(ActionKind::Scale, &pod_ref(), 0, Some(1)));
        history.push_back(make_action_entry(
            ActionKind::Delete,
            &other_ref(),
            2,
            Some(3),
        ));
        history.push_back(make_action_entry(
            ActionKind::Restart,
            &pod_ref(),
            3,
            Some(4),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        // Should have: Scale(0), Restart(3), Pulled(5) — Delete for other_ref excluded
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], TimelineEntry::Action { .. }));
        assert!(matches!(result[1], TimelineEntry::Action { .. }));
        assert!(matches!(result[2], TimelineEntry::Event { .. }));
    }

    #[test]
    fn events_only_all_uncorrelated() {
        let events = vec![
            make_event("Pulling", 5),
            make_event("Pulled", 10),
            make_event("Started", 15),
        ];
        let result = build_timeline(&events, &VecDeque::new(), &pod_ref());
        assert_eq!(result.len(), 3);
        for (i, entry) in result.iter().enumerate() {
            match entry {
                TimelineEntry::Event {
                    correlated_action_idx,
                    ..
                } => assert_eq!(*correlated_action_idx, None, "event at index {i}"),
                _ => panic!("expected Event at index {i}"),
            }
        }
    }

    #[test]
    fn boundary_plus_one_second_not_correlated() {
        // Event at action_ts + 301 seconds (one second past the 5-minute window)
        let events = vec![{
            let mut e = make_event("Late", 0);
            e.last_timestamp = ts(10).checked_add(301.seconds()).unwrap();
            e.first_timestamp = e.last_timestamp;
            e
        }];
        let mut history = VecDeque::new();
        history.push_back(make_action_entry(
            ActionKind::Scale,
            &pod_ref(),
            10,
            Some(11),
        ));

        let result = build_timeline(&events, &history, &pod_ref());
        assert_eq!(result.len(), 2);
        match &result[1] {
            TimelineEntry::Event {
                correlated_action_idx,
                ..
            } => assert_eq!(*correlated_action_idx, None),
            _ => panic!("expected Event"),
        }
    }
}
