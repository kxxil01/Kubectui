//! View-specific renderers.

pub mod dashboard;
pub mod deployments;
pub mod detail;
pub mod nodes;
pub mod services;

pub use dashboard::render_dashboard;
