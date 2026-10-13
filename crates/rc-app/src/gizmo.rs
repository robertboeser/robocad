//! Transform gizmo: move arrows and planes, rotation rings, scale handles.
//!
//! Handles are hit-tested and drawn in screen space; dragging maps the pointer ray back onto
//! the handle's axis, plane or ring in world space. Every drag frame recomputes the new
//! transforms from the ones captured at drag start, so there is no drift.

use eframe::egui::{self, Color32, Pos2, Stroke};
use glam::{DMat4, DQuat, DVec2, DVec3};
use rc_doc::{Command, Document, NodeId, NodeKind, Primitive, Transform};
use rc_render::{OrbitCamera, Projection};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// Move along an axis.
    Axis(usize),
    /// Move in the plane perpendicular to the given axis.
    Plane(usize),
    /// Rotate around an axis.
    Ring(usize),
    /// Scale along a (local) axis.
    ScaleAxis(usize),
    ScaleUniform,
    /// Free move on the horizontal plane through a point (dragging an object directly).
    Surface,
}

/// Where the gizmo is and how big it is.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub pivot: DVec3,
    pub axes: [DVec3; 3],
    /// World length of an arrow.
    pub length: f64,
}

const ARROW_PX: f64 = 90.0;
const PICK_PX: f32 = 9.0;
const RING_SEGMENTS: usize = 72;
pub const ROTATE_SNAP_DEG: f64 = 15.0;

/// State captured when a drag starts.
struct Drag {
    handle: Handle,
    frame: Frame,
    /// Reference value at the start: axis parameter, plane point, angle vector, or pixel distance.
    start_param: f64,
    start_point: DVec3,
    start_px: DVec2,
    pivot_px: DVec2,
    originals: Vec<Original>,
    merge_key: u64,
}

struct Original {
    id: NodeId,
    transform: Transform,
    world: DMat4,
    parent_world: DMat4,
    primitive: Option<Primitive>,
}

#[derive(Default)]
pub struct Gizmo {
    pub mode: GizmoMode,
    pub hovered: Option<Handle>,
    drag: Option<Drag>,
    drags: u64,
}

/// World units per screen point at a world position.
pub fn units_per_point(cam: &OrbitCamera, at: DVec3, viewport_h: f64) -> f64 {
    match cam.projection {
        Projection::Orthographic => cam.units_per_pixel(viewport_h),
        Projection::Perspective => {
            let depth = (at - cam.eye()).dot(cam.forward()).max(1e-6);
            2.0 * depth * (cam.fov_y * 0.5).tan() / viewport_h.max(1.0)
        }
    }
}

/// Parameter `s` of the point on line `p + a·s` closest to the ray `o + d·t`.
pub fn closest_on_axis(p: DVec3, a: DVec3, o: DVec3, d: DVec3) -> Option<f64> {
    let w = p - o;
    let (aa, ad, dd) = (a.dot(a), a.dot(d), d.dot(d));
    let denom = aa * dd - ad * ad;
    if denom.abs() < 1e-12 {
        return None; // axis parallel to the view ray
    }
    Some((ad * d.dot(w) - dd * a.dot(w)) / denom)
}

/// Intersection of a ray with the plane through `p` with normal `n`.
pub fn ray_plane(o: DVec3, d: DVec3, p: DVec3, n: DVec3) -> Option<DVec3> {
    let denom = d.dot(n);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (p - o).dot(n) / denom;
    (t > 0.0).then(|| o + d * t)
}

/// Signed angle from `a` to `b` around `axis`.
pub fn signed_angle(a: DVec3, b: DVec3, axis: DVec3) -> f64 {
    axis.dot(a.cross(b)).atan2(a.dot(b))
}

fn snap(v: f64, step: f64) -> f64 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(232, 72, 64),
    Color32::from_rgb(98, 196, 72),
    Color32::from_rgb(64, 128, 240),
];
const HOT: Color32 = Color32::from_rgb(255, 214, 64);

