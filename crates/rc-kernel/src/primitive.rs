//! Parametric primitives.
//!
//! Every primitive is described by its bounding-box `size` (mm) plus shape parameters. In its
//! local frame it stands on z = 0 and is centred in x and y, so "drop to plate" and sizing are
//! intuitive. Tessellations are watertight and outward oriented.

use std::f64::consts::{PI, TAU};

use glam::DVec3;
use rc_mesh::Mesh;
use serde::{Deserialize, Serialize};

/// Maximum distance between a true curved surface and its tessellation, in mm.
pub const CHORD_TOLERANCE: f64 = 0.025;
const MIN_SEGMENTS: u32 = 16;
const MAX_SEGMENTS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveKind {
    Box,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    Wedge,
}

impl PrimitiveKind {
    pub const ALL: [Self; 6] = [
        Self::Box,
        Self::Cylinder,
        Self::Cone,
        Self::Sphere,
        Self::Torus,
        Self::Wedge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Sphere => "Sphere",
            Self::Torus => "Torus",
            Self::Wedge => "Wedge",
        }
    }

    /// A sensible default primitive of this kind (20 mm class).
    pub fn default_primitive(self) -> Primitive {
        let s = DVec3::splat(20.0);
        match self {
            Self::Box => Primitive::Box { size: s },
            Self::Cylinder => Primitive::Cylinder {
                size: s,
                top: 1.0,
                segments: None,
            },
            Self::Cone => Primitive::Cylinder {
                size: s,
                top: 0.0,
                segments: None,
            },
            Self::Sphere => Primitive::Sphere {
                size: s,
                segments: None,
            },
            Self::Torus => Primitive::Torus {
                size: DVec3::new(30.0, 30.0, 8.0),
                tube: 8.0,
                segments: None,
            },
            Self::Wedge => Primitive::Wedge { size: s },
        }
    }
}

/// A parametric solid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    Box {
        size: DVec3,
    },
    /// Elliptic cylinder or (truncated) cone. `top` is the top radius relative to the bottom.
    Cylinder {
        size: DVec3,
        top: f64,
        segments: Option<u32>,
    },
    /// Ellipsoid filling the bounding box.
    Sphere {
        size: DVec3,
        segments: Option<u32>,
    },
    /// Torus: outer diameter from `size.x/y`, tube height `size.z`, radial tube width `tube`.
    Torus {
        size: DVec3,
        tube: f64,
        segments: Option<u32>,
    },
    /// Triangular prism: full height at -x sloping down to zero at +x.
    Wedge {
        size: DVec3,
    },
}

impl Primitive {
    pub fn kind(&self) -> PrimitiveKind {
        match self {
            Self::Box { .. } => PrimitiveKind::Box,
            Self::Cylinder { top, .. } if *top == 0.0 => PrimitiveKind::Cone,
            Self::Cylinder { .. } => PrimitiveKind::Cylinder,
            Self::Sphere { .. } => PrimitiveKind::Sphere,
            Self::Torus { .. } => PrimitiveKind::Torus,
            Self::Wedge { .. } => PrimitiveKind::Wedge,
        }
    }

    /// Bounding-box size in mm.
    pub fn size(&self) -> DVec3 {
        match *self {
            Self::Box { size }
            | Self::Cylinder { size, .. }
            | Self::Sphere { size, .. }
            | Self::Torus { size, .. }
            | Self::Wedge { size } => size,
        }
    }

    /// Returns a copy with a new bounding-box size (each component clamped to ≥ 0.01 mm).
    pub fn with_size(&self, new: DVec3) -> Self {
        let new = new.max(DVec3::splat(0.01));
        let mut p = *self;
        match &mut p {
            Self::Box { size }
            | Self::Cylinder { size, .. }
            | Self::Sphere { size, .. }
            | Self::Wedge { size } => *size = new,
            Self::Torus { size, tube, .. } => {
                // Keep the tube proportional to the ring.
                let f = new.x.min(new.y) / size.x.min(size.y).max(1e-9);
                *tube = (*tube * f).clamp(0.01, new.x.min(new.y) * 0.5 - 1e-3);
                *size = new;
            }
        }
        p
    }

