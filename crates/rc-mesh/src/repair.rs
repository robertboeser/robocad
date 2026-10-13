//! Mesh repair: makes typical broken STL files watertight and consistently oriented.
//!
//! Each step is a separate function so it can be tested and reported on its own.
//! [`repair`] runs them in a sensible order and returns a [`RepairReport`].

use std::collections::VecDeque;

use glam::{DVec2, DVec3};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::Mesh;
use crate::topology::{TopologyReport, UnionFind, analyze, edge_records, is_degenerate};

/// Which repair steps to run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairOptions {
    /// Merge vertices closer than this fraction of the bounding-box diagonal (0 = off).
    pub weld_tolerance: f64,
    pub fill_holes: bool,
    /// Largest hole (in boundary edges) that is filled.
    pub max_hole_edges: usize,
    /// Remove connected parts with fewer triangles than this (0 = keep all).
    pub min_component_triangles: usize,
}

impl RepairOptions {
    /// Safe, lossless-in-practice fixes applied automatically on import.
    pub const SAFE: Self = Self {
        // The loader already merged identical vertices; near-welding is left to Repair.
        weld_tolerance: 0.0,
        fill_holes: false,
        max_hole_edges: 0,
        min_component_triangles: 0,
    };

    /// The full Repair command.
    pub const FULL: Self = Self {
        weld_tolerance: 1e-6,
        fill_holes: true,
        max_hole_edges: 5000,
        min_component_triangles: 0,
    };
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self::FULL
    }
}

/// What a repair changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub before: TopologyReport,
    pub after: TopologyReport,
    pub merged_vertices: usize,
    pub removed_degenerate: usize,
    pub removed_duplicates: usize,
    pub split_vertices: usize,
    pub flipped_triangles: usize,
    pub flipped_parts: usize,
    pub holes_filled: usize,
    pub holes_skipped: usize,
    pub removed_parts: usize,
}

impl RepairReport {
    /// True if any step changed the mesh.
    pub fn changed(&self) -> bool {
        self.merged_vertices
            + self.removed_degenerate
            + self.removed_duplicates
            + self.split_vertices
            + self.flipped_triangles
            + self.flipped_parts
            + self.holes_filled
            + self.removed_parts
            > 0
    }

    /// Human-readable list of changes.
    pub fn summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut add = |n: usize, what: &str| {
            if n > 0 {
                out.push(format!("{n} {what}"));
            }
        };
        add(self.merged_vertices, "duplicate vertices merged");
        add(self.removed_degenerate, "degenerate triangles removed");
        add(self.removed_duplicates, "duplicate triangles removed");
        add(self.split_vertices, "touching parts separated");
        add(self.flipped_triangles, "flipped triangles fixed");
        add(self.flipped_parts, "inside-out parts turned outward");
        add(self.holes_filled, "holes filled");
        add(self.holes_skipped, "holes too large or complex to fill");
        add(self.removed_parts, "tiny parts removed");
        out
    }
}

/// Runs the repair pipeline. Steps run only when the analysis shows they are needed, so clean
/// meshes cost a single analysis.
pub fn repair(mesh: &Mesh, opts: &RepairOptions) -> (Mesh, RepairReport) {
    let before = analyze(mesh);
    let mut r = RepairReport {
        before,
        after: before,
        ..RepairReport::default()
    };
    if before.is_clean_solid() && opts.min_component_triangles == 0 && mesh.volume() > 0.0 {
        return (mesh.clone(), r);
    }
    let mut m = mesh.clone();
    // Closing tiny gaps only helps meshes with open edges.
    if opts.weld_tolerance > 0.0 && before.boundary_edges > 0 {
        let tol = f64::from(m.aabb().diagonal()) * opts.weld_tolerance;
        r.merged_vertices = weld_epsilon(&mut m, tol);
    }
    r.removed_degenerate = remove_degenerate(&mut m);
    r.removed_duplicates = remove_duplicates(&mut m);
    if opts.min_component_triangles > 0 {
        r.removed_parts = remove_small_components(&mut m, opts.min_component_triangles);
    }
    let mut now = analyze(&m);
    if now.non_manifold_edges > 0 || now.non_manifold_vertices > 0 {
        r.split_vertices = split_touching(&mut m);
        now = analyze(&m);
    }
    if now.inconsistent_edges > 0 {
        r.flipped_triangles = orient_consistently(&mut m);
    }
    if opts.fill_holes && now.boundary_edges > 0 {
        let (filled, skipped) = fill_holes(&mut m, opts.max_hole_edges);
        r.holes_filled = filled;
        r.holes_skipped = skipped;
        if filled > 0 {
            r.flipped_triangles += orient_consistently(&mut m);
        }
    }
    r.flipped_parts = orient_outward(&mut m);
    compact(&mut m);
    r.after = analyze(&m);
    (m, r)
}

