//! Drives the editor headlessly through commands, checking undo/redo after every step.

use std::sync::Arc;

use glam::{DMat4, DVec3, Vec3};
use rc_doc::{Command, Editor, GroupOp, NodeId, NodeKind, Role, Transform};
use rc_mesh::Mesh;

fn cube(size: f32) -> Arc<Mesh> {
    Arc::new(Mesh::cuboid(Vec3::ZERO, Vec3::splat(size)))
}

fn import(ed: &mut Editor, name: &str, at: DVec3) -> NodeId {
    ed.apply(Command::Import {
        name: name.into(),
        mesh: cube(10.0),
        transform: Transform::from_translation(at),
        color: [200, 100, 50],
    })
    .unwrap()
    .created[0]
}

fn approx_mat(a: DMat4, b: DMat4) -> bool {
    a.abs_diff_eq(b, 1e-9)
}

#[test]
fn import_rename_move_delete_with_undo() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a.stl", DVec3::ZERO);
    let b = import(&mut ed, "a.stl", DVec3::X * 20.0);
    assert_eq!(ed.doc().get(b).unwrap().name, "a.stl 2", "names are unique");
    assert_eq!(
        ed.doc().meshes().count(),
        1,
        "identical meshes are stored once"
    );
    assert_eq!(ed.selection.ids(), &[b]);

    ed.apply(Command::Rename(a, "Base".into())).unwrap();
    assert!(ed.apply(Command::Rename(a, "  ".into())).is_err());
    ed.apply(Command::TranslateWorld(vec![a], DVec3::new(0.0, 5.0, 0.0)))
        .unwrap();
    assert_eq!(
        ed.doc().get(a).unwrap().transform.translation,
        DVec3::new(0.0, 5.0, 0.0)
    );
    ed.apply(Command::Delete(vec![b])).unwrap();
    assert!(!ed.doc().contains(b));
    assert!(ed.selection.is_empty(), "deleted nodes leave the selection");

    // Undo everything step by step.
    let snapshots = 5;
    for _ in 0..snapshots {
        assert!(ed.undo());
    }
    assert!(!ed.undo());
    assert!(ed.doc().nodes.is_empty());
    for _ in 0..snapshots {
        assert!(ed.redo());
    }
    assert!(!ed.doc().contains(b));
    assert_eq!(ed.doc().get(a).unwrap().name, "Base");
}

#[test]
fn group_and_ungroup_keep_world_transforms() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::new(1.0, 2.0, 3.0));
    let b = import(&mut ed, "b", DVec3::new(-5.0, 0.0, 0.0));
    let wa = ed.doc().world_matrix(a);
    let g = ed.apply(Command::Group(vec![a, b])).unwrap().created[0];
    assert_eq!(ed.doc().roots, vec![g]);
    assert_eq!(ed.doc().get(g).unwrap().children(), &[a, b]);
    assert!(approx_mat(ed.doc().world_matrix(a), wa));

    // Rotate and move the group, then ungroup: children keep their world placement.
    let mut t = ed.doc().get(g).unwrap().transform;
    t.translation = DVec3::new(10.0, 0.0, 0.0);
    t.rotation_deg = DVec3::new(0.0, 0.0, 90.0);
    ed.apply(Command::SetTransforms(vec![(g, t)])).unwrap();
    let wa2 = ed.doc().world_matrix(a);
    assert!(!approx_mat(wa2, wa));
    ed.apply(Command::Ungroup(vec![g])).unwrap();
    assert!(!ed.doc().contains(g));
    assert_eq!(ed.doc().roots, vec![a, b]);
    assert!(approx_mat(ed.doc().world_matrix(a), wa2));
    assert_eq!(ed.selection.ids(), &[a, b]);

    ed.undo();
    assert!(ed.doc().contains(g));
    ed.undo();
    ed.undo();
    assert_eq!(ed.doc().roots, vec![a, b]);
}

