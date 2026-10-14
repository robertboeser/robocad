//! Rounding of convex edges: a morphological opening (erode, then dilate) by a ball.
//!
//! Opening rounds every convex (outward) edge and corner with the ball radius, keeps flat faces
//! exactly where they were and leaves concave edges sharp. Parts thinner than the ball's diameter
//! disappear. Convex solids take a fast path (offset face planes, then one hull); others use
//! Manifold's Minkowski operations, whose cost grows with triangle count × ball vertices.

use std::collections::HashMap;
use std::f64::consts::PI;

use glam::DVec3;
use manifold_csg::Manifold;

use crate::primitive::CHORD_TOLERANCE;

const MIN_BALL_SEGMENTS: u32 = 8;
const MAX_BALL_SEGMENTS: u32 = 32;
/// Upper bound of input triangles × ball segments² on the general (non-convex) path, which keeps
/// typical parts within about a second.
const GENERAL_BUDGET: f64 = 200.0 * 32.0 * 32.0;

/// Ball resolution for `radius`: adaptive to [`CHORD_TOLERANCE`], reduced for large non-convex
/// inputs, a multiple of 4 (Manifold's sphere is a subdivided octahedron).
pub fn ball_segments(radius: f64, triangles: usize, convex: bool) -> u32 {
    let ratio = (1.0 - CHORD_TOLERANCE / radius.max(CHORD_TOLERANCE)).clamp(-1.0, 1.0);
    let adaptive = (PI / ratio.acos().max(1e-9)).ceil();
    let cap = if convex {
        f64::from(MAX_BALL_SEGMENTS)
    } else {
        (GENERAL_BUDGET / triangles.max(1) as f64).sqrt()
    };
    let n = adaptive
        .min(cap)
        .clamp(f64::from(MIN_BALL_SEGMENTS), f64::from(MAX_BALL_SEGMENTS)) as u32;
    n.div_ceil(4) * 4
}

/// Rounds the convex edges of `m` with `radius` mm. Returns an empty solid when nothing is left
/// (the part is thinner than 2 × `radius` everywhere).
pub(crate) fn round_edges(m: &Manifold, radius: f64) -> Manifold {
    if m.is_empty() || !(radius.is_finite() && radius > 0.0) {
        return m.clone();
    }
    let convex = is_convex(m);
    let ball = Manifold::sphere(radius, ball_segments(radius, m.num_tri(), convex) as i32);
    if convex && let Some(r) = open_convex(m, &ball) {
        return r;
    }
    open_general(m, &ball)
}

/// True if the solid equals its convex hull (one convex shell). The tolerance absorbs the f32
/// rounding of tessellated curved primitives (~1e-8 relative for a sphere).
fn is_convex(m: &Manifold) -> bool {
    let v = m.volume();
    v > 0.0 && (m.hull().volume() - v).abs() <= 1e-7 * v
}

/// Opening through Manifold's Minkowski difference and sum (any solid).
fn open_general(m: &Manifold, ball: &Manifold) -> Manifold {
    let eroded = m.minkowski_difference(ball);
    if eroded.is_empty() {
        // The sum with an empty solid would return the ball itself.
        return Manifold::empty();
    }
    eroded.minkowski_sum(ball)
}

/// Positions of a Manifold mesh plus its triangles.
fn mesh_points(m: &Manifold) -> (Vec<DVec3>, Vec<[usize; 3]>) {
    let (verts, n_props, tris) = m.to_mesh_f64();
    let n = n_props.max(3);
    let pts = verts
        .chunks_exact(n)
        .map(|v| DVec3::new(v[0], v[1], v[2]))
        .collect();
    let tris = tris
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|i| i as usize))
        .collect();
    (pts, tris)
}

