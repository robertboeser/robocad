//! Bounding volume hierarchy over a mesh's triangles, for picking and ray queries.

use glam::Vec3;

use crate::{Aabb, Mesh};

/// A ray with origin and (not necessarily normalised) direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir }
    }
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

/// The closest intersection of a ray with a mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Ray parameter: `point = origin + dir * t`.
    pub t: f32,
    pub triangle: u32,
    pub point: Vec3,
    /// Geometric (face) normal, unit length.
    pub normal: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// Leaf: first index into `order`. Internal: index of the left child.
    start: u32,
    /// Number of triangles for leaves, 0 for internal nodes.
    count: u32,
    /// Internal: index of the right child.
    right: u32,
}

/// A flat, SAH-built BVH. It stores triangle indices only; the mesh is passed to queries.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

const BINS: usize = 12;
const LEAF_MAX: usize = 4;

impl Bvh {
    /// Builds a BVH for all triangles of `mesh`. Large subtrees are built in parallel.
    pub fn build(mesh: &Mesh) -> Self {
        use rayon::prelude::*;
        let n = mesh.triangles.len();
        if n == 0 {
            return Self::default();
        }
        let bounds: Vec<Aabb> = (0..n)
            .into_par_iter()
            .map(|i| Aabb::from_points(mesh.triangle(i)))
            .collect();
        let centroids: Vec<Vec3> = bounds.par_iter().map(Aabb::center).collect();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let nodes = build_rec(&mut order, 0, &bounds, &centroids);
        Self { nodes, order }
    }

    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or(Aabb::EMPTY, |n| n.bounds)
    }

    /// Finds the closest hit with `t` in `(0, t_max]`. Triangles are hit from both sides.
    pub fn raycast(&self, mesh: &Mesh, ray: &Ray, t_max: f32) -> Option<RayHit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = ray.dir.recip();
        let mut best: Option<(f32, u32)> = None;
        let mut limit = t_max;
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        stack.push(0);
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if node.bounds.ray_entry(ray.origin, inv, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                let range = node.start as usize..(node.start + node.count) as usize;
                for &tri in &self.order[range] {
                    if let Some(t) = intersect_triangle(ray, mesh.triangle(tri as usize))
                        && t <= limit
                    {
                        limit = t;
                        best = Some((t, tri));
                    }
                }
            } else {
                // Visit the nearer child first.
                let (l, r) = (node.start, node.right);
                let dl = self.nodes[l as usize]
                    .bounds
                    .ray_entry(ray.origin, inv, limit);
                let dr = self.nodes[r as usize]
                    .bounds
                    .ray_entry(ray.origin, inv, limit);
                match (dl, dr) {
                    (Some(a), Some(b)) if a <= b => {
                        stack.push(r);
                        stack.push(l);
                    }
                    (Some(_), Some(_)) => {
                        stack.push(l);
                        stack.push(r);
                    }
                    (Some(_), None) => stack.push(l),
                    (None, Some(_)) => stack.push(r),
                    (None, None) => {}
                }
            }
        }
        best.map(|(t, tri)| RayHit {
            t,
            triangle: tri,
            point: ray.at(t),
            normal: mesh.face_normal(tri as usize),
        })
    }

    /// Calls `f` for every triangle whose bounding box overlaps `query`.
    pub fn query_aabb(&self, mesh: &Mesh, query: &Aabb, mut f: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if !node.bounds.intersects(query) {
                continue;
            }
            if node.count > 0 {
                let range = node.start as usize..(node.start + node.count) as usize;
                for &tri in &self.order[range] {
                    if Aabb::from_points(mesh.triangle(tri as usize)).intersects(query) {
                        f(tri);
                    }
                }
            } else {
                stack.push(node.start);
                stack.push(node.right);
            }
        }
    }
}

/// Subtrees with more triangles than this are built on separate threads.
const PARALLEL_MIN: usize = 16_384;

/// Builds the subtree for `items` (which start at `offset` in the final order). The returned
/// nodes have the subtree root at index 0 and child indices relative to the returned vector.
fn build_rec(items: &mut [u32], offset: u32, bounds: &[Aabb], centroids: &[Vec3]) -> Vec<Node> {
    let node_bounds = items
        .iter()
        .fold(Aabb::EMPTY, |b, &t| b.union(&bounds[t as usize]));
    let leaf = |count: usize| {
        vec![Node {
            bounds: node_bounds,
            start: offset,
            count: count as u32,
            right: 0,
        }]
    };
    if items.len() <= LEAF_MAX {
        return leaf(items.len());
    }
    let Some(mid) = split(items, bounds, centroids) else {
        return leaf(items.len());
    };
    let len = items.len();
    let (l_items, r_items) = items.split_at_mut(mid);
    let (left, right) = if len >= PARALLEL_MIN {
        rayon::join(
            || build_rec(l_items, offset, bounds, centroids),
            || build_rec(r_items, offset + mid as u32, bounds, centroids),
        )
    } else {
        (
            build_rec(l_items, offset, bounds, centroids),
            build_rec(r_items, offset + mid as u32, bounds, centroids),
        )
    };
    let mut nodes = Vec::with_capacity(1 + left.len() + right.len());
    let right_root = 1 + left.len() as u32;
    nodes.push(Node {
        bounds: node_bounds,
        start: 1,
        count: 0,
        right: right_root,
    });
    let shift = |n: &Node, by: u32| {
        if n.count == 0 {
            Node {
                start: n.start + by,
                right: n.right + by,
                ..*n
            }
        } else {
            *n
        }
    };
    nodes.extend(left.iter().map(|n| shift(n, 1)));
    nodes.extend(right.iter().map(|n| shift(n, right_root)));
    nodes
}

