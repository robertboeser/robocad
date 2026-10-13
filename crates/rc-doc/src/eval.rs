//! Evaluation of groups into solids (booleans), with caching and a background worker.
//!
//! Each node has a *content hash* describing its geometry in its own local frame (its own
//! transform excluded). Moving a group therefore never invalidates its cached result.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use glam::DMat4;
use rc_kernel::{GeometryKernel, KernelError, ManifoldKernel, Solid};
use rc_mesh::edges::{FeatureEdges, feature_edges};
use rc_mesh::{Bvh, Mesh, MeshInfo};

use crate::{Document, GroupOp, NodeId, NodeKind, Role};

/// Dihedral angle above which edges are drawn.
pub const FEATURE_ANGLE_DEG: f32 = 30.0;

/// A failure while evaluating a node.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{node}: {message}")]
pub struct EvalError {
    pub node: String,
    pub message: String,
}

/// Content hash of a node in its local frame, or `None` if the node does not exist.
pub fn content_hash(doc: &Document, id: NodeId) -> Option<u64> {
    let node = doc.get(id)?;
    let mut h = std::hash::DefaultHasher::new();
    match &node.kind {
        NodeKind::Imported { mesh, .. } => (1u8, mesh).hash(&mut h),
        NodeKind::Primitive(p) => (2u8, p.cache_key()).hash(&mut h),
        NodeKind::Group { children, op } => {
            (3u8, *op as u8).hash(&mut h);
            for &c in children {
                let Some(child) = doc.get(c) else { continue };
                if !child.visible {
                    continue;
                }
                (child.role as u8).hash(&mut h);
                for v in [
                    child.transform.translation,
                    child.transform.rotation_deg,
                    child.transform.scale,
                ] {
                    for x in v.to_array() {
                        x.to_bits().hash(&mut h);
                    }
                }
                content_hash(doc, c)?.hash(&mut h);
            }
        }
    }
    Some(h.finish())
}

/// Cache of solids by content hash, with a simple least-recently-used bound.
pub struct SolidCache {
    map: HashMap<u64, (Solid, u64)>,
    tick: u64,
    capacity: usize,
}

impl Default for SolidCache {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            capacity: 512,
        }
    }
}

