//! Viewport interaction: mesh cache, drawing, picking, gizmo, box selection, toolbar.

use std::sync::Arc;

use eframe::egui::{self, Pos2, Rect};
use glam::{DVec2, DVec3};
use rc_doc::{AlignMode, Command, NodeId, NodeKind, Primitive, PrimitiveKind};
use rc_mesh::edges::feature_edges;
use rc_mesh::{Bvh, topology};
use rc_render::SceneStyle;

use crate::app::{MeshExtras, RobocadApp};
use crate::gizmo::{GizmoMode, Handle};
use crate::loader::FEATURE_ANGLE_DEG;

/// Salt separating primitive display keys from imported mesh hashes.
const PRIM_SALT: u64 = 0x9e37_79b9_7f4a_7c15;

/// Salt separating text display keys.
const TEXT_SALT: u64 = 0x2545_f491_4f6c_dd1d;

pub fn prim_key(p: &Primitive) -> u64 {
    p.cache_key() ^ PRIM_SALT
}

/// A picked surface point.
#[derive(Clone, Copy, Debug)]
pub struct Pick {
    pub node: NodeId,
    pub point: DVec3,
    /// Outward face normal in world space.
    pub normal: DVec3,
    /// Corners of the hit triangle in world space.
    pub triangle: [DVec3; 3],
}

/// The active viewport tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    /// Select and transform with the gizmo.
    #[default]
    Select,
    /// Click two points to measure distance and angle.
    Measure,
    /// Click a face to rotate the object so that face lies on the plate.
    LayFlat,
}

/// A measured point.
#[derive(Clone, Copy, Debug)]
pub struct MeasurePoint {
    pub pos: DVec3,
    pub normal: DVec3,
}

/// Result of measuring between two points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub distance: f64,
    pub delta: DVec3,
    /// Angle between the two face normals in degrees.
    pub angle_deg: f64,
    /// Perpendicular distance between parallel faces (wall thickness), if they are parallel.
    pub face_distance: Option<f64>,
}

pub fn measure(a: MeasurePoint, b: MeasurePoint) -> Measurement {
    let delta = b.pos - a.pos;
    let cos = a.normal.dot(b.normal).clamp(-1.0, 1.0);
    let angle_deg = cos.acos().to_degrees();
    let parallel = angle_deg < 1.0 || angle_deg > 179.0;
    Measurement {
        distance: delta.length(),
        delta,
        angle_deg,
        face_distance: parallel.then(|| delta.dot(b.normal).abs()),
    }
}

/// Pattern presets offered in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternChoice {
    Flip,
    MirrorCopy,
    Linear,
    Grid,
    Circular,
}

impl RobocadApp {
    /// Display key of a leaf, making sure its mesh, BVH and GPU buffers exist.
    pub(crate) fn ensure_leaf(&mut self, id: NodeId) -> Option<u64> {
        let kind = self.editor.doc().get(id)?.kind.clone();
        match kind {
            NodeKind::Imported { mesh, .. } => {
                let m = self.editor.doc().mesh(mesh)?.clone();
                let needs_gpu = self
                    .viewport
                    .as_ref()
                    .is_some_and(|vp| !vp.renderer().has_mesh(mesh));
                if needs_gpu || !self.extras.contains_key(&mesh) {
                    self.cache_mesh(mesh, &m, None);
                }
                Some(mesh)
            }
            NodeKind::Primitive(p) => {
                let key = prim_key(&p);
                if !self.prim_meshes.contains_key(&key) {
                    let m = Arc::new(p.mesh());
                    self.cache_mesh(key, &m, None);
                    self.prim_meshes.insert(key, m);
                }
                Some(key)
            }
            NodeKind::Text {
                text,
                height,
                depth,
            } => {
                let params = rc_kernel::text::TextParams {
                    text: &text,
                    height,
                    depth,
                };
                let key = params.cache_key() ^ TEXT_SALT;
                if !self.prim_meshes.contains_key(&key) {
                    let (m, _) = rc_kernel::text::text_mesh(&params).ok()?;
                    self.cache_mesh(key, &m, None);
                    self.prim_meshes.insert(key, m);
                }
                Some(key)
            }
            NodeKind::Group { .. } => None,
        }
    }