impl Gizmo {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Computes the gizmo frame for the selection, or `None` if nothing can be transformed.
    pub fn frame(
        &self,
        doc: &Document,
        sel: &[NodeId],
        cam: &OrbitCamera,
        vp: DVec2,
    ) -> Option<Frame> {
        if let Some(d) = &self.drag {
            return Some(d.frame);
        }
        let ids: Vec<NodeId> = rc_doc::editor::top_level(doc, sel)
            .into_iter()
            .filter(|id| !doc.is_effectively_locked(*id))
            .collect();
        if ids.is_empty() {
            return None;
        }
        let bounds = doc.bounds_of(&ids);
        if bounds.is_empty() {
            return None;
        }
        let (pivot, axes) = if self.mode == GizmoMode::Scale {
            if ids.len() != 1 {
                return None;
            }
            let w = doc.world_matrix(ids[0]);
            let axes = [0, 1, 2].map(|i| w.col(i).truncate().normalize_or(DVec3::AXES[i]));
            (w.transform_point3(DVec3::ZERO), axes)
        } else {
            (bounds.center().as_dvec3(), [DVec3::X, DVec3::Y, DVec3::Z])
        };
        let length = units_per_point(cam, pivot, vp.y) * ARROW_PX;
        Some(Frame {
            pivot,
            axes,
            length,
        })
    }

