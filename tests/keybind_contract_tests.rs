#![allow(clippy::field_reassign_with_default)]

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kubectui::{
    app::{AppAction, AppState, AppView, DetailViewState, Focus, ResourceRef},
    events::route_keyboard_input,
    policy::DetailAction,
    workbench::{ResourceYamlTabState, WorkbenchTabState},
    workspaces::{HotkeyAction, HotkeyBinding, HotkeyTarget, WorkspaceBank},
};

#[derive(Debug)]
enum DetailExpectation {
    Action(AppAction),
    OpensDeleteConfirm,
    OpensDrainConfirm,
}

#[derive(Debug)]
struct DetailShortcutCase {
    action: DetailAction,
    expected_hint: &'static str,
    expected_key: char,
    make_detail: fn() -> DetailViewState,
    expectation: DetailExpectation,
}

fn pod_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Pod("pod-a".into(), "default".into())),
        yaml: Some("kind: Pod".into()),
        ..DetailViewState::default()
    }
}

fn deployment_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Deployment("deploy-a".into(), "default".into())),
        yaml: Some("kind: Deployment".into()),
        ..DetailViewState::default()
    }
}

fn node_cordoned_detail() -> DetailViewState {
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::Node("node-a".into())),
        yaml: Some("kind: Node".into()),
        ..DetailViewState::default()
    };
    detail.metadata.node_unschedulable = Some(true);
    detail
}

fn node_uncordoned_detail() -> DetailViewState {
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::Node("node-a".into())),
        yaml: Some("kind: Node".into()),
        ..DetailViewState::default()
    };
    detail.metadata.node_unschedulable = Some(false);
    detail
}

fn secret_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::Secret("secret-a".into(), "default".into())),
        yaml: Some("kind: Secret".into()),
        ..DetailViewState::default()
    }
}

fn helm_release_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::HelmRelease(
            "release-a".into(),
            "default".into(),
        )),
        yaml: Some("kind: HelmRelease".into()),
        ..DetailViewState::default()
    }
}

fn cronjob_suspended_detail() -> DetailViewState {
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::CronJob("job-a".into(), "default".into())),
        yaml: Some("kind: CronJob".into()),
        ..DetailViewState::default()
    };
    detail.metadata.cronjob_suspended = Some(true);
    detail
}

fn cronjob_unsuspended_detail() -> DetailViewState {
    let mut detail = DetailViewState {
        resource: Some(ResourceRef::CronJob("job-a".into(), "default".into())),
        yaml: Some("kind: CronJob".into()),
        ..DetailViewState::default()
    };
    detail.metadata.cronjob_suspended = Some(false);
    detail
}

fn flux_kustomization_detail() -> DetailViewState {
    DetailViewState {
        resource: Some(ResourceRef::CustomResource {
            name: "apps".into(),
            namespace: Some("flux-system".into()),
            group: "kustomize.toolkit.fluxcd.io".into(),
            version: "v1".into(),
            kind: "Kustomization".into(),
            plural: "kustomizations".into(),
        }),
        yaml: Some("kind: Kustomization".into()),
        ..DetailViewState::default()
    }
}