/// Merges vertices within `tol` of each other (spatial hash, 27-cell neighbourhood).
/// Vertices at bit-identical positions are left separate: the loader already welded exact
/// duplicates, so remaining ones were split on purpose.
pub fn weld_epsilon(m: &mut Mesh, tol: f64) -> usize {
    if tol <= 0.0 || m.positions.is_empty() {
        return 0;
    }
    let cell = |p: [f32; 3]| -> [i64; 3] { p.map(|c| (f64::from(c) / tol).floor() as i64) };
    let mut grid: FxHashMap<[i64; 3], Vec<u32>> = FxHashMap::default();
    let mut remap = vec![0u32; m.positions.len()];
    let mut kept: Vec<[f32; 3]> = Vec::with_capacity(m.positions.len());
    let tol2 = tol * tol;
    for (i, &p) in m.positions.iter().enumerate() {
        let c = cell(p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                        for &k in list {
                            let q = kept[k as usize];
                            // Bit-identical copies are intentional (separated touching parts).
                            if q == p {
                                continue;
                            }
                            let d2: f64 = (0..3)
                                .map(|j| (f64::from(p[j]) - f64::from(q[j])).powi(2))
                                .sum();
                            if d2 <= tol2 {
                                found = Some(k);
                                break 'search;
                            }
                        }
                    }
                }
            }
        }
        remap[i] = found.unwrap_or_else(|| {
            let k = kept.len() as u32;
            kept.push(p);
            grid.entry(c).or_default().push(k);
            k
        });
    }
    let merged = m.positions.len() - kept.len();
    m.positions = kept;
    for t in &mut m.triangles {
        *t = t.map(|v| remap[v as usize]);
    }
    merged
}

/// Removes triangles with repeated vertices or (near) zero area.
pub fn remove_degenerate(m: &mut Mesh) -> usize {
    let keep: Vec<bool> = (0..m.triangles.len())
        .into_par_iter()
        .map(|i| !is_degenerate(m, i))
        .collect();
    let before = m.triangles.len();
    let mut i = 0;
    m.triangles.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    before - m.triangles.len()
}

/// Removes triangles that use the same three vertices as another. Pairs with opposite
/// orientation (internal double walls) are both removed.
pub fn remove_duplicates(m: &mut Mesh) -> usize {
    let mut keys: Vec<([u32; 3], bool, usize)> = m
        .triangles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            // Canonical rotation starting at the smallest index keeps orientation information.
            let r = rotate_min(*t);
            let mut sorted = *t;
            sorted.sort_unstable();
            (sorted, r[1] < r[2], i)
        })
        .collect();
    keys.par_sort_unstable();
    let mut remove = vec![false; m.triangles.len()];
    let mut i = 0;
    while i < keys.len() {
        let mut j = i + 1;
        while j < keys.len() && keys[j].0 == keys[i].0 {
            j += 1;
        }
        if j - i > 1 {
            let fwd = keys[i..j].iter().filter(|k| k.1).count();
            let back = (j - i) - fwd;
            // Opposite pairs cancel; keep one of the surplus orientation.
            let cancel = fwd.min(back);
            let mut keep_one_fwd = fwd > cancel;
            let mut keep_one_back = back > cancel;
            for k in &keys[i..j] {
                let keep = if k.1 {
                    &mut keep_one_fwd
                } else {
                    &mut keep_one_back
                };
                if *keep {
                    *keep = false;
                } else {
                    remove[k.2] = true;
                }
            }
        }
        i = j;
    }
    let before = m.triangles.len();
    let mut idx = 0;
    m.triangles.retain(|_| {
        idx += 1;
        !remove[idx - 1]
    });
    before - m.triangles.len()
}

fn rotate_min(t: [u32; 3]) -> [u32; 3] {
    let k = (0..3).min_by_key(|&k| t[k]).unwrap_or(0);
    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
}

