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

/// Builds the app in a harness, or `None` if no GPU adapter is available (headless CI).
fn harness() -> Option<Harness<'static, RobocadApp>> {
    rc_render::headless_device()?;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .wgpu()
        .build_eframe(|cc| RobocadApp::new(cc, Vec::new()));
    h.run_steps(2);
    assert!(h.state().has_viewport());
    Some(h)
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
