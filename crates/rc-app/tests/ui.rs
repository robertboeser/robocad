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
