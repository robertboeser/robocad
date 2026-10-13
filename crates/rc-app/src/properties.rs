//! Properties panel: exact numeric editing of the selection.

use eframe::egui::{self, RichText};
use glam::DVec3;
use rc_doc::{AlignMode, Command, GroupOp, NodeId, NodeKind, Primitive, Role, Transform, expr};

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
                    let lock = app.props_ui.lock_aspect;
                    if let NodeKind::Primitive(p) = node.kind {
                        let new_size = proportional(size, v, lock) / t.scale.abs();
                        commands.push((
                            Command::SetPrimitives(vec![(id, p.with_size(new_size))]),
                            Some(key),
                        ));
                    } else {
                        let nt = resize(t, base, size, v, lock);
                        commands.push((Command::SetTransforms(vec![(id, nt)]), Some(key)));
                    }
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

    ui.horizontal(|ui| {
        if ui.button("Drop to plate").on_hover_text("D").clicked() {
            commands.push((Command::DropToPlate(vec![id]), None));
        }
        if ui.button("Center on plate").clicked() {
            app.center_selection_on_plate();
        }
    });

    if let NodeKind::Primitive(p) = node.kind {
        shape_section(ui, id, p, &mut commands);
    }
    if node.is_group() {
        group_info(app, ui, id);
    }
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
        NodeKind::Imported { .. } | NodeKind::Primitive(_) => {
            doc.local_bounds(id).size().as_dvec3()
        }
        NodeKind::Group { .. } => {
            doc.world_bounds(id).size().as_dvec3()
                / node.transform.scale.abs().max(DVec3::splat(1e-9))
        }
    }
}

/// Applies "keep proportions": scales all axes by the factor of the edited one.
fn proportional(old: DVec3, new: DVec3, lock: bool) -> DVec3 {
    if !lock {
        return new;
    }
    match (0..3).find(|&i| (new[i] - old[i]).abs() > 1e-12) {
        Some(i) if old[i] > 1e-12 => old * (new[i] / old[i]),
        _ => new,
    }
}

/// Shape parameters of a primitive.
fn shape_section(
    ui: &mut egui::Ui,
    id: NodeId,
    p: Primitive,
    commands: &mut Vec<(Command, Option<u64>)>,
) {
    if matches!(p, Primitive::Box { .. } | Primitive::Wedge { .. }) {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Shape").strong());
    let mut new = p;
    let mut key = None;
    egui::Grid::new("props_shape")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            match &mut new {
                Primitive::Cylinder { top, .. } => {
                    ui.label("Top size");
                    let mut pct = *top * 100.0;
                    let r = ui.add(number(&mut pct, 1.0, " %").range(0.0..=500.0));
                    if r.changed() {
                        *top = pct / 100.0;
                        key = Some(r.id.value());
                    }
                    ui.end_row();
                }
                Primitive::Torus { tube, .. } => {
                    ui.label("Tube width");
                    let r = ui.add(number(tube, 0.1, " mm").range(0.01..=10_000.0));
                    if r.changed() {
                        key = Some(r.id.value());
                    }
                    ui.end_row();
                }
                Primitive::Box { .. } | Primitive::Sphere { .. } | Primitive::Wedge { .. } => {}
            }
            let segments = match &mut new {
                Primitive::Cylinder { segments, .. }
                | Primitive::Sphere { segments, .. }
                | Primitive::Torus { segments, .. } => Some(segments),
                Primitive::Box { .. } | Primitive::Wedge { .. } => None,
            };
            if let Some(segments) = segments {
                ui.label("Segments");
                ui.horizontal(|ui| {
                    let mut auto = segments.is_none();
                    if ui
                        .checkbox(&mut auto, "Auto")
                        .on_hover_text("Smooth enough for printing (0.025 mm deviation)")
                        .changed()
                    {
                        *segments = if auto { None } else { Some(64) };
                        key = Some(ui.id().with("seg_auto").value());
                    }
                    if let Some(n) = segments.as_mut() {
                        let r = ui.add(egui::DragValue::new(n).range(3..=512));
                        if r.changed() {
                            key = Some(r.id.value());
                        }
                    }
                });
                ui.end_row();
            }
        });
    if new != p {
        commands.push((Command::SetPrimitives(vec![(id, new)]), key));
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

fn group_info(app: &RobocadApp, ui: &mut egui::Ui, id: NodeId) {
    ui.add_space(6.0);
    ui.label(RichText::new("Result").strong());
    if let Some(err) = app.group_error(id) {
        ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {err}"));
        return;
    }
    let Some(info) = app.group_info(id) else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Combining…");
        });
        return;
    };
    egui::Grid::new("props_group")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Triangles");
            ui.label(info.triangle_count.to_string());
            ui.end_row();
            ui.label("Volume");
            ui.label(format!("{:.2} cm³", info.volume / 1000.0));
            ui.end_row();
            ui.label("Area");
            ui.label(format!("{:.2} cm²", info.area / 100.0));
            ui.end_row();
        });
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
    ui.add_space(6.0);
    ui.label(RichText::new("Align").strong());
    egui::Grid::new("align").num_columns(4).show(ui, |ui| {
        for (axis, name) in ["X", "Y", "Z"].iter().enumerate() {
            ui.label(*name);
            for (mode, label) in [
                (AlignMode::Min, "Min"),
                (AlignMode::Center, "Center"),
                (AlignMode::Max, "Max"),
            ] {
                if ui.button(label).clicked() {
                    app.align_selection(axis, mode);
                }
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Drop to plate").clicked() {
            app.drop_selection_to_plate();
        }
        if ui.button("Center on plate").clicked() {
            app.center_selection_on_plate();
        }
    });
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
