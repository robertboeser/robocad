//! Headless UI tests: drive the real app with simulated input (egui_kittest + wgpu).
//!
//! Set `ROBOCAD_SNAPSHOT_DIR` to write rendered frames as PNG files for visual review.

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use rc_app::RobocadApp;
use rc_doc::NodeKind;

fn fixture(name: &str) -> String {
    format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Temporary settings/data directories so tests never touch the user's real files.
fn temp_dirs() -> (tempfile::TempDir, rc_app::settings::AppDirs) {
    let t = tempfile::tempdir().unwrap();
    let dirs = rc_app::settings::AppDirs {
        config: t.path().join("config"),
        data: t.path().join("data"),
    };
    // Skip the first-run card so it does not cover the viewport in tests.
    rc_app::settings::Settings {
        hints_shown: true,
        ..Default::default()
    }
    .save(&dirs.config);
    (t, dirs)
}

/// Builds the app in a harness with the given directories, or `None` without a GPU.
fn harness_with(dirs: rc_app::settings::AppDirs) -> Option<Harness<'static, RobocadApp>> {
    rc_render::headless_device()?;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .wgpu()
        .build_eframe(move |cc| RobocadApp::with_dirs(cc, Vec::new(), dirs));
    h.run_steps(2);
    assert!(h.state().has_viewport());
    Some(h)
}

/// Builds the app in a harness, or `None` if no GPU adapter is available (headless CI).
fn harness() -> Option<Harness<'static, RobocadApp>> {
    let (t, dirs) = temp_dirs();
    // Keep the directory alive for the whole test process (tiny, cleaned by the OS).
    std::mem::forget(t);
    harness_with(dirs)
}

fn snapshot(h: &mut Harness<'_, RobocadApp>, name: &str) {
    if let Ok(dir) = std::env::var("ROBOCAD_SNAPSHOT_DIR") {
        let img = h.render().expect("render");
        img.save(format!("{dir}/{name}.png")).expect("save png");
    }
}

fn click(h: &mut Harness<'_, RobocadApp>, pos: Pos2, modifiers: Modifiers) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
            modifiers,
        );
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn name_field<'a>(h: &'a Harness<'_, RobocadApp>, value: &str) -> egui_kittest::Node<'a> {
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.value().as_deref() == Some(value))
        .expect("name field")
}

fn roots(h: &Harness<'_, RobocadApp>) -> Vec<rc_doc::NodeId> {
    h.state().editor.doc().roots.clone()
}

#[test]
fn import_select_in_viewport_and_delete_undo() {
    let Some(mut h) = harness() else { return };
    h.state_mut()
        .import_blocking(fixture("cube_binary.stl"))
        .unwrap();
    h.state_mut()
        .import_blocking(fixture("two_overlapping_cubes.stl"))
        .unwrap();
    h.run_steps(3);
    let ids = roots(&h);
    assert_eq!(ids.len(), 2);

    // Click empty space: deselect.
    click(&mut h, Pos2::new(300.0, 700.0), Modifiers::NONE);
    assert!(h.state().editor.selection.is_empty());

    // Click the first object in the viewport: it becomes selected.
    let p = h.state().screen_pos_of(ids[0]).expect("on screen");
    click(&mut h, p, Modifiers::NONE);
    assert_eq!(h.state().editor.selection.ids(), &[ids[0]]);
    snapshot(&mut h, "selected");

    // Delete with the keyboard, then undo.
    h.key_press(Key::Delete);
    h.run_steps(2);
    assert_eq!(roots(&h).len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(roots(&h).len(), 2);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(2);
    assert_eq!(roots(&h).len(), 1);
}

#[test]
fn tree_selection_group_and_rename() {
    let Some(mut h) = harness() else { return };
    h.state_mut()
        .import_blocking(fixture("cube_ascii.stl"))
        .unwrap();
    h.state_mut()
        .import_blocking(fixture("cube_binary.stl"))
        .unwrap();
    h.run_steps(3);
    // Same mesh imported twice: second gets a unique name.
    h.get_by_label("cube_ascii.stl").click();
    h.run_steps(2);
    let first = roots(&h)[0];
    assert_eq!(h.state().editor.selection.ids(), &[first]);

    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    h.run_steps(3);
    let r = roots(&h);
    assert_eq!(r.len(), 1);
    assert!(matches!(
        h.state().editor.doc().get(r[0]).unwrap().kind,
        NodeKind::Group { .. }
    ));
    snapshot(&mut h, "grouped");

    // Rename via the properties panel name field.
    name_field(&h, "Group").click();
    h.run_steps(1);
    h.key_combination_modifiers(Modifiers::COMMAND, &[Key::A]);
    h.run_steps(1);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(egui_kittest::Node::is_focused)
        .expect("focused field")
        .type_text("Bracket");
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc().get(r[0]).unwrap().name, "Bracket");
}

