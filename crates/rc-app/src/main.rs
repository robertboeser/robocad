//! robocad desktop application.
// Release builds on Windows are GUI programs: no console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("robocad")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "robocad",
        options,
        Box::new(move |cc| Ok(Box::new(rc_app::RobocadApp::new(cc, files)))),
    )
}
