//! Repaired broken fixtures must be accepted by the boolean kernel.

use rc_kernel::{GeometryKernel, ManifoldKernel};
use rc_mesh::repair::{RepairOptions, repair};

#[test]
fn kernel_accepts_repaired_fixtures() {
    for name in [
        "cube_missing_triangle.stl",
        "cube_flipped_triangle.stl",
        "cube_inverted.stl",
        "cube_degenerate.stl",
        "bowtie_vertex.stl",
        "shared_edge_cubes.stl",
    ] {
        let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let mesh = rc_mesh::stl::read_file(&path).unwrap().mesh;
        let (fixed, _) = repair(&mesh, &RepairOptions::FULL);
        let solid = ManifoldKernel
            .solid_from_mesh(&fixed)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(ManifoldKernel.volume(&solid) > 0.0, "{name}");
    }
}