/// Components connected through manifold edges (edges with exactly two triangles).
fn manifold_components(m: &Mesh) -> UnionFind {
    let edges = edge_records(m);
    let mut uf = UnionFind::new(m.triangles.len());
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].key == edges[i].key {
            j += 1;
        }
        if j - i == 2 {
            uf.union(edges[i].tri, edges[i + 1].tri);
        }
        i = j;
    }
    uf
}

/// Gives every part (connected through manifold edges) its own copy of shared vertices.
/// This separates solids that merely touch at a vertex or along an edge.
pub fn split_touching(m: &mut Mesh) -> usize {
    let mut uf = manifold_components(m);
    // vertex -> first component seen; and per (vertex, component) the new vertex index.
    let mut first: Vec<Option<u32>> = vec![None; m.positions.len()];
    let mut copies: FxHashMap<(u32, u32), u32> = FxHashMap::default();
    let mut split = 0;
    for ti in 0..m.triangles.len() {
        let comp = uf.find(ti as u32);
        for k in 0..3 {
            let v = m.triangles[ti][k];
            match first[v as usize] {
                None => first[v as usize] = Some(comp),
                Some(c) if c == comp => {}
                Some(_) => {
                    let nv = *copies.entry((v, comp)).or_insert_with(|| {
                        m.positions.push(m.positions[v as usize]);
                        split += 1;
                        (m.positions.len() - 1) as u32
                    });
                    m.triangles[ti][k] = nv;
                }
            }
        }
    }
    // A vertex can still be shared by several fans of one part (bow-tie inside one surface).
    split + split_fans(m)
}