#[test]
fn reparent_rejects_cycles_and_keeps_world() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::ZERO);
    let b = import(&mut ed, "b", DVec3::new(0.0, 30.0, 0.0));
    let g = ed.apply(Command::Group(vec![a])).unwrap().created[0];
    let wb = ed.doc().world_matrix(b);
    ed.apply(Command::Reparent {
        nodes: vec![b],
        parent: Some(g),
        index: Some(0),
    })
    .unwrap();
    assert_eq!(ed.doc().get(g).unwrap().children(), &[b, a]);
    assert!(approx_mat(ed.doc().world_matrix(b), wb));
    let rev = ed.revision();
    assert!(
        ed.apply(Command::Reparent {
            nodes: vec![g],
            parent: Some(g),
            index: None
        })
        .is_err()
    );
    assert_eq!(ed.revision(), rev, "failed commands change nothing");
    // Reorder at root level.
    ed.apply(Command::Reparent {
        nodes: vec![b],
        parent: None,
        index: Some(0),
    })
    .unwrap();
    assert_eq!(ed.doc().roots, vec![b, g]);
}

#[test]
fn duplicate_copies_subtrees() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::ZERO);
    let b = import(&mut ed, "b", DVec3::X);
    let g = ed.apply(Command::Group(vec![a, b])).unwrap().created[0];
    let copy = ed.apply(Command::Duplicate(vec![g])).unwrap().created[0];
    assert_eq!(ed.doc().roots, vec![g, copy]);
    let kids = ed.doc().get(copy).unwrap().children().to_vec();
    assert_eq!(kids.len(), 2);
    assert!(
        kids.iter()
            .all(|k| ed.doc().get(*k).unwrap().parent == Some(copy))
    );
    assert_eq!(ed.doc().nodes.len(), 6);
    assert_eq!(ed.selection.ids(), &[copy]);
}

#[test]
fn merged_edits_are_one_undo_step() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::ZERO);
    for i in 1..=10 {
        let t = Transform::from_translation(DVec3::X * f64::from(i));
        ed.apply_merged(Command::SetTransforms(vec![(a, t)]), 42)
            .unwrap();
    }
    ed.end_merge();
    assert_eq!(ed.doc().get(a).unwrap().transform.translation.x, 10.0);
    assert_eq!(ed.undo_label(), Some("Transform"));
    ed.undo();
    assert_eq!(ed.doc().get(a).unwrap().transform.translation.x, 0.0);
    ed.undo();
    assert!(ed.doc().nodes.is_empty());
}

#[test]
fn locked_nodes_resist_edits_and_roles_toggle() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::ZERO);
    ed.apply(Command::SetLocked(a, true)).unwrap();
    ed.apply(Command::TranslateWorld(vec![a], DVec3::X))
        .unwrap();
    ed.apply(Command::Delete(vec![a])).unwrap();
    assert!(ed.doc().contains(a));
    assert_eq!(ed.doc().get(a).unwrap().transform.translation, DVec3::ZERO);
    ed.apply(Command::ToggleRole(vec![a])).unwrap();
    assert_eq!(ed.doc().get(a).unwrap().role, Role::Hole);
    let g = ed.apply(Command::Group(vec![a])).unwrap().created[0];
    ed.apply(Command::SetGroupOp(g, GroupOp::Intersect))
        .unwrap();
    assert!(matches!(
        ed.doc().get(g).unwrap().kind,
        NodeKind::Group {
            op: GroupOp::Intersect,
            ..
        }
    ));
    assert!(ed.apply(Command::SetGroupOp(a, GroupOp::Combine)).is_err());
}

#[test]
fn invalid_transforms_are_rejected() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::ZERO);
    let mut t = Transform::IDENTITY;
    t.scale.x = 0.0;
    assert!(ed.apply(Command::SetTransforms(vec![(a, t)])).is_err());
    t.scale.x = f64::NAN;
    assert!(ed.apply(Command::SetTransforms(vec![(a, t)])).is_err());
}

#[test]
fn world_bounds_follow_transforms() {
    let mut ed = Editor::default();
    let a = import(&mut ed, "a", DVec3::new(100.0, 0.0, 0.0));
    let b = ed.doc().world_bounds(a);
    assert_eq!(b.min, Vec3::new(100.0, 0.0, 0.0));
    assert_eq!(b.max, Vec3::new(110.0, 10.0, 10.0));
    assert!(ed.is_dirty());
    ed.mark_saved();
    assert!(!ed.is_dirty());
}

