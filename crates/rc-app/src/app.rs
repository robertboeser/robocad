//! Main application window: state, layout, menus, shortcuts, picking and file handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Pos2, Rect};
use glam::DVec2;
use rc_doc::{Command, Editor, NodeId};
use rc_mesh::{Bvh, MeshInfo, TopologyReport};
use rc_render::StandardView;

use crate::loader::{LoadedMesh, Loader};
use crate::viewport::Viewport;

/// Default object colours (sRGB), cycled for new objects.
pub const PALETTE: [[u8; 3]; 6] = [
    [74, 144, 226],
    [80, 190, 120],
    [240, 170, 60],
    [200, 100, 200],
    [90, 200, 210],
    [230, 100, 90],
];

/// A menu action on the app.
type Action = fn(&mut RobocadApp);

/// Derived per-mesh data that is not part of the document.
pub struct MeshExtras {
    pub bvh: Arc<Bvh>,
    pub topology: TopologyReport,
    pub info: MeshInfo,
    pub path: Option<PathBuf>,
    /// Fixes applied automatically on import, or by the last repair.
    pub notes: Vec<String>,
}

/// Units an imported STL is assumed to be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ImportUnits {
    #[default]
    Millimetres,
    Centimetres,
    Metres,
    Inches,
}

impl ImportUnits {
    pub const ALL: [Self; 4] = [
        Self::Millimetres,
        Self::Centimetres,
        Self::Metres,
        Self::Inches,
    ];

    pub fn factor(self) -> f64 {
        match self {
            Self::Millimetres => 1.0,
            Self::Centimetres => 10.0,
            Self::Metres => 1000.0,
            Self::Inches => 25.4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "Millimetres",
            Self::Centimetres => "Centimetres",
            Self::Metres => "Metres",
            Self::Inches => "Inches",
        }
    }
}

/// A finished background repair.
pub(crate) struct RepairDone {
    node: NodeId,
    name: String,
    loaded: LoadedMesh,
    report: rc_mesh::repair::RepairReport,
}

pub struct RobocadApp {
    pub editor: Editor,
    pub extras: HashMap<u64, MeshExtras>,
    pub(crate) viewport: Option<Viewport>,
    loader: Loader,
    pub(crate) messages: Vec<String>,
    last_load_ms: Option<f64>,
    pub(crate) show_edges: bool,
    pub(crate) hovered: Option<NodeId>,
    /// Viewport-local start of a box selection drag.
    pub(crate) box_start: Option<Pos2>,
    pub(crate) gizmo: crate::gizmo::Gizmo,
    /// Whether the current gizmo drag changed anything (for Esc = cancel).
    pub(crate) gizmo_changed: bool,
    pub snapping: bool,
    /// Meshes generated for primitives, keyed by display key.
    pub(crate) prim_meshes: HashMap<u64, Arc<rc_mesh::Mesh>>,
    pub(crate) evaluator: rc_doc::eval::Evaluator,
    pub(crate) groups: crate::scene_view::GroupResults,
    /// What the viewport shows this frame.
    pub(crate) items: Vec<crate::scene_view::Item>,
    pub(crate) exports: Vec<std::sync::mpsc::Receiver<Result<String, String>>>,
    pub(crate) repairs: Vec<std::sync::mpsc::Receiver<RepairDone>>,
    pub import_units: ImportUnits,
    pub place_on_plate: bool,
    pub show_overhangs: bool,
    pub(crate) dirs: crate::settings::AppDirs,
    pub settings: crate::settings::Settings,
    pub(crate) settings_saved: crate::settings::Settings,
    pub(crate) project: crate::project_ui::ProjectState,
    pub scene_ui: crate::scene_panel::SceneUiState,
    pub props_ui: crate::properties::PropsUiState,
    color_index: usize,
    title: String,
    pub(crate) last_viewport_rect: Option<Rect>,
}

