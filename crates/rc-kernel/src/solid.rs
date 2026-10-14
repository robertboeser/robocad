//! Solids and boolean operations, backed by the Manifold library.

use glam::DMat4;
use rc_mesh::Mesh;

/// Errors from the geometry kernel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KernelError {
    #[error("the mesh is not a closed, manifold solid ({0})")]
    NotManifold(String),
    #[error("geometry kernel error: {0}")]
    Other(String),
}

/// Operations a geometry backend must provide. robocad uses [`ManifoldKernel`]; the trait keeps
/// other backends (e.g. pure Rust for WASM) possible.
pub trait GeometryKernel: Send + Sync {
    type Solid: Clone + Send + Sync;

    fn solid_from_mesh(&self, mesh: &Mesh) -> Result<Self::Solid, KernelError>;
    fn solid_to_mesh(&self, solid: &Self::Solid) -> Mesh;
    fn transform(&self, solid: &Self::Solid, m: &DMat4) -> Self::Solid;
    fn union(&self, parts: &[Self::Solid]) -> Self::Solid;
    fn difference(&self, a: &Self::Solid, b: &Self::Solid) -> Self::Solid;
    fn intersection(&self, parts: &[Self::Solid]) -> Self::Solid;
    /// Rounds every convex edge and corner with `radius` mm; concave edges stay sharp and parts
    /// thinner than 2 × `radius` disappear (morphological opening by a ball, see [`crate::round`]).
    fn round_edges(&self, solid: &Self::Solid, radius: f64) -> Self::Solid;
    fn empty(&self) -> Self::Solid;
    fn volume(&self, solid: &Self::Solid) -> f64;
    fn is_empty(&self, solid: &Self::Solid) -> bool;
    fn triangle_count(&self, solid: &Self::Solid) -> usize;
}

/// The Manifold backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManifoldKernel;

/// A solid in the Manifold backend (cheap to clone: Manifold shares its internals).
#[derive(Clone, Debug)]
pub struct Solid(pub(crate) manifold_csg::Manifold);

impl Solid {
    pub(crate) fn wrap(m: manifold_csg::Manifold) -> Self {
        Self(m)
    }
}

impl ManifoldKernel {
    fn build(mesh: &Mesh) -> Result<manifold_csg::Manifold, manifold_csg::CsgError> {
        let verts: Vec<f64> = mesh
            .positions
            .iter()
            .flat_map(|p| p.map(f64::from))
            .collect();
        let tris: Vec<u64> = mesh
            .triangles
            .iter()
            .flat_map(|t| t.map(u64::from))
            .collect();
        manifold_csg::Manifold::from_mesh_f64(&verts, 3, &tris)
    }
}

impl GeometryKernel for ManifoldKernel {
    type Solid = Solid;

    fn solid_from_mesh(&self, mesh: &Mesh) -> Result<Solid, KernelError> {
        if mesh.is_empty() {
            return Ok(self.empty());
        }
        match Self::build(mesh) {
            Ok(m) => Ok(Solid(m)),
            Err(manifold_csg::CsgError::ManifoldStatus(s)) => {
                Err(KernelError::NotManifold(format!("{s:?}")))
            }
            Err(e) => Err(KernelError::Other(e.to_string())),
        }
    }