fn detail_shortcut_cases() -> Vec<DetailShortcutCase> {
    vec![
        DetailShortcutCase {
            action: DetailAction::ViewYaml,
            expected_hint: "[y]",
            expected_key: 'y',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenResourceYaml),
        },
        DetailShortcutCase {
            action: DetailAction::ViewConfigDrift,
            expected_hint: "[D]",
            expected_key: 'D',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenResourceDiff),
        },
        DetailShortcutCase {
            action: DetailAction::ViewRollout,
            expected_hint: "[O]",
            expected_key: 'O',
            make_detail: deployment_detail,
            expectation: DetailExpectation::Action(AppAction::OpenRollout),
        },
        DetailShortcutCase {
            action: DetailAction::ViewHelmHistory,
            expected_hint: "[h]",
            expected_key: 'h',
            make_detail: helm_release_detail,
            expectation: DetailExpectation::Action(AppAction::OpenHelmHistory),
        },
        DetailShortcutCase {
            action: DetailAction::ViewDecodedSecret,
            expected_hint: "[o]",
            expected_key: 'o',
            make_detail: secret_detail,
            expectation: DetailExpectation::Action(AppAction::OpenDecodedSecret),
        },
        DetailShortcutCase {
            action: DetailAction::ToggleBookmark,
            expected_hint: "[B]",
            expected_key: 'B',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::ToggleBookmark),
        },
        DetailShortcutCase {
            action: DetailAction::ViewEvents,
            expected_hint: "[v]",
            expected_key: 'v',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenResourceEvents),
        },
        DetailShortcutCase {
            action: DetailAction::ViewAccessReview,
            expected_hint: "[A]",
            expected_key: 'A',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenAccessReview),
        },
        DetailShortcutCase {
            action: DetailAction::Logs,
            expected_hint: "[l]",
            expected_key: 'l',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::LogsViewerOpen),
        },
        DetailShortcutCase {
            action: DetailAction::Exec,
            expected_hint: "[x]",
            expected_key: 'x',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenExec),
        },
        DetailShortcutCase {
            action: DetailAction::DebugContainer,
            expected_hint: "[g]",
            expected_key: 'g',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::DebugContainerDialogOpen),
        },
        DetailShortcutCase {
            action: DetailAction::NodeDebugShell,
            expected_hint: "[g]",
            expected_key: 'g',
            make_detail: node_uncordoned_detail,
            expectation: DetailExpectation::Action(AppAction::NodeDebugDialogOpen),
        },
        DetailShortcutCase {
            action: DetailAction::PortForward,
            expected_hint: "[f]",
            expected_key: 'f',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::PortForwardOpen),
        },
        DetailShortcutCase {
            action: DetailAction::Probes,
            expected_hint: "[p]",
            expected_key: 'p',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::ProbePanelOpen),
        },
        DetailShortcutCase {
            action: DetailAction::Scale,
            expected_hint: "[s]",
            expected_key: 's',
            make_detail: deployment_detail,
            expectation: DetailExpectation::Action(AppAction::ScaleDialogOpen),
        },
        DetailShortcutCase {
            action: DetailAction::Restart,
            expected_hint: "[R]",
            expected_key: 'R',
            make_detail: deployment_detail,
            expectation: DetailExpectation::Action(AppAction::RolloutRestart),
        },
        DetailShortcutCase {
            action: DetailAction::FluxReconcile,
            expected_hint: "[R]",
            expected_key: 'R',
            make_detail: flux_kustomization_detail,
            expectation: DetailExpectation::Action(AppAction::FluxReconcile),
        },
        DetailShortcutCase {
            action: DetailAction::EditYaml,
            expected_hint: "[e]",
            expected_key: 'e',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::EditYaml),
        },
        DetailShortcutCase {
            action: DetailAction::Delete,
            expected_hint: "[d]",
            expected_key: 'd',
            make_detail: pod_detail,
            expectation: DetailExpectation::OpensDeleteConfirm,
        },
        DetailShortcutCase {
            action: DetailAction::Trigger,
            expected_hint: "[T]",
            expected_key: 'T',
            make_detail: cronjob_unsuspended_detail,
            expectation: DetailExpectation::Action(AppAction::TriggerCronJob),
        },
        DetailShortcutCase {
            action: DetailAction::SuspendCronJob,
            expected_hint: "[S]",
            expected_key: 'S',
            make_detail: cronjob_unsuspended_detail,
            expectation: DetailExpectation::Action(AppAction::ConfirmCronJobSuspend(true)),
        },
        DetailShortcutCase {
            action: DetailAction::ResumeCronJob,
            expected_hint: "[S]",
            expected_key: 'S',
            make_detail: cronjob_suspended_detail,
            expectation: DetailExpectation::Action(AppAction::ConfirmCronJobSuspend(false)),
        },
        DetailShortcutCase {
            action: DetailAction::ViewNetworkPolicies,
            expected_hint: "[N]",
            expected_key: 'N',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenNetworkPolicyView),
        },
        DetailShortcutCase {
            action: DetailAction::CheckNetworkConnectivity,
            expected_hint: "[C]",
            expected_key: 'C',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenNetworkConnectivity),
        },
        DetailShortcutCase {
            action: DetailAction::ViewTrafficDebug,
            expected_hint: "[t]",
            expected_key: 't',
            make_detail: pod_detail,
            expectation: DetailExpectation::Action(AppAction::OpenTrafficDebug),
        },
        DetailShortcutCase {
            action: DetailAction::ViewRelationships,
            expected_hint: "[w]",
            expected_key: 'w',
            make_detail: deployment_detail,
            expectation: DetailExpectation::Action(AppAction::OpenRelationships),
        },
        DetailShortcutCase {
            action: DetailAction::Cordon,
            expected_hint: "[c]",
            expected_key: 'c',
            make_detail: node_uncordoned_detail,
            expectation: DetailExpectation::Action(AppAction::CordonNode),
        },
        DetailShortcutCase {
            action: DetailAction::Uncordon,
            expected_hint: "[u]",
            expected_key: 'u',
            make_detail: node_cordoned_detail,
            expectation: DetailExpectation::Action(AppAction::UncordonNode),
        },
        DetailShortcutCase {
            action: DetailAction::Drain,
            expected_hint: "[D]",
            expected_key: 'D',
            make_detail: node_uncordoned_detail,
            expectation: DetailExpectation::OpensDrainConfirm,
        },
    ]
}

