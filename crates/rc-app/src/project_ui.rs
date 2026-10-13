//! Project workflow: new / open / save, recent files, autosave and crash recovery,
//! the unsaved-changes prompt, and the preferences window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use rc_doc::project::{self, ViewState};
use rc_doc::{DocSettings, Document};
use rc_render::Projection;

use crate::app::RobocadApp;
use crate::loader::LoadedMesh;
use crate::settings::{MousePreset, Theme};

/// What to do once the user has dealt with unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum AfterPrompt {
    Quit,
    New,
    Open(PathBuf),
}

/// A project loaded on a worker thread, with mesh analyses ready for display.
pub struct ProjectLoaded {
    path: PathBuf,
    result: Result<(project::Project, Vec<LoadedMesh>), String>,
}

/// Result of a background save: (revision saved, path, outcome).
type SaveResult = (u64, PathBuf, Result<(), String>);

/// Project-related state of the app.
pub struct ProjectState {
    pub path: Option<PathBuf>,
    pub prompt: Option<AfterPrompt>,
    pub allow_close: bool,
    pub recovery_offer: bool,
    pub show_preferences: bool,
    last_autosave: Instant,
    autosaved_revision: u64,
    loading: Option<Receiver<ProjectLoaded>>,
    saving: Option<Receiver<SaveResult>>,
    autosaving: Option<Receiver<()>>,
}

impl ProjectState {
    pub fn new(recovery_offer: bool) -> Self {
        Self {
            path: None,
            prompt: None,
            allow_close: false,
            recovery_offer,
            show_preferences: false,
            last_autosave: Instant::now(),
            autosaved_revision: 0,
            loading: None,
            saving: None,
            autosaving: None,
        }
    }
}

impl RobocadApp {
    // ------------------------------------------------------------------ settings

    /// Applies loaded settings to the app (called once at startup).
    pub(crate) fn apply_settings(&mut self, ctx: &egui::Context) {
        let s = self.settings.clone();
        ctx.set_theme(match s.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
        ctx.set_zoom_factor(s.ui_scale.clamp(0.5, 3.0));
        self.import_units = s.import_units;
        self.place_on_plate = s.place_on_plate;
        self.show_edges = s.show_edges;
        self.snapping = s.snapping;
        if let Some(vp) = self.viewport.as_mut() {
            vp.mouse = s.mouse;
        }
        self.editor.set_settings(self.default_doc_settings());
        self.editor.mark_saved();
    }

    fn default_doc_settings(&self) -> DocSettings {
        DocSettings {
            plate_size: self.settings.plate_size,
            grid_step: self.settings.grid_step,
        }
    }

    /// Copies live UI choices into the settings and saves them when they changed.
    pub(crate) fn persist_settings(&mut self) {
        self.settings.import_units = self.import_units;
        self.settings.place_on_plate = self.place_on_plate;
        self.settings.show_edges = self.show_edges;
        self.settings.snapping = self.snapping;
        if let Some(vp) = self.viewport.as_ref() {
            self.settings.mouse = vp.mouse;
        }
        if self.settings != self.settings_saved {
            self.settings.save(&self.dirs.config);
            self.settings_saved = self.settings.clone();
        }
    }

    // ------------------------------------------------------------------ titles

    pub fn window_title_for_tests(&self) -> String {
        self.window_title()
    }

    pub(crate) fn window_title(&self) -> String {
        let name = self
            .project
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Untitled".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
        let dirty = if self.editor.is_dirty() { " •" } else { "" };
        format!("{name}{dirty} — robocad")
    }

    // ------------------------------------------------------------------ new / open

    /// Runs `action` now, or first asks to save unsaved changes.
    pub fn request(&mut self, action: AfterPrompt, ctx: &egui::Context) {
        if self.editor.is_dirty() && !self.editor.doc().nodes.is_empty() {
            self.project.prompt = Some(action);
        } else {
            self.run_action(action, ctx);
        }
    }

    fn run_action(&mut self, action: AfterPrompt, ctx: &egui::Context) {
        match action {
            AfterPrompt::Quit => {
                self.project.allow_close = true;
                self.discard_autosave();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            AfterPrompt::New => self.new_project(),
            AfterPrompt::Open(path) => self.open_project(path, ctx),
        }
    }

    pub fn new_project(&mut self) {
        let mut doc = Document::new();
        doc.settings = self.default_doc_settings();
        self.reset_with(doc, None);
    }

    fn reset_with(&mut self, doc: Document, path: Option<PathBuf>) {
        self.editor.replace_document(doc);
        self.project.path = path;
        self.project.autosaved_revision = self.editor.revision();
        self.groups = crate::scene_view::GroupResults::default();
        self.items.clear();
        self.hovered = None;
        self.gizmo.end();
    }

    /// Opens a `.rcad` project (asynchronously) or imports an STL.
    pub fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        let is_project = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(project::EXTENSION));
        if is_project {
            self.request(AfterPrompt::Open(path), ctx);
        } else {
            self.import_path(path, ctx);
        }
    }

