//! Library exports for KubecTUI.

#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

pub mod app;
pub mod coordinator;
pub mod events;
pub mod k8s;
pub mod policy;
pub mod state;
pub mod ui;
pub mod workbench;