#[test]
fn primitives_add_resize_drop_and_align() {
    use rc_doc::{AlignMode, PrimitiveKind};
    let mut ed = Editor::default();
    let a = ed
        .apply(Command::AddPrimitive {
            primitive: PrimitiveKind::Box.default_primitive(),
            position: DVec3::new(0.0, 0.0, 15.0),
            color: [1, 2, 3],
        })
        .unwrap()
        .created[0];
    assert_eq!(ed.doc().get(a).unwrap().name, "Box");
    let b = ed
        .apply(Command::AddPrimitive {
            primitive: PrimitiveKind::Cylinder.default_primitive(),
            position: DVec3::new(40.0, 0.0, 0.0),
            color: [1, 2, 3],
        })
        .unwrap()
        .created[0];
    let bounds = ed.doc().world_bounds(a);
    assert_eq!(bounds.min, Vec3::new(-10.0, -10.0, 15.0));

    ed.apply(Command::DropToPlate(vec![a])).unwrap();
    assert_eq!(ed.doc().world_bounds(a).min.z, 0.0);

    let bigger = PrimitiveKind::Box
        .default_primitive()
        .with_size(DVec3::new(40.0, 20.0, 5.0));
    ed.apply(Command::SetPrimitives(vec![(a, bigger)])).unwrap();
    assert_eq!(ed.doc().world_bounds(a).size(), Vec3::new(40.0, 20.0, 5.0));

    ed.apply(Command::Align {
        nodes: vec![a, b],
        axis: 0,
        mode: AlignMode::Max,
    })
    .unwrap();
    let (ba, bb) = (ed.doc().world_bounds(a), ed.doc().world_bounds(b));
    assert!((ba.max.x - bb.max.x).abs() < 1e-5);

    let invalid = rc_doc::Primitive::Torus {
        size: DVec3::splat(10.0),
        tube: 20.0,
        segments: None,
    };
    assert!(
        ed.apply(Command::SetPrimitives(vec![(a, invalid)]))
            .is_err()
    );
}

#[test]
fn free_position_avoids_existing_objects() {
    let mut ed = Editor::default();
    let size = DVec3::splat(20.0);
    assert_eq!(ed.doc().free_position(size), DVec3::ZERO);
    let p = rc_doc::PrimitiveKind::Box.default_primitive();
    ed.apply(Command::AddPrimitive {
        primitive: p,
        position: DVec3::ZERO,
        color: [0; 3],
    })
    .unwrap();
    let spot = ed.doc().free_position(size);
    assert_ne!(spot, DVec3::ZERO);
    assert!(
        spot.length() <= 25.0 + 1e-9,
        "nearest free grid spot: {spot:?}"
    );
}

#[test]
fn group_bounds_ignore_holes() {
    use rc_doc::PrimitiveKind;
    let mut ed = Editor::default();
    let plate = ed
        .apply(Command::AddPrimitive {
            primitive: PrimitiveKind::Box
                .default_primitive()
                .with_size(DVec3::new(40.0, 40.0, 5.0)),
            position: DVec3::ZERO,
            color: [0; 3],
        })
        .unwrap()
        .created[0];
    let pin = ed
        .apply(Command::AddPrimitive {
            primitive: PrimitiveKind::Cylinder.default_primitive(),
            position: DVec3::new(0.0, 0.0, -5.0),
            color: [0; 3],
        })
        .unwrap()
        .created[0];
    ed.apply(Command::ToggleRole(vec![pin])).unwrap();
    assert_eq!(
        ed.doc().world_bounds(pin).min.z,
        -5.0,
        "a hole on its own has bounds"
    );
    let g = ed.apply(Command::Group(vec![plate, pin])).unwrap().created[0];
    let b = ed.doc().world_bounds(g);
    assert_eq!((b.min.z, b.max.z), (0.0, 5.0));
    ed.apply(Command::DropToPlate(vec![g])).unwrap();
    assert_eq!(
        ed.doc().world_bounds(g).min.z,
        0.0,
        "dropping uses the solid part"
    );
}