    /// Checks that the parameters describe a valid solid.
    pub fn validate(&self) -> Result<(), String> {
        let s = self.size();
        if !(s.is_finite() && s.min_element() > 0.0) {
            return Err("size must be positive".into());
        }
        match *self {
            Self::Cylinder { top, .. } if !(top.is_finite() && top >= 0.0) => {
                Err("top ratio must be ≥ 0".into())
            }
            Self::Torus { size, tube, .. }
                if !(tube.is_finite() && tube > 0.0 && tube < size.x.min(size.y) * 0.5) =>
            {
                Err("tube width must be between 0 and half the diameter".into())
            }
            Self::Cylinder {
                segments: Some(n), ..
            }
            | Self::Sphere {
                segments: Some(n), ..
            }
            | Self::Torus {
                segments: Some(n), ..
            } if !(3..=1024).contains(&n) => Err("segments must be between 3 and 1024".into()),
            _ => Ok(()),
        }
    }

    /// Stable hash of the parameters (for mesh caches).
    pub fn cache_key(&self) -> u64 {
        // FNV-1a over the parameter bits.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |x: u64| {
            for b in x.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        let s = self.size();
        feed(self.kind() as u64 + 1000);
        for c in [s.x, s.y, s.z] {
            feed(c.to_bits());
        }
        match *self {
            Self::Cylinder { top, segments, .. } => {
                feed(top.to_bits());
                feed(u64::from(segments.unwrap_or(0)));
            }
            Self::Sphere { segments, .. } => feed(u64::from(segments.unwrap_or(0))),
            Self::Torus { tube, segments, .. } => {
                feed(tube.to_bits());
                feed(u64::from(segments.unwrap_or(0)));
            }
            Self::Box { .. } | Self::Wedge { .. } => {}
        }
        h
    }

    /// Tessellates the primitive into a watertight, outward-facing mesh.
    pub fn mesh(&self) -> Mesh {
        match *self {
            Self::Box { size } => {
                let h = (size * 0.5).as_vec3();
                Mesh::cuboid(
                    glam::Vec3::new(-h.x, -h.y, 0.0),
                    glam::Vec3::new(h.x, h.y, size.z as f32),
                )
            }
            Self::Cylinder {
                size,
                top,
                segments,
            } => cylinder(size, top, segments),
            Self::Sphere { size, segments } => sphere(size, segments),
            Self::Torus {
                size,
                tube,
                segments,
            } => torus(size, tube, segments),
            Self::Wedge { size } => wedge(size),
        }
    }

    /// Exact volume of the ideal (untessellated) shape, in mm³.
    pub fn ideal_volume(&self) -> f64 {
        match *self {
            Self::Box { size } => size.x * size.y * size.z,
            Self::Cylinder { size, top, .. } => {
                let (a, b) = (size.x * 0.5, size.y * 0.5);
                PI * a * b * size.z * (1.0 + top + top * top) / 3.0
            }
            Self::Sphere { size, .. } => PI / 6.0 * size.x * size.y * size.z,
            Self::Torus { size, tube, .. } => {
                // Circular ring of radius R (centre of tube), elliptic tube (tube/2, size.z/2).
                let r_major = size.x.min(size.y) * 0.5 - tube * 0.5;
                TAU * r_major * PI * (tube * 0.5) * (size.z * 0.5)
            }
            Self::Wedge { size } => size.x * size.y * size.z * 0.5,
        }
    }
}

