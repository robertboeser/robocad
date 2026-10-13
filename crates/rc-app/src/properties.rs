//! Properties panel: exact numeric editing of the selection.

use eframe::egui::{self, RichText};
use glam::DVec3;
use rc_doc::{Command, GroupOp, NodeId, NodeKind, Role, Transform, expr};

use crate::app::RobocadApp;

pub struct PropsUiState {
    pub lock_aspect: bool,
    name_edit: Option<(NodeId, String)>,
}

impl Default for PropsUiState {
    fn default() -> Self {
        Self {
            lock_aspect: true,
            name_edit: None,
        }
    }
}

/// A DragValue that also accepts typed arithmetic (`20/2+3`).
fn number<'a>(value: &'a mut f64, speed: f64, suffix: &str) -> egui::DragValue<'a> {
    egui::DragValue::new(value)
        .speed(speed)
        .max_decimals(4)
        .custom_parser(|s| expr::eval(s).ok())
        .suffix(suffix.to_string())
}

/// Shows three fields; returns the edited vector and the merge key of the active field.
fn vec3_row(
    ui: &mut egui::Ui,
    label: &str,
    v: DVec3,
    speed: f64,
    suffix: &str,
) -> Option<(DVec3, u64)> {
    let mut out = None;
    ui.label(label);
    ui.horizontal(|ui| {
        let mut vals = v.to_array();
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            let color = [
                egui::Color32::from_rgb(230, 90, 80),
                egui::Color32::from_rgb(110, 190, 90),
                egui::Color32::from_rgb(90, 140, 240),
            ][i];
            ui.label(RichText::new(*axis).color(color).small());
            let r = ui.add_sized([62.0, 18.0], number(&mut vals[i], speed, suffix));
            if r.changed() {
                out = Some((DVec3::from_array(vals), r.id.value()));
            }
        }
    });
    ui.end_row();
    out
}

pub fn show(app: &mut RobocadApp, ui: &mut egui::Ui) {
    ui.heading("Properties");
    ui.separator();
    let sel = app.editor.selection.ids().to_vec();
    match sel.len() {
        0 => {
            ui.label(RichText::new("Nothing selected").weak());
            settings(app, ui);
        }
        1 => single(app, ui, sel[0]),
        n => multi(app, ui, &sel, n),
    }
}

fn single(app: &mut RobocadApp, ui: &mut egui::Ui, id: NodeId) {
    let Some(node) = app.editor.doc().get(id).cloned() else {
        return;
    };
    let locked = app.editor.doc().is_effectively_locked(id);
    let mut commands: Vec<(Command, Option<u64>)> = Vec::new();

    egui::Grid::new("props_main")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            let edit = app
                .props_ui
                .name_edit
                .get_or_insert_with(|| (id, node.name.clone()));
            if edit.0 != id {
                *edit = (id, node.name.clone());
            }
            let r = ui.add(egui::TextEdit::singleline(&mut edit.1).desired_width(160.0));
            if r.lost_focus() && edit.1.trim() != node.name {
                commands.push((Command::Rename(id, edit.1.clone()), None));
            }
            if !r.has_focus() && !r.lost_focus() {
                edit.1.clone_from(&node.name);
            }
            ui.end_row();

            ui.label("Type");
            let mut role = node.role;
            egui::ComboBox::from_id_salt("role")
                .selected_text(match role {
                    Role::Solid => "Solid",
                    Role::Hole => "Hole",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut role, Role::Solid, "Solid");
                    ui.selectable_value(&mut role, Role::Hole, "Hole");
                });
            if role != node.role {
                commands.push((Command::SetRole(vec![id], role), None));
            }
            ui.end_row();

            if let NodeKind::Group { op, .. } = node.kind {
                ui.label("Combine");
                let mut new_op = op;
                egui::ComboBox::from_id_salt("group_op")
                    .selected_text(match op {
                        GroupOp::Combine => "Union − holes",
                        GroupOp::Intersect => "Intersection",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut new_op, GroupOp::Combine, "Union − holes");
                        ui.selectable_value(&mut new_op, GroupOp::Intersect, "Intersection");
                    });
                if new_op != op {
                    commands.push((Command::SetGroupOp(id, new_op), None));
                }
                ui.end_row();
            }
        });

    ui.add_space(6.0);
    ui.label(RichText::new("Transform").strong());
    ui.add_enabled_ui(!locked, |ui| {
        egui::Grid::new("props_transform")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let t = node.transform;
                if let Some((v, key)) = vec3_row(ui, "Position", t.translation, 0.1, "") {
                    let mut nt = t;
                    nt.translation = v;
                    commands.push((Command::SetTransforms(vec![(id, nt)]), Some(key)));
                }
                if let Some((v, key)) = vec3_row(ui, "Rotation °", t.rotation_deg, 0.5, "") {
                    let mut nt = t;
                    nt.rotation_deg = v;
                    commands.push((Command::SetTransforms(vec![(id, nt)]), Some(key)));
                }
                let base = base_size(app, id, &node);
                let size = base * t.scale.abs();
                if let Some((v, key)) = vec3_row(ui, "Size mm", size, 0.1, "") {
                    let nt = resize(t, base, size, v, app.props_ui.lock_aspect);
                    commands.push((Command::SetTransforms(vec![(id, nt)]), Some(key)));
                }
                ui.label("");
                ui.checkbox(&mut app.props_ui.lock_aspect, "Keep proportions");
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui.button("Reset rotation").clicked() {
                let mut nt = node.transform;
                nt.rotation_deg = DVec3::ZERO;
                commands.push((Command::SetTransforms(vec![(id, nt)]), None));
            }
            if ui.button("Reset scale").clicked() {
                let mut nt = node.transform;
                nt.scale = DVec3::ONE;
                commands.push((Command::SetTransforms(vec![(id, nt)]), None));
            }
        });
    });
    if locked {
        ui.label(RichText::new("🔒 Locked").weak());
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Color");
        let mut c = node.color;
        if ui.color_edit_button_srgb(&mut c).changed() {
            let key = ui.id().with("color").value();
            commands.push((Command::SetColor(vec![id], c), Some(key)));
        }
    });

    if let NodeKind::Imported { mesh, ref source } = node.kind {
        mesh_info(app, ui, mesh, source);
    }

    for (cmd, key) in commands {
        match key {
            Some(k) => app.apply_merged(cmd, k),
            None => app.apply(cmd),
        }
    }
}

