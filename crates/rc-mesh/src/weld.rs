//! Vertex welding: turning triangle soups into indexed meshes.

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::Mesh;

/// Bit pattern key of a position. `-0.0` is normalised to `0.0` so both weld together.
#[inline]
fn key(p: [f32; 3]) -> [u32; 3] {
    // Adding 0.0 turns -0.0 into +0.0 and leaves every other value unchanged.
    [
        (p[0] + 0.0).to_bits(),
        (p[1] + 0.0).to_bits(),
        (p[2] + 0.0).to_bits(),
    ]
}

/// Welds a triangle soup by exact position equality (lossless).
///
/// Vertices keep the order of their first appearance, which preserves memory locality.
/// Triangles containing non-finite coordinates are dropped; the count is returned.
pub fn weld_exact(soup: &[[[f32; 3]; 3]]) -> (Mesh, usize) {
    let mut map: FxHashMap<[u32; 3], u32> =
        FxHashMap::with_capacity_and_hasher(soup.len() / 2 + 16, FxBuildHasher);
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(soup.len() / 2 + 16);
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(soup.len());
    let mut dropped = 0;
    for tri in soup {
        if !tri.iter().flatten().all(|v| v.is_finite()) {
            dropped += 1;
            continue;
        }
        let mut idx = [0u32; 3];
        for (slot, &p) in idx.iter_mut().zip(tri) {
            let next = positions.len() as u32;
            *slot = *map.entry(key(p)).or_insert_with(|| {
                positions.push([p[0] + 0.0, p[1] + 0.0, p[2] + 0.0]);
                next
            });
        }
        triangles.push(idx);
    }
    (
        Mesh {
            positions,
            triangles,
        },
        dropped,
    )
}

/// Merges vertices of an indexed mesh that are identical, removing unused vertices.
pub fn reweld_exact(mesh: &Mesh) -> Mesh {
    let soup: Vec<_> = (0..mesh.triangles.len())
        .map(|i| mesh.triangle_positions(i))
        .collect();
    weld_exact(&soup).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welds_shared_vertices_and_negative_zero() {
        let soup = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [-0.0, 1.0, 0.0]],
        ];
        let (m, dropped) = weld_exact(&soup);
        assert_eq!(dropped, 0);
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.triangles, vec![[0, 1, 2], [1, 3, 2]]);
    }

    #[test]
    fn drops_non_finite() {
        let soup = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[f32::NAN, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        ];
        let (m, dropped) = weld_exact(&soup);
        assert_eq!(dropped, 1);
        assert_eq!(m.triangles.len(), 1);
    }
}