    /// Screen-space hit test of the gizmo handles.
    pub fn hit(&self, f: &Frame, cam: &OrbitCamera, vp: DVec2, pointer: Pos2) -> Option<Handle> {
        let proj = |p: DVec3| {
            cam.project(p, vp)
                .map(|v| Pos2::new(v.x as f32, v.y as f32))
        };
        let centre = proj(f.pivot)?;
        let mut best: Option<(f32, Handle)> = None;
        let mut consider = |d: f32, h: Handle| {
            if d < PICK_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, h));
            }
        };
        match self.mode {
            GizmoMode::Move => {
                for i in 0..3 {
                    if let Some(tip) = proj(f.pivot + f.axes[i] * f.length) {
                        consider(dist_to_segment(pointer, centre, tip), Handle::Axis(i));
                    }
                    // Plane square between the two other axes.
                    let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
                    let c = f.pivot + (u + v) * f.length * 0.3;
                    if let Some(cp) = proj(c) {
                        let d = (cp - pointer).length() - 6.0;
                        consider(d.max(0.0) * 1.5, Handle::Plane(i));
                    }
                }
            }
            GizmoMode::Rotate => {
                for i in 0..3 {
                    let pts = ring_points(f, i);
                    for w in pts.windows(2) {
                        if let (Some(a), Some(b)) = (proj(w[0]), proj(w[1])) {
                            consider(dist_to_segment(pointer, a, b), Handle::Ring(i));
                        }
                    }
                }
            }
            GizmoMode::Scale => {
                for i in 0..3 {
                    if let Some(tip) = proj(f.pivot + f.axes[i] * f.length) {
                        consider(dist_to_segment(pointer, centre, tip), Handle::ScaleAxis(i));
                    }
                }
                consider((centre - pointer).length() - 4.0, Handle::ScaleUniform);
            }
        }
        best.map(|(_, h)| h)
    }

    /// Starts dragging `handle`. For [`Handle::Surface`], `surface_point` is the picked point.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        handle: Handle,
        frame: Frame,
        doc: &Document,
        sel: &[NodeId],
        cam: &OrbitCamera,
        vp: DVec2,
        pointer: DVec2,
        surface_point: Option<DVec3>,
    ) {
        let ids: Vec<NodeId> = rc_doc::editor::top_level(doc, sel)
            .into_iter()
            .filter(|id| !doc.is_effectively_locked(*id))
            .collect();
        let originals = ids
            .iter()
            .filter_map(|&id| {
                let node = doc.get(id)?;
                Some(Original {
                    id,
                    transform: node.transform,
                    world: doc.world_matrix(id),
                    parent_world: node.parent.map_or(DMat4::IDENTITY, |p| doc.world_matrix(p)),
                    primitive: match node.kind {
                        NodeKind::Primitive(p) => Some(p),
                        _ => None,
                    },
                })
            })
            .collect();
        let (o, d) = cam.ray(pointer, vp);
        let mut frame = frame;
        let (start_param, start_point) = match handle {
            Handle::Axis(i) | Handle::ScaleAxis(i) => (
                closest_on_axis(frame.pivot, frame.axes[i], o, d).unwrap_or(0.0),
                DVec3::ZERO,
            ),
            Handle::Plane(i) | Handle::Ring(i) => (
                0.0,
                ray_plane(o, d, frame.pivot, frame.axes[i]).unwrap_or(frame.pivot),
            ),
            Handle::Surface => {
                let p = surface_point.unwrap_or(frame.pivot);
                frame.pivot = p;
                (0.0, p)
            }
            Handle::ScaleUniform => (0.0, DVec3::ZERO),
        };
        let pivot_px = cam.project(frame.pivot, vp).unwrap_or(pointer);
        self.drags += 1;
        self.drag = Some(Drag {
            handle,
            frame,
            start_param,
            start_point,
            start_px: pointer,
            pivot_px,
            originals,
            merge_key: 0x6a5f_0000 + self.drags,
        });
    }

    /// Ends the current drag.
    pub fn end(&mut self) {
        self.drag = None;
    }

    /// Commands for the current pointer position, plus the merge key for one undo step.
    pub fn update(
        &self,
        cam: &OrbitCamera,
        vp: DVec2,
        pointer: DVec2,
        snapping: bool,
        grid: f64,
    ) -> Option<(Vec<Command>, u64)> {
        let drag = self.drag.as_ref()?;
        let f = &drag.frame;
        let (o, d) = cam.ray(pointer, vp);
        let step = if snapping { grid } else { 0.0 };
        let translate = |delta: DVec3| {
            let list = drag
                .originals
                .iter()
                .map(|og| {
                    let mut t = og.transform;
                    t.translation += og.parent_world.inverse().transform_vector3(delta);
                    (og.id, t)
                })
                .collect();
            Command::SetTransforms(list)
        };
        let cmds = match drag.handle {
            Handle::Axis(i) => {
                let s = closest_on_axis(f.pivot, f.axes[i], o, d)?;
                vec![translate(f.axes[i] * snap(s - drag.start_param, step))]
            }
            Handle::Plane(_) | Handle::Surface => {
                let n = match drag.handle {
                    Handle::Plane(i) => f.axes[i],
                    _ => DVec3::Z,
                };
                let p = ray_plane(o, d, drag.start_point, n)?;
                let mut delta = p - drag.start_point;
                if snapping {
                    delta = delta.map(|c| snap(c, step));
                }
                vec![translate(delta)]
            }
            Handle::Ring(i) => {
                let p = ray_plane(o, d, f.pivot, f.axes[i])?;
                let mut angle = signed_angle(drag.start_point - f.pivot, p - f.pivot, f.axes[i]);
                if snapping {
                    angle = snap(angle.to_degrees(), ROTATE_SNAP_DEG).to_radians();
                }
                let r = DMat4::from_translation(f.pivot)
                    * DMat4::from_quat(DQuat::from_axis_angle(f.axes[i], angle))
                    * DMat4::from_translation(-f.pivot);
                let list = drag
                    .originals
                    .iter()
                    .map(|og| {
                        let local = og.parent_world.inverse() * r * og.world;
                        let mut t = Transform::from_matrix(&local);
                        // Keep the original scale exactly (decomposition noise).
                        t.scale = og.transform.scale;
                        (og.id, t)
                    })
                    .collect();
                vec![Command::SetTransforms(list)]
            }
            Handle::ScaleAxis(i) => {
                let s = closest_on_axis(f.pivot, f.axes[i], o, d)?;
                let factor = (s / drag.start_param.max(1e-9)).max(0.01);
                let mut fac = DVec3::ONE;
                fac[i] = factor;
                Self::scale_commands(drag, fac, snapping, grid)
            }
            Handle::ScaleUniform => {
                let d0 = (drag.start_px - drag.pivot_px).length().max(1.0);
                let d1 = (pointer - drag.pivot_px).length();
                let factor = (1.0 + (d1 - d0) / 100.0).max(0.01);
                Self::scale_commands(drag, DVec3::splat(factor), snapping, grid)
            }
        };
        Some((cmds, drag.merge_key))
    }

    fn scale_commands(drag: &Drag, fac: DVec3, snapping: bool, grid: f64) -> Vec<Command> {
        let mut cmds = Vec::new();
        for og in &drag.originals {
            if let Some(p) = og.primitive {
                let mut size = p.size() * fac;
                if snapping {
                    size = size.map(|c| snap(c, grid).max(grid));
                }
                cmds.push(Command::SetPrimitives(vec![(og.id, p.with_size(size))]));
            } else {
                let mut t = og.transform;
                t.scale *= fac;
                cmds.push(Command::SetTransforms(vec![(og.id, t)]));
            }
        }
        cmds
    }

    /// Draws the gizmo on top of the viewport.
    pub fn draw(
        &self,
        painter: &egui::Painter,
        f: &Frame,
        cam: &OrbitCamera,
        vp: DVec2,
        origin: Pos2,
    ) {
        let proj = |p: DVec3| {
            cam.project(p, vp)
                .map(|v| origin + egui::vec2(v.x as f32, v.y as f32))
        };
        let Some(centre) = proj(f.pivot) else { return };
        let active = self.drag.as_ref().map(|d| d.handle).or(self.hovered);
        let color = |i: usize, h: Handle| {
            if active == Some(h) {
                HOT
            } else {
                AXIS_COLORS[i]
            }
        };
        match self.mode {
            GizmoMode::Move => {
                for i in 0..3 {
                    let h = Handle::Axis(i);
                    let Some(tip) = proj(f.pivot + f.axes[i] * f.length) else {
                        continue;
                    };
                    let c = color(i, h);
                    painter.line_segment([centre, tip], Stroke::new(3.0, c));
                    let dir = (tip - centre).normalized();
                    let side = egui::vec2(-dir.y, dir.x);
                    painter.add(egui::Shape::convex_polygon(
                        vec![tip + dir * 12.0, tip + side * 5.5, tip - side * 5.5],
                        c,
                        Stroke::NONE,
                    ));
                    let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
                    let corners = [(0.2, 0.2), (0.4, 0.2), (0.4, 0.4), (0.2, 0.4)]
                        .map(|(a, b)| proj(f.pivot + (u * a + v * b) * f.length));
                    if corners.iter().all(Option::is_some) {
                        let pts: Vec<Pos2> = corners.into_iter().flatten().collect();
                        let pc = color(i, Handle::Plane(i));
                        painter.add(egui::Shape::convex_polygon(
                            pts,
                            pc.gamma_multiply(0.45),
                            Stroke::new(1.0, pc),
                        ));
                    }
                }
                painter.circle_filled(centre, 3.5, Color32::WHITE);
            }
            GizmoMode::Rotate => {
                for i in 0..3 {
                    let pts: Vec<Pos2> = ring_points(f, i).into_iter().filter_map(proj).collect();
                    let w = if active == Some(Handle::Ring(i)) {
                        3.5
                    } else {
                        2.5
                    };
                    painter.add(egui::Shape::line(
                        pts,
                        Stroke::new(w, color(i, Handle::Ring(i))),
                    ));
                }
                if let Some(d) = &self.drag
                    && let Some(p0) = proj(d.start_point)
                {
                    painter.line_segment([centre, p0], Stroke::new(1.0, Color32::WHITE));
                }
            }
            GizmoMode::Scale => {
                for i in 0..3 {
                    let h = Handle::ScaleAxis(i);
                    let Some(tip) = proj(f.pivot + f.axes[i] * f.length) else {
                        continue;
                    };
                    let c = color(i, h);
                    painter.line_segment([centre, tip], Stroke::new(3.0, c));
                    painter.rect_filled(
                        egui::Rect::from_center_size(tip, egui::vec2(11.0, 11.0)),
                        1.0,
                        c,
                    );
                }
                let c = if active == Some(Handle::ScaleUniform) {
                    HOT
                } else {
                    Color32::WHITE
                };
                painter.rect_filled(
                    egui::Rect::from_center_size(centre, egui::vec2(10.0, 10.0)),
                    1.0,
                    c,
                );
            }
        }
    }
}