    fn open_project(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let p = path.clone();
        std::thread::spawn(move || {
            let result = project::load(&p).map_err(|e| e.to_string()).map(|proj| {
                let analyses = proj
                    .document
                    .meshes()
                    .map(|(_, m)| {
                        crate::loader::analyse(
                            p.clone(),
                            String::new(),
                            m.as_ref().clone(),
                            Instant::now(),
                        )
                    })
                    .collect();
                (proj, analyses)
            });
            let _ = tx.send(ProjectLoaded { path: p, result });
            ctx.request_repaint();
        });
        self.project.loading = Some(rx);
        let _ = path;
    }

    /// Opens a project synchronously (tests and command-line arguments).
    pub fn open_project_blocking(&mut self, path: &Path) -> Result<(), String> {
        let proj = project::load(path).map_err(|e| e.to_string())?;
        self.finish_open(path.to_path_buf(), proj, Vec::new());
        Ok(())
    }

    fn finish_open(&mut self, path: PathBuf, proj: project::Project, analyses: Vec<LoadedMesh>) {
        for a in &analyses {
            self.register_analysis(a);
        }
        self.reset_with(proj.document, Some(path.clone()));
        if let (Some(v), Some(vp)) = (proj.view, self.viewport.as_mut()) {
            vp.camera.target = glam::DVec3::from_array(v.target);
            vp.camera.yaw = v.yaw;
            vp.camera.pitch = v.pitch;
            vp.camera.distance = v.distance;
            vp.camera.projection = if v.orthographic {
                Projection::Orthographic
            } else {
                Projection::Perspective
            };
        } else {
            self.frame_all();
        }
        self.settings.add_recent(&path);
    }

    pub(crate) fn receive_project_io(&mut self) {
        if let Some(rx) = &self.project.loading
            && let Ok(done) = rx.try_recv()
        {
            self.project.loading = None;
            match done.result {
                Ok((proj, analyses)) => self.finish_open(done.path, proj, analyses),
                Err(e) => self
                    .messages
                    .push(format!("Could not open {}: {e}", done.path.display())),
            }
        }
        if let Some(rx) = &self.project.saving
            && let Ok((rev, path, result)) = rx.try_recv()
        {
            self.project.saving = None;
            match result {
                Ok(()) => {
                    self.editor.mark_saved_at(rev);
                    self.settings.add_recent(&path);
                    self.discard_autosave();
                }
                Err(e) => self.messages.push(format!("Saving failed: {e}")),
            }
        }
        if let Some(rx) = &self.project.autosaving
            && rx.try_recv().is_ok()
        {
            self.project.autosaving = None;
        }
    }

    pub fn is_loading_project(&self) -> bool {
        self.project.loading.is_some() || self.project.saving.is_some()
    }

    // ------------------------------------------------------------------ save

    fn view_state(&self) -> Option<ViewState> {
        self.viewport.as_ref().map(|vp| ViewState {
            target: vp.camera.target.to_array(),
            yaw: vp.camera.yaw,
            pitch: vp.camera.pitch,
            distance: vp.camera.distance,
            orthographic: vp.camera.projection == Projection::Orthographic,
        })
    }

    fn thumbnail(&self) -> Option<Vec<u8>> {
        self.viewport
            .as_ref()
            .and_then(crate::viewport::Viewport::thumbnail_png)
    }

    pub fn save(&mut self) {
        match self.project.path.clone() {
            Some(p) => self.save_as_path(p),
            None => self.save_as_dialog(),
        }
    }