impl SolidCache {
    fn get(&mut self, key: u64) -> Option<Solid> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(&key).map(|(s, t)| {
            *t = tick;
            s.clone()
        })
    }

    fn put(&mut self, key: u64, s: Solid) {
        self.tick += 1;
        self.map.insert(key, (s, self.tick));
        if self.map.len() > self.capacity {
            let mut ages: Vec<(u64, u64)> = self.map.iter().map(|(k, (_, t))| (*t, *k)).collect();
            ages.sort_unstable();
            for (_, k) in ages.iter().take(self.map.len() - self.capacity * 3 / 4) {
                self.map.remove(k);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Leaf geometry in local space.
fn leaf_mesh(doc: &Document, id: NodeId) -> Option<Arc<Mesh>> {
    match &doc.get(id)?.kind {
        NodeKind::Imported { mesh, .. } => doc.mesh(*mesh).cloned(),
        NodeKind::Primitive(p) => Some(Arc::new(p.mesh())),
        NodeKind::Group { .. } => None,
    }
}

fn describe(e: &KernelError) -> String {
    match e {
        KernelError::NotManifold(_) => {
            "the mesh is not watertight, so it cannot be combined (try Repair)".to_string()
        }
        KernelError::Other(m) => m.clone(),
    }
}

/// Evaluates a node to a solid in its local frame (its own transform not applied).
pub fn eval_solid(doc: &Document, id: NodeId, cache: &mut SolidCache) -> Result<Solid, EvalError> {
    let k = ManifoldKernel;
    let node = doc.get(id).ok_or_else(|| EvalError {
        node: "?".into(),
        message: "node not found".into(),
    })?;
    let key = content_hash(doc, id).unwrap_or(0);
    if let Some(s) = cache.get(key) {
        return Ok(s);
    }
    let solid = match &node.kind {
        NodeKind::Imported { .. } | NodeKind::Primitive(_) => {
            let mesh = leaf_mesh(doc, id).ok_or_else(|| EvalError {
                node: node.name.clone(),
                message: "mesh data is missing".into(),
            })?;
            k.solid_from_mesh(&mesh).map_err(|e| EvalError {
                node: node.name.clone(),
                message: describe(&e),
            })?
        }
        NodeKind::Group { children, op } => {
            let mut solids = Vec::new();
            let mut holes = Vec::new();
            for &c in children {
                let Some(child) = doc.get(c) else { continue };
                if !child.visible {
                    continue;
                }
                let s = eval_solid(doc, c, cache)?;
                let placed = k.transform(&s, &child.transform.matrix());
                match child.role {
                    Role::Solid => solids.push(placed),
                    Role::Hole => holes.push(placed),
                }
            }
            let base = match op {
                GroupOp::Combine => k.union(&solids),
                GroupOp::Intersect => k.intersection(&solids),
            };
            if holes.is_empty() {
                base
            } else {
                k.difference(&base, &k.union(&holes))
            }
        }
    };
    cache.put(key, solid.clone());
    Ok(solid)
}

/// Display data for an evaluated group.
pub struct EvalOutput {
    pub mesh: Arc<Mesh>,
    pub edges: FeatureEdges,
    pub bvh: Arc<Bvh>,
    pub info: MeshInfo,
    pub elapsed: Duration,
}

/// Evaluates a node and prepares everything needed to display it (local frame).
pub fn eval_display(
    doc: &Document,
    id: NodeId,
    cache: &mut SolidCache,
) -> Result<EvalOutput, EvalError> {
    let start = Instant::now();
    let solid = eval_solid(doc, id, cache)?;
    let mesh = ManifoldKernel.solid_to_mesh(&solid);
    let edges = feature_edges(&mesh, FEATURE_ANGLE_DEG);
    let bvh = Arc::new(Bvh::build(&mesh));
    let info = mesh.info();
    Ok(EvalOutput {
        mesh: Arc::new(mesh),
        edges,
        bvh,
        info,
        elapsed: start.elapsed(),
    })
}

/// Result of [`export_mesh`].
pub struct ExportResult {
    pub mesh: Mesh,
    /// Problems the user should know about (the export still happened).
    pub warnings: Vec<String>,
}

/// Builds the printable world-space mesh for `ids` (visible solids only; top-level holes are
/// ignored because holes only cut within a group). Parts that cannot be combined (not
/// watertight) are appended unchanged and reported.
pub fn export_mesh(doc: &Document, ids: &[NodeId], cache: &mut SolidCache) -> ExportResult {
    let k = ManifoldKernel;
    let mut solids = Vec::new();
    let mut raw = Mesh::default();
    let mut warnings = Vec::new();
    for id in crate::editor::top_level(doc, ids) {
        let Some(node) = doc.get(id) else { continue };
        if !doc.is_effectively_visible(id) || node.role == Role::Hole {
            continue;
        }
        let parent_world = node.parent.map_or(DMat4::IDENTITY, |p| doc.world_matrix(p));
        let world = parent_world * node.transform.matrix();
        match eval_solid(doc, id, cache) {
            Ok(s) => solids.push(k.transform(&s, &world)),
            Err(e) => {
                warnings.push(format!("{e} — exported without merging"));
                for leaf in doc.leaves(id) {
                    if let Some(m) = leaf_mesh(doc, leaf) {
                        raw.append(&m.transformed(&doc.world_matrix(leaf)));
                    }
                }
            }
        }
    }
    let mut mesh = k.solid_to_mesh(&k.union(&solids));
    if !raw.is_empty() {
        mesh.append(&raw);
    }
    ExportResult { mesh, warnings }
}

// ---------------------------------------------------------------------- worker

struct Job {
    doc: Document,
    targets: Vec<(NodeId, u64)>,
}

/// A finished evaluation.
pub struct EvalMessage {
    pub node: NodeId,
    pub hash: u64,
    pub result: Result<Arc<EvalOutput>, String>,
}

/// Evaluates groups on a background thread. Newer requests supersede older ones per node.
pub struct Evaluator {
    tx: Sender<Job>,
    rx: Receiver<EvalMessage>,
    pending: HashMap<NodeId, u64>,
}

impl Evaluator {
    /// `wake` is called (from the worker) whenever results are ready.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (res_tx, res_rx) = channel::<EvalMessage>();
        std::thread::Builder::new()
            .name("robocad-eval".into())
            .spawn(move || worker(&job_rx, &res_tx, &wake))
            .ok();
        Self {
            tx: job_tx,
            rx: res_rx,
            pending: HashMap::new(),
        }
    }

    /// Requests evaluation of `(node, hash)` pairs not already pending with that hash.
    pub fn request(&mut self, doc: &Document, targets: &[(NodeId, u64)]) {
        let new: Vec<(NodeId, u64)> = targets
            .iter()
            .copied()
            .filter(|(id, h)| self.pending.get(id) != Some(h))
            .collect();
        if new.is_empty() {
            return;
        }
        for (id, h) in &new {
            self.pending.insert(*id, *h);
        }
        let _ = self.tx.send(Job {
            doc: doc.clone(),
            targets: new,
        });
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn poll(&mut self) -> Vec<EvalMessage> {
        let msgs: Vec<EvalMessage> = self.rx.try_iter().collect();
        for m in &msgs {
            if self.pending.get(&m.node) == Some(&m.hash) {
                self.pending.remove(&m.node);
            }
        }
        msgs
    }
}

fn worker(jobs: &Receiver<Job>, results: &Sender<EvalMessage>, wake: &dyn Fn()) {
    let mut cache = SolidCache::default();
    while let Ok(mut job) = jobs.recv() {
        // Merge queued jobs: newest document, latest hash per node.
        let mut targets: HashMap<NodeId, u64> = job.targets.drain(..).collect();
        while let Ok(newer) = jobs.try_recv() {
            job.doc = newer.doc;
            targets.extend(newer.targets);
        }
        let mut done = HashSet::new();
        for (node, hash) in targets {
            if content_hash(&job.doc, node) != Some(hash) || !done.insert(hash) {
                // Superseded or duplicate: report so the requester can clear its pending entry.
                let _ = results.send(EvalMessage {
                    node,
                    hash,
                    result: Err(String::new()),
                });
                continue;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                eval_display(&job.doc, node, &mut cache)
            }))
            .map_err(|_| "internal error in the geometry kernel".to_string())
            .and_then(|r| r.map(Arc::new).map_err(|e| e.to_string()));
            let _ = results.send(EvalMessage { node, hash, result });
        }
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Editor, PrimitiveKind, Transform};
    use glam::DVec3;

    fn add(ed: &mut Editor, kind: PrimitiveKind, size: DVec3, at: DVec3) -> NodeId {
        ed.apply(Command::AddPrimitive {
            primitive: kind.default_primitive().with_size(size),
            position: at,
            color: [0; 3],
        })
        .unwrap()
        .created[0]
    }

    #[test]
    fn plate_with_hole() {
        let mut ed = Editor::default();
        let plate = add(
            &mut ed,
            PrimitiveKind::Box,
            DVec3::new(40.0, 40.0, 5.0),
            DVec3::ZERO,
        );
        let hole = add(
            &mut ed,
            PrimitiveKind::Cylinder,
            DVec3::new(10.0, 10.0, 20.0),
            DVec3::new(0.0, 0.0, -5.0),
        );
        ed.apply(Command::ToggleRole(vec![hole])).unwrap();
        let g = ed.apply(Command::Group(vec![plate, hole])).unwrap().created[0];
        let mut cache = SolidCache::default();
        let out = eval_display(ed.doc(), g, &mut cache).unwrap();
        let cyl = PrimitiveKind::Cylinder
            .default_primitive()
            .with_size(DVec3::new(10.0, 10.0, 20.0))
            .mesh()
            .volume();
        let expected = 40.0 * 40.0 * 5.0 - cyl * 5.0 / 20.0;
        assert!(
            (out.info.volume - expected).abs() < 1e-3,
            "{} vs {expected}",
            out.info.volume
        );
        assert!(rc_mesh::topology::analyze(&out.mesh).is_watertight());

        // Moving the group does not change its content hash; moving a child does.
        let h = content_hash(ed.doc(), g).unwrap();
        ed.apply(Command::SetTransforms(vec![(
            g,
            Transform::from_translation(DVec3::X * 50.0),
        )]))
        .unwrap();
        assert_eq!(content_hash(ed.doc(), g), Some(h));
        ed.apply(Command::TranslateWorld(vec![hole], DVec3::X * 3.0))
            .unwrap();
        assert_ne!(content_hash(ed.doc(), g), Some(h));
    }

    #[test]
    fn intersect_hidden_and_errors() {
        let mut ed = Editor::default();
        let a = add(&mut ed, PrimitiveKind::Box, DVec3::splat(20.0), DVec3::ZERO);
        let b = add(
            &mut ed,
            PrimitiveKind::Box,
            DVec3::splat(20.0),
            DVec3::new(10.0, 0.0, 0.0),
        );
        let g = ed.apply(Command::Group(vec![a, b])).unwrap().created[0];
        ed.apply(Command::SetGroupOp(g, GroupOp::Intersect))
            .unwrap();
        let mut cache = SolidCache::default();
        let v = eval_display(ed.doc(), g, &mut cache).unwrap().info.volume;
        assert!((v - 10.0 * 20.0 * 20.0).abs() < 1e-6, "{v}");
        // Hidden children are ignored.
        ed.apply(Command::SetVisible(b, false)).unwrap();
        let v = eval_display(ed.doc(), g, &mut cache).unwrap().info.volume;
        assert!((v - 8000.0).abs() < 1e-6, "{v}");

        // An open imported mesh makes the group fail with a clear message.
        let mut open = Mesh::cuboid(glam::Vec3::ZERO, glam::Vec3::ONE);
        open.triangles.pop();
        let bad = ed
            .apply(Command::Import {
                name: "broken.stl".into(),
                mesh: Arc::new(open),
                transform: Transform::IDENTITY,
                color: [0; 3],
            })
            .unwrap()
            .created[0];
        ed.apply(Command::Reparent {
            nodes: vec![bad],
            parent: Some(g),
            index: None,
        })
        .unwrap();
        let err = eval_display(ed.doc(), g, &mut cache).err().unwrap();
        assert_eq!(err.node, "broken.stl");
    }

    #[test]
    fn export_unions_roots_and_skips_holes() {
        let mut ed = Editor::default();
        let a = add(&mut ed, PrimitiveKind::Box, DVec3::splat(10.0), DVec3::ZERO);
        let b = add(
            &mut ed,
            PrimitiveKind::Box,
            DVec3::splat(10.0),
            DVec3::new(5.0, 0.0, 0.0),
        );
        let h = add(
            &mut ed,
            PrimitiveKind::Box,
            DVec3::splat(10.0),
            DVec3::new(50.0, 0.0, 0.0),
        );
        ed.apply(Command::ToggleRole(vec![h])).unwrap();
        let roots = ed.doc().roots.clone();
        let mut cache = SolidCache::default();
        let r = export_mesh(ed.doc(), &roots, &mut cache);
        assert!(r.warnings.is_empty());
        assert!(
            (r.mesh.volume() - 1500.0).abs() < 1e-6,
            "{}",
            r.mesh.volume()
        );
        assert!(rc_mesh::topology::analyze(&r.mesh).is_watertight());
        let _ = (a, b);
    }

    #[test]
    fn worker_returns_results() {
        let mut ed = Editor::default();
        let a = add(&mut ed, PrimitiveKind::Box, DVec3::splat(10.0), DVec3::ZERO);
        let g = ed.apply(Command::Group(vec![a])).unwrap().created[0];
        let (tx, rx) = channel();
        let mut ev = Evaluator::new(move || {
            let _ = tx.send(());
        });
        let h = content_hash(ed.doc(), g).unwrap();
        ev.request(ed.doc(), &[(g, h)]);
        assert!(ev.is_busy());
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let msgs = ev.poll();
        assert_eq!(msgs.len(), 1);
        assert!((msgs[0].result.as_ref().unwrap().info.volume - 1000.0).abs() < 1e-6);
        assert!(!ev.is_busy());
    }
}
