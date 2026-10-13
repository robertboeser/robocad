//! Integration tests for the `robocad-cli` binary.

use assert_cmd::Command;

fn fixture(name: &str) -> String {
    format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn cli() -> Command {
    Command::cargo_bin("robocad-cli").unwrap()
}

#[test]
fn info_reports_cube() {
    let out = cli()
        .args(["info", &fixture("cube_ascii.stl")])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("triangles:   12"), "{text}");
    assert!(text.contains("volume:      1000.000"), "{text}");
    assert!(text.contains("watertight:  yes"), "{text}");
}

#[test]
fn info_reports_issues() {
    let out = cli()
        .args(["info", &fixture("cube_missing_triangle.stl")])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("3 open (boundary) edges"), "{text}");
}

#[test]
fn info_fails_on_truncated() {
    cli()
        .args(["info", &fixture("truncated.stl")])
        .assert()
        .failure();
}

#[test]
fn convert_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let ascii = dir.path().join("a.stl");
    let binary = dir.path().join("b.stl");
    cli()
        .args([
            "convert",
            &fixture("cube_binary.stl"),
            ascii.to_str().unwrap(),
            "--ascii",
        ])
        .assert()
        .success();
    assert!(
        std::fs::read_to_string(&ascii)
            .unwrap()
            .starts_with("solid ")
    );
    cli()
        .args(["convert", ascii.to_str().unwrap(), binary.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(std::fs::metadata(&binary).unwrap().len(), 84 + 12 * 50);
}

#[test]
fn repair_fills_hole() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("fixed.stl");
    let o = cli()
        .args([
            "repair",
            &fixture("cube_missing_triangle.stl"),
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    let text = String::from_utf8(o.get_output().stdout.clone()).unwrap();
    assert!(text.contains("1 holes filled"), "{text}");
    assert!(text.contains("watertight: yes"), "{text}");
    cli()
        .args(["info", out.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("watertight:  yes"));
}

#[test]
fn project_info_and_export() {
    let project = format!(
        "{}/../../tests/fixtures/projects/v1_bracket.rcad",
        env!("CARGO_MANIFEST_DIR")
    );
    cli()
        .args(["info", &project])
        .assert()
        .success()
        .stdout(predicates::str::contains("Bracket (group)"));
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("bracket.stl");
    cli()
        .args(["export", &project, out.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("watertight: yes"));
    assert!(rc_mesh::stl::read_file(&out).unwrap().mesh.triangle_count() > 0);
}