/// Binned SAH split. Partitions `items` and returns the split position, or `None` for a leaf.
fn split(items: &mut [u32], bounds: &[Aabb], centroids: &[Vec3]) -> Option<usize> {
    let cb = Aabb::from_points(items.iter().map(|&t| centroids[t as usize]));
    let extent = cb.size();
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let (lo, span) = (cb.min[axis], extent[axis]);
    if span <= 0.0 {
        // All centroids coincide: split in the middle to bound leaf size.
        return Some(items.len() / 2);
    }
    let bin_of = |t: u32| -> usize {
        (((centroids[t as usize][axis] - lo) / span * BINS as f32) as usize).min(BINS - 1)
    };
    let mut bin_bounds = [Aabb::EMPTY; BINS];
    let mut bin_counts = [0usize; BINS];
    for &t in items.iter() {
        let b = bin_of(t);
        bin_counts[b] += 1;
        bin_bounds[b] = bin_bounds[b].union(&bounds[t as usize]);
    }
    // Sweep to find the cheapest split plane.
    let mut right_area = [0.0f32; BINS];
    let mut acc = Aabb::EMPTY;
    let mut cnt = 0;
    let mut right_cnt = [0usize; BINS];
    for i in (1..BINS).rev() {
        acc = acc.union(&bin_bounds[i]);
        cnt += bin_counts[i];
        right_area[i] = acc.surface_area();
        right_cnt[i] = cnt;
    }
    let mut best = (f32::INFINITY, 0);
    let mut acc = Aabb::EMPTY;
    let mut cnt = 0;
    for i in 0..BINS - 1 {
        acc = acc.union(&bin_bounds[i]);
        cnt += bin_counts[i];
        if cnt == 0 || right_cnt[i + 1] == 0 {
            continue;
        }
        let cost = acc.surface_area() * cnt as f32 + right_area[i + 1] * right_cnt[i + 1] as f32;
        if cost < best.0 {
            best = (cost, i);
        }
    }
    if !best.0.is_finite() {
        return Some(items.len() / 2);
    }
    let mut left = 0;
    for k in 0..items.len() {
        if bin_of(items[k]) <= best.1 {
            items.swap(k, left);
            left += 1;
        }
    }
    (left > 0 && left < items.len()).then_some(left)
}

/// Möller–Trumbore ray/triangle intersection (two-sided). Returns `t > 0`.
fn intersect_triangle(ray: &Ray, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = ray.dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < f32::EPSILON * e1.length() * e2.length() * ray.dir.length() {
        return None;
    }
    let inv = 1.0 / det;
    let s = ray.origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> Mesh {
        // n×n unit cubes spaced 2 apart.
        let mut m = Mesh::default();
        for i in 0..n {
            for j in 0..n {
                let o = Vec3::new(i as f32 * 2.0, j as f32 * 2.0, 0.0);
                m.append(&Mesh::cuboid(o, o + Vec3::ONE));
            }
        }
        m
    }

    #[test]
    fn hits_match_brute_force() {
        let m = grid(10);
        let bvh = Bvh::build(&m);
        for k in 0..200 {
            let x = (k % 20) as f32 + 0.37;
            let y = (k / 10) as f32 * 0.97 + 0.11;
            let ray = Ray::new(Vec3::new(x, y, 10.0), Vec3::new(0.01, -0.02, -1.0));
            let fast = bvh.raycast(&m, &ray, f32::INFINITY).map(|h| h.t);
            let brute = (0..m.triangles.len())
                .filter_map(|i| intersect_triangle(&ray, m.triangle(i)))
                .min_by(f32::total_cmp);
            assert_eq!(fast, brute, "ray {k}");
        }
    }

    #[test]
    fn top_face_hit() {
        let m = Mesh::cuboid(Vec3::ZERO, Vec3::splat(10.0));
        let bvh = Bvh::build(&m);
        let hit = bvh
            .raycast(
                &m,
                &Ray::new(Vec3::new(5.0, 5.0, 20.0), -Vec3::Z),
                f32::INFINITY,
            )
            .unwrap();
        assert!((hit.t - 10.0).abs() < 1e-5);
        assert!((hit.normal - Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn aabb_query() {
        let m = grid(4);
        let bvh = Bvh::build(&m);
        let mut found = Vec::new();
        bvh.query_aabb(&m, &Aabb::new(Vec3::splat(-0.5), Vec3::splat(1.5)), |t| {
            found.push(t);
        });
        assert_eq!(found.len(), 12);
    }
}