/// Opening of a convex solid. The eroded body is the intersection of the face half-spaces, each
/// moved inward by the ball's support distance (so the dilation restores flat faces exactly). It
/// is computed by polar duality around an interior point: the convex hull of the dual points
/// `n / distance` has one facet per vertex of the eroded body. Returns `None` if the centroid
/// is not inside the eroded body (the caller then takes the general path).
fn open_convex(m: &Manifold, ball: &Manifold) -> Option<Manifold> {
    let (pts, tris) = mesh_points(m);
    let (ball_pts, _) = mesh_points(ball);

    // Volume centroid and the distinct face planes (n · x = d).
    let mut six_vol = 0.0;
    let mut weighted = DVec3::ZERO;
    let mut planes: HashMap<[i64; 4], (DVec3, f64)> = HashMap::new();
    let q = |x: f64| (x * 1e7).round() as i64;
    for t in &tris {
        let [a, b, c] = t.map(|i| pts[i]);
        let v = a.dot(b.cross(c));
        six_vol += v;
        weighted += (a + b + c) * v;
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-12 {
            continue;
        }
        let n = n / len;
        let d = n.dot(a);
        planes
            .entry([q(n.x), q(n.y), q(n.z), q(d)])
            .or_insert((n, d));
    }
    if six_vol <= 0.0 {
        return None;
    }
    let centre = weighted / (4.0 * six_vol);

    let mut dual = Vec::with_capacity(planes.len());
    for (n, d) in planes.values() {
        let support = ball_pts
            .iter()
            .map(|p| n.dot(*p))
            .fold(f64::NEG_INFINITY, f64::max);
        let dist = d - support - n.dot(centre);
        if dist <= 1e-9 {
            return None;
        }
        dual.push((*n / dist).to_array());
    }
    let (dual_pts, dual_tris) = mesh_points(&Manifold::hull_pts(&dual));
    let mut eroded = Vec::with_capacity(dual_tris.len());
    for t in &dual_tris {
        let [a, b, c] = t.map(|i| dual_pts[i]);
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-15 {
            continue;
        }
        let n = n / len;
        let e = n.dot(a);
        if e <= 1e-15 {
            return None;
        }
        eroded.push((centre + n / e).to_array());
    }
    let eroded = Manifold::hull_pts(&eroded);
    if eroded.is_empty() {
        return None;
    }
    Some(eroded.minkowski_sum(ball))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GeometryKernel, ManifoldKernel, PrimitiveKind, Solid};
    use glam::DMat4;
    use rc_mesh::topology::analyze;

    fn k() -> ManifoldKernel {
        ManifoldKernel
    }

    fn prim(kind: PrimitiveKind, size: DVec3, at: DVec3) -> Solid {
        let s = k()
            .solid_from_mesh(&kind.default_primitive().with_size(size).mesh())
            .expect("primitive is manifold");
        k().transform(&s, &DMat4::from_translation(at))
    }

    /// True if a tiny cube around `p` lies inside `s`.
    fn contains(s: &Solid, p: DVec3) -> bool {
        let probe = prim(PrimitiveKind::Box, DVec3::splat(0.01), p - DVec3::Z * 0.005);
        k().volume(&k().intersection(&[s.clone(), probe])) > 0.5e-6
    }

    fn check_solid(s: &Solid) {
        let m = k().solid_to_mesh(s);
        let r = analyze(&m);
        assert!(r.is_watertight() && r.orientation_consistent(), "{r:?}");
    }

    #[test]
    fn segments_adapt_to_radius_and_size() {
        assert_eq!(ball_segments(0.1, 12, true), MIN_BALL_SEGMENTS);
        assert_eq!(ball_segments(50.0, 12, true), MAX_BALL_SEGMENTS);
        let r2 = ball_segments(2.0, 12, true);
        assert!(r2.is_multiple_of(4) && (16..=24).contains(&r2), "{r2}");
        assert_eq!(ball_segments(50.0, 100_000, false), MIN_BALL_SEGMENTS);
    }

    #[test]
    fn rounded_box_matches_analytic_volume_and_keeps_its_size() {
        let (size, r) = (DVec3::new(40.0, 30.0, 10.0), 2.0);
        let b = prim(PrimitiveKind::Box, size, DVec3::ZERO);
        let out = k().round_edges(&b, r);
        check_solid(&out);
        // Edges lose (1 − π/4)·r² per unit length, corners (1 − π/6)·r³ each.
        let edges = 4.0 * (size.x + size.y + size.z - 6.0 * r);
        let expected = size.x * size.y * size.z
            - (1.0 - PI / 4.0) * r * r * edges
            - 8.0 * (1.0 - PI / 6.0) * r * r * r;
        let v = k().volume(&out);
        assert!((v - expected).abs() / expected < 2e-3, "{v} vs {expected}");
        // Flat faces stay where they were: the bounding box is unchanged.
        let (a, o) = (k().solid_to_mesh(&b).aabb(), k().solid_to_mesh(&out).aabb());
        assert!((a.min - o.min).length() < 1e-4 && (a.max - o.max).length() < 1e-4);
    }

    #[test]
    fn convex_fast_path_agrees_with_general_path() {
        for kind in [
            PrimitiveKind::Box,
            PrimitiveKind::Cylinder,
            PrimitiveKind::Wedge,
        ] {
            let s = prim(kind, DVec3::new(20.0, 16.0, 12.0), DVec3::ZERO);
            assert!(is_convex(&s.0), "{kind:?}");
            let ball = Manifold::sphere(1.5, 16);
            let fast = open_convex(&s.0, &ball).expect("fast path applies");
            let slow = open_general(&s.0, &ball);
            let (vf, vs) = (fast.volume(), slow.volume());
            assert!((vf - vs).abs() / vs < 1e-6, "{kind:?}: {vf} vs {vs}");
        }
    }

    #[test]
    fn concave_edges_stay_sharp() {
        // An L-shaped prism: the inner (concave) edge runs along x at y = z = 10.
        let a = prim(
            PrimitiveKind::Box,
            DVec3::new(20.0, 20.0, 10.0),
            DVec3::new(10.0, 10.0, 0.0),
        );
        let b = prim(
            PrimitiveKind::Box,
            DVec3::new(20.0, 10.0, 20.0),
            DVec3::new(10.0, 5.0, 0.0),
        );
        let l = k().union(&[a, b]);
        assert!(!is_convex(&l.0));
        let out = k().round_edges(&l, 1.0);
        check_solid(&out);
        assert!(k().volume(&out) < k().volume(&l));
        // Nothing is added: the result lies inside the input.
        assert!(k().volume(&k().difference(&out, &l)) < 1e-6);
        // Material right at the concave edge is kept, the convex edge is rounded off.
        assert!(contains(&out, DVec3::new(10.0, 9.98, 9.98)));
        assert!(!contains(&out, DVec3::new(10.0, 19.98, 0.02)));
        assert!(contains(&out, DVec3::new(10.0, 15.0, 5.0)));
    }

    #[test]
    fn too_large_radius_leaves_nothing() {
        let thin = prim(PrimitiveKind::Box, DVec3::new(20.0, 20.0, 2.0), DVec3::ZERO);
        assert!(k().is_empty(&k().round_edges(&thin, 1.5)));
        let a = prim(PrimitiveKind::Box, DVec3::new(20.0, 20.0, 2.0), DVec3::ZERO);
        let b = prim(PrimitiveKind::Box, DVec3::new(2.0, 20.0, 20.0), DVec3::ZERO);
        let l = k().union(&[a, b]);
        assert!(k().is_empty(&k().round_edges(&l, 1.5)));
    }
}