    pub fn save_as_dialog(&mut self) {
        let name = self
            .project
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Untitled.rcad".into(),
                |n| n.to_string_lossy().into_owned(),
            );
        if let Some(mut path) = rfd::FileDialog::new()
            .add_filter("robocad project", &[project::EXTENSION])
            .set_file_name(name)
            .save_file()
        {
            if path.extension().is_none() {
                path.set_extension(project::EXTENSION);
            }
            self.save_as_path(path);
        }
    }

    /// Saves in the background; the document is marked saved at the revision captured now.
    pub fn save_as_path(&mut self, path: PathBuf) {
        let doc = self.editor.doc().clone();
        let rev = self.editor.revision();
        let view = self.view_state();
        let thumb = self.thumbnail();
        self.project.path = Some(path.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = project::save(&doc, view, thumb.as_deref(), &path).map_err(|e| e.to_string());
            let _ = tx.send((rev, path, r));
        });
        self.project.saving = Some(rx);
    }

    /// Saves synchronously (used before quitting and by tests).
    pub fn save_blocking(&mut self, path: &Path) -> Result<(), String> {
        let thumb = self.thumbnail();
        project::save(self.editor.doc(), self.view_state(), thumb.as_deref(), path)
            .map_err(|e| e.to_string())?;
        self.project.path = Some(path.to_path_buf());
        self.editor.mark_saved();
        self.settings.add_recent(path);
        self.discard_autosave();
        Ok(())
    }

    pub fn project_path(&self) -> Option<&Path> {
        self.project.path.as_deref()
    }

    // ------------------------------------------------------------------ autosave

    /// Writes an autosave if the document changed and the interval elapsed.
    pub(crate) fn maybe_autosave(&mut self) {
        let minutes = self.settings.autosave_minutes;
        if minutes == 0
            || !self.editor.is_dirty()
            || self.editor.revision() == self.project.autosaved_revision
            || self.project.autosaving.is_some()
            || self.project.last_autosave.elapsed() < Duration::from_secs(u64::from(minutes) * 60)
        {
            return;
        }
        self.autosave_now();
    }

    /// Writes the autosave file in the background now.
    pub fn autosave_now(&mut self) {
        let doc = self.editor.doc().clone();
        let view = self.view_state();
        let dirs = self.dirs.clone();
        let origin = self.project.path.clone();
        self.project.last_autosave = Instant::now();
        self.project.autosaved_revision = self.editor.revision();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dirs.data);
            if project::save(&doc, view, None, dirs.autosave_file()).is_ok() {
                let text = origin.map(|p| p.display().to_string()).unwrap_or_default();
                let _ = std::fs::write(dirs.autosave_origin(), text);
            }
            let _ = tx.send(());
        });
        self.project.autosaving = Some(rx);
    }

    /// Waits for a running autosave (tests).
    pub fn wait_for_autosave(&mut self) {
        if let Some(rx) = self.project.autosaving.take() {
            let _ = rx.recv();
        }
    }

    pub(crate) fn discard_autosave(&mut self) {
        let _ = std::fs::remove_file(self.dirs.autosave_file());
        let _ = std::fs::remove_file(self.dirs.autosave_origin());
    }

    pub fn recovery_offered(&self) -> bool {
        self.project.recovery_offer
    }

    /// Restores the autosaved document (keeping it dirty so the user saves it).
    pub fn restore_autosave(&mut self) -> Result<(), String> {
        let proj = project::load(self.dirs.autosave_file()).map_err(|e| e.to_string())?;
        let origin = std::fs::read_to_string(self.dirs.autosave_origin())
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        self.reset_with(proj.document, origin);
        // Mark as unsaved: the recovered state is newer than any file on disk.
        self.editor.mark_saved_at(u64::MAX);
        self.project.recovery_offer = false;
        self.frame_all();
        Ok(())
    }

    // ------------------------------------------------------------------ dialogs

    pub(crate) fn project_dialogs(&mut self, ctx: &egui::Context) {
        // The OS close button: ask first when there are unsaved changes.
        if ctx.input(|i| i.viewport().close_requested()) && !self.project.allow_close {
            if self.editor.is_dirty() && !self.editor.doc().nodes.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.project.prompt = Some(AfterPrompt::Quit);
            } else {
                self.discard_autosave();
            }
        }

        if let Some(action) = self.project.prompt.clone() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
                ui.heading("Unsaved changes");
                ui.label(format!(
                    "Do you want to save the changes to “{}”?",
                    self.window_title()
                        .trim_end_matches(" — robocad")
                        .trim_end_matches(" •")
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(3);
                    }
                });
            });
            match choice {
                Some(1) => {
                    let path = self.project.path.clone().or_else(|| {
                        rfd::FileDialog::new()
                            .add_filter("robocad project", &[project::EXTENSION])
                            .set_file_name("Untitled.rcad")
                            .save_file()
                    });
                    if let Some(p) = path {
                        match self.save_blocking(&p) {
                            Ok(()) => {
                                self.project.prompt = None;
                                self.run_action(action, ctx);
                            }
                            Err(e) => {
                                self.project.prompt = None;
                                self.messages.push(format!("Saving failed: {e}"));
                            }
                        }
                    }
                }
                Some(2) => {
                    self.project.prompt = None;
                    self.run_action(action, ctx);
                }
                Some(_) => self.project.prompt = None,
                None => {}
            }
        }

        if self.project.recovery_offer {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
                ui.heading("Restore unsaved work?");
                ui.label("robocad did not shut down properly last time. An automatic backup of your work is available.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Restore").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    if let Err(e) = self.restore_autosave() {
                        self.messages
                            .push(format!("Could not restore the backup: {e}"));
                        self.project.recovery_offer = false;
                    }
                }
                Some(false) => {
                    self.project.recovery_offer = false;
                    self.discard_autosave();
                }
                None => {}
            }
        }

        if self.project.show_preferences {
            self.preferences_window(ctx);
        }
    }

    fn preferences_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Preferences")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("prefs")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Theme");
                        ui.horizontal(|ui| {
                            for (t, l) in [
                                (Theme::System, "System"),
                                (Theme::Light, "Light"),
                                (Theme::Dark, "Dark"),
                            ] {
                                if ui.radio_value(&mut self.settings.theme, t, l).changed() {
                                    ui.ctx().set_theme(match t {
                                        Theme::System => egui::ThemePreference::System,
                                        Theme::Light => egui::ThemePreference::Light,
                                        Theme::Dark => egui::ThemePreference::Dark,
                                    });
                                }
                            }
                        });
                        ui.end_row();
                        ui.label("UI scale");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.settings.ui_scale, 0.75..=2.0)
                                    .step_by(0.05),
                            )
                            .drag_stopped()
                        {
                            ui.ctx().set_zoom_factor(self.settings.ui_scale);
                        }
                        ui.end_row();
                        ui.label("Mouse");
                        if let Some(vp) = self.viewport.as_mut() {
                            egui::ComboBox::from_id_salt("mouse")
                                .selected_text(vp.mouse.label())
                                .show_ui(ui, |ui| {
                                    for m in MousePreset::ALL {
                                        ui.selectable_value(&mut vp.mouse, m, m.label());
                                    }
                                });
                        }
                        ui.end_row();
                        ui.label("New projects: plate");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.settings.plate_size[0])
                                    .range(10.0..=2000.0)
                                    .suffix(" mm"),
                            );
                            ui.label("×");
                            ui.add(
                                egui::DragValue::new(&mut self.settings.plate_size[1])
                                    .range(10.0..=2000.0)
                                    .suffix(" mm"),
                            );
                        });
                        ui.end_row();
                        ui.label("New projects: grid");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.grid_step)
                                .range(0.01..=100.0)
                                .speed(0.1)
                                .suffix(" mm"),
                        );
                        ui.end_row();
                        ui.label("Autosave every");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.autosave_minutes)
                                .range(0..=60)
                                .suffix(" min"),
                        )
                        .on_hover_text("0 turns autosave off");
                        ui.end_row();
                    });
            });
        if !open {
            self.project.show_preferences = false;
        }
    }

    pub(crate) fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let recent = self.settings.recent_files.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Open recent", |ui| {
                for p in recent {
                    let label = p.file_name().map_or_else(
                        || p.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    if ui
                        .button(label)
                        .on_hover_text(p.display().to_string())
                        .clicked()
                    {
                        ui.close();
                        let ctx = ui.ctx().clone();
                        self.open_path(p, &ctx);
                    }
                }
                ui.separator();
                if ui.button("Clear list").clicked() {
                    self.settings.recent_files.clear();
                    ui.close();
                }
            });
        });
    }
}
