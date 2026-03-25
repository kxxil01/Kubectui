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
pub mod icons;
pub mod k8s;
pub mod network_policy_analysis;
pub mod network_policy_connectivity;
pub mod network_policy_semantics;
pub mod policy;
pub mod preferences;
pub mod resource_diff;
pub mod secret;
pub mod state;
pub mod time;
pub mod timeline;
pub mod ui;
pub mod workbench;
