//! Viewport interaction: mesh cache, drawing, picking, gizmo, box selection, toolbar.

use std::sync::Arc;

use eframe::egui::{self, Pos2, Rect};
use glam::{DVec2, DVec3};
use rc_doc::{AlignMode, Command, NodeId, NodeKind, Primitive, PrimitiveKind, Role};
use rc_mesh::edges::feature_edges;
use rc_mesh::{Bvh, Ray, topology};
use rc_render::{DrawObject, DrawStyle, SceneStyle};

use crate::app::{MeshExtras, RobocadApp};
use crate::gizmo::{GizmoMode, Handle};
use crate::loader::FEATURE_ANGLE_DEG;

/// Salt separating primitive display keys from imported mesh hashes.
const PRIM_SALT: u64 = 0x9e37_79b9_7f4a_7c15;

pub fn prim_key(p: &Primitive) -> u64 {
    p.cache_key() ^ PRIM_SALT
}

/// A picked surface point.
#[derive(Clone, Copy, Debug)]
pub struct Pick {
    pub node: NodeId,
    pub point: DVec3,
}

impl RobocadApp {
    /// Display key of a leaf, making sure its mesh, BVH and GPU buffers exist.
    fn ensure_leaf(&mut self, id: NodeId) -> Option<u64> {
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
        });
        if let Some(vp) = self.viewport.as_mut()
            && !vp.renderer().has_mesh(key)
        {
            let device = vp.device().clone();
            vp.renderer_mut()
                .upload_mesh(&device, key, m, &feature_edges(m, FEATURE_ANGLE_DEG));
        }
    }

    /// Mesh behind a display key.
    fn mesh_for_key(&self, key: u64) -> Option<&Arc<rc_mesh::Mesh>> {
        self.prim_meshes
            .get(&key)
            .or_else(|| self.editor.doc().mesh(key))
    }

    /// Drops cached primitive meshes that no node uses any more.
    fn evict_unused(&mut self, used: &[u64]) {
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

    /// Visible leaves with their display keys (creating meshes as needed).
    fn visible_leaves(&mut self) -> Vec<(NodeId, u64)> {
        let ids: Vec<NodeId> = {
            let doc = self.editor.doc();
            doc.nodes
                .iter()
                .filter(|(id, n)| !n.is_group() && doc.is_effectively_visible(*id))
                .map(|(id, _)| id)
                .collect()
        };
        ids.into_iter()
            .filter_map(|id| self.ensure_leaf(id).map(|k| (id, k)))
            .collect()
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

    /// Closest visible, unlocked leaf under a viewport pixel.
    pub(crate) fn pick(&self, pixel: DVec2, size: DVec2) -> Option<Pick> {
        let vp = self.viewport.as_ref()?;
        let (origin, dir) = vp.camera.ray(pixel, size);
        let doc = self.editor.doc();
        let mut best: Option<(f64, Pick)> = None;
        for (id, node) in &doc.nodes {
            let key = match &node.kind {
                NodeKind::Imported { mesh, .. } => *mesh,
                NodeKind::Primitive(p) => prim_key(p),
                NodeKind::Group { .. } => continue,
            };
            if !doc.is_effectively_visible(id) || doc.is_effectively_locked(id) {
                continue;
            }
            let (Some(m), Some(extra)) = (self.mesh_for_key(key), self.extras.get(&key)) else {
                continue;
            };
            let world = doc.world_matrix(id);
            let inv = world.inverse();
            let ray = Ray::new(
                inv.transform_point3(origin).as_vec3(),
                inv.transform_vector3(dir).as_vec3(),
            );
            if let Some(hit) = extra.bvh.raycast(m, &ray, f32::INFINITY) {
                let wp = world.transform_point3(hit.point.as_dvec3());
                let dist = (wp - origin).dot(dir);
                if best.is_none_or(|(d, _)| dist < d) {
                    best = Some((
                        dist,
                        Pick {
                            node: id,
                            point: wp,
                        },
                    ));
                }
            }
        }
        best.map(|(_, p)| p)
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

    fn draw_objects(&mut self) -> Vec<DrawObject> {
        let leaves = self.visible_leaves();
        let used: Vec<u64> = leaves.iter().map(|(_, k)| *k).collect();
        self.evict_unused(&used);
        let doc = self.editor.doc();
        let sel = &self.editor.selection;
        let in_chain = |id: NodeId, f: &dyn Fn(NodeId) -> bool| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if f(c) {
                    return true;
                }
                cur = doc.get(c).and_then(|n| n.parent);
            }
            false
        };
        leaves
            .into_iter()
            .filter_map(|(id, key)| {
                let node = doc.get(id)?;
                let hole = in_chain(id, &|c| doc.get(c).is_some_and(|n| n.role == Role::Hole));
                let (color, style) = if hole {
                    ([0.35, 0.37, 0.40, 0.35], DrawStyle::Hole)
                } else {
                    let c = node
                        .color
                        .map(|v| rc_render::srgb_to_linear(f32::from(v) / 255.0));
                    ([c[0], c[1], c[2], 1.0], DrawStyle::Solid)
                };
                Some(DrawObject {
                    mesh: key,
                    model: doc.world_matrix(id),
                    color,
                    style,
                    selected: in_chain(id, &|c| sel.contains(c)),
                    hovered: self.hovered.is_some_and(|h| in_chain(id, &|c| c == h)),
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
                let r = ui
                    .add(egui::Button::new(kind.name()).sense(egui::Sense::click_and_drag()))
                    .on_hover_text(format!(
                        "Click to add a {0} at the centre, or drag it onto the plate",
                        kind.name().to_lowercase()
                    ));
                r.dnd_set_drag_payload(kind);
                if r.clicked() {
                    self.add_primitive(kind, None);
                }
            }
            ui.separator();
            for (mode, label, key) in [
                (GizmoMode::Move, "Move", "W"),
                (GizmoMode::Rotate, "Rotate", "E"),
                (GizmoMode::Scale, "Scale", "R"),
            ] {
                if ui
                    .selectable_label(self.gizmo.mode == mode, label)
                    .on_hover_text(format!("{label} tool ({key})"))
                    .clicked()
                {
                    self.gizmo.mode = mode;
                }
            }
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
        let objects = self.draw_objects();
        let mut style = if ui.visuals().dark_mode {
            SceneStyle::dark()
        } else {
            SceneStyle::light()
        };
        style.show_edges = self.show_edges;
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
        if out.drag_started_primary && !alt {
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

        if let Some(click) = out.clicked
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
