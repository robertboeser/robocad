//! robocad desktop application (library part, so UI tests can drive the app headlessly).

pub mod app;
pub mod extensions;
pub mod gizmo;
pub mod help;
pub mod icons;
pub mod interaction;
pub mod loader;
pub mod project_ui;
pub mod properties;
pub mod scene_panel;
pub mod scene_view;
pub mod settings;
pub mod viewport;

pub use app::RobocadApp;