/// Splits vertices whose incident triangles form several separate fans.
fn split_fans(m: &mut Mesh) -> usize {
    let n = m.positions.len();
    let mut incident: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (ti, t) in m.triangles.iter().enumerate() {
        for &v in t {
            incident[v as usize].push(ti as u32);
        }
    }
    let mut split = 0;
    for (v, slot) in incident.iter_mut().enumerate() {
        let tris = std::mem::take(slot);
        if tris.len() < 2 {
            continue;
        }
        // Local union-find over fans linked by shared neighbour vertices.
        let mut parent: Vec<usize> = (0..tris.len()).collect();
        let mut others: Vec<(u32, usize)> = Vec::new();
        for (k, &t) in tris.iter().enumerate() {
            for &o in &m.triangles[t as usize] {
                if o != v as u32 {
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
        let mut fan_vertex: FxHashMap<usize, u32> = FxHashMap::default();
        let mut first_root = None;
        for (k, &t) in tris.iter().enumerate() {
            let root = find(&mut parent, k);
            if first_root.is_none() {
                first_root = Some(root);
            }
            if Some(root) == first_root {
                continue;
            }
            let nv = *fan_vertex.entry(root).or_insert_with(|| {
                m.positions.push(m.positions[v]);
                split += 1;
                (m.positions.len() - 1) as u32
            });
            for c in &mut m.triangles[t as usize] {
                if *c == v as u32 {
                    *c = nv;
                }
            }
        }
    }
    split
}

fn find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

/// Makes neighbouring triangles agree on orientation (BFS over manifold edges).
/// Returns the number of flipped triangles.
pub fn orient_consistently(m: &mut Mesh) -> usize {
    let edges = edge_records(m);
    // Adjacency: for each triangle, (neighbour, same_direction) pairs over manifold edges.
    let mut adj: Vec<Vec<(u32, bool)>> = vec![Vec::new(); m.triangles.len()];
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].key == edges[i].key {
            j += 1;
        }
        if j - i == 2 {
            let (a, b) = (edges[i], edges[i + 1]);
            let same = a.forward == b.forward;
            adj[a.tri as usize].push((b.tri, same));
            adj[b.tri as usize].push((a.tri, same));
        }
        i = j;
    }
    let mut flipped = vec![None::<bool>; m.triangles.len()];
    let mut count = 0;
    let mut queue = VecDeque::new();
    for seed in 0..m.triangles.len() {
        if flipped[seed].is_some() {
            continue;
        }
        flipped[seed] = Some(false);
        queue.push_back(seed);
        while let Some(t) = queue.pop_front() {
            let ft = flipped[t].unwrap_or(false);
            for &(n, same) in &adj[t] {
                if flipped[n as usize].is_none() {
                    // Consistent neighbours traverse the shared edge in opposite directions.
                    let fnb = ft ^ same;
                    flipped[n as usize] = Some(fnb);
                    queue.push_back(n as usize);
                }
            }
        }
    }
    for (t, f) in m.triangles.iter_mut().zip(&flipped) {
        if *f == Some(true) {
            t.swap(1, 2);
            count += 1;
        }
    }
    count
}

/// Flips every closed part whose signed volume is negative. Returns the number of parts flipped.
pub fn orient_outward(m: &mut Mesh) -> usize {
    let mut uf = manifold_components(m);
    let c = m.aabb().center().as_dvec3();
    let mut vol: FxHashMap<u32, f64> = FxHashMap::default();
    for ti in 0..m.triangles.len() {
        let [a, b, d] = m.triangle(ti).map(|v| v.as_dvec3() - c);
        *vol.entry(uf.find(ti as u32)).or_default() += a.dot(b.cross(d));
    }
    let flip: std::collections::HashSet<u32> = vol
        .iter()
        .filter(|(_, v)| **v < 0.0)
        .map(|(k, _)| *k)
        .collect();
    for ti in 0..m.triangles.len() {
        if flip.contains(&uf.find(ti as u32)) {
            m.triangles[ti].swap(1, 2);
        }
    }
    flip.len()
}

/// Removes parts with fewer than `min_triangles` triangles.
pub fn remove_small_components(m: &mut Mesh, min_triangles: usize) -> usize {
    let mut uf = manifold_components(m);
    let roots: Vec<u32> = (0..m.triangles.len() as u32).map(|t| uf.find(t)).collect();
    let mut size: FxHashMap<u32, usize> = FxHashMap::default();
    for r in &roots {
        *size.entry(*r).or_default() += 1;
    }
    let removed = size.values().filter(|s| **s < min_triangles).count();
    let mut i = 0;
    m.triangles.retain(|_| {
        i += 1;
        size[&roots[i - 1]] >= min_triangles
    });
    removed
}

/// Fills boundary loops. Returns (filled, skipped).
pub fn fill_holes(m: &mut Mesh, max_edges: usize) -> (usize, usize) {
    let edges = edge_records(m);
    // Boundary half-edges a→b; the hole polygon runs along the reversed edges b→a.
    let mut next: FxHashMap<u32, u32> = FxHashMap::default();
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].key == edges[i].key {
            j += 1;
        }
        if j - i == 1 {
            let e = edges[i];
            let (lo, hi) = ((e.key >> 32) as u32, e.key as u32);
            let (a, b) = if e.forward { (lo, hi) } else { (hi, lo) };
            next.insert(b, a);
        }
        i = j;
    }
    let (mut filled, mut skipped) = (0, 0);
    let mut starts: Vec<u32> = next.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        if !next.contains_key(&s) {
            continue;
        }
        let mut loop_v = vec![s];
        let mut cur = s;
        let mut closed = false;
        while let Some(n) = next.remove(&cur) {
            if n == s {
                closed = true;
                break;
            }
            loop_v.push(n);
            cur = n;
            if loop_v.len() > max_edges + 1 {
                break;
            }
        }
        if !closed || loop_v.len() < 3 || loop_v.len() > max_edges {
            skipped += 1;
            continue;
        }
        triangulate_loop(m, &loop_v);
        filled += 1;
    }
    (filled, skipped)
}

/// Triangulates a closed vertex loop (winding = desired orientation).
fn triangulate_loop(m: &mut Mesh, lv: &[u32]) {
    if lv.len() == 3 {
        m.triangles.push([lv[0], lv[1], lv[2]]);
        return;
    }
    let pts: Vec<DVec3> = lv.iter().map(|&v| m.vertex(v).as_dvec3()).collect();
    // Newell normal of the polygon in its winding.
    let mut n = DVec3::ZERO;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n += DVec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    let n = n.normalize_or(DVec3::Z);
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    let p2: Vec<DVec2> = pts.iter().map(|p| DVec2::new(p.dot(u), p.dot(v))).collect();
    if let Some(tris) = ear_clip(&p2) {
        for [a, b, c] in tris {
            m.triangles.push([lv[a], lv[b], lv[c]]);
        }
    } else {
        // Fallback: fan around a new centre vertex.
        let c = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
        m.positions.push(c.as_vec3().to_array());
        let ci = (m.positions.len() - 1) as u32;
        for i in 0..lv.len() {
            m.triangles.push([lv[i], lv[(i + 1) % lv.len()], ci]);
        }
    }
}