#[test]
fn typed_math_moves_object() {
    let Some(mut h) = harness() else { return };
    h.state_mut().place_on_plate = false;
    h.state_mut()
        .import_blocking(fixture("cube_binary.stl"))
        .unwrap();
    h.run_steps(3);
    let id = roots(&h)[0];
    // Position X is the first drag value showing 0.0.
    let fields: Vec<_> = h.get_all_by_value("0.0").collect();
    assert!(fields.len() >= 3);
    fields[0].click();
    h.run_steps(1);
    h.key_combination_modifiers(Modifiers::COMMAND, &[Key::A]);
    h.run_steps(1);
    h.get_all_by_value("0.0")
        .next()
        .unwrap()
        .type_text("20/2+3");
    h.key_press(Key::Enter);
    h.run_steps(3);
    let t = h.state().editor.doc().get(id).unwrap().transform;
    assert_eq!(t.translation.x, 13.0, "typed expression applied");
    // One undo step restores the old position.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    let t = h.state().editor.doc().get(id).unwrap().transform;
    assert_eq!(t.translation.x, 0.0);
}

#[test]
fn box_select_picks_contained_objects() {
    let Some(mut h) = harness() else { return };
    h.state_mut()
        .import_blocking(fixture("cube_binary.stl"))
        .unwrap();
    h.run_steps(3);
    let id = roots(&h)[0];
    let rect = h.state().last_rect_for_tests().expect("viewport");
    // Drag over the whole viewport from a corner (empty space).
    let a = rect.min + egui::vec2(5.0, 5.0);
    let b = rect.max - egui::vec2(5.0, 5.0);
    h.hover_at(a);
    h.run_steps(1);
    h.event(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        h.hover_at(a + (b - a) * t);
        h.run_steps(1);
    }
    snapshot(&mut h, "box_select");
    h.event(Event::PointerButton {
        pos: b,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    assert_eq!(h.state().editor.selection.ids(), &[id]);
}

fn drag(h: &mut Harness<'_, RobocadApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
    h.hover_at(from);
    h.run_steps(1);
    h.event_modifiers(
        Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        },
        modifiers,
    );
    h.run_steps(1);
    for i in 1..=12 {
        let t = i as f32 / 12.0;
        h.hover_at(from + (to - from) * t);
        h.run_steps(1);
    }
    h.event_modifiers(
        Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        },
        modifiers,
    );
    h.run_steps(3);
}

fn add_box(h: &mut Harness<'_, RobocadApp>) -> rc_doc::NodeId {
    h.get_by_label("Box").click();
    h.run_steps(3);
    let id = *roots(h).last().unwrap();
    assert!(matches!(
        h.state().editor.doc().get(id).unwrap().kind,
        NodeKind::Primitive(_)
    ));
    id
}

fn transform(h: &Harness<'_, RobocadApp>, id: rc_doc::NodeId) -> rc_doc::Transform {
    h.state().editor.doc().get(id).unwrap().transform
}

