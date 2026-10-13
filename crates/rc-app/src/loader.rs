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
            let result = load_file(path);
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

fn load_file(path: PathBuf) -> LoadResult {
    let start = Instant::now();
    let file = match stl::read_file(&path) {
        Ok(f) => f,
        Err(e) => return Err((path, e.to_string())),
    };
    let mesh = file.mesh;
    // Independent analyses run in parallel.
    let ((edges, bvh), (info, topology)) = rayon::join(
        || {
            rayon::join(
                || feature_edges(&mesh, FEATURE_ANGLE_DEG),
                || Arc::new(Bvh::build(&mesh)),
            )
        },
        || rayon::join(|| mesh.info(), || topology::analyze(&mesh)),
    );
    let name = path
        .file_name()
        .map_or_else(|| "mesh".to_string(), |n| n.to_string_lossy().into_owned());
    Ok(LoadedMesh {
        path,
        name,
        mesh: Arc::new(mesh),
        bvh,
        edges,
        info,
        topology,
        warnings: file.warnings,
        elapsed: start.elapsed(),
    })
}