/// Ear clipping of a simple counter-clockwise polygon. `None` if it gets stuck.
fn ear_clip(p: &[DVec2]) -> Option<Vec<[usize; 3]>> {
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut idx: Vec<usize> = (0..p.len()).collect();
    let mut out = Vec::with_capacity(p.len() - 2);
    let mut guard = 0;
    while idx.len() > 3 {
        let n = idx.len();
        let mut clipped = false;
        for i in 0..n {
            let (a, b, c) = (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
            if cross(p[a], p[b], p[c]) <= 1e-14 {
                continue; // reflex or degenerate corner
            }
            let inside = idx.iter().any(|&q| {
                q != a && q != b && q != c && {
                    let d1 = cross(p[a], p[b], p[q]);
                    let d2 = cross(p[b], p[c], p[q]);
                    let d3 = cross(p[c], p[a], p[q]);
                    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
                }
            });
            if !inside {
                out.push([a, b, c]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        guard += 1;
        if !clipped || guard > p.len() * 2 {
            return None;
        }
    }
    out.push([idx[0], idx[1], idx[2]]);
    Some(out)
}

/// Removes unused vertices.
pub fn compact(m: &mut Mesh) {
    let mut used = vec![u32::MAX; m.positions.len()];
    let mut positions = Vec::with_capacity(m.positions.len());
    for t in &mut m.triangles {
        for v in t.iter_mut() {
            let slot = &mut used[*v as usize];
            if *slot == u32::MAX {
                *slot = positions.len() as u32;
                positions.push(m.positions[*v as usize]);
            }
            *v = *slot;
        }
    }
    m.positions = positions;
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn cube() -> Mesh {
        Mesh::cuboid(Vec3::ZERO, Vec3::splat(10.0))
    }

    #[test]
    fn fills_a_missing_triangle_and_a_missing_face() {
        for remove in [1, 2] {
            let mut m = cube();
            m.triangles.truncate(12 - remove);
            let (fixed, r) = repair(&m, &RepairOptions::FULL);
            assert!(r.after.is_clean_solid(), "{r:?}");
            assert_eq!(r.holes_filled, 1);
            assert!((fixed.volume() - 1000.0).abs() < 1e-6);
        }
    }

    #[test]
    fn fixes_orientation() {
        let mut m = cube();
        m.triangles[3].swap(1, 2);
        m.triangles[7].swap(1, 2);
        let (fixed, r) = repair(&m, &RepairOptions::SAFE);
        assert!(r.after.is_clean_solid());
        assert_eq!(r.flipped_triangles, 2);
        assert!((fixed.volume() - 1000.0).abs() < 1e-6);
        m.flip();
        let (fixed, r) = repair(&cube_flipped(), &RepairOptions::SAFE);
        assert_eq!(r.flipped_parts, 1);
        assert!(fixed.volume() > 0.0);
    }

    fn cube_flipped() -> Mesh {
        let mut m = cube();
        m.flip();
        m
    }

    #[test]
    fn removes_duplicates_and_double_walls() {
        let mut m = cube();
        let t = m.triangles[0];
        m.triangles.push(t); // exact duplicate
        let (_, r) = repair(&m, &RepairOptions::SAFE);
        assert_eq!(r.removed_duplicates, 1);
        assert!(r.after.is_clean_solid());
        let mut m = cube();
        m.triangles.push([0, 1, 2]);
        m.triangles.push([0, 2, 1]); // internal double wall
        assert_eq!(remove_duplicates(&mut m), 2);
    }

    #[test]
    fn welds_tiny_gaps() {
        let mut m = cube();
        // Detach one corner by 1e-6 mm in one triangle.
        let v = m.triangles[0][0];
        let mut p = m.positions[v as usize];
        p[0] += 1e-6;
        m.positions.push(p);
        let nv = (m.positions.len() - 1) as u32;
        m.triangles[0][0] = nv;
        assert!(!analyze(&m).is_watertight());
        let (_, r) = repair(&m, &RepairOptions::FULL);
        assert!(r.merged_vertices >= 1);
        assert!(r.after.is_clean_solid(), "{r:?}");
    }

    #[test]
    fn ear_clip_concave() {
        // L-shape, counter-clockwise.
        let p = [
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let tris = ear_clip(&p).unwrap();
        assert_eq!(tris.len(), 4);
        let area: f64 = tris
            .iter()
            .map(|[a, b, c]| 0.5 * (p[*b] - p[*a]).perp_dot(p[*c] - p[*a]))
            .sum();
        assert!((area - 3.0).abs() < 1e-12);
    }
}