#[test]
fn detail_shortcut_cases_cover_all_hinted_actions() {
    let covered = detail_shortcut_cases()
        .iter()
        .map(|case| case.action)
        .collect::<HashSet<_>>();
    let hinted = DetailAction::ALL
        .iter()
        .copied()
        .filter(|action| action.shortcut_hint().is_some())
        .collect::<HashSet<_>>();

    assert_eq!(covered, hinted);
}

#[test]
fn detail_shortcuts_route_expected_actions() {
    for case in detail_shortcut_cases() {
        assert_eq!(
            case.action.shortcut_hint(),
            Some(case.expected_hint),
            "shortcut hint drift for {:?}",
            case.action
        );

        let mut app = AppState::default();
        app.detail_view = Some((case.make_detail)());
        app.focus = Focus::Content;
        let key = KeyEvent::from(KeyCode::Char(case.expected_key));
        let action = route_keyboard_input(key, &mut app);

        match case.expectation {
            DetailExpectation::Action(expected_action) => {
                assert_eq!(
                    action, expected_action,
                    "action mismatch for {:?}",
                    case.action
                );
            }
            DetailExpectation::OpensDeleteConfirm => {
                assert_eq!(action, AppAction::None);
                assert!(
                    app.detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.confirm_delete),
                    "delete confirm not opened for {:?}",
                    case.action
                );
            }
            DetailExpectation::OpensDrainConfirm => {
                assert_eq!(action, AppAction::None);
                assert!(
                    app.detail_view
                        .as_ref()
                        .is_some_and(|detail| detail.confirm_drain),
                    "drain confirm not opened for {:?}",
                    case.action
                );
            }
        }
    }
}

fn reserved_modifier_variants() -> [KeyModifiers; 7] {
    [
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
        KeyModifiers::META,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SUPER,
    ]
}

#[test]
fn detail_shortcuts_reject_reserved_modifier_variants() {
    for case in detail_shortcut_cases() {
        for modifiers in reserved_modifier_variants() {
            let mut app = AppState::default();
            app.detail_view = Some((case.make_detail)());
            app.focus = Focus::Content;

            let action = route_keyboard_input(
                KeyEvent::new(KeyCode::Char(case.expected_key), modifiers),
                &mut app,
            );

            match &case.expectation {
                DetailExpectation::Action(expected_action) => {
                    assert_ne!(
                        &action, expected_action,
                        "{:?} fired from {:?}+{:?}",
                        case.action, modifiers, case.expected_key
                    );
                }
                DetailExpectation::OpensDeleteConfirm => {
                    assert!(
                        app.detail_view
                            .as_ref()
                            .is_none_or(|detail| !detail.confirm_delete),
                        "delete confirm opened from {:?}+{:?}",
                        modifiers,
                        case.expected_key
                    );
                }
                DetailExpectation::OpensDrainConfirm => {
                    assert!(
                        app.detail_view
                            .as_ref()
                            .is_none_or(|detail| !detail.confirm_drain),
                        "drain confirm opened from {:?}+{:?}",
                        modifiers,
                        case.expected_key
                    );
                }
            }
        }
    }
}