    /// Computes derived data for a mesh and uploads it to the GPU.
    pub(crate) fn cache_mesh(
        &mut self,
        key: u64,
        m: &rc_mesh::Mesh,
        path: Option<std::path::PathBuf>,
    ) {
        self.extras.entry(key).or_insert_with(|| MeshExtras {
            bvh: Arc::new(Bvh::build(m)),
            topology: topology::analyze(m),
            info: m.info(),
            path,
            notes: Vec::new(),
        });
        if let Some(vp) = self.viewport.as_mut()
            && !vp.renderer().has_mesh(key)
        {
            let device = vp.device().clone();
            vp.renderer_mut()
                .upload_mesh(&device, key, m, &feature_edges(m, FEATURE_ANGLE_DEG));
        }
    }

    /// Drops cached primitive meshes that no node uses any more.
    pub(crate) fn evict_unused(&mut self, used: &[u64]) {
        if self.prim_meshes.len() <= used.len() + 32 {
            return;
        }
        let used: std::collections::HashSet<u64> = used.iter().copied().collect();
        let stale: Vec<u64> = self
            .prim_meshes
            .keys()
            .copied()
            .filter(|k| !used.contains(k))
            .collect();
        for k in stale {
            self.prim_meshes.remove(&k);
            self.extras.remove(&k);
            if let Some(vp) = self.viewport.as_mut() {
                vp.renderer_mut().remove_mesh(k);
            }
        }
    }

