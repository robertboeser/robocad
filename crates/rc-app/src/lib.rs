//! robocad desktop application (library part, so UI tests can drive the app headlessly).

pub mod app;
pub mod gizmo;
pub mod interaction;
pub mod loader;
pub mod properties;
pub mod scene_panel;
pub mod scene_view;
pub mod viewport;

pub use app::RobocadApp;
