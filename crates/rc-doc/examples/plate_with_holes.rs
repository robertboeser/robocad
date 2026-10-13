//! Builds a mounting plate with four holes and a boss, and exports it as STL.
//!
//! Usage: `cargo run --release -p rc-doc --example plate_with_holes -- out.stl` (or `out.3mf`)

use glam::DVec3;
use rc_doc::eval::{SolidCache, export_mesh};
use rc_doc::{Command, Editor, PrimitiveKind};

fn add(ed: &mut Editor, kind: PrimitiveKind, size: DVec3, at: DVec3) -> rc_doc::NodeId {
    ed.apply(Command::AddPrimitive {
        primitive: kind.default_primitive().with_size(size),
        position: at,
        color: [74, 144, 226],
    })
    .expect("valid primitive")
    .created[0]
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "plate.stl".into());
    let mut ed = Editor::default();
    let plate = add(
        &mut ed,
        PrimitiveKind::Box,
        DVec3::new(60.0, 40.0, 4.0),
        DVec3::ZERO,
    );
    let boss = add(
        &mut ed,
        PrimitiveKind::Cylinder,
        DVec3::new(16.0, 16.0, 12.0),
        DVec3::ZERO,
    );
    let mut parts = vec![plate, boss];
    for (x, y) in [
        (-24.0, -14.0),
        (24.0, -14.0),
        (-24.0, 14.0),
        (24.0, 14.0),
        (0.0, 0.0),
    ] {
        let d = if x == 0.0 { 8.0 } else { 4.2 };
        let h = add(
            &mut ed,
            PrimitiveKind::Cylinder,
            DVec3::new(d, d, 30.0),
            DVec3::new(x, y, -5.0),
        );
        ed.apply(Command::ToggleRole(vec![h])).expect("toggle");
        parts.push(h);
    }
    ed.apply(Command::Group(parts)).expect("group");
    ed.apply(Command::AddText {
        text: "robocad".into(),
        height: 6.0,
        depth: 1.5,
        position: DVec3::new(0.0, -26.0, 0.0),
        color: [240, 170, 60],
    })
    .expect("text");
    let roots = ed.doc().roots.clone();
    if std::path::Path::new(&out)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("3mf"))
    {
        let (n, warnings) = rc_doc::export3mf::export_3mf(ed.doc(), &roots, &out).expect("3mf");
        println!("wrote {out}: {n} objects, warnings: {warnings:?}");
        return;
    }
    let r = export_mesh(ed.doc(), &roots, &mut SolidCache::default());
    rc_mesh::stl::write_file(&r.mesh, &out, rc_mesh::stl::StlFormat::Binary, "plate")
        .expect("write stl");
    println!(
        "wrote {out}: {} triangles, {:.1} mm³, watertight: {}",
        r.mesh.triangle_count(),
        r.mesh.volume(),
        rc_mesh::topology::analyze(&r.mesh).is_watertight()
    );
}