fn ring_points(f: &Frame, i: usize) -> Vec<DVec3> {
    let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
    let r = f.length * 0.85;
    (0..=RING_SEGMENTS)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / RING_SEGMENTS as f64;
            f.pivot + (u * a.cos() + v * a.sin()) * r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_on_axis() {
        // Ray straight down at x = 7 hits the x axis at s = 7.
        let s = closest_on_axis(DVec3::ZERO, DVec3::X, DVec3::new(7.0, 0.0, 10.0), -DVec3::Z);
        assert!((s.unwrap() - 7.0).abs() < 1e-12);
        assert!(
            closest_on_axis(DVec3::ZERO, DVec3::Z, DVec3::new(0.0, 0.0, 10.0), -DVec3::Z).is_none()
        );
    }

    #[test]
    fn plane_and_angle() {
        let p = ray_plane(DVec3::new(1.0, 2.0, 5.0), -DVec3::Z, DVec3::ZERO, DVec3::Z).unwrap();
        assert_eq!(p, DVec3::new(1.0, 2.0, 0.0));
        assert!(ray_plane(DVec3::new(0.0, 0.0, 5.0), DVec3::Z, DVec3::ZERO, DVec3::Z).is_none());
        let a = signed_angle(DVec3::X, DVec3::Y, DVec3::Z);
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!(
            (signed_angle(DVec3::Y, DVec3::X, DVec3::Z) + std::f64::consts::FRAC_PI_2).abs()
                < 1e-12
        );
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(1.4, 1.0), 1.0);
        assert_eq!(snap(-2.6, 1.0), -3.0);
        assert_eq!(snap(1.37, 0.0), 1.37);
        assert_eq!(snap(22.0, 15.0), 15.0);
    }
}