/// Segment count giving at most `CHORD_TOLERANCE` deviation for a circle of radius `r`.
pub fn adaptive_segments(r: f64) -> u32 {
    if r <= CHORD_TOLERANCE {
        return MIN_SEGMENTS;
    }
    let angle = 2.0 * (1.0 - CHORD_TOLERANCE / r).acos();
    let n = (TAU / angle).ceil() as u32;
    // Round up to a multiple of 4 so shapes stay symmetric about both axes.
    (n.div_ceil(4) * 4).clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

fn push(m: &mut Mesh, p: DVec3) -> u32 {
    m.positions.push(p.as_vec3().to_array());
    (m.positions.len() - 1) as u32
}

fn cylinder(size: DVec3, top: f64, segments: Option<u32>) -> Mesh {
    let (a, b) = (size.x * 0.5, size.y * 0.5);
    let n = segments.unwrap_or_else(|| adaptive_segments(a.max(b)));
    let mut m = Mesh::default();
    let bottom_c = push(&mut m, DVec3::ZERO);
    let top_c = push(&mut m, DVec3::new(0.0, 0.0, size.z));
    let ring = |m: &mut Mesh, scale: f64, z: f64| -> u32 {
        let first = m.positions.len() as u32;
        for i in 0..n {
            let t = TAU * f64::from(i) / f64::from(n);
            push(m, DVec3::new(a * scale * t.cos(), b * scale * t.sin(), z));
        }
        first
    };
    let b0 = ring(&mut m, 1.0, 0.0);
    let cone = top <= 1e-12;
    let t0 = if cone { 0 } else { ring(&mut m, top, size.z) };
    for i in 0..n {
        let j = (i + 1) % n;
        // Bottom cap faces -z.
        m.triangles.push([bottom_c, b0 + j, b0 + i]);
        if cone {
            m.triangles.push([b0 + i, b0 + j, top_c]);
        } else {
            m.triangles.push([top_c, t0 + i, t0 + j]);
            m.triangles.push([b0 + i, b0 + j, t0 + j]);
            m.triangles.push([b0 + i, t0 + j, t0 + i]);
        }
    }
    m
}

fn sphere(size: DVec3, segments: Option<u32>) -> Mesh {
    let r = size * 0.5;
    let n = segments.unwrap_or_else(|| adaptive_segments(r.max_element()));
    let rings = (n / 2).max(2);
    let mut m = Mesh::default();
    let centre = DVec3::new(0.0, 0.0, r.z);
    let south = push(&mut m, centre - DVec3::Z * r.z);
    let first = m.positions.len() as u32;
    for k in 1..rings {
        let phi = PI * f64::from(k) / f64::from(rings); // from south pole
        let (sp, cp) = phi.sin_cos();
        for i in 0..n {
            let t = TAU * f64::from(i) / f64::from(n);
            push(
                &mut m,
                centre + DVec3::new(r.x * sp * t.cos(), r.y * sp * t.sin(), -r.z * cp),
            );
        }
    }
    let north = push(&mut m, centre + DVec3::Z * r.z);
    let idx = |k: u32, i: u32| first + (k - 1) * n + (i % n);
    for i in 0..n {
        m.triangles.push([south, idx(1, i + 1), idx(1, i)]);
        m.triangles
            .push([north, idx(rings - 1, i), idx(rings - 1, i + 1)]);
    }
    for k in 1..rings - 1 {
        for i in 0..n {
            let (a, b, c, d) = (idx(k, i), idx(k, i + 1), idx(k + 1, i), idx(k + 1, i + 1));
            m.triangles.push([a, b, d]);
            m.triangles.push([a, d, c]);
        }
    }
    m
}

fn torus(size: DVec3, tube: f64, segments: Option<u32>) -> Mesh {
    let outer = size.x.min(size.y) * 0.5;
    let rt = tube * 0.5; // radial tube radius
    let rz = size.z * 0.5; // vertical tube radius
    let rc = outer - rt; // ring centre radius
    let (sx, sy) = (size.x / (2.0 * outer), size.y / (2.0 * outer)); // elliptic ring
    let n = segments.unwrap_or_else(|| adaptive_segments(outer));
    let k = segments.map_or_else(|| adaptive_segments(rt.max(rz)), |s| (s / 2).max(8));
    let mut m = Mesh::default();
    for i in 0..n {
        let u = TAU * f64::from(i) / f64::from(n);
        for j in 0..k {
            let v = TAU * f64::from(j) / f64::from(k);
            let rr = rc + rt * v.cos();
            push(
                &mut m,
                DVec3::new(sx * rr * u.cos(), sy * rr * u.sin(), rz + rz * v.sin()),
            );
        }
    }
    let idx = |i: u32, j: u32| (i % n) * k + (j % k);
    for i in 0..n {
        for j in 0..k {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i, j + 1), idx(i + 1, j + 1));
            m.triangles.push([a, b, d]);
            m.triangles.push([a, d, c]);
        }
    }
    m
}