    /// The root-level ancestor of a node (what a viewport click selects).
    pub(crate) fn root_of(&self, id: NodeId) -> NodeId {
        let doc = self.editor.doc();
        let mut cur = id;
        while let Some(p) = doc.get(cur).and_then(|n| n.parent) {
            cur = p;
        }
        cur
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

    /// Screen positions of the gizmo pivot and axis tips (for tests and tooltips).
    pub fn gizmo_screen_handles(&self) -> Option<(Pos2, [Pos2; 3])> {
        let vp = self.viewport.as_ref()?;
        let rect = self.last_viewport_rect?;
        let size = DVec2::new(f64::from(rect.width()), f64::from(rect.height()));
        let f = self.gizmo.frame(
            self.editor.doc(),
            self.editor.selection.ids(),
            &vp.camera,
            size,
        )?;
        let proj = |p: DVec3| {
            vp.camera
                .project(p, size)
                .map(|v| rect.min + egui::vec2(v.x as f32, v.y as f32))
        };
        let pivot = proj(f.pivot)?;
        let tips = [0, 1, 2].map(|i| proj(f.pivot + f.axes[i] * f.length));
        Some((pivot, [tips[0]?, tips[1]?, tips[2]?]))
    }

    // ------------------------------------------------------------------ editing helpers

    pub fn add_primitive(&mut self, kind: PrimitiveKind, at: Option<DVec3>) {
        let color = self.next_color();
        let primitive = kind.default_primitive();
        let position = at.unwrap_or_else(|| self.editor.doc().free_position(primitive.size()));
        self.apply(Command::AddPrimitive {
            primitive,
            position,
            color,
        });
    }

    pub fn add_text(&mut self) {
        let color = self.next_color();
        let position = self.editor.doc().free_position(DVec3::new(60.0, 15.0, 3.0));
        self.apply(Command::AddText {
            text: "robocad".into(),
            height: 10.0,
            depth: 3.0,
            position,
            color,
        });
    }

    pub fn toggle_tool(&mut self, tool: Tool) {
        self.tool = if self.tool == tool {
            Tool::Select
        } else {
            tool
        };
        self.measure_points.clear();
    }

    /// Wraps the selection in a pattern group with sensible defaults.
    pub fn pattern_selection(&mut self, choice: PatternChoice) {
        use rc_doc::Pattern;
        let nodes = self.editor.selection.ids().to_vec();
        if nodes.is_empty() {
            return;
        }
        let size = self.editor.doc().bounds_of(&nodes).size().as_dvec3();
        let pattern = match choice {
            PatternChoice::Flip => Pattern::Mirror {
                axis: 0,
                offset: 0.0,
                keep_original: false,
            },
            PatternChoice::MirrorCopy => Pattern::Mirror {
                axis: 0,
                offset: size.x * 0.5,
                keep_original: true,
            },
            PatternChoice::Linear => Pattern::Grid {
                counts: [3, 1, 1],
                spacing: DVec3::new(size.x + 5.0, size.y + 5.0, size.z + 5.0),
            },
            PatternChoice::Grid => Pattern::Grid {
                counts: [3, 3, 1],
                spacing: DVec3::new(size.x + 5.0, size.y + 5.0, size.z + 5.0),
            },
            PatternChoice::Circular => Pattern::Circular {
                count: 6,
                angle_deg: 360.0,
                center: DVec3::ZERO,
            },
        };
        self.apply(Command::MakePattern { nodes, pattern });
    }

    /// Rotates the picked object so the picked face points down, then drops it to the plate.
    pub fn lay_flat(&mut self, pick: Pick) {
        let root = self.root_of(pick.node);
        if self.editor.doc().is_effectively_locked(root) {
            return;
        }
        let doc = self.editor.doc();
        let Some(node) = doc.get(root) else { return };
        let world = doc.world_matrix(root);
        let parent = node
            .parent
            .map_or(glam::DMat4::IDENTITY, |p| doc.world_matrix(p));
        let pivot = doc.world_bounds(root).center().as_dvec3();
        let q = glam::DQuat::from_rotation_arc(pick.normal.normalize_or(-DVec3::Z), -DVec3::Z);
        let r = glam::DMat4::from_translation(pivot)
            * glam::DMat4::from_quat(q)
            * glam::DMat4::from_translation(-pivot);
        let mut t = rc_doc::Transform::from_matrix(&(parent.inverse() * r * world));
        t.scale = node.transform.scale;
        let key = 0x1a7_f1a7 ^ self.editor.revision();
        self.apply_merged(Command::SetTransforms(vec![(root, t)]), key);
        self.apply_merged(Command::DropToPlate(vec![root]), key);
        self.editor.end_merge();
        self.editor.selection.set([root]);
    }

    pub fn drop_selection_to_plate(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::DropToPlate(ids));
        }
    }

    pub fn center_selection_on_plate(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        let b = self.editor.doc().bounds_of(&ids);
        if !b.is_empty() {
            let c = b.center().as_dvec3();
            self.apply(Command::TranslateWorld(ids, DVec3::new(-c.x, -c.y, 0.0)));
        }
    }

    pub fn align_selection(&mut self, axis: usize, mode: AlignMode) {
        let nodes = self.editor.selection.ids().to_vec();
        if nodes.len() > 1 {
            self.apply(Command::Align { nodes, axis, mode });
        }
    }

    pub(crate) fn cancel_gizmo_drag(&mut self) {
        self.gizmo.end();
        if self.gizmo_changed {
            self.editor.undo();
        }
        self.gizmo_changed = false;
        self.editor.end_merge();
    }

    /// Arrow keys move the selection on the plate relative to the view; PgUp/PgDn move in Z.
    pub(crate) fn handle_nudge(&mut self, ctx: &egui::Context) {
        if self.editor.selection.is_empty() {
            return;
        }
        let (keys, shift) = ctx.input_mut(|i| {
            let mut k = Vec::new();
            for key in [
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::PageUp,
                egui::Key::PageDown,
            ] {
                if i.consume_key(egui::Modifiers::NONE, key)
                    || i.consume_key(egui::Modifiers::SHIFT, key)
                {
                    k.push(key);
                }
            }
            (k, i.modifiers.shift)
        });
        if keys.is_empty() {
            return;
        }
        let step = self.editor.doc().settings.grid_step * if shift { 10.0 } else { 1.0 };
        let Some(vp) = self.viewport.as_ref() else {
            return;
        };
        // Snap the camera's right/forward directions to the nearest world axis on the plate.
        let snap_axis = |v: DVec3| {
            if v.x.abs() >= v.y.abs() {
                DVec3::new(v.x.signum(), 0.0, 0.0)
            } else {
                DVec3::new(0.0, v.y.signum(), 0.0)
            }
        };
        let right = snap_axis(vp.camera.right());
        let fwd = snap_axis(DVec3::Z.cross(vp.camera.right()));
        let mut delta = DVec3::ZERO;
        for k in keys {
            delta += match k {
                egui::Key::ArrowLeft => -right,
                egui::Key::ArrowRight => right,
                egui::Key::ArrowUp => fwd,
                egui::Key::ArrowDown => -fwd,
                egui::Key::PageUp => DVec3::Z,
                _ => -DVec3::Z,
            } * step;
        }
        let ids = self.editor.selection.ids().to_vec();
        self.apply(Command::TranslateWorld(ids, delta));
    }

    // ------------------------------------------------------------------ toolbar

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for kind in PrimitiveKind::ALL {
                let icon = match kind {
                    PrimitiveKind::Box => crate::icons::Icon::Box,
                    PrimitiveKind::Cylinder => crate::icons::Icon::Cylinder,
                    PrimitiveKind::Cone => crate::icons::Icon::Cone,
                    PrimitiveKind::Sphere => crate::icons::Icon::Sphere,
                    PrimitiveKind::Torus => crate::icons::Icon::Torus,
                    PrimitiveKind::Wedge => crate::icons::Icon::Wedge,
                };
                let r = crate::icons::button(ui, icon, kind.name(), false).on_hover_text(format!(
                    "Click to add a {0} at a free spot, or drag it onto the plate",
                    kind.name().to_lowercase()
                ));
                r.dnd_set_drag_payload(kind);
                if r.clicked() {
                    self.add_primitive(kind, None);
                }
            }
            if crate::icons::button(ui, crate::icons::Icon::Text, "Text", false)
                .on_hover_text("Add 3D text")
                .clicked()
            {
                self.add_text();
            }
            ui.separator();
            for (mode, icon, label, key) in [
                (GizmoMode::Move, crate::icons::Icon::Move, "Move", "W"),
                (GizmoMode::Rotate, crate::icons::Icon::Rotate, "Rotate", "E"),
                (GizmoMode::Scale, crate::icons::Icon::Scale, "Scale", "R"),
            ] {
                let active = self.gizmo.mode == mode && self.tool == Tool::Select;
                if crate::icons::button(ui, icon, label, active)
                    .on_hover_text(format!("{label} tool ({key})"))
                    .clicked()
                {
                    self.gizmo.mode = mode;
                    self.tool = Tool::Select;
                }
            }
            ui.separator();
            if crate::icons::button(
                ui,
                crate::icons::Icon::Measure,
                "Measure",
                self.tool == Tool::Measure,
            )
            .on_hover_text("Click two points to measure (M). Shift: snap off.")
            .clicked()
            {
                self.toggle_tool(Tool::Measure);
            }
            if crate::icons::button(
                ui,
                crate::icons::Icon::LayFlat,
                "Lay flat",
                self.tool == Tool::LayFlat,
            )
            .on_hover_text("Click a face to put it on the plate (L)")
            .clicked()
            {
                self.toggle_tool(Tool::LayFlat);
            }
            let has_sel = !self.editor.selection.is_empty();
            ui.add_enabled_ui(has_sel, |ui| {
                ui.menu_button("Pattern", |ui| {
                    if ui.button("Mirror (flip in place)").clicked() {
                        self.pattern_selection(PatternChoice::Flip);
                        ui.close();
                    }
                    if ui.button("Mirror copy").clicked() {
                        self.pattern_selection(PatternChoice::MirrorCopy);
                        ui.close();
                    }
                    if ui.button("Linear array (3 along X)").clicked() {
                        self.pattern_selection(PatternChoice::Linear);
                        ui.close();
                    }
                    if ui.button("Grid array (3 × 3)").clicked() {
                        self.pattern_selection(PatternChoice::Grid);
                        ui.close();
                    }
                    if ui
                        .button("Circular array (6 around plate centre)")
                        .clicked()
                    {
                        self.pattern_selection(PatternChoice::Circular);
                        ui.close();
                    }
                });
            });
            self.extensions_toolbar(ui);
            ui.separator();
            ui.checkbox(&mut self.snapping, "Snap")
                .on_hover_text("Snap to the grid and to 15° (hold Alt while dragging to invert)");
            let mut grid = self.editor.doc().settings.grid_step;
            let r = ui.add(
                egui::DragValue::new(&mut grid)
                    .range(0.01..=100.0)
                    .speed(0.1)
                    .suffix(" mm"),
            );
            if r.changed() {
                self.editor.set_settings(rc_doc::DocSettings {
                    grid_step: grid,
                    ..self.editor.doc().settings.clone()
                });
            }
            ui.separator();
            let has_sel = !self.editor.selection.is_empty();
            if ui
                .add_enabled(has_sel, egui::Button::new("Drop"))
                .on_hover_text("Drop selection onto the plate (D)")
                .clicked()
            {
                self.drop_selection_to_plate();
            }
            if ui
                .add_enabled(has_sel, egui::Button::new("Center"))
                .on_hover_text("Center selection on the plate")
                .clicked()
            {
                self.center_selection_on_plate();
            }
            if ui
                .add_enabled(has_sel, egui::Button::new("Solid/Hole"))
                .on_hover_text("Toggle solid / hole (H)")
                .clicked()
            {
                self.toggle_hole_selection();
            }
        });
    }

    // ------------------------------------------------------------------ viewport

    pub(crate) fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        self.items = self.build_items();
        let objects = self.draw_list(&self.items);
        let mut style = if ui.visuals().dark_mode {
            SceneStyle::dark()
        } else {
            SceneStyle::light()
        };
        style.show_edges = self.show_edges;
        style.show_overhangs = self.show_overhangs;
        style.outline_width = 2.0 * ui.ctx().pixels_per_point();
        let plate = self.editor.doc().settings.plate_size;
        style.plate_size = [plate[0] as f32, plate[1] as f32];
        if self.viewport.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("No GPU renderer available (wgpu failed to initialise).");
            });
            return;
        }
        let gizmo_busy = self.gizmo.is_dragging();
        let Some(vp) = self.viewport.as_mut() else {
            return;
        };
        let out = vp.ui(ui, &objects, &style, gizmo_busy);
        let camera = vp.camera;
        self.last_viewport_rect = Some(out.rect);
        let origin = out.rect.min;
        let size = out.viewport_size;
        let (shift, command, alt) =
            ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let grid = self.editor.doc().settings.grid_step;

        let sel = self.editor.selection.ids().to_vec();
        let frame = self.gizmo.frame(self.editor.doc(), &sel, &camera, size);
        if !self.gizmo.is_dragging() {
            self.gizmo.hovered = match (frame, out.hover) {
                (Some(f), Some(p)) => {
                    self.gizmo
                        .hit(&f, &camera, size, Pos2::new(p.x as f32, p.y as f32))
                }
                _ => None,
            };
        }
        self.hovered = if self.gizmo.hovered.is_some() || self.gizmo.is_dragging() {
            None
        } else {
            out.hover
                .and_then(|p| self.pick(p, size))
                .map(|p| self.root_of(p.node))
        };

        // Primary drag: gizmo handle, object surface, or box selection.
        if out.drag_started_primary && !alt && self.tool == Tool::Select {
            let start = out.press_origin.unwrap_or_default();
            let start_px = Pos2::new(start.x as f32, start.y as f32);
            let handle = frame.and_then(|f| self.gizmo.hit(&f, &camera, size, start_px));
            if let (Some(h), Some(f)) = (handle, frame) {
                self.gizmo
                    .begin(h, f, self.editor.doc(), &sel, &camera, size, start, None);
                self.gizmo_changed = false;
            } else if let Some(pick) = self.pick(start, size) {
                let root = self.root_of(pick.node);
                if !self.editor.selection.contains(root) {
                    self.editor.selection.set([root]);
                }
                let sel = self.editor.selection.ids().to_vec();
                let f = self
                    .gizmo
                    .frame(self.editor.doc(), &sel, &camera, size)
                    .unwrap_or(crate::gizmo::Frame {
                        pivot: pick.point,
                        axes: [DVec3::X, DVec3::Y, DVec3::Z],
                        length: 1.0,
                    });
                self.gizmo.begin(
                    Handle::Surface,
                    f,
                    self.editor.doc(),
                    &sel,
                    &camera,
                    size,
                    start,
                    Some(pick.point),
                );
                self.gizmo_changed = false;
            } else {
                self.box_start = Some(start_px);
            }
        }

        if self.gizmo.is_dragging() {
            if let Some(p) = out.pointer_local
                && let Some((cmds, key)) =
                    self.gizmo
                        .update(&camera, size, p, self.snapping != alt, grid)
            {
                let before = self.editor.revision();
                for c in cmds {
                    self.apply_merged(c, key);
                }
                self.gizmo_changed |= self.editor.revision() != before;
            }
            if out.drag_stopped_primary || !ui.input(|i| i.pointer.primary_down()) {
                self.gizmo.end();
                self.editor.end_merge();
            }
        }

        if let Some(f) = self.gizmo.frame(
            self.editor.doc(),
            self.editor.selection.ids(),
            &camera,
            size,
        ) {
            self.gizmo.draw(ui.painter(), &f, &camera, size, origin);
        }

        self.box_selection(ui, &out, shift || command);

        if self.tool != Tool::Select {
            self.tool_ui(ui, &out, &camera, shift);
        } else if let Some(click) = out.clicked
            && self.gizmo.hovered.is_none()
        {
            let hit = self.pick(click, size).map(|p| self.root_of(p.node));
            match hit {
                Some(id) if shift || command => self.editor.selection.toggle(id),
                Some(id) => self.editor.selection.set([id]),
                None if !(shift || command) => self.editor.selection.clear(),
                None => {}
            }
        }

        // Primitive dragged from the toolbar and released over the viewport.
        let released = ui.input(|i| i.pointer.any_released());
        if released
            && let Some(p) = out.hover
            && let Some(kind) = egui::DragAndDrop::take_payload::<PrimitiveKind>(ui.ctx())
        {
            let (o, d) = camera.ray(p, size);
            let at = crate::gizmo::ray_plane(o, d, DVec3::ZERO, DVec3::Z).map(|v| {
                if self.snapping {
                    (v / grid).round() * grid
                } else {
                    v
                }
            });
            self.add_primitive(*kind, at.map(|v| DVec3::new(v.x, v.y, 0.0)));
        }
    }

    /// Measure and lay-flat tools.
    fn tool_ui(
        &mut self,
        ui: &egui::Ui,
        out: &crate::viewport::ViewportOutput,
        camera: &rc_render::OrbitCamera,
        no_snap: bool,
    ) {
        let size = out.viewport_size;
        let origin = out.rect.min;
        let to_screen = |p: DVec3| {
            camera
                .project(p, size)
                .map(|v| origin + egui::vec2(v.x as f32, v.y as f32))
        };
        let painter = ui.painter();
        let accent = egui::Color32::from_rgb(255, 196, 0);
        // Snap a picked point to the nearest triangle corner within 10 px.
        let snapped = |pick: &Pick, at: DVec2| -> DVec3 {
            if no_snap {
                return pick.point;
            }
            let click = origin + egui::vec2(at.x as f32, at.y as f32);
            pick.triangle
                .iter()
                .filter_map(|v| to_screen(*v).map(|s| (s.distance(click), *v)))
                .filter(|(d, _)| *d < 10.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(pick.point, |(_, v)| v)
        };
        if let Some(h) = out.hover
            && let Some(pick) = self.pick(h, size)
            && let Some(s) = to_screen(snapped(&pick, h))
        {
            painter.circle_stroke(s, 5.0, egui::Stroke::new(1.5, accent));
        }
        if let Some(click) = out.clicked
            && let Some(pick) = self.pick(click, size)
        {
            match self.tool {
                Tool::Measure => {
                    if self.measure_points.len() >= 2 {
                        self.measure_points.clear();
                    }
                    self.measure_points.push(MeasurePoint {
                        pos: snapped(&pick, click),
                        normal: pick.normal,
                    });
                }
                Tool::LayFlat => {
                    self.lay_flat(pick);
                    self.tool = Tool::Select;
                }
                Tool::Select => {}
            }
        }
        if self.tool != Tool::Measure {
            return;
        }
        let pts: Vec<egui::Pos2> = self
            .measure_points
            .iter()
            .filter_map(|p| to_screen(p.pos))
            .collect();
        for p in &pts {
            painter.circle_filled(*p, 4.0, accent);
        }
        if let [a, b] = self.measure_points[..]
            && let [pa, pb] = pts[..]
        {
            painter.line_segment([pa, pb], egui::Stroke::new(2.0, accent));
            let m = measure(a, b);
            let mut text = format!(
                "{:.2} mm\nΔ {:.2}, {:.2}, {:.2}\nangle {:.1}°",
                m.distance,
                m.delta.x + 0.0,
                m.delta.y + 0.0,
                m.delta.z + 0.0,
                m.angle_deg
            );
            if let Some(fd) = m.face_distance {
                use std::fmt::Write as _;
                let _ = write!(text, "\nface distance {fd:.2} mm");
            }
            let mid = pa + (pb - pa) * 0.5 + egui::vec2(10.0, -10.0);
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let r = egui::Rect::from_min_size(mid, galley.size()).expand(5.0);
            painter.rect_filled(r, 4.0, egui::Color32::from_black_alpha(190));
            painter.galley(mid, galley, egui::Color32::WHITE);
            self.last_measurement = Some(m);
        }
    }

    fn box_selection(
        &mut self,
        ui: &egui::Ui,
        out: &crate::viewport::ViewportOutput,
        additive: bool,
    ) {
        let Some(start) = self.box_start else { return };
        if let Some(cur) = out.pointer_local {
            let r = Rect::from_two_pos(start, Pos2::new(cur.x as f32, cur.y as f32));
            let screen = r.translate(out.rect.min.to_vec2());
            let accent = ui.visuals().selection.bg_fill;
            let painter = ui.painter();
            painter.rect_filled(screen, 0.0, accent.gamma_multiply(0.15));
            painter.rect_stroke(
                screen,
                0.0,
                egui::Stroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );
            if out.drag_stopped_primary {
                let picked = self.box_select(r, out.viewport_size);
                if !additive {
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
}
