//! Orbit camera with perspective and orthographic projection (Z up, reverse-Z depth).

use glam::dcamera::rh::{proj::directx, view};
use glam::{DMat4, DVec2, DVec3};
use rc_mesh::Aabb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

/// Standard view directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Iso,
}

/// A camera orbiting around `target`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: DVec3,
    /// Rotation around +Z in radians. 0 = front view (eye at -Y looking along +Y).
    pub yaw: f64,
    /// Elevation above the XY plane in radians.
    pub pitch: f64,
    pub distance: f64,
    /// Vertical field of view in radians (also defines the orthographic zoom level).
    pub fov_y: f64,
    pub projection: Projection,
    /// Radius of the scene around the target, used for orthographic depth range.
    pub scene_radius: f64,
}

const PITCH_LIMIT: f64 = 89.95 * std::f64::consts::PI / 180.0;
const MIN_DISTANCE: f64 = 0.05;
const MAX_DISTANCE: f64 = 1.0e6;

impl Default for OrbitCamera {
    fn default() -> Self {
        let mut c = Self {
            target: DVec3::new(0.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            distance: 400.0,
            fov_y: 35f64.to_radians(),
            projection: Projection::Perspective,
            scene_radius: 200.0,
        };
        c.set_view(StandardView::Iso);
        c
    }
}

impl OrbitCamera {
    /// Unit vector from target to eye.
    pub fn back_dir(&self) -> DVec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        DVec3::new(cp * sy, -cp * cy, sp)
    }

    pub fn eye(&self) -> DVec3 {
        self.target + self.back_dir() * self.distance
    }

    /// Viewing direction (from eye towards target).
    pub fn forward(&self) -> DVec3 {
        -self.back_dir()
    }

    /// Screen-right vector in world space.
    pub fn right(&self) -> DVec3 {
        self.forward().cross(DVec3::Z).normalize_or(DVec3::X)
    }

    /// Screen-up vector in world space.
    pub fn up(&self) -> DVec3 {
        self.right().cross(self.forward())
    }

    pub fn view(&self) -> DMat4 {
        view::look_at_mat4(self.eye(), self.target, self.up())
    }

    /// Half height of the view volume at the target distance.
    fn half_height(&self) -> f64 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// Projection matrix with reverse-Z depth (near = 1, far = 0).
    pub fn proj(&self, aspect: f64) -> DMat4 {
        let aspect = aspect.max(1e-6);
        match self.projection {
            Projection::Perspective => {
                directx::perspective_infinite_reverse(self.fov_y, aspect, self.near())
            }
            Projection::Orthographic => {
                let h = self.half_height();
                let w = h * aspect;
                let depth = self.scene_radius.max(1.0) * 4.0 + self.distance;
                // Swapped near/far gives reverse-Z.
                directx::orthographic(-w, w, -h, h, depth, -depth)
            }
        }
    }

    fn near(&self) -> f64 {
        (self.distance * 0.002).max(0.01)
    }

    pub fn view_proj(&self, aspect: f64) -> DMat4 {
        self.proj(aspect) * self.view()
    }

    /// World units per screen pixel at the target distance.
    pub fn units_per_pixel(&self, viewport_height: f64) -> f64 {
        2.0 * self.half_height() / viewport_height.max(1.0)
    }

    /// World-space ray through a pixel (`pixel` in points, origin at the top-left).
    pub fn ray(&self, pixel: DVec2, viewport: DVec2) -> (DVec3, DVec3) {
        let ndc = DVec2::new(
            2.0 * pixel.x / viewport.x.max(1.0) - 1.0,
            1.0 - 2.0 * pixel.y / viewport.y.max(1.0),
        );
        let inv = self.view_proj(viewport.x / viewport.y.max(1.0)).inverse();
        let p_near = inv.project_point3(ndc.extend(1.0));
        let p_mid = inv.project_point3(ndc.extend(0.5));
        (p_near, (p_mid - p_near).normalize_or(self.forward()))
    }

    /// Projects a world point to pixel coordinates (top-left origin). `None` if behind the camera.
    pub fn project(&self, p: DVec3, viewport: DVec2) -> Option<DVec2> {
        let clip = self.view_proj(viewport.x / viewport.y.max(1.0)) * p.extend(1.0);
        if clip.w <= 1e-9 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(DVec2::new(
            f64::midpoint(ndc.x, 1.0) * viewport.x,
            f64::midpoint(1.0, -ndc.y) * viewport.y,
        ))
    }