#[test]
fn workbench_page_keys_reject_reserved_modifier_variants() {
    for modifiers in reserved_modifier_variants() {
        let mut app = AppState::default();
        app.focus = Focus::Workbench;
        app.workbench_mut()
            .open_tab(WorkbenchTabState::ResourceYaml(ResourceYamlTabState {
                resource: ResourceRef::Pod("pod-a".into(), "default".into()),
                pending_request_id: None,
                yaml: Some(
                    (0..30)
                        .map(|idx| format!("line-{idx}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                scroll: 5,
                loading: false,
                error: None,
            }));

        let action = route_keyboard_input(KeyEvent::new(KeyCode::PageDown, modifiers), &mut app);
        assert_eq!(action, AppAction::None);

        let scroll = app
            .workbench()
            .active_tab()
            .and_then(|tab| match &tab.state {
                WorkbenchTabState::ResourceYaml(tab) => Some(tab.scroll),
                _ => None,
            })
            .expect("expected resource yaml tab");

        assert_eq!(scroll, 5, "PageDown changed scroll for {modifiers:?}");

        let action = route_keyboard_input(KeyEvent::new(KeyCode::PageUp, modifiers), &mut app);
        assert_eq!(action, AppAction::None);

        let scroll = app
            .workbench()
            .active_tab()
            .and_then(|tab| match &tab.state {
                WorkbenchTabState::ResourceYaml(tab) => Some(tab.scroll),
                _ => None,
            })
            .expect("expected resource yaml tab");

        assert_eq!(scroll, 5, "PageUp changed scroll for {modifiers:?}");
    }
}

#[test]
fn readme_lists_canonical_workbench_tab_keys() {
    let readme = include_str!("../README.md");
    assert!(
        readme.contains("| `,` / `.` | Switch tabs |"),
        "README workbench table must list runtime workbench tab keys"
    );
    assert!(
        !readme.contains("| `[` / `]` | Switch tabs |"),
        "README must not advertise removed workbench tab keys"
    );
}

#[test]
fn workspace_hotkey_targets_route_expected_actions() {
    let mut app = AppState::default();
    app.focus = Focus::Content;

    let prefs = app.preferences.get_or_insert_with(Default::default);
    prefs.workspaces.hotkeys = vec![
        HotkeyBinding {
            key: "alt+1".into(),
            target: HotkeyTarget::View {
                view: AppView::Pods,
            },
        },
        HotkeyBinding {
            key: "alt+2".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::OpenCommandPalette,
            },
        },
        HotkeyBinding {
            key: "alt+3".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::RefreshData,
            },
        },
        HotkeyBinding {
            key: "alt+4".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::OpenActionHistory,
            },
        },
        HotkeyBinding {
            key: "alt+5".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::OpenNamespacePicker,
            },
        },
        HotkeyBinding {
            key: "alt+6".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::OpenContextPicker,
            },
        },
        HotkeyBinding {
            key: "alt+7".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::SaveWorkspace,
            },
        },
        HotkeyBinding {
            key: "alt+8".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::ApplyPreviousWorkspace,
            },
        },
        HotkeyBinding {
            key: "alt+9".into(),
            target: HotkeyTarget::Action {
                action: HotkeyAction::ApplyNextWorkspace,
            },
        },
        HotkeyBinding {
            key: "alt+w".into(),
            target: HotkeyTarget::Workspace {
                name: "incident".into(),
            },
        },
        HotkeyBinding {
            key: "alt+b".into(),
            target: HotkeyTarget::Bank {
                name: "prod".into(),
            },
        },
    ];

    let cases = vec![
        (
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            AppAction::NavigateTo(AppView::Pods),
        ),
        (
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT),
            AppAction::OpenCommandPalette,
        ),
        (
            KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT),
            AppAction::RefreshData,
        ),
        (
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT),
            AppAction::OpenActionHistory,
        ),
        (
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT),
            AppAction::OpenNamespacePicker,
        ),
        (
            KeyEvent::new(KeyCode::Char('6'), KeyModifiers::ALT),
            AppAction::OpenContextPicker,
        ),
        (
            KeyEvent::new(KeyCode::Char('7'), KeyModifiers::ALT),
            AppAction::SaveWorkspace,
        ),
        (
            KeyEvent::new(KeyCode::Char('8'), KeyModifiers::ALT),
            AppAction::ApplyPreviousWorkspace,
        ),
        (
            KeyEvent::new(KeyCode::Char('9'), KeyModifiers::ALT),
            AppAction::ApplyNextWorkspace,
        ),
        (
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
            AppAction::ApplyWorkspace("incident".into()),
        ),
        (
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
            AppAction::ActivateWorkspaceBank("prod".into()),
        ),
    ];

    for (key, expected) in cases {
        let action = route_keyboard_input(key, &mut app);
        assert_eq!(action, expected);
    }
}

#[test]
fn workspace_bank_hotkey_fallback_routes_when_binding_list_empty() {
    let mut app = AppState::default();
    app.focus = Focus::Content;

    let prefs = app.preferences.get_or_insert_with(Default::default);
    prefs.workspaces.banks = vec![WorkspaceBank {
        name: "production".into(),
        context: Some("prod".into()),
        namespace: "payments".into(),
        view: AppView::Pods,
        search_query: Some("checkout".into()),
        hotkey: Some("alt+p".into()),
    }];

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
        &mut app,
    );
    assert_eq!(
        action,
        AppAction::ActivateWorkspaceBank("production".into())
    );
}

#[test]
fn workspace_hotkeys_do_not_fire_when_detail_is_open() {
    let mut app = AppState::default();
    app.focus = Focus::Content;
    app.detail_view = Some(pod_detail());

    let prefs = app.preferences.get_or_insert_with(Default::default);
    prefs.workspaces.hotkeys = vec![HotkeyBinding {
        key: "alt+1".into(),
        target: HotkeyTarget::View {
            view: AppView::Pods,
        },
    }];

    let action = route_keyboard_input(
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
        &mut app,
    );
    assert_eq!(action, AppAction::None);
}
