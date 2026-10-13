//! Background loading of mesh files so the UI never blocks.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use rc_mesh::edges::{FeatureEdges, feature_edges};
use rc_mesh::{Bvh, Mesh, MeshInfo, TopologyReport, stl, topology};

/// Dihedral angle above which an edge is drawn as a feature edge.
pub const FEATURE_ANGLE_DEG: f32 = 30.0;

/// A mesh loaded and analysed off the UI thread.
pub struct LoadedMesh {
    pub path: PathBuf,
    pub name: String,
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    pub edges: FeatureEdges,
    pub info: MeshInfo,
    pub topology: TopologyReport,
    pub warnings: Vec<String>,
    /// Safe fixes applied automatically while loading.
    pub auto_fixes: Vec<String>,
    pub elapsed: Duration,
}

pub type LoadResult = Result<LoadedMesh, (PathBuf, String)>;

pub struct Loader {
    tx: Sender<LoadResult>,
    rx: Receiver<LoadResult>,
    pending: usize,
}

impl Default for Loader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, pending: 0 }
    }
}

impl Loader {
    /// Starts loading a file on a worker thread. `wake` is called when the result is ready.
    pub fn load(&mut self, path: PathBuf, wake: impl Fn() + Send + 'static) {
        self.pending += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let p = path.clone();
            let result = std::panic::catch_unwind(move || load_file(path))
                .unwrap_or_else(|_| Err((p, "internal error while reading the file".into())));
            let _ = tx.send(result);
            wake();
        });
    }

    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Returns all results that arrived since the last call.
    pub fn poll(&mut self) -> Vec<LoadResult> {
        let results: Vec<_> = self.rx.try_iter().collect();
        self.pending = self.pending.saturating_sub(results.len());
        results
    }
}

/// Loads and analyses a mesh file on the calling thread.
pub fn load_file(path: PathBuf) -> LoadResult {
    let start = Instant::now();
    let is_obj = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("obj"));
    let (raw, warnings) = if is_obj {
        match rc_mesh::obj::read_file(&path) {
            Ok(r) => r,
            Err(e) => return Err((path, e.to_string())),
        }
    } else {
        match stl::read_file(&path) {
            Ok(f) => (f.mesh, f.warnings),
            Err(e) => return Err((path, e.to_string())),
        }
    };
    let (mesh, report) = rc_mesh::repair::repair(&raw, &rc_mesh::repair::RepairOptions::SAFE);
    let auto_fixes = report.summary();
    let name = path
        .file_name()
        .map_or_else(|| "mesh".to_string(), |n| n.to_string_lossy().into_owned());
    let mut loaded = analyse(path, name, mesh, start);
    loaded.warnings = warnings;
    loaded.auto_fixes = auto_fixes;
    Ok(loaded)
}

/// Computes everything the app needs for a mesh (edges, BVH, info, topology) in parallel.
pub fn analyse(path: PathBuf, name: String, mesh: Mesh, start: Instant) -> LoadedMesh {
    let ((edges, bvh), (info, topology)) = rayon::join(
        || {
            rayon::join(
                || feature_edges(&mesh, FEATURE_ANGLE_DEG),
                || Arc::new(Bvh::build(&mesh)),
            )
        },
        || rayon::join(|| mesh.info(), || topology::analyze(&mesh)),
    );
    LoadedMesh {
        path,
        name,
        mesh: Arc::new(mesh),
        bvh,
        edges,
        info,
        topology,
        warnings: Vec::new(),
        auto_fixes: Vec::new(),
        elapsed: start.elapsed(),
    }
}