impl RobocadApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        Self::with_dirs(cc, files, crate::settings::AppDirs::system())
    }

    /// Creates the app with explicit settings/data directories (tests use temporary ones).
    pub fn with_dirs(
        cc: &eframe::CreationContext<'_>,
        files: Vec<PathBuf>,
        dirs: crate::settings::AppDirs,
    ) -> Self {
        let settings = crate::settings::Settings::load(&dirs.config);
        let recovery = dirs.autosave_file().exists();
        let viewport = cc.wgpu_render_state.clone().map(Viewport::new);
        let mut app = Self {
            editor: Editor::default(),
            extras: HashMap::new(),
            viewport,
            loader: Loader::default(),
            messages: Vec::new(),
            last_load_ms: None,
            show_edges: true,
            hovered: None,
            box_start: None,
            gizmo: crate::gizmo::Gizmo::default(),
            gizmo_changed: false,
            snapping: true,
            prim_meshes: HashMap::new(),
            evaluator: {
                let ctx = cc.egui_ctx.clone();
                rc_doc::eval::Evaluator::new(move || ctx.request_repaint())
            },
            groups: crate::scene_view::GroupResults::default(),
            items: Vec::new(),
            exports: Vec::new(),
            repairs: Vec::new(),
            import_units: ImportUnits::default(),
            place_on_plate: true,
            show_overhangs: false,
            settings_saved: settings.clone(),
            settings,
            dirs,
            project: crate::project_ui::ProjectState::new(recovery),
            scene_ui: crate::scene_panel::SceneUiState::default(),
            props_ui: crate::properties::PropsUiState::default(),
            color_index: 0,
            title: String::new(),
            last_viewport_rect: None,
        };
        app.apply_settings(&cc.egui_ctx);
        for f in files {
            app.open_path(f, &cc.egui_ctx);
        }
        app
    }

    pub fn next_color(&mut self) -> [u8; 3] {
        let c = PALETTE[self.color_index % PALETTE.len()];
        self.color_index += 1;
        c
    }

    /// Applies a command and reports errors to the user.
    pub fn apply(&mut self, cmd: Command) {
        if let Err(e) = self.editor.apply(cmd) {
            self.messages.push(e.to_string());
        }
    }

    pub fn apply_merged(&mut self, cmd: Command, key: u64) {
        if let Err(e) = self.editor.apply_merged(cmd, key) {
            self.messages.push(e.to_string());
        }
    }

    /// Imports a mesh file in the background.
    pub fn import_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.loader.load(path, move || ctx.request_repaint());
    }

    /// Open dialog for projects and STL files.
    pub(crate) fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("robocad projects and STL files", &["rcad", "stl", "STL"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.open_path(path, ctx);
        }
    }

    /// Import dialog for STL files (several at once).
    pub(crate) fn import_dialog(&mut self, ctx: &egui::Context) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("STL files", &["stl", "STL"])
            .add_filter("All files", &["*"])
            .pick_files()
        {
            for p in paths {
                self.import_path(p, ctx);
            }
        }
    }

    pub(crate) fn register_analysis(&mut self, loaded: &LoadedMesh) {
        self.register_loaded(loaded, Vec::new());
    }

    fn receive_loads(&mut self) {
        for result in self.loader.poll() {
            match result {
                Ok(loaded) => self.add_loaded(loaded),
                Err((path, err)) => self
                    .messages
                    .push(format!("Could not open {}: {err}", path.display())),
            }
        }
    }

    /// Loads a file synchronously (used by tests and scripted runs).
    pub fn import_blocking(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let loaded = crate::loader::load_file(path.into()).map_err(|(_, e)| e)?;
        self.add_loaded(loaded);
        Ok(())
    }

    /// Screen position (points) of the centre of a node's world bounds, if on screen.
    pub fn screen_pos_of(&self, id: NodeId) -> Option<Pos2> {
        let vp = self.viewport.as_ref()?;
        let rect = self.last_viewport_rect?;
        let c = self.editor.doc().world_bounds(id).center().as_dvec3();
        let size = DVec2::new(f64::from(rect.width()), f64::from(rect.height()));
        let p = vp.camera.project(c, size)?;
        Some(rect.min + egui::vec2(p.x as f32, p.y as f32))
    }

    /// The viewport's screen rectangle from the last frame.
    pub fn last_rect_for_tests(&self) -> Option<Rect> {
        self.last_viewport_rect
    }

    pub fn has_viewport(&self) -> bool {
        self.viewport.is_some()
    }

    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    /// Uploads a loaded mesh and records its derived data.
    fn register_loaded(&mut self, loaded: &LoadedMesh, notes: Vec<String>) -> u64 {
        let key = loaded.mesh.content_hash();
        if let Some(vp) = self.viewport.as_mut()
            && !vp.renderer().has_mesh(key)
        {
            let device = vp.device().clone();
            vp.renderer_mut()
                .upload_mesh(&device, key, &loaded.mesh, &loaded.edges);
        }
        self.extras.insert(
            key,
            MeshExtras {
                bvh: loaded.bvh.clone(),
                topology: loaded.topology,
                info: loaded.info,
                path: Some(loaded.path.clone()),
                notes,
            },
        );
        key
    }

    fn add_loaded(&mut self, loaded: LoadedMesh) {
        self.register_loaded(&loaded, loaded.auto_fixes.clone());
        for w in &loaded.warnings {
            self.messages.push(format!("{}: {w}", loaded.name));
        }
        self.last_load_ms = Some(loaded.elapsed.as_secs_f64() * 1e3);
        let color = self.next_color();
        let mut transform = rc_doc::Transform::IDENTITY;
        transform.scale = glam::DVec3::splat(self.import_units.factor());
        if self.place_on_plate {
            let b = loaded.info.aabb;
            let f = self.import_units.factor();
            let c = b.center().as_dvec3() * f;
            // Adding 0.0 turns -0.0 into 0.0 so the fields do not show "-0.0".
            transform.translation =
                glam::DVec3::new(-c.x, -c.y, -f64::from(b.min.z) * f) + glam::DVec3::ZERO;
        }
        self.apply(Command::Import {
            name: loaded.name,
            mesh: loaded.mesh,
            transform,
            color,
        });
        self.frame_all();
    }

    /// Repairs an imported mesh in the background; the result replaces it (undoable).
    pub fn start_repair(&mut self, node: NodeId) {
        let Some(n) = self.editor.doc().get(node) else {
            return;
        };
        let rc_doc::NodeKind::Imported { mesh, .. } = n.kind else {
            return;
        };
        let Some(m) = self.editor.doc().mesh(mesh).cloned() else {
            return;
        };
        let name = n.name.clone();
        let path = self
            .extras
            .get(&mesh)
            .and_then(|e| e.path.clone())
            .unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let (fixed, report) =
                rc_mesh::repair::repair(&m, &rc_mesh::repair::RepairOptions::FULL);
            let loaded = crate::loader::analyse(path, name.clone(), fixed, start);
            let _ = tx.send(RepairDone {
                node,
                name,
                loaded,
                report,
            });
        });
        self.repairs.push(rx);
    }

    /// True while a repair is running.
    pub fn repairing(&self) -> bool {
        !self.repairs.is_empty()
    }

    fn receive_repairs(&mut self) {
        let mut finished = Vec::new();
        self.repairs.retain(|rx| match rx.try_recv() {
            Ok(done) => {
                finished.push(done);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for done in finished {
            let mut lines = done.report.summary();
            if lines.is_empty() {
                self.messages
                    .push(format!("{}: nothing to repair.", done.name));
                continue;
            }
            let after = done.report.after;
            let status = if after.is_clean_solid() {
                "The mesh is now watertight. ✔".to_string()
            } else {
                format!(
                    "Remaining problems: {} open edges, {} non-manifold edges.",
                    after.boundary_edges, after.non_manifold_edges
                )
            };
            self.register_loaded(&done.loaded, lines.clone());
            self.apply(Command::ReplaceMesh {
                node: done.node,
                mesh: done.loaded.mesh,
            });
            lines.insert(0, format!("Repaired {}:", done.name));
            lines.push(status);
            self.messages
                .push(lines.join("\n• ").replacen("\n• ", "\n", 1));
        }
    }

    pub(crate) fn frame_all(&mut self) {
        let b = self.editor.doc().scene_bounds();
        if let Some(vp) = self.viewport.as_mut() {
            vp.camera.frame(&b, 1.6);
        }
    }

    pub(crate) fn frame_selection(&mut self) {
        let doc = self.editor.doc();
        let b = self
            .editor
            .selection
            .ids()
            .iter()
            .fold(rc_mesh::Aabb::EMPTY, |b, id| {
                b.union(&doc.world_bounds(*id))
            });
        if b.is_empty() {
            self.frame_all();
        } else if let Some(vp) = self.viewport.as_mut() {
            vp.camera.frame(&b, 1.6);
        }
    }

    /// Asks for a file name and exports the scene (or the selection) in the background.
    pub(crate) fn export_dialog(&mut self, selection_only: bool) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("STL file", &["stl"])
            .set_file_name("model.stl")
            .save_file()
        else {
            return;
        };
        self.export_to(path, selection_only);
    }

    /// Exports the visible scene (or selection) to an STL file on a worker thread.
    pub fn export_to(&mut self, path: PathBuf, selection_only: bool) {
        let doc = self.editor.doc().clone();
        let ids = if selection_only {
            self.editor.selection.ids().to_vec()
        } else {
            doc.roots.clone()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut cache = rc_doc::eval::SolidCache::default();
            let r = rc_doc::eval::export_mesh(&doc, &ids, &mut cache);
            let result = if r.mesh.is_empty() {
                Err("Nothing to export: no visible solid objects.".to_string())
            } else {
                rc_mesh::stl::write_file(&r.mesh, &path, rc_mesh::stl::StlFormat::Binary, "robocad")
                    .map(|()| {
                        let mut msg = format!(
                            "Exported {} triangles to {}",
                            r.mesh.triangle_count(),
                            path.display()
                        );
                        let topo = rc_mesh::topology::analyze(&r.mesh);
                        if !topo.is_watertight() {
                            msg.push_str("\nWarning: the exported mesh is not watertight.");
                        }
                        for w in r.warnings {
                            msg.push_str("\nWarning: ");
                            msg.push_str(&w);
                        }
                        msg
                    })
                    .map_err(|e| format!("Export failed: {e}"))
            };
            let _ = tx.send(result);
        });
        self.exports.push(rx);
    }

    fn receive_exports(&mut self) {
        let mut done = Vec::new();
        for (i, rx) in self.exports.iter().enumerate() {
            if let Ok(r) = rx.try_recv() {
                self.messages.push(match r {
                    Ok(m) | Err(m) => m,
                });
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            self.exports.remove(i);
        }
    }

    /// True while an export is running.
    pub fn exporting(&self) -> bool {
        !self.exports.is_empty()
    }

    pub fn set_plate_size(&mut self, size: [f64; 2]) {
        self.editor.set_settings(rc_doc::DocSettings {
            plate_size: size,
            ..self.editor.doc().settings.clone()
        });
    }

    pub fn delete_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Delete(ids));
        }
    }

    pub fn duplicate_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Duplicate(ids));
        }
    }

    pub fn group_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Group(ids));
        }
    }

    pub fn ungroup_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Ungroup(ids));
        }
    }

    pub fn toggle_hole_selection(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::ToggleRole(ids));
        }
    }

    pub(crate) fn select_all(&mut self) {
        let roots = self.editor.doc().roots.clone();
        self.editor.selection.set(roots);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut as KS, Modifiers as M};
        let cmd = M::COMMAND;
        let cmd_shift = M::COMMAND | M::SHIFT;
        // Shortcuts with a modifier work even while a text field has focus, except editing ones.
        let text_focus = ctx.egui_wants_keyboard_input();
        let pressed = |ks: KS| ctx.input_mut(|i| i.consume_shortcut(&ks));
        if pressed(KS::new(cmd, Key::O)) {
            self.open_dialog(ctx);
        }
        if pressed(KS::new(cmd, Key::I)) {
            self.import_dialog(ctx);
        }
        if pressed(KS::new(cmd_shift, Key::S)) {
            self.save_as_dialog();
        }
        if pressed(KS::new(cmd, Key::S)) {
            self.save();
        }
        if pressed(KS::new(cmd, Key::N)) {
            self.request(crate::project_ui::AfterPrompt::New, ctx);
        }
        if pressed(KS::new(cmd, Key::Q)) {
            self.request(crate::project_ui::AfterPrompt::Quit, ctx);
        }
        if pressed(KS::new(cmd, Key::Comma)) {
            self.project.show_preferences = true;
        }
        if text_focus {
            return;
        }
        if pressed(KS::new(cmd_shift, Key::Z)) || pressed(KS::new(cmd, Key::Y)) {
            self.editor.redo();
        }
        if pressed(KS::new(cmd, Key::Z)) {
            self.editor.undo();
        }
        if pressed(KS::new(cmd_shift, Key::G)) {
            self.ungroup_selection();
        }
        if pressed(KS::new(cmd, Key::G)) {
            self.group_selection();
        }
        if pressed(KS::new(cmd, Key::D)) {
            self.duplicate_selection();
        }
        if pressed(KS::new(cmd, Key::A)) {
            self.select_all();
        }
        let plain = |key: Key| ctx.input_mut(|i| i.consume_key(M::NONE, key));
        if plain(Key::Delete) || plain(Key::Backspace) {
            self.delete_selection();
        }
        if plain(Key::Escape) {
            if self.gizmo.is_dragging() {
                self.cancel_gizmo_drag();
            } else {
                self.editor.selection.clear();
            }
        }
        if plain(Key::W) {
            self.gizmo.mode = crate::gizmo::GizmoMode::Move;
        }
        if plain(Key::E) {
            self.gizmo.mode = crate::gizmo::GizmoMode::Rotate;
        }
        if plain(Key::R) {
            self.gizmo.mode = crate::gizmo::GizmoMode::Scale;
        }
        if plain(Key::D) {
            self.drop_selection_to_plate();
        }
        self.handle_nudge(ctx);
        if plain(Key::H) {
            self.toggle_hole_selection();
        }
        if plain(Key::F) {
            self.frame_selection();
        }
        if ctx.input_mut(|i| i.consume_key(M::SHIFT, Key::F)) {
            self.frame_all();
        }
        let mut view = None;
        for (key, v) in [
            (Key::Num1, StandardView::Front),
            (Key::Num3, StandardView::Right),
            (Key::Num7, StandardView::Top),
            (Key::Num0, StandardView::Iso),
        ] {
            if plain(key) {
                view = Some(v);
            }
        }
        let toggle = plain(Key::Num5);
        if let Some(vp) = self.viewport.as_mut() {
            if let Some(v) = view {
                vp.camera.set_view(v);
            }
            if toggle {
                vp.toggle_projection();
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for p in dropped {
            self.open_path(p, ctx);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                let ctx = ui.ctx().clone();
                if ui.button("New  ⌘N").clicked() {
                    ui.close();
                    self.request(crate::project_ui::AfterPrompt::New, &ctx);
                }
                if ui.button("Open…  ⌘O").clicked() {
                    ui.close();
                    self.open_dialog(&ctx);
                }
                self.recent_menu(ui);
                ui.separator();
                if ui.button("Save  ⌘S").clicked() {
                    ui.close();
                    self.save();
                }
                if ui.button("Save as…  ⇧⌘S").clicked() {
                    ui.close();
                    self.save_as_dialog();
                }
                ui.separator();
                if ui.button("Import STL…  ⌘I").clicked() {
                    ui.close();
                    self.import_dialog(&ctx);
                }
                ui.menu_button("Import options", |ui| {
                    ui.label("Units of imported STL files");
                    for u in ImportUnits::ALL {
                        ui.radio_value(&mut self.import_units, u, u.label());
                    }
                    ui.separator();
                    ui.checkbox(&mut self.place_on_plate, "Place on the plate after import");
                });
                if ui.button("Export STL…").clicked() {
                    ui.close();
                    self.export_dialog(false);
                }
                if ui
                    .add_enabled(
                        !self.editor.selection.is_empty(),
                        egui::Button::new("Export selection as STL…"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.export_dialog(true);
                }
                ui.separator();
                if ui.button("Preferences…  ⌘,").clicked() {
                    ui.close();
                    self.project.show_preferences = true;
                }
                ui.separator();
                if ui.button("Quit  ⌘Q").clicked() {
                    ui.close();
                    self.request(crate::project_ui::AfterPrompt::Quit, &ctx);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self
                    .editor
                    .undo_label()
                    .map_or("Undo".to_string(), |l| format!("Undo {l}"));
                if ui
                    .add_enabled(
                        self.editor.can_undo(),
                        egui::Button::new(format!("{undo}  ⌘Z")),
                    )
                    .clicked()
                {
                    self.editor.undo();
                    ui.close();
                }
                let redo = self
                    .editor
                    .redo_label()
                    .map_or("Redo".to_string(), |l| format!("Redo {l}"));
                if ui
                    .add_enabled(
                        self.editor.can_redo(),
                        egui::Button::new(format!("{redo}  ⇧⌘Z")),
                    )
                    .clicked()
                {
                    self.editor.redo();
                    ui.close();
                }
                ui.separator();
                let has_sel = !self.editor.selection.is_empty();
                let items: [(&str, Action); 5] = [
                    ("Duplicate  ⌘D", Self::duplicate_selection),
                    ("Delete  ⌫", Self::delete_selection),
                    ("Group  ⌘G", Self::group_selection),
                    ("Ungroup  ⇧⌘G", Self::ungroup_selection),
                    ("Toggle solid / hole  H", Self::toggle_hole_selection),
                ];
                for (label, f) in items {
                    if ui.add_enabled(has_sel, egui::Button::new(label)).clicked() {
                        f(self);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Select all  ⌘A").clicked() {
                    self.select_all();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Frame selection  F").clicked() {
                    self.frame_selection();
                    ui.close();
                }
                if ui.button("Frame all  ⇧F").clicked() {
                    self.frame_all();
                    ui.close();
                }
                ui.checkbox(&mut self.show_edges, "Show edges");
                ui.checkbox(&mut self.show_overhangs, "Show overhangs (> 45°)");
                ui.separator();
                let views = [
                    ("Front  1", StandardView::Front),
                    ("Right  3", StandardView::Right),
                    ("Top  7", StandardView::Top),
                    ("Isometric  0", StandardView::Iso),
                ];
                for (label, v) in views {
                    if ui.button(label).clicked() {
                        if let Some(vp) = self.viewport.as_mut() {
                            vp.camera.set_view(v);
                        }
                        ui.close();
                    }
                }
                if ui.button("Perspective / Ortho  5").clicked() {
                    if let Some(vp) = self.viewport.as_mut() {
                        vp.toggle_projection();
                    }
                    ui.close();
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let doc = self.editor.doc();
            let tris = self.drawn_triangles();
            ui.label(format!("{} objects · {tris} triangles", doc.nodes.len()));
            let sel = self.editor.selection.ids();
            if !sel.is_empty() {
                let b = sel.iter().fold(rc_mesh::Aabb::EMPTY, |b, id| {
                    b.union(&doc.world_bounds(*id))
                });
                let s = b.size();
                ui.separator();
                ui.label(format!(
                    "{} selected · {:.2} × {:.2} × {:.2} mm",
                    sel.len(),
                    s.x,
                    s.y,
                    s.z
                ));
            }
            if !self.evaluation_idle() {
                ui.separator();
                ui.spinner();
                ui.label("Combining…");
            } else if let Some(ms) = self.groups.last_eval_ms {
                ui.separator();
                ui.label(format!("combined in {ms:.0} ms"));
            }
            if self.loader.pending() > 0 {
                ui.separator();
                ui.spinner();
                ui.label("Loading…");
            } else if let Some(ms) = self.last_load_ms {
                ui.separator();
                ui.label(format!("loaded in {ms:.0} ms"));
            }
            if let Some(vp) = &self.viewport {
                ui.separator();
                ui.label(format!(
                    "GPU {:.1} MB",
                    vp.renderer().mesh_bytes() as f64 / 1_048_576.0
                ));
            }
        });
    }

    fn messages_window(&mut self, ctx: &egui::Context) {
        if self.messages.is_empty() {
            return;
        }
        let mut close = false;
        egui::Window::new("Messages")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
            .show(ctx, |ui| {
                for m in &self.messages {
                    ui.label(m);
                }
                close = ui.button("OK").clicked();
            });
        if close {
            self.messages.clear();
        }
    }
}

impl eframe::App for RobocadApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_loads();
        self.receive_evaluations();
        self.receive_exports();
        self.receive_repairs();
        self.receive_project_io();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        // A continuous edit ends when no widget is being dragged or edited.
        if !ctx.egui_is_using_pointer() && ctx.memory(|m| m.focused().is_none()) {
            self.editor.end_merge();
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("scene")
            .resizable(true)
            .default_size(230.0)
            .show(ui, |ui| crate::scene_panel::show(self, ui));
        egui::Panel::right("properties")
            .resizable(true)
            .default_size(270.0)
            .show(ui, |ui| crate::properties::show(self, ui));
        self.messages_window(&ctx);
        self.project_dialogs(&ctx);
        self.maybe_autosave();
        self.persist_settings();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui));
        let title = self.window_title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