fn wedge(size: DVec3) -> Mesh {
    let h = size * 0.5;
    let mut m = Mesh::default();
    let p = [
        DVec3::new(-h.x, -h.y, 0.0),
        DVec3::new(h.x, -h.y, 0.0),
        DVec3::new(-h.x, -h.y, size.z),
        DVec3::new(-h.x, h.y, 0.0),
        DVec3::new(h.x, h.y, 0.0),
        DVec3::new(-h.x, h.y, size.z),
    ];
    for v in p {
        push(&mut m, v);
    }
    m.triangles = vec![
        [0, 1, 2], // front (-y)
        [3, 5, 4], // back (+y)
        [0, 4, 1], // bottom
        [0, 3, 4],
        [0, 5, 3], // left (-x)
        [0, 2, 5],
        [1, 5, 2], // slope
        [1, 4, 5],
    ];
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_mesh::topology::analyze;

    fn check(p: &Primitive, rel_tol: f64) {
        let m = p.mesh();
        let r = analyze(&m);
        assert!(r.is_clean_solid(), "{p:?}: {r:?}");
        assert_eq!(r.components, 1, "{p:?}");
        let v = m.volume();
        let ideal = p.ideal_volume();
        assert!(v > 0.0, "{p:?} is inside out");
        assert!(
            (v - ideal).abs() / ideal < rel_tol,
            "{p:?}: volume {v} vs ideal {ideal}"
        );
        let b = m.aabb();
        let s = p.size().as_vec3();
        assert!(
            (b.size() - s).abs().max_element() < 1e-3,
            "{p:?}: bbox {:?}",
            b.size()
        );
        assert!(b.min.z.abs() < 1e-6, "{p:?} must stand on z = 0");
        assert!(
            b.center().truncate().length() < 1e-4,
            "{p:?} must be centred in xy"
        );
    }

    #[test]
    fn defaults_are_clean_solids() {
        for k in PrimitiveKind::ALL {
            check(&k.default_primitive(), 0.01);
        }
    }

    #[test]
    fn sphere_volume_within_one_percent() {
        check(
            &Primitive::Sphere {
                size: DVec3::splat(50.0),
                segments: None,
            },
            0.01,
        );
    }

    #[test]
    fn non_uniform_and_explicit_segments() {
        check(
            &Primitive::Cylinder {
                size: DVec3::new(40.0, 10.0, 5.0),
                top: 0.5,
                segments: Some(48),
            },
            0.01,
        );
        check(
            &Primitive::Sphere {
                size: DVec3::new(10.0, 20.0, 30.0),
                segments: Some(64),
            },
            0.02,
        );
        check(
            &Primitive::Torus {
                size: DVec3::new(50.0, 50.0, 6.0),
                tube: 10.0,
                segments: Some(64),
            },
            0.01,
        );
        check(
            &Primitive::Wedge {
                size: DVec3::new(30.0, 10.0, 7.0),
            },
            1e-9,
        );
    }

    #[test]
    fn adaptive_segments_grow_with_radius() {
        assert_eq!(adaptive_segments(0.01), MIN_SEGMENTS);
        assert!(adaptive_segments(5.0) < adaptive_segments(50.0));
        assert_eq!(adaptive_segments(10_000.0), MAX_SEGMENTS);
        assert_eq!(adaptive_segments(20.0) % 4, 0);
    }

    #[test]
    fn cache_key_and_resize() {
        let a = PrimitiveKind::Box.default_primitive();
        let b = a.with_size(DVec3::new(10.0, 20.0, 30.0));
        assert_ne!(a.cache_key(), b.cache_key());
        assert_eq!(b.size(), DVec3::new(10.0, 20.0, 30.0));
        assert_eq!(
            PrimitiveKind::Cone.default_primitive().kind(),
            PrimitiveKind::Cone
        );
        assert!(a.validate().is_ok());
        assert!(a.with_size(DVec3::ZERO).validate().is_ok(), "clamped");
    }

    proptest::proptest! {
        #[test]
        fn random_primitives_are_watertight(
            kind in 0usize..6,
            x in 0.5f64..200.0, y in 0.5f64..200.0, z in 0.5f64..200.0,
        ) {
            let p = PrimitiveKind::ALL[kind].default_primitive().with_size(DVec3::new(x, y, z));
            let r = analyze(&p.mesh());
            proptest::prop_assert!(r.is_clean_solid(), "{:?}: {:?}", p, r);
            proptest::prop_assert!(p.mesh().volume() > 0.0);
        }
    }
}
