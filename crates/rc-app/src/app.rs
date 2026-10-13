//! Main application window: layout, menus, panels and file handling.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, RichText};
use glam::DMat4;
use rc_mesh::{Aabb, Bvh, Mesh, MeshInfo, Ray, TopologyReport};
use rc_render::{DrawObject, DrawStyle, MeshKey, SceneStyle, StandardView, srgb_to_linear};

use crate::loader::{LoadedMesh, Loader};
use crate::viewport::Viewport;

/// Default object colours (sRGB).
const PALETTE: [[u8; 3]; 6] = [
    [74, 144, 226],
    [80, 190, 120],
    [240, 170, 60],
    [200, 100, 200],
    [90, 200, 210],
    [230, 100, 90],
];

struct Model {
    name: String,
    path: PathBuf,
    mesh: Arc<Mesh>,
    bvh: Arc<Bvh>,
    key: MeshKey,
    info: MeshInfo,
    topology: TopologyReport,
    color: [u8; 3],
    visible: bool,
}

pub struct RobocadApp {
    viewport: Option<Viewport>,
    loader: Loader,
    models: Vec<Model>,
    selected: Option<usize>,
    messages: Vec<String>,
    last_load_ms: Option<f64>,
    show_edges: bool,
}

impl RobocadApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::System);
        let viewport = cc.wgpu_render_state.clone().map(Viewport::new);
        let mut app = Self {
            viewport,
            loader: Loader::default(),
            models: Vec::new(),
            selected: None,
            messages: Vec::new(),
            last_load_ms: None,
            show_edges: true,
        };
        for f in files {
            app.open(f, &cc.egui_ctx);
        }
        app
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
                Ok(loaded) => self.add_model(loaded),
                Err((path, err)) => self
                    .messages
                    .push(format!("Could not open {}: {err}", path.display())),
            }
        }
    }

    fn add_model(&mut self, loaded: LoadedMesh) {
        let LoadedMesh {
            path,
            name,
            mesh,
            bvh,
            edges,
            info,
            topology,
            warnings,
            elapsed,
        } = loaded;
        let key = mesh.content_hash();
        if let Some(vp) = self.viewport.as_mut()
            && !vp.renderer().has_mesh(key)
        {
            let device = vp.device().clone();
            vp.renderer_mut().upload_mesh(&device, key, &mesh, &edges);
        }
        for w in warnings {
            self.messages.push(format!("{name}: {w}"));
        }
        self.last_load_ms = Some(elapsed.as_secs_f64() * 1e3);
        let color = PALETTE[self.models.len() % PALETTE.len()];
        self.models.push(Model {
            name,
            path,
            mesh,
            bvh,
            key,
            info,
            topology,
            color,
            visible: true,
        });
        self.selected = Some(self.models.len() - 1);
        self.frame_all();
    }

    fn scene_bounds(&self) -> Aabb {
        self.models
            .iter()
            .filter(|m| m.visible)
            .fold(Aabb::EMPTY, |b, m| b.union(&m.info.aabb))
    }

    fn frame_all(&mut self) {
        let b = self.scene_bounds();
        if let Some(vp) = self.viewport.as_mut() {
            vp.camera.frame(&b, 1.6);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (open, frame, views, toggle) = ctx.input_mut(|i| {
            let open = i.consume_key(egui::Modifiers::COMMAND, egui::Key::O);
            let frame = i.key_pressed(egui::Key::F);
            let mut view = None;
            for (key, v) in [
                (egui::Key::Num1, StandardView::Front),
                (egui::Key::Num3, StandardView::Right),
                (egui::Key::Num7, StandardView::Top),
                (egui::Key::Num0, StandardView::Iso),
            ] {
                if i.key_pressed(key) && !i.modifiers.any() {
                    view = Some(v);
                }
            }
            if i.key_pressed(egui::Key::Num1) && i.modifiers.ctrl {
                view = Some(StandardView::Back);
            }
            (open, frame, view, i.key_pressed(egui::Key::Num5))
        });
        if open {
            self.open_dialog(ctx);
        }
        if frame {
            self.frame_all();
        }
        if let Some(vp) = self.viewport.as_mut() {
            if let Some(v) = views {
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
                if ui.button("Open STL…").clicked() {
                    ui.close();
                    self.open_dialog(ui.ctx());
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Frame all (F)").clicked() {
                    self.frame_all();
                    ui.close();
                }
                ui.checkbox(&mut self.show_edges, "Show edges");
                ui.separator();
                let views = [
                    ("Front (1)", StandardView::Front),
                    ("Right (3)", StandardView::Right),
                    ("Top (7)", StandardView::Top),
                    ("Isometric (0)", StandardView::Iso),
                ];
                for (label, v) in views {
                    if ui.button(label).clicked() {
                        if let Some(vp) = self.viewport.as_mut() {
                            vp.camera.set_view(v);
                        }
                        ui.close();
                    }
                }
                if ui.button("Perspective / Ortho (5)").clicked() {
                    if let Some(vp) = self.viewport.as_mut() {
                        vp.toggle_projection();
                    }
                    ui.close();
                }
            });
        });
    }

    fn scene_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scene");
        ui.separator();
        if self.models.is_empty() {
            ui.label(RichText::new("Drop an STL file here\nor use File → Open (⌘O).").weak());
        }
        let mut remove = None;
        for (i, m) in self.models.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut m.visible, "");
                let warn = if m.topology.is_watertight() {
                    ""
                } else {
                    " ⚠"
                };
                let label = format!("{}{warn}", m.name);
                if ui
                    .selectable_label(self.selected == Some(i), label)
                    .clicked()
                {
                    self.selected = Some(i);
                }
                if ui.small_button("🗙").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.models.remove(i);
            self.selected = None;
            let keys: Vec<MeshKey> = self.models.iter().map(|m| m.key).collect();
            if let Some(vp) = self.viewport.as_mut() {
                vp.renderer_mut().retain_meshes(|k| keys.contains(&k));
            }
        }
    }

    fn properties_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        ui.separator();
        let Some(m) = self.selected.and_then(|i| self.models.get_mut(i)) else {
            ui.label(RichText::new("Nothing selected").weak());
            return;
        };
        let s = m.info.aabb.size();
        egui::Grid::new("props")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Name");
                ui.label(&m.name)
                    .on_hover_text(m.path.display().to_string());
                ui.end_row();
                ui.label("Size");
                ui.label(format!("{:.2} × {:.2} × {:.2} mm", s.x, s.y, s.z));
                ui.end_row();
                ui.label("Volume");
                ui.label(format!("{:.2} cm³", m.info.volume / 1000.0));
                ui.end_row();
                ui.label("Area");
                ui.label(format!("{:.2} cm²", m.info.area / 100.0));
                ui.end_row();
                ui.label("Triangles");
                ui.label(m.info.triangle_count.to_string());
                ui.end_row();
                ui.label("Watertight");
                ui.label(if m.topology.is_watertight() {
                    "yes"
                } else {
                    "no"
                });
                ui.end_row();
                ui.label("Color");
                ui.color_edit_button_srgb(&mut m.color);
                ui.end_row();
            });
        let t = &m.topology;
        if !t.is_clean_solid() {
            ui.add_space(8.0);
            ui.label(RichText::new("Issues").strong());
            for (n, what) in [
                (t.boundary_edges, "open edges"),
                (t.non_manifold_edges, "non-manifold edges"),
                (t.non_manifold_vertices, "non-manifold vertices"),
                (t.degenerate_triangles, "degenerate triangles"),
                (t.inconsistent_edges, "flipped edges"),
            ] {
                if n > 0 {
                    ui.label(format!("• {n} {what}"));
                }
            }
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let tris: usize = self.models.iter().map(|m| m.info.triangle_count).sum();
            ui.label(format!("{} objects · {tris} triangles", self.models.len()));
            if let Some(m) = self.selected.and_then(|i| self.models.get(i)) {
                let s = m.info.aabb.size();
                ui.separator();
                ui.label(format!(
                    "{}: {:.2} × {:.2} × {:.2} mm",
                    m.name, s.x, s.y, s.z
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

    /// Returns the index of the visible model under a viewport pixel.
    fn pick(&self, pixel: glam::DVec2, size: glam::DVec2) -> Option<usize> {
        let vp = self.viewport.as_ref()?;
        let (origin, dir) = vp.camera.ray(pixel, size);
        let ray = Ray::new(origin.as_vec3(), dir.as_vec3());
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| m.visible)
            .filter_map(|(i, m)| {
                m.bvh
                    .raycast(&m.mesh, &ray, f32::INFINITY)
                    .map(|h| (h.t, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    fn draw_objects(&self) -> Vec<DrawObject> {
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| m.visible)
            .map(|(i, m)| {
                let c = m.color.map(|v| srgb_to_linear(f32::from(v) / 255.0));
                DrawObject {
                    mesh: m.key,
                    model: DMat4::IDENTITY,
                    color: [c[0], c[1], c[2], 1.0],
                    style: DrawStyle::Solid,
                    selected: self.selected == Some(i),
                    hovered: false,
                }
            })
            .collect()
    }
}

impl eframe::App for RobocadApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_loads();
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("scene")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| self.scene_panel(ui));
        egui::Panel::right("properties")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| self.properties_panel(ui));

        if !self.messages.is_empty() {
            let mut close = false;
            egui::Window::new("Messages")
                .collapsible(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
                .show(&ctx, |ui| {
                    for m in &self.messages {
                        ui.label(m);
                    }
                    close = ui.button("OK").clicked();
                });
            if close {
                self.messages.clear();
            }
        }

        let objects = self.draw_objects();
        let mut style = if ui.visuals().dark_mode {
            SceneStyle::dark()
        } else {
            SceneStyle::light()
        };
        style.show_edges = self.show_edges;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| match self.viewport.as_mut() {
                Some(vp) => {
                    let out = vp.ui(ui, &objects, &style);
                    if let Some(click) = out.clicked {
                        self.selected = self.pick(click, out.viewport_size);
                    }
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.label("No GPU renderer available (wgpu failed to initialise).");
                    });
                }
            });
    }
}
