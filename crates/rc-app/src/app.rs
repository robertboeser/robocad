//! Main application window: state, layout, menus, shortcuts, picking and file handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Pos2, Rect};
use glam::DVec2;
use rc_doc::{Command, Editor, NodeId, Role};
use rc_mesh::{Bvh, MeshInfo, Ray, TopologyReport};
use rc_render::{DrawObject, DrawStyle, SceneStyle, StandardView, srgb_to_linear};

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
}

pub struct RobocadApp {
    pub editor: Editor,
    pub extras: HashMap<u64, MeshExtras>,
    viewport: Option<Viewport>,
    loader: Loader,
    messages: Vec<String>,
    last_load_ms: Option<f64>,
    show_edges: bool,
    hovered: Option<NodeId>,
    /// Viewport-local start of a box selection drag.
    box_start: Option<Pos2>,
    pub scene_ui: crate::scene_panel::SceneUiState,
    pub props_ui: crate::properties::PropsUiState,
    color_index: usize,
    title: String,
    last_viewport_rect: Option<Rect>,
}

impl RobocadApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::System);
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
            scene_ui: crate::scene_panel::SceneUiState::default(),
            props_ui: crate::properties::PropsUiState::default(),
            color_index: 0,
            title: String::new(),
            last_viewport_rect: None,
        };
        for f in files {
            app.open(f, &cc.egui_ctx);
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

    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.loader.load(path, move || ctx.request_repaint());
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("STL files", &["stl", "STL"])
            .add_filter("All files", &["*"])
            .pick_files()
        {
            for p in paths {
                self.open(p, ctx);
            }
        }
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

    fn add_loaded(&mut self, loaded: LoadedMesh) {
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
                bvh: loaded.bvh,
                topology: loaded.topology,
                info: loaded.info,
                path: Some(loaded.path),
            },
        );
        for w in &loaded.warnings {
            self.messages.push(format!("{}: {w}", loaded.name));
        }
        self.last_load_ms = Some(loaded.elapsed.as_secs_f64() * 1e3);
        let color = self.next_color();
        self.apply(Command::Import {
            name: loaded.name,
            mesh: loaded.mesh,
            transform: rc_doc::Transform::IDENTITY,
            color,
        });
        self.frame_all();
    }

    fn frame_all(&mut self) {
        let b = self.editor.doc().scene_bounds();
        if let Some(vp) = self.viewport.as_mut() {
            vp.camera.frame(&b, 1.6);
        }
    }

    fn frame_selection(&mut self) {
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

    fn select_all(&mut self) {
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
            self.editor.selection.clear();
        }
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
            self.open(p, ctx);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Import STL…  ⌘O").clicked() {
                    ui.close();
                    self.open_dialog(ui.ctx());
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
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
            let tris: usize = doc
                .nodes
                .values()
                .filter_map(|n| match n.kind {
                    rc_doc::NodeKind::Imported { mesh, .. } => doc.mesh(mesh),
                    rc_doc::NodeKind::Group { .. } => None,
                })
                .map(|m| m.triangle_count())
                .sum::<usize>();
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

    /// The root-level ancestor of a node (what a viewport click selects).
    fn root_of(&self, id: NodeId) -> NodeId {
        let doc = self.editor.doc();
        let mut cur = id;
        while let Some(p) = doc.get(cur).and_then(|n| n.parent) {
            cur = p;
        }
        cur
    }

    /// Visible, unlocked leaf under a viewport pixel and the hit distance.
    fn pick(&self, pixel: DVec2, size: DVec2) -> Option<NodeId> {
        let vp = self.viewport.as_ref()?;
        let (origin, dir) = vp.camera.ray(pixel, size);
        let doc = self.editor.doc();
        let mut best: Option<(f64, NodeId)> = None;
        for (id, node) in &doc.nodes {
            let rc_doc::NodeKind::Imported { mesh, .. } = node.kind else {
                continue;
            };
            if !doc.is_effectively_visible(id) || doc.is_effectively_locked(id) {
                continue;
            }
            let (Some(m), Some(extra)) = (doc.mesh(mesh), self.extras.get(&mesh)) else {
                continue;
            };
            let world = doc.world_matrix(id);
            let inv = world.inverse();
            let lo = inv.transform_point3(origin);
            let ld = inv.transform_vector3(dir);
            let ray = Ray::new(lo.as_vec3(), ld.as_vec3());
            if let Some(hit) = extra.bvh.raycast(m, &ray, f32::INFINITY) {
                let wp = world.transform_point3(hit.point.as_dvec3());
                let dist = (wp - origin).dot(dir);
                if best.is_none_or(|(d, _)| dist < d) {
                    best = Some((dist, id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// Root nodes whose projected bounds lie inside a viewport rectangle.
    fn box_select(&self, rect: Rect, size: DVec2) -> Vec<NodeId> {
        let Some(vp) = self.viewport.as_ref() else {
            return Vec::new();
        };
        let doc = self.editor.doc();
        doc.roots
            .iter()
            .copied()
            .filter(|id| doc.is_effectively_visible(*id) && !doc.is_effectively_locked(*id))
            .filter(|id| {
                let b = doc.world_bounds(*id);
                !b.is_empty()
                    && b.corners().iter().all(|c| {
                        vp.camera
                            .project(c.as_dvec3(), size)
                            .is_some_and(|p| rect.contains(Pos2::new(p.x as f32, p.y as f32)))
                    })
            })
            .collect()
    }

    fn draw_objects(&self) -> Vec<DrawObject> {
        let doc = self.editor.doc();
        let sel = &self.editor.selection;
        let in_set = |id: NodeId, f: &dyn Fn(NodeId) -> bool| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if f(c) {
                    return true;
                }
                cur = doc.get(c).and_then(|n| n.parent);
            }
            false
        };
        doc.nodes
            .iter()
            .filter_map(|(id, node)| {
                let rc_doc::NodeKind::Imported { mesh, .. } = node.kind else {
                    return None;
                };
                if !doc.is_effectively_visible(id) {
                    return None;
                }
                let hole = in_set(id, &|c| doc.get(c).is_some_and(|n| n.role == Role::Hole));
                let (color, style) = if hole {
                    ([0.35, 0.37, 0.40, 0.35], DrawStyle::Hole)
                } else {
                    let c = node.color.map(|v| srgb_to_linear(f32::from(v) / 255.0));
                    ([c[0], c[1], c[2], 1.0], DrawStyle::Solid)
                };
                Some(DrawObject {
                    mesh,
                    model: doc.world_matrix(id),
                    color,
                    style,
                    selected: in_set(id, &|c| sel.contains(c)),
                    hovered: self.hovered.is_some_and(|h| in_set(id, &|c| c == h)),
                })
            })
            .collect()
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        let objects = self.draw_objects();
        let mut style = if ui.visuals().dark_mode {
            SceneStyle::dark()
        } else {
            SceneStyle::light()
        };
        style.show_edges = self.show_edges;
        let plate = self.editor.doc().settings.plate_size;
        style.plate_size = [plate[0] as f32, plate[1] as f32];
        let Some(vp) = self.viewport.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label("No GPU renderer available (wgpu failed to initialise).");
            });
            return;
        };
        let out = vp.ui(ui, &objects, &style);
        self.last_viewport_rect = Some(out.rect);
        let origin = out.rect.min;
        let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));

        self.hovered = out
            .hover
            .and_then(|p| self.pick(p, out.viewport_size))
            .map(|id| self.root_of(id));

        // Box selection with the primary button on empty space.
        if out.drag_started_primary && !ui.input(|i| i.modifiers.alt) {
            let start = out.press_origin.unwrap_or_default();
            if self.pick(start, out.viewport_size).is_none() {
                self.box_start = Some(Pos2::new(start.x as f32, start.y as f32));
            }
        }
        if let Some(start) = self.box_start {
            if let Some(cur) = out.hover.or(out.pointer_local) {
                let r = Rect::from_two_pos(start, Pos2::new(cur.x as f32, cur.y as f32));
                let screen = r.translate(origin.to_vec2());
                let painter = ui.painter();
                let accent = ui.visuals().selection.bg_fill;
                painter.rect_filled(screen, 0.0, accent.gamma_multiply(0.15));
                painter.rect_stroke(
                    screen,
                    0.0,
                    egui::Stroke::new(1.0, accent),
                    egui::StrokeKind::Inside,
                );
                if out.drag_stopped_primary {
                    let picked = self.box_select(r, out.viewport_size);
                    if !(shift || command) {
                        self.editor.selection.clear();
                    }
                    for id in picked {
                        self.editor.selection.add(id);
                    }
                    self.box_start = None;
                }
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                self.box_start = None;
            }
        }

        if let Some(click) = out.clicked {
            let hit = self
                .pick(click, out.viewport_size)
                .map(|id| self.root_of(id));
            match hit {
                Some(id) if shift || command => self.editor.selection.toggle(id),
                Some(id) => self.editor.selection.set([id]),
                None if !(shift || command) => self.editor.selection.clear(),
                None => {}
            }
        }
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
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        // A continuous edit ends when no widget is being dragged or edited.
        if !ctx.egui_is_using_pointer() && ctx.memory(|m| m.focused().is_none()) {
            self.editor.end_merge();
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui));
        let title = format!("robocad{}", if self.editor.is_dirty() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
