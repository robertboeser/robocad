//! Indexed triangle mesh.

use glam::{DMat4, DVec3, Vec3};

use crate::Aabb;

/// An indexed triangle mesh. Triangles are counter-clockwise when seen from outside.
///
/// Meshes are usually shared as `Arc<Mesh>` and treated as immutable once built.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Cheap-to-copy summary of a mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshInfo {
    pub aabb: Aabb,
    /// Signed volume in mm³ (negative if the mesh is inside out).
    pub volume: f64,
    /// Surface area in mm².
    pub area: f64,
    pub triangle_count: usize,
    pub vertex_count: usize,
}

impl Mesh {
    pub fn new(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Self {
        Self {
            positions,
            triangles,
        }
    }

    /// Builds a mesh from a triangle soup with exact welding, dropping non-finite triangles.
    pub fn from_soup(soup: &[[[f32; 3]; 3]]) -> Self {
        crate::weld::weld_exact(soup).0
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    #[inline]
    pub fn vertex(&self, i: u32) -> Vec3 {
        Vec3::from_array(self.positions[i as usize])
    }

    /// The three corner positions of triangle `i`.
    #[inline]
    pub fn triangle(&self, i: usize) -> [Vec3; 3] {
        let [a, b, c] = self.triangles[i];
        [self.vertex(a), self.vertex(b), self.vertex(c)]
    }

    #[inline]
    pub fn triangle_positions(&self, i: usize) -> [[f32; 3]; 3] {
        let [a, b, c] = self.triangles[i];
        [
            self.positions[a as usize],
            self.positions[b as usize],
            self.positions[c as usize],
        ]
    }

    /// Unit face normal (zero for degenerate triangles), computed in `f64`.
    pub fn face_normal(&self, i: usize) -> Vec3 {
        let [a, b, c] = self.triangle(i).map(Vec3::as_dvec3);
        (b - a).cross(c - a).normalize_or_zero().as_vec3()
    }

    pub fn aabb(&self) -> Aabb {
        // Only referenced vertices count; unreferenced ones are ignored.
        let mut b = Aabb::EMPTY;
        for t in &self.triangles {
            for &v in t {
                b.extend(self.vertex(v));
            }
        }
        b
    }

    /// Signed volume via the divergence theorem, relative to the bbox centre for precision.
    pub fn volume(&self) -> f64 {
        let c = self.aabb().center().as_dvec3();
        let mut sum = 0.0f64;
        for i in 0..self.triangles.len() {
            let [a, b, d] = self.triangle(i).map(|v| v.as_dvec3() - c);
            sum += a.dot(b.cross(d));
        }
        sum / 6.0
    }

    pub fn area(&self) -> f64 {
        (0..self.triangles.len())
            .map(|i| {
                let [a, b, c] = self.triangle(i).map(Vec3::as_dvec3);
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    pub fn info(&self) -> MeshInfo {
        MeshInfo {
            aabb: self.aabb(),
            volume: self.volume(),
            area: self.area(),
            triangle_count: self.triangle_count(),
            vertex_count: self.vertex_count(),
        }
    }

    /// Reverses the winding of every triangle.
    pub fn flip(&mut self) {
        for t in &mut self.triangles {
            t.swap(1, 2);
        }
    }

    /// Applies an affine transform. Mirroring transforms flip the winding so faces stay outward.
    pub fn transformed(&self, m: &DMat4) -> Self {
        let positions = self
            .positions
            .iter()
            .map(|p| {
                m.transform_point3(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                    .as_vec3()
                    .to_array()
            })
            .collect();
        let mut out = Self {
            positions,
            triangles: self.triangles.clone(),
        };
        if m.determinant() < 0.0 {
            out.flip();
        }
        out
    }

    /// Appends another mesh (no welding between the two).
    pub fn append(&mut self, other: &Self) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.triangles.extend(
            other
                .triangles
                .iter()
                .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
        );
    }

    /// Stable 64-bit content hash (xxh3) over positions and triangles.
    pub fn content_hash(&self) -> u64 {
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(&(self.positions.len() as u64).to_le_bytes());
        h.update(bytemuck::cast_slice(&self.positions));
        h.update(bytemuck::cast_slice(&self.triangles));
        h.digest()
    }

    /// Approximate heap memory used, in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.positions.len() * 12 + self.triangles.len() * 12
    }

    /// An axis-aligned box from `min` to `max`, 12 triangles, outward facing.
    pub fn cuboid(min: Vec3, max: Vec3) -> Self {
        let positions: Vec<[f32; 3]> = Aabb::new(min, max)
            .corners()
            .iter()
            .map(glam::Vec3::to_array)
            .collect();
        // Corner index bits: x = 1, y = 2, z = 4.
        let triangles = vec![
            [0, 2, 1],
            [1, 2, 3], // bottom (-z)
            [4, 5, 6],
            [5, 7, 6], // top (+z)
            [0, 1, 4],
            [1, 5, 4], // front (-y)
            [2, 6, 3],
            [3, 6, 7], // back (+y)
            [0, 4, 2],
            [2, 4, 6], // left (-x)
            [1, 3, 5],
            [3, 7, 5], // right (+x)
        ];
        Self {
            positions,
            triangles,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_volume_area() {
        let m = Mesh::cuboid(Vec3::ZERO, Vec3::splat(10.0));
        assert!((m.volume() - 1000.0).abs() < 1e-9);
        assert!((m.area() - 600.0).abs() < 1e-9);
        assert_eq!(m.aabb().size(), Vec3::splat(10.0));
    }

    #[test]
    fn mirror_keeps_outward() {
        let m = Mesh::cuboid(Vec3::ZERO, Vec3::splat(2.0));
        let mirrored = m.transformed(&DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)));
        assert!((mirrored.volume() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn hash_changes_with_content() {
        let a = Mesh::cuboid(Vec3::ZERO, Vec3::ONE);
        let b = Mesh::cuboid(Vec3::ZERO, Vec3::splat(2.0));
        assert_eq!(a.content_hash(), a.clone().content_hash());
        assert_ne!(a.content_hash(), b.content_hash());
    }
}
