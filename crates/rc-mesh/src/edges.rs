//! Feature edge extraction for display (sharp, boundary and non-manifold edges).

use crate::Mesh;
use crate::topology::edge_records;

/// Kind of an extracted edge, used for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Two faces meet at an angle sharper than the threshold.
    Sharp,
    /// Used by a single triangle (hole in the surface).
    Boundary,
    /// Used by three or more triangles.
    NonManifold,
}

/// Edges grouped by kind, as vertex index pairs into the mesh.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureEdges {
    pub sharp: Vec<[u32; 2]>,
    pub boundary: Vec<[u32; 2]>,
    pub non_manifold: Vec<[u32; 2]>,
}

impl FeatureEdges {
    pub fn len(&self) -> usize {
        self.sharp.len() + self.boundary.len() + self.non_manifold.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Extracts edges whose dihedral angle exceeds `angle_deg`, plus all boundary and
/// non-manifold edges.
pub fn feature_edges(mesh: &Mesh, angle_deg: f32) -> FeatureEdges {
    let cos_limit = angle_deg.to_radians().cos();
    let normals: Vec<glam::Vec3> = (0..mesh.triangles.len())
        .map(|i| mesh.face_normal(i))
        .collect();
    let edges = edge_records(mesh);
    let mut out = FeatureEdges::default();
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].key == edges[i].key {
            j += 1;
        }
        let key = edges[i].key;
        let pair = [(key >> 32) as u32, key as u32];
        match j - i {
            1 => out.boundary.push(pair),
            2 => {
                let (a, b) = (edges[i].tri as usize, edges[i + 1].tri as usize);
                if normals[a].dot(normals[b]) < cos_limit {
                    out.sharp.push(pair);
                }
            }
            _ => out.non_manifold.push(pair),
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn cube_has_12_sharp_edges() {
        let e = feature_edges(&Mesh::cuboid(Vec3::ZERO, Vec3::ONE), 30.0);
        assert_eq!(e.sharp.len(), 12);
        assert!(e.boundary.is_empty() && e.non_manifold.is_empty());
    }

    #[test]
    fn open_cube_has_boundary() {
        let mut m = Mesh::cuboid(Vec3::ZERO, Vec3::ONE);
        m.triangles.truncate(10);
        let e = feature_edges(&m, 30.0);
        assert_eq!(e.boundary.len(), 4);
    }
}
