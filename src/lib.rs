//! Library exports for KubecTUI.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

pub mod action_history;
pub mod app;
pub mod authorization;
pub mod bookmarks;
pub mod clipboard;
pub mod columns;
pub mod coordinator;
pub mod cronjob;
pub mod detail_sections;
pub mod events;
pub mod export;
pub mod extensions;
pub mod global_search;
pub mod governance;
pub mod icons;
pub mod k8s;
pub mod log_investigation;
pub mod network_policy_analysis;
pub mod network_policy_connectivity;
pub mod network_policy_semantics;
pub mod policy;
pub mod preferences;
pub mod projects;
pub mod resource_diff;
pub mod resource_templates;
pub mod runbooks;
pub mod secret;
pub mod state;
pub mod time;
pub mod timeline;
pub mod traffic_debug;
pub mod ui;
pub mod workbench;
pub mod workspaces;