#[test]
fn toolbar_adds_primitives() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    assert_eq!(h.state().editor.selection.ids(), &[id]);
    assert_eq!(h.state().editor.doc().get(id).unwrap().name, "Box");
    for name in ["Cylinder", "Cone", "Sphere", "Torus", "Wedge"] {
        h.get_by_label(name).click();
        h.run_steps(2);
    }
    assert_eq!(roots(&h).len(), 6);
    snapshot(&mut h, "primitives");
}

#[test]
fn gizmo_move_along_x_snaps_and_undoes_in_one_step() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    let (pivot, tips) = h.state().gizmo_screen_handles().expect("gizmo");
    let grab = pivot + (tips[0] - pivot) * 0.7;
    let dir = (tips[0] - pivot).normalized();
    drag(&mut h, grab, grab + dir * 120.0, Modifiers::NONE);
    let t = transform(&h, id);
    assert!(t.translation.x > 5.0, "moved along +x: {:?}", t.translation);
    assert_eq!(t.translation.x, t.translation.x.round(), "snapped to 1 mm");
    assert_eq!(t.translation.y, 0.0);
    assert_eq!(t.translation.z, 0.0);
    snapshot(&mut h, "gizmo_move");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(transform(&h, id).translation.x, 0.0, "one undo step");
    assert_eq!(roots(&h).len(), 1, "the box itself is still there");
}

#[test]
fn dragging_an_object_slides_it_on_the_plate() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    h.key_press(Key::Escape);
    h.run_steps(1);
    let centre = h.state().screen_pos_of(id).unwrap();
    // Grab the top face (above the centre) and drag sideways.
    let grab = centre + egui::vec2(0.0, -20.0);
    drag(
        &mut h,
        grab,
        grab + egui::vec2(-150.0, 40.0),
        Modifiers::NONE,
    );
    let t = transform(&h, id);
    assert!(
        t.translation.truncate().length() > 5.0,
        "{:?}",
        t.translation
    );
    assert_eq!(t.translation.z, 0.0, "stays on the plate");
    assert_eq!(h.state().editor.selection.ids(), &[id]);
}

#[test]
fn scale_and_rotate_tools() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    // Scale along X.
    h.key_press(Key::R);
    h.run_steps(2);
    let (pivot, tips) = h.state().gizmo_screen_handles().expect("gizmo");
    let dir = (tips[0] - pivot).normalized();
    drag(&mut h, tips[0], tips[0] + dir * 60.0, Modifiers::NONE);
    let NodeKind::Primitive(p) = h.state().editor.doc().get(id).unwrap().kind else {
        panic!()
    };
    let s = p.size();
    assert!(s.x > 20.0, "box grew along x: {s:?}");
    assert_eq!(s.x, s.x.round(), "size snapped to 1 mm");
    assert_eq!((s.y, s.z), (20.0, 20.0));
    // Rotate around Z using the ring (start on the ring opposite to the X axis).
    h.key_press(Key::E);
    h.run_steps(2);
    let (pivot, tips) = h.state().gizmo_screen_handles().expect("gizmo");
    let on_ring = pivot + (tips[0] - pivot) * 0.85;
    let towards_y = pivot + (tips[1] - pivot) * 0.85;
    drag(&mut h, on_ring, towards_y, Modifiers::NONE);
    let r = transform(&h, id).rotation_deg;
    assert!(r.z.abs() > 1.0, "rotated: {r:?}");
    assert!(
        (r.z / 15.0 - (r.z / 15.0).round()).abs() < 1e-6,
        "snapped to 15°: {r:?}"
    );
    snapshot(&mut h, "rotated");
}

#[test]
fn drop_to_plate_and_drag_from_toolbar() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    let mut t = transform(&h, id);
    t.translation.z = 12.0;
    h.state_mut()
        .editor
        .apply(rc_doc::Command::SetTransforms(vec![(id, t)]))
        .unwrap();
    h.run_steps(1);
    h.key_press(Key::D);
    h.run_steps(2);
    assert_eq!(transform(&h, id).translation.z, 0.0);

    // Drag a sphere from the toolbar onto the viewport.
    let from = h.get_by_label("Sphere").rect().center();
    let rect = h.state().last_rect_for_tests().unwrap();
    let to = rect.center() + egui::vec2(150.0, 80.0);
    drag(&mut h, from, to, Modifiers::NONE);
    let r = roots(&h);
    assert_eq!(r.len(), 2, "sphere added by drag and drop");
    let t = transform(&h, r[1]);
    assert!(
        t.translation.truncate().length() > 1.0,
        "placed under the cursor: {t:?}"
    );
    assert_eq!(t.translation.z, 0.0);
}

