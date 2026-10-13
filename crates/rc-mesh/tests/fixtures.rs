//! Loads every STL fixture in `tests/fixtures` and checks the parsed result and topology.

use rc_mesh::stl::{self, StlError, StlFormat};
use rc_mesh::topology::analyze;

fn path(name: &str) -> String {
    format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn load(name: &str) -> stl::StlFile {
    stl::read_file(path(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn ascii_and_binary_cubes_agree() {
    let a = load("cube_ascii.stl");
    let b = load("cube_binary.stl");
    let c = load("cube_binary_solid_header.stl");
    assert_eq!(a.format, StlFormat::Ascii);
    assert_eq!(b.format, StlFormat::Binary);
    assert_eq!(c.format, StlFormat::Binary, "binary with 'solid' header");
    assert_eq!(a.name, "cube");
    assert_eq!(a.mesh, b.mesh);
    assert_eq!(b.mesh, c.mesh);
    assert_eq!(a.mesh.vertex_count(), 8);
    assert!((a.mesh.volume() - 1000.0).abs() < 1e-9);
    assert!(analyze(&a.mesh).is_clean_solid());
}

#[test]
fn missing_triangle() {
    let r = analyze(&load("cube_missing_triangle.stl").mesh);
    assert_eq!(r.boundary_edges, 3);
    assert!(!r.is_watertight());
}

#[test]
fn overlapping_cubes_are_two_components() {
    let r = analyze(&load("two_overlapping_cubes.stl").mesh);
    assert!(r.is_clean_solid());
    assert_eq!(r.components, 2);
}

#[test]
fn bowtie_vertex_is_non_manifold() {
    let f = load("bowtie_vertex.stl");
    let r = analyze(&f.mesh);
    assert_eq!(r.non_manifold_vertices, 1, "{r:?}");
    assert_eq!(r.non_manifold_edges, 0);
    assert_eq!(r.boundary_edges, 0);
    assert!(!r.is_watertight());
    assert!(f.mesh.volume() > 0.0);
}

#[test]
fn shared_edge_is_non_manifold() {
    let r = analyze(&load("shared_edge_cubes.stl").mesh);
    assert_eq!(r.non_manifold_edges, 1, "{r:?}");
    assert!(!r.is_watertight());
}

#[test]
fn flipped_and_inverted() {
    let r = analyze(&load("cube_flipped_triangle.stl").mesh);
    assert!(r.is_watertight());
    assert_eq!(r.inconsistent_edges, 3);
    let inv = load("cube_inverted.stl").mesh;
    assert!(analyze(&inv).is_clean_solid());
    assert!((inv.volume() + 1000.0).abs() < 1e-9);
}

#[test]
fn degenerate_triangle_detected() {
    let r = analyze(&load("cube_degenerate.stl").mesh);
    assert_eq!(r.degenerate_triangles, 1);
}

#[test]
fn nan_triangle_dropped() {
    let f = load("cube_with_nan.stl");
    assert_eq!(f.dropped_non_finite, 1);
    assert_eq!(f.triangles_in_file, 13);
    assert_eq!(f.mesh.triangle_count(), 12);
    assert!(!f.warnings.is_empty());
}

#[test]
fn truncated_and_empty_are_errors() {
    assert!(matches!(
        stl::read_file(path("truncated.stl")),
        Err(StlError::Truncated {
            expected: 12,
            found: 5
        })
    ));
    assert!(matches!(
        stl::read_file(path("empty.stl")),
        Err(StlError::Empty)
    ));
    assert!(matches!(
        stl::read_file(path("does_not_exist.stl")),
        Err(StlError::Io(_))
    ));
}

#[test]
fn trailing_bytes_are_tolerated() {
    let f = load("trailing_bytes.stl");
    assert_eq!(f.mesh.triangle_count(), 12);
    assert_eq!(f.warnings.len(), 1);
}

#[test]
fn multi_solid_uppercase_ascii() {
    let f = load("multi_solid_upper.stl");
    assert_eq!(f.format, StlFormat::Ascii);
    assert_eq!(f.mesh.triangle_count(), 24);
    assert_eq!(analyze(&f.mesh).components, 2);
}

#[test]
fn write_read_write_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    for format in [StlFormat::Binary, StlFormat::Ascii] {
        let first = load("two_overlapping_cubes.stl").mesh;
        let p = dir.path().join("out.stl");
        stl::write_file(&first, &p, format, "rt").unwrap();
        let second = stl::read_file(&p).unwrap().mesh;
        assert_eq!(first, second);
        stl::write_file(&second, &p, format, "rt").unwrap();
        assert_eq!(second, stl::read_file(&p).unwrap().mesh);
    }
}

#[test]
fn full_repair_fixes_broken_fixtures() {
    use rc_mesh::repair::{RepairOptions, repair};
    for (name, expected_volume) in [
        ("cube_missing_triangle.stl", Some(1000.0)),
        ("cube_flipped_triangle.stl", Some(1000.0)),
        ("cube_inverted.stl", Some(1000.0)),
        ("cube_degenerate.stl", Some(1000.0)),
        ("cube_with_nan.stl", Some(1000.0)),
        ("bowtie_vertex.stl", None),
        ("shared_edge_cubes.stl", Some(2000.0)),
        ("two_overlapping_cubes.stl", None),
    ] {
        let mesh = load(name).mesh;
        let (fixed, report) = repair(&mesh, &RepairOptions::FULL);
        assert!(report.after.is_clean_solid(), "{name}: {report:?}");
        let v = fixed.volume();
        assert!(v > 0.0, "{name}: volume {v}");
        if let Some(e) = expected_volume {
            assert!((v - e).abs() < 1e-6, "{name}: volume {v} vs {e}");
        }
        // Repairing again changes nothing.
        let (_, again) = repair(&fixed, &RepairOptions::FULL);
        assert!(!again.changed(), "{name}: second repair changed {again:?}");
    }
}