/// Size of a node at scale 1: mesh bounds for leaves, current bounds / scale for groups.
fn base_size(app: &RobocadApp, id: NodeId, node: &rc_doc::Node) -> DVec3 {
    let doc = app.editor.doc();
    match node.kind {
        NodeKind::Imported { .. } => doc.local_bounds(id).size().as_dvec3(),
        NodeKind::Group { .. } => {
            doc.world_bounds(id).size().as_dvec3()
                / node.transform.scale.abs().max(DVec3::splat(1e-9))
        }
    }
}

/// New transform after the user typed a new size.
fn resize(t: Transform, base: DVec3, old: DVec3, new: DVec3, lock: bool) -> Transform {
    let mut nt = t;
    let changed = (0..3).find(|&i| (new[i] - old[i]).abs() > 1e-12);
    let Some(i) = changed else { return t };
    if base[i] <= 1e-12 || new[i] <= 1e-9 {
        return t;
    }
    let sign = t.scale.signum();
    if lock {
        let f = new[i] / old[i].max(1e-12);
        nt.scale = t.scale * f;
    } else {
        nt.scale[i] = sign[i] * new[i] / base[i];
    }
    nt
}

fn mesh_info(app: &RobocadApp, ui: &mut egui::Ui, mesh: u64, source: &str) {
    let Some(e) = app.extras.get(&mesh) else {
        return;
    };
    ui.add_space(6.0);
    ui.label(RichText::new("Mesh").strong());
    let info = e.info;
    egui::Grid::new("props_mesh").num_columns(2).show(ui, |ui| {
        ui.label("Source");
        let r = ui.label(source);
        if let Some(p) = &e.path {
            r.on_hover_text(p.display().to_string());
        }
        ui.end_row();
        ui.label("Triangles");
        ui.label(info.triangle_count.to_string());
        ui.end_row();
        ui.label("Volume");
        ui.label(format!("{:.2} cm³", info.volume / 1000.0));
        ui.end_row();
        ui.label("Area");
        ui.label(format!("{:.2} cm²", info.area / 100.0));
        ui.end_row();
        ui.label("Watertight");
        ui.label(if e.topology.is_watertight() {
            "yes ✔"
        } else {
            "no ⚠"
        });
        ui.end_row();
    });
    let t = &e.topology;
    for (n, what) in [
        (t.boundary_edges, "open edges"),
        (t.non_manifold_edges, "non-manifold edges"),
        (t.non_manifold_vertices, "non-manifold vertices"),
        (t.degenerate_triangles, "degenerate triangles"),
        (t.inconsistent_edges, "flipped edges"),
    ] {
        if n > 0 {
            ui.label(format!("• {n} {what}"));
        }
    }
}

fn multi(app: &mut RobocadApp, ui: &mut egui::Ui, sel: &[NodeId], n: usize) {
    ui.label(format!("{n} objects selected"));
    let doc = app.editor.doc();
    let b = sel.iter().fold(rc_mesh::Aabb::EMPTY, |b, id| {
        b.union(&doc.world_bounds(*id))
    });
    let s = b.size();
    ui.label(format!("Bounds: {:.2} × {:.2} × {:.2} mm", s.x, s.y, s.z));
    let first_color = doc.get(sel[0]).map_or([128; 3], |n| n.color);
    ui.horizontal(|ui| {
        ui.label("Color");
        let mut c = first_color;
        if ui.color_edit_button_srgb(&mut c).changed() {
            let key = ui.id().with("multi_color").value();
            app.apply_merged(Command::SetColor(sel.to_vec(), c), key);
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Make solid").clicked() {
            app.apply(Command::SetRole(sel.to_vec(), Role::Solid));
        }
        if ui.button("Make hole").clicked() {
            app.apply(Command::SetRole(sel.to_vec(), Role::Hole));
        }
    });
    if ui.button("Group (⌘G)").clicked() {
        app.group_selection();
    }
}

fn settings(app: &mut RobocadApp, ui: &mut egui::Ui) {
    ui.add_space(12.0);
    ui.label(RichText::new("Build plate").strong());
    let mut plate = app.editor.doc().settings.plate_size;
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.add(number(&mut plate[0], 1.0, " mm")).changed();
        ui.label("×");
        changed |= ui.add(number(&mut plate[1], 1.0, " mm")).changed();
    });
    if changed {
        // Plate size is a view setting for now; it is stored with the document in M7.
        app.set_plate_size([plate[0].clamp(10.0, 2000.0), plate[1].clamp(10.0, 2000.0)]);
    }
}