    /// Orbits by a pixel delta.
    pub fn orbit(&mut self, delta: DVec2) {
        const SPEED: f64 = 0.008;
        self.yaw -= delta.x * SPEED;
        self.pitch = (self.pitch + delta.y * SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Pans so the scene follows the cursor by a pixel delta.
    pub fn pan(&mut self, delta: DVec2, viewport_height: f64) {
        let s = self.units_per_pixel(viewport_height);
        self.target += (-self.right() * delta.x + self.up() * delta.y) * s;
    }

    /// Zooms by `factor` (< 1 zooms in) towards the point under the cursor.
    pub fn zoom(&mut self, factor: f64, cursor: Option<(DVec2, DVec2)>) {
        let new_distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let applied = new_distance / self.distance;
        if let Some((pixel, viewport)) = cursor {
            // Point on the plane through the target, facing the camera.
            let (o, d) = self.ray(pixel, viewport);
            let n = self.forward();
            let denom = d.dot(n);
            if denom.abs() > 1e-9 {
                let t = (self.target - o).dot(n) / denom;
                let p = o + d * t;
                self.target += (p - self.target) * (1.0 - applied);
            }
        }
        self.distance = new_distance;
    }

    /// Fits a bounding box into view, keeping the current direction.
    pub fn frame(&mut self, aabb: &Aabb, aspect: f64) {
        if aabb.is_empty() {
            return;
        }
        let radius = (aabb.diagonal() as f64 * 0.5).max(1.0);
        self.target = aabb.center().as_dvec3();
        let half_fov =
            (self.fov_y * 0.5).min((self.fov_y * 0.5).tan().atan2(1.0 / aspect.max(0.1)));
        self.distance = (radius / half_fov.sin() * 1.1).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.scene_radius = self.scene_radius.max(radius);
    }

    pub fn set_view(&mut self, view: StandardView) {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let (yaw, pitch) = match view {
            StandardView::Front => (0.0, 0.0),
            StandardView::Back => (PI, 0.0),
            StandardView::Right => (FRAC_PI_2, 0.0),
            StandardView::Left => (-FRAC_PI_2, 0.0),
            StandardView::Top => (0.0, PITCH_LIMIT),
            StandardView::Bottom => (0.0, -PITCH_LIMIT),
            StandardView::Iso => (FRAC_PI_4, 35.264f64.to_radians()),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-6
    }

    #[test]
    fn front_view_looks_along_plus_y() {
        let mut c = OrbitCamera::default();
        c.set_view(StandardView::Front);
        assert!(approx(c.forward(), DVec3::Y));
        assert!(approx(c.up(), DVec3::Z));
        assert!(approx(c.right(), DVec3::X));
        c.set_view(StandardView::Right);
        assert!(approx(c.forward(), -DVec3::X));
    }

    #[test]
    fn project_and_ray_are_inverse() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let c = OrbitCamera {
                projection,
                ..OrbitCamera::default()
            };
            let vp = DVec2::new(800.0, 600.0);
            let p = DVec3::new(12.0, -7.0, 3.0);
            let px = c.project(p, vp).unwrap();
            let (o, d) = c.ray(px, vp);
            let closest = o + d * (p - o).dot(d);
            assert!((closest - p).length() < 1e-6, "{projection:?}");
        }
    }

    #[test]
    fn target_projects_to_center_and_depth_is_reversed() {
        let c = OrbitCamera::default();
        let px = c.project(c.target, DVec2::new(800.0, 600.0)).unwrap();
        assert!((px - DVec2::new(400.0, 300.0)).length() < 1e-6);
        let vp = c.view_proj(800.0 / 600.0);
        let near = vp.project_point3(c.eye() + c.forward() * 1.0);
        let far = vp.project_point3(c.eye() + c.forward() * 1000.0);
        assert!(near.z > far.z, "reverse-Z: nearer points have larger depth");
        assert!((0.0..=1.0).contains(&near.z) && (0.0..=1.0).contains(&far.z));
    }

    #[test]
    fn zoom_towards_cursor_keeps_point_fixed() {
        let mut c = OrbitCamera::default();
        let vp = DVec2::new(800.0, 600.0);
        let cursor = DVec2::new(600.0, 200.0);
        let (o, d) = c.ray(cursor, vp);
        let t = (c.target - o).dot(c.forward()) / d.dot(c.forward());
        let world = o + d * t;
        c.zoom(0.5, Some((cursor, vp)));
        let after = c.project(world, vp).unwrap();
        assert!((after - cursor).length() < 1e-6);
    }

    #[test]
    fn frame_fits_box() {
        let mut c = OrbitCamera::default();
        let b = Aabb::new(glam::Vec3::splat(-10.0), glam::Vec3::splat(10.0));
        c.frame(&b, 1.5);
        let vp = DVec2::new(900.0, 600.0);
        for corner in b.corners() {
            let px = c.project(corner.as_dvec3(), vp).unwrap();
            assert!(px.x >= 0.0 && px.x <= vp.x && px.y >= 0.0 && px.y <= vp.y);
        }
    }
}
