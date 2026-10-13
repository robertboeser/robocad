//! Topological analysis: watertightness, manifoldness, orientation, components.

use rayon::prelude::*;

use crate::Mesh;

/// Result of analysing a mesh's topology.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopologyReport {
    /// Edges used by exactly one triangle (holes in the surface).
    pub boundary_edges: usize,
    /// Edges used by more than two triangles.
    pub non_manifold_edges: usize,
    /// Vertices where several separate triangle fans meet (e.g. two cones touching at a tip).
    pub non_manifold_vertices: usize,
    /// Triangles with repeated vertex indices or (near) zero area.
    pub degenerate_triangles: usize,
    /// Manifold edges whose two triangles traverse them in the same direction.
    pub inconsistent_edges: usize,
    /// Edge-connected components.
    pub components: usize,
}

impl TopologyReport {
    /// Closed surface without holes or non-manifold edges/vertices.
    pub fn is_watertight(&self) -> bool {
        self.boundary_edges == 0 && self.non_manifold_edges == 0 && self.non_manifold_vertices == 0
    }

    /// Watertight, consistently oriented and free of degenerate triangles.
    pub fn is_clean_solid(&self) -> bool {
        self.is_watertight() && self.inconsistent_edges == 0 && self.degenerate_triangles == 0
    }

    pub fn orientation_consistent(&self) -> bool {
        self.inconsistent_edges == 0
    }
}

/// Undirected edge key with the original direction encoded in the lowest bit of `dir`.
#[derive(Clone, Copy)]
pub(crate) struct EdgeRec {
    pub(crate) key: u64,
    pub(crate) tri: u32,
    /// `true` if the triangle traverses the edge from lower to higher index.
    pub(crate) forward: bool,
}

/// All directed triangle edges, sorted by undirected key (degenerate edges skipped).
pub(crate) fn edge_records(mesh: &Mesh) -> Vec<EdgeRec> {
    let mut edges: Vec<EdgeRec> = mesh
        .triangles
        .par_iter()
        .enumerate()
        .flat_map_iter(|(ti, t)| {
            (0..3).filter_map(move |k| {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if a == b {
                    return None;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                Some(EdgeRec {
                    key: (u64::from(lo) << 32) | u64::from(hi),
                    tri: ti as u32,
                    forward: a < b,
                })
            })
        })
        .collect();
    edges.par_sort_unstable_by_key(|e| e.key);
    edges
}

/// Union-find with path halving.
pub(crate) struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }
    pub(crate) fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = p;
            x = p;
        }
        x
    }
    pub(crate) fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb) as usize] = ra.min(rb);
        }
    }
}

/// Analyses the topology of a mesh.
pub fn analyze(mesh: &Mesh) -> TopologyReport {
    let mut r = TopologyReport::default();
    let tri_count = mesh.triangles.len();
    if tri_count == 0 {
        return r;
    }
    r.degenerate_triangles = (0..tri_count)
        .into_par_iter()
        .filter(|&i| is_degenerate(mesh, i))
        .count();

    let edges = edge_records(mesh);
    let mut uf = UnionFind::new(tri_count);
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].key == edges[i].key {
            j += 1;
        }
        let group = &edges[i..j];
        match group.len() {
            1 => r.boundary_edges += 1,
            2 => {
                if group[0].forward == group[1].forward {
                    r.inconsistent_edges += 1;
                }
            }
            _ => r.non_manifold_edges += 1,
        }
        for e in &group[1..] {
            uf.union(group[0].tri, e.tri);
        }
        i = j;
    }
    r.components = (0..tri_count as u32).filter(|&t| uf.find(t) == t).count();
    r.non_manifold_vertices = count_non_manifold_vertices(mesh);
    r
}

/// Degenerate = repeated index or area below a tolerance relative to the longest edge.
pub fn is_degenerate(mesh: &Mesh, i: usize) -> bool {
    let t = mesh.triangles[i];
    if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
        return true;
    }
    let [a, b, c] = mesh.triangle(i).map(glam::Vec3::as_dvec3);
    let cross = (b - a).cross(c - a).length();
    let longest = (b - a)
        .length_squared()
        .max((c - b).length_squared())
        .max((a - c).length_squared());
    cross <= 1e-12 * longest || cross == 0.0
}

/// Counts vertices whose incident triangles form more than one edge-connected fan.
fn count_non_manifold_vertices(mesh: &Mesh) -> usize {
    let n = mesh.positions.len();
    // CSR adjacency: vertex -> incident triangles.
    let mut counts = vec![0u32; n + 1];
    for t in &mesh.triangles {
        for &v in t {
            counts[v as usize + 1] += 1;
        }
    }
    for i in 0..n {
        counts[i + 1] += counts[i];
    }
    let offsets = counts.clone();
    let mut fill = counts;
    let mut incident = vec![0u32; offsets[n] as usize];
    for (ti, t) in mesh.triangles.iter().enumerate() {
        for &v in t {
            incident[fill[v as usize] as usize] = ti as u32;
            fill[v as usize] += 1;
        }
    }
    (0..n)
        .into_par_iter()
        .filter(|&v| {
            let tris = &incident[offsets[v] as usize..offsets[v + 1] as usize];
            tris.len() > 1 && fan_count(mesh, v as u32, tris) > 1
        })
        .count()
}

fn fan_count(mesh: &Mesh, v: u32, tris: &[u32]) -> usize {
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    // Small local union-find over the incident triangles, linked via shared "other" vertices.
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    let mut others: Vec<(u32, usize)> = Vec::with_capacity(tris.len() * 2);
    for (k, &t) in tris.iter().enumerate() {
        for &o in &mesh.triangles[t as usize] {
            if o != v {
                others.push((o, k));
            }
        }
    }
    others.sort_unstable();
    for w in others.windows(2) {
        if w[0].0 == w[1].0 {
            let (a, b) = (find(&mut parent, w[0].1), find(&mut parent, w[1].1));
            if a != b {
                parent[a] = b;
            }
        }
    }
    (0..tris.len())
        .filter(|&k| find(&mut parent, k) == k)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn cube_is_clean() {
        let r = analyze(&Mesh::cuboid(Vec3::ZERO, Vec3::ONE));
        assert!(r.is_clean_solid(), "{r:?}");
        assert_eq!(r.components, 1);
    }

    #[test]
    fn missing_triangle_gives_boundary() {
        let mut m = Mesh::cuboid(Vec3::ZERO, Vec3::ONE);
        m.triangles.pop();
        let r = analyze(&m);
        assert_eq!(r.boundary_edges, 3);
        assert!(!r.is_watertight());
    }

    #[test]
    fn flipped_triangle_is_inconsistent() {
        let mut m = Mesh::cuboid(Vec3::ZERO, Vec3::ONE);
        m.triangles[0].swap(1, 2);
        let r = analyze(&m);
        assert!(r.is_watertight());
        assert_eq!(r.inconsistent_edges, 3);
    }
}