    fn solid_to_mesh(&self, solid: &Solid) -> Mesh {
        let (verts, n_props, tris) = solid.0.to_mesh_f64();
        let n_props = n_props.max(3);
        let positions = verts
            .chunks_exact(n_props)
            .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
            .collect();
        let triangles = tris
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| i as u32))
            .collect();
        Mesh::new(positions, triangles)
    }

    fn transform(&self, solid: &Solid, m: &DMat4) -> Solid {
        let c = m.to_cols_array();
        // Column-major 3×4: x basis, y basis, z basis, translation.
        let m12 = [
            c[0], c[1], c[2], c[4], c[5], c[6], c[8], c[9], c[10], c[12], c[13], c[14],
        ];
        Solid(solid.0.transform(&m12))
    }

    fn union(&self, parts: &[Solid]) -> Solid {
        match parts {
            [] => self.empty(),
            [one] => one.clone(),
            _ => {
                let ms: Vec<manifold_csg::Manifold> = parts.iter().map(|s| s.0.clone()).collect();
                Solid(manifold_csg::Manifold::batch_union(&ms))
            }
        }
    }

    fn difference(&self, a: &Solid, b: &Solid) -> Solid {
        Solid(a.0.difference(&b.0))
    }

    fn intersection(&self, parts: &[Solid]) -> Solid {
        let Some((first, rest)) = parts.split_first() else {
            return self.empty();
        };
        rest.iter()
            .fold(first.clone(), |acc, s| Solid(acc.0.intersection(&s.0)))
    }

    fn round_edges(&self, solid: &Solid, radius: f64) -> Solid {
        Solid(crate::round::round_edges(&solid.0, radius))
    }

    fn empty(&self) -> Solid {
        Solid(manifold_csg::Manifold::empty())
    }

    fn volume(&self, solid: &Solid) -> f64 {
        solid.0.volume()
    }

    fn is_empty(&self, solid: &Solid) -> bool {
        solid.0.is_empty()
    }

    fn triangle_count(&self, solid: &Solid) -> usize {
        solid.0.num_tri()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, PrimitiveKind};
    use glam::{DQuat, DVec3};
    use rc_mesh::topology::analyze;

    fn k() -> ManifoldKernel {
        ManifoldKernel
    }

    fn solid(p: &Primitive, m: DMat4) -> Solid {
        let s = k()
            .solid_from_mesh(&p.mesh())
            .expect("primitive is manifold");
        k().transform(&s, &m)
    }

    fn rel(a: f64, b: f64) -> f64 {
        (a - b).abs() / a.abs().max(b.abs()).max(1e-9)
    }

    #[test]
    fn box_minus_cylinder() {
        let b = solid(
            &PrimitiveKind::Box
                .default_primitive()
                .with_size(DVec3::new(40.0, 40.0, 10.0)),
            DMat4::IDENTITY,
        );
        let c = solid(
            &PrimitiveKind::Cylinder
                .default_primitive()
                .with_size(DVec3::new(10.0, 10.0, 30.0)),
            DMat4::from_translation(DVec3::new(0.0, 0.0, -10.0)),
        );
        let r = k().difference(&b, &c);
        let m = k().solid_to_mesh(&r);
        assert!(analyze(&m).is_watertight());
        let expected = 40.0 * 40.0 * 10.0 - k().volume(&c) * 10.0 / 30.0;
        assert!(
            rel(k().volume(&r), expected) < 1e-6,
            "{} vs {expected}",
            k().volume(&r)
        );
        assert!(rel(m.volume(), k().volume(&r)) < 1e-6);
        // One through-hole: genus 1, still one component.
        assert_eq!(analyze(&m).components, 1);
    }

    #[test]
    fn open_mesh_is_rejected() {
        let mut m = Mesh::cuboid(glam::Vec3::ZERO, glam::Vec3::ONE);
        m.triangles.pop();
        assert!(matches!(
            k().solid_from_mesh(&m),
            Err(KernelError::NotManifold(_))
        ));
    }

    #[test]
    fn transform_matches_glam() {
        let p = PrimitiveKind::Box.default_primitive();
        let m = DMat4::from_scale_rotation_translation(
            DVec3::new(1.0, 2.0, 0.5),
            DQuat::from_rotation_z(0.7),
            DVec3::new(5.0, -3.0, 2.0),
        );
        let ours = p.mesh().transformed(&m).aabb();
        let theirs = k().solid_to_mesh(&solid(&p, m)).aabb();
        assert!((ours.min - theirs.min).length() < 1e-4 && (ours.max - theirs.max).length() < 1e-4);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(48))]
        #[test]
        fn boolean_volume_identities(
            ka in 0usize..2, kb in 0usize..2,
            sa in proptest::array::uniform3(5.0f64..40.0),
            sb in proptest::array::uniform3(5.0f64..40.0),
            off in proptest::array::uniform3(-20.0f64..20.0),
            angle in 0.0f64..std::f64::consts::TAU,
        ) {
            let kinds = [PrimitiveKind::Box, PrimitiveKind::Cylinder];
            let pa = kinds[ka].default_primitive().with_size(DVec3::from_array(sa));
            let pb = kinds[kb].default_primitive().with_size(DVec3::from_array(sb));
            let a = solid(&pa, DMat4::IDENTITY);
            let b = solid(&pb, DMat4::from_rotation_translation(
                DQuat::from_rotation_z(angle) * DQuat::from_rotation_x(angle * 0.3),
                DVec3::from_array(off),
            ));
            let (va, vb) = (k().volume(&a), k().volume(&b));
            let u = k().union(&[a.clone(), b.clone()]);
            let i = k().intersection(&[a.clone(), b.clone()]);
            let d = k().difference(&a, &b);
            let (vu, vi, vd) = (k().volume(&u), k().volume(&i), k().volume(&d));
            proptest::prop_assert!(rel(vu + vi, va + vb) < 1e-6, "union/intersection: {} vs {}", vu + vi, va + vb);
            proptest::prop_assert!((vd - (va - vi)).abs() <= 1e-6 * va.max(1.0), "difference: {} vs {}", vd, va - vi);
            for s in [&u, &i, &d] {
                let m = k().solid_to_mesh(s);
                if !m.is_empty() {
                    let r = analyze(&m);
                    proptest::prop_assert!(r.is_watertight() && r.orientation_consistent(), "{:?}", r);
                }
            }
        }
    }
}