/// Runs frames until background evaluation and exports are finished.
fn settle(h: &mut Harness<'_, RobocadApp>) {
    for _ in 0..500 {
        h.run_steps(1);
        if h.state().evaluation_idle() && !h.state().exporting() && !h.state().repairing() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("background work did not finish");
}

#[test]
fn hole_group_is_combined_and_exported() {
    let Some(mut h) = harness() else { return };
    let plate = add_box(&mut h);
    let prim = rc_doc::PrimitiveKind::Box
        .default_primitive()
        .with_size(glam::DVec3::new(40.0, 40.0, 5.0));
    h.state_mut()
        .editor
        .apply(rc_doc::Command::SetPrimitives(vec![(plate, prim)]))
        .unwrap();
    h.get_by_label("Cylinder").click();
    h.run_steps(2);
    let cyl = *roots(&h).last().unwrap();
    // Centre the cylinder on the plate and make it a hole (H).
    let mut t = transform(&h, cyl);
    t.translation = glam::DVec3::new(0.0, 0.0, -5.0);
    h.state_mut()
        .editor
        .apply(rc_doc::Command::SetTransforms(vec![(cyl, t)]))
        .unwrap();
    h.key_press(Key::H);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    settle(&mut h);
    let g = roots(&h)[0];
    let info = h.state().group_info(g).expect("group evaluated");
    let cyl_vol = rc_doc::PrimitiveKind::Cylinder
        .default_primitive()
        .mesh()
        .volume();
    let expected = 40.0 * 40.0 * 5.0 - cyl_vol * 5.0 / 20.0;
    assert!(
        (info.volume - expected).abs() < 1e-2,
        "{} vs {expected}",
        info.volume
    );
    snapshot(&mut h, "hole_group");

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("plate.stl");
    h.state_mut().export_to(out.clone(), false);
    settle(&mut h);
    let f = rc_mesh::stl::read_file(&out).expect("exported file");
    assert!(rc_mesh::topology::analyze(&f.mesh).is_watertight());
    assert!((f.mesh.volume() - expected).abs() < 1e-2);
    assert!(h.state().messages().iter().any(|m| m.contains("Exported")));
}

#[test]
fn repair_button_fixes_open_mesh_and_undo_restores() {
    let Some(mut h) = harness() else { return };
    h.state_mut()
        .import_blocking(fixture("cube_missing_triangle.stl"))
        .unwrap();
    h.run_steps(3);
    let id = roots(&h)[0];
    let mesh_key = |h: &Harness<'_, RobocadApp>| match h.state().editor.doc().get(id).unwrap().kind
    {
        NodeKind::Imported { mesh, .. } => mesh,
        _ => unreachable!(),
    };
    let broken = mesh_key(&h);
    snapshot(&mut h, "broken_mesh");
    h.get_by_label("🔧 Repair mesh").click();
    h.run_steps(1);
    settle(&mut h);
    let fixed = mesh_key(&h);
    assert_ne!(fixed, broken);
    let doc = h.state().editor.doc();
    let m = doc.mesh(fixed).unwrap();
    assert!(rc_mesh::topology::analyze(m).is_clean_solid());
    assert!(
        h.state()
            .messages()
            .iter()
            .any(|m| m.contains("now watertight"))
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(mesh_key(&h), broken, "repair is undoable");
}

#[test]
fn import_units_and_place_on_plate() {
    let Some(mut h) = harness() else { return };
    h.state_mut().import_units = rc_app::app::ImportUnits::Inches;
    h.state_mut()
        .import_blocking(fixture("cube_binary.stl"))
        .unwrap();
    h.run_steps(2);
    let id = roots(&h)[0];
    let b = h.state().editor.doc().world_bounds(id);
    assert!(
        (b.size().x - 254.0).abs() < 1e-3,
        "10 inch cube: {:?}",
        b.size()
    );
    assert!(
        b.min.z.abs() < 1e-4 && b.center().x.abs() < 1e-3,
        "placed on plate: {b:?}"
    );
}

#[test]
fn overhang_view_renders() {
    let Some(mut h) = harness() else { return };
    h.get_by_label("Sphere").click();
    h.run_steps(1);
    h.get_by_label("Torus").click();
    h.run_steps(1);
    h.state_mut().show_overhangs = true;
    h.key_press(Key::Escape);
    h.key_press(Key::Num1);
    h.key_press_modifiers(Modifiers::SHIFT, Key::F);
    h.run_steps(3);
    snapshot(&mut h, "overhangs");
}

#[test]
fn save_and_reopen_project() {
    let (_t, dirs) = temp_dirs();
    let Some(mut h) = harness_with(dirs.clone()) else {
        return;
    };
    add_box(&mut h);
    h.state_mut()
        .import_blocking(fixture("bowtie_vertex.stl"))
        .unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    settle(&mut h);
    let path = dirs.data.join("bracket.rcad");
    std::fs::create_dir_all(&dirs.data).unwrap();
    h.state_mut().save_blocking(&path).unwrap();
    assert!(!h.state().editor.is_dirty());
    let saved = h.state().editor.doc().clone();
    h.run_steps(1);
    assert!(
        h.state()
            .window_title_for_tests()
            .starts_with("bracket.rcad")
    );
    assert!(
        rc_doc::project::read_thumbnail(&path).is_some(),
        "thumbnail stored"
    );

    let Some(mut h2) = harness_with(dirs.clone()) else {
        return;
    };
    h2.state_mut().open_project_blocking(&path).unwrap();
    h2.run_steps(2);
    assert_eq!(h2.state().editor.doc(), &saved);
    assert!(!h2.state().editor.is_dirty());
    assert_eq!(h2.state().settings.recent_files.first(), Some(&path));
}

#[test]
fn crash_recovery_restores_autosave() {
    let (_t, dirs) = temp_dirs();
    {
        let Some(mut h) = harness_with(dirs.clone()) else {
            return;
        };
        add_box(&mut h);
        h.state_mut().autosave_now();
        h.state_mut().wait_for_autosave();
        assert!(dirs.autosave_file().exists());
        // Dropping the harness without quitting simulates a crash.
    }
    let Some(mut h) = harness_with(dirs.clone()) else {
        return;
    };
    assert!(h.state().recovery_offered());
    h.run_steps(2);
    snapshot(&mut h, "recovery");
    h.get_by_label("Restore").click();
    h.run_steps(3);
    assert_eq!(roots(&h).len(), 1);
    assert!(h.state().editor.is_dirty(), "recovered work is unsaved");
    assert!(!h.state().recovery_offered());
}

#[test]
fn new_project_asks_about_unsaved_changes() {
    let Some(mut h) = harness() else { return };
    add_box(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(2);
    snapshot(&mut h, "unsaved_prompt");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(roots(&h).len(), 1, "cancel keeps the document");
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(2);
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert!(roots(&h).is_empty(), "new document");
    assert!(!h.state().editor.is_dirty());
}

#[test]
fn settings_persist_between_sessions() {
    let (_t, dirs) = temp_dirs();
    {
        let Some(mut h) = harness_with(dirs.clone()) else {
            return;
        };
        h.state_mut().place_on_plate = false;
        h.state_mut().import_units = rc_app::app::ImportUnits::Inches;
        h.state_mut().settings.grid_step = 0.5;
        h.run_steps(2);
    }
    let Some(h) = harness_with(dirs.clone()) else {
        return;
    };
    assert!(!h.state().place_on_plate);
    assert_eq!(h.state().import_units, rc_app::app::ImportUnits::Inches);
    assert_eq!(
        h.state().editor.doc().settings.grid_step,
        0.5,
        "new projects use the setting"
    );
}

#[test]
fn text_tool_adds_editable_text() {
    let Some(mut h) = harness() else { return };
    h.get_by_label("Text").click();
    h.run_steps(3);
    let id = roots(&h)[0];
    let kind = h.state().editor.doc().get(id).unwrap().kind.clone();
    assert!(matches!(kind, NodeKind::Text { ref text, .. } if text == "robocad"));
    let b = h.state().editor.doc().world_bounds(id);
    assert!(
        b.size().x > 40.0 && (b.size().z - 3.0).abs() < 1e-3,
        "{:?}",
        b.size()
    );
    h.state_mut()
        .editor
        .apply(rc_doc::Command::SetText {
            node: id,
            text: "Hi!".into(),
            height: 20.0,
            depth: 2.0,
        })
        .unwrap();
    h.key_press_modifiers(Modifiers::SHIFT, Key::F);
    h.run_steps(3);
    snapshot(&mut h, "text");
    assert!(h.state().editor.doc().world_bounds(id).size().y > 19.0);
}

#[test]
fn grid_array_pattern_from_toolbar() {
    let Some(mut h) = harness() else { return };
    let b = add_box(&mut h);
    h.get_by_label("Pattern").click();
    h.run_steps(2);
    h.get_by_label("Grid array (3 × 3)").click();
    h.run_steps(2);
    settle(&mut h);
    let g = roots(&h)[0];
    assert_ne!(g, b);
    let info = h.state().group_info(g).expect("evaluated");
    assert!((info.volume - 9.0 * 8000.0).abs() < 1e-3, "{}", info.volume);
    let size = h.state().editor.doc().world_bounds(g).size();
    assert!(
        (size.x - 70.0).abs() < 1e-3,
        "3 boxes of 20 mm, 5 mm gaps: {size:?}"
    );
    h.key_press_modifiers(Modifiers::SHIFT, Key::F);
    h.run_steps(3);
    snapshot(&mut h, "grid_array");
}

#[test]
fn measure_tool_reports_distance() {
    let Some(mut h) = harness() else { return };
    let id = add_box(&mut h);
    h.key_press(Key::Escape);
    h.key_press(Key::M);
    h.run_steps(1);
    let t = transform(&h, id).translation;
    let p1 = h
        .state()
        .screen_pos_of_point(t + glam::DVec3::new(-3.0, 0.0, 20.0))
        .unwrap();
    let p2 = h
        .state()
        .screen_pos_of_point(t + glam::DVec3::new(3.0, 0.0, 20.0))
        .unwrap();
    click(&mut h, p1, Modifiers::NONE);
    click(&mut h, p2, Modifiers::NONE);
    h.run_steps(2);
    snapshot(&mut h, "measure");
    let m = h.state().last_measurement.expect("measured");
    assert!((m.distance - 6.0).abs() < 0.2, "{m:?}");
    assert!(m.angle_deg < 1.0 && m.face_distance.is_some());
    assert_eq!(roots(&h).len(), 1, "measuring does not change the document");
}

#[test]
fn lay_flat_puts_slope_on_plate() {
    let Some(mut h) = harness() else { return };
    h.get_by_label("Wedge").click();
    h.run_steps(3);
    let id = roots(&h)[0];
    h.key_press(Key::Escape);
    h.key_press(Key::L);
    h.run_steps(1);
    let t = transform(&h, id).translation;
    // Centre of the sloped face of the default 20 mm wedge.
    let p = h
        .state()
        .screen_pos_of_point(t + glam::DVec3::new(0.0, 0.0, 10.0))
        .unwrap();
    click(&mut h, p, Modifiers::NONE);
    h.run_steps(2);
    let after = transform(&h, id);
    assert!(after.rotation_deg.length() > 1.0, "rotated: {after:?}");
    let b = h.state().editor.doc().world_bounds(id);
    assert!(b.min.z.abs() < 1e-4, "on the plate: {b:?}");
    // The slope's normal (1,0,1)/√2 must now point straight down: rotation about Y by 135°.
    let q = after.rotation();
    let n = q * glam::DVec3::new(1.0, 0.0, 1.0).normalize();
    assert!(
        (n - glam::DVec3::new(0.0, 0.0, -1.0)).length() < 1e-6,
        "{n:?}"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(
        transform(&h, id).rotation_deg,
        glam::DVec3::ZERO,
        "one undo step"
    );
}

#[test]
fn obj_import_and_3mf_export() {
    let Some(mut h) = harness() else { return };
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("cube.obj");
    std::fs::write(
        &obj,
        "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 8 4 1 5\n",
    )
    .unwrap();
    h.state_mut().import_blocking(&obj).unwrap();
    add_box(&mut h);
    h.run_steps(2);
    assert_eq!(roots(&h).len(), 2);
    let out = dir.path().join("scene.3mf");
    h.state_mut().export_3mf_to(out.clone());
    settle(&mut h);
    assert!(
        h.state()
            .messages()
            .iter()
            .any(|m| m.contains("Exported 2 objects")),
        "{:?}",
        h.state().messages()
    );
    assert!(std::fs::metadata(&out).unwrap().len() > 300);
}

#[test]
fn first_run_card_and_shortcut_window() {
    let t = tempfile::tempdir().unwrap();
    let dirs = rc_app::settings::AppDirs {
        config: t.path().join("config"),
        data: t.path().join("data"),
    };
    let Some(mut h) = harness_with(dirs.clone()) else {
        return;
    };
    h.run_steps(2);
    snapshot(&mut h, "first_run");
    h.get_by_label("Got it").click();
    h.run_steps(2);
    assert!(h.state().settings.hints_shown);
    h.key_press(Key::F1);
    h.run_steps(2);
    snapshot(&mut h, "shortcuts");
    assert!(h.query_by_label("Keyboard shortcuts").is_some());
    drop(h);
    let Some(h2) = harness_with(dirs) else { return };
    assert!(h2.state().settings.hints_shown, "dismissal is remembered");
}

#[test]
fn round_edges_extension_is_off_by_default_and_rounds_selection() {
    let (_t, dirs) = temp_dirs();
    {
        let Some(mut h) = harness_with(dirs.clone()) else {
            return;
        };
        let b = add_box(&mut h);
        assert!(
            h.query_by_label("Round").is_none(),
            "extensions add nothing to the UI until enabled"
        );

        // Turn the extension on in the preferences window.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        h.run_steps(2);
        h.get_by_label("Round edges").click();
        h.run_steps(2);
        assert!(h.state().extension_enabled("round-edges"));

        h.get_by_label("Round").click();
        h.run_steps(2);
        settle(&mut h);
        let g = roots(&h)[0];
        let node = h.state().editor.doc().get(g).unwrap();
        assert_eq!(
            node.modifier(),
            Some(rc_doc::Modifier::RoundEdges { radius: 1.0 })
        );
        assert_eq!(node.children(), &[b]);
        assert_eq!(h.state().editor.selection.ids(), &[g]);
        // 20 mm cube, r = 1: 12 edges of 18 mm lose (1 − π/4) mm² each, 8 corners (1 − π/6).
        let pi = std::f64::consts::PI;
        let expected = 8000.0 - (1.0 - pi / 4.0) * 12.0 * 18.0 - 8.0 * (1.0 - pi / 6.0);
        let info = h.state().group_info(g).expect("evaluated");
        assert!(
            (info.volume - expected).abs() < 0.005 * expected,
            "{} vs {expected}",
            info.volume
        );
        h.key_press(Key::F);
        h.run_steps(3);
        snapshot(&mut h, "rounded_box");

        h.get_by_label("Remove rounding").click();
        h.run_steps(2);
        assert_eq!(roots(&h), vec![b]);
    }
    // The choice is saved with the settings.
    let Some(h) = harness_with(dirs) else { return };
    assert!(h.state().extension_enabled("round-edges"));
    assert!(h.query_by_label("Round").is_some());
}
