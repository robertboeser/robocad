//! The scene tree: hierarchy, selection, visibility, lock, rename and drag-to-reparent.

use eframe::egui::{self, RichText, Sense};
use rc_doc::{Command, NodeId, NodeKind, Role};

use crate::app::RobocadApp;

#[derive(Default)]
pub struct SceneUiState {
    renaming: Option<(NodeId, String)>,
}

/// Where a dragged node should go.
enum DropTarget {
    Into(NodeId),
    Before(NodeId),
    RootEnd,
}

pub fn show(app: &mut RobocadApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Scene");
    });
    ui.separator();
    if app.editor.doc().roots.is_empty() {
        ui.label(RichText::new("Drop an STL file here\nor use File → Import (⌘O).").weak());
        return;
    }
    let mut pending: Vec<Command> = Vec::new();
    let mut select: Option<(NodeId, bool)> = None;
    let mut drop: Option<(NodeId, DropTarget)> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let roots = app.editor.doc().roots.clone();
            for id in roots {
                node_row(app, ui, id, &mut pending, &mut select, &mut drop);
            }
            // Empty area below the tree: drop to the end of the root list.
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
            if let Some(dragged) = resp.dnd_release_payload::<NodeId>() {
                drop = Some((*dragged, DropTarget::RootEnd));
            }
            if resp.dnd_hover_payload::<NodeId>().is_some() {
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                );
            }
            if resp.clicked() {
                app.editor.selection.clear();
            }
        });

    if let Some((id, additive)) = select {
        if additive {
            app.editor.selection.toggle(id);
        } else {
            app.editor.selection.set([id]);
        }
    }
    if let Some((dragged, target)) = drop {
        let doc = app.editor.doc();
        let nodes = if app.editor.selection.contains(dragged) {
            app.editor.selection.ids().to_vec()
        } else {
            vec![dragged]
        };
        let cmd = match target {
            DropTarget::Into(g) => Command::Reparent {
                nodes,
                parent: Some(g),
                index: None,
            },
            DropTarget::Before(other) => Command::Reparent {
                nodes,
                parent: doc.get(other).and_then(|n| n.parent),
                index: doc.index_in_parent(other),
            },
            DropTarget::RootEnd => Command::Reparent {
                nodes,
                parent: None,
                index: None,
            },
        };
        pending.push(cmd);
    }
    for cmd in pending {
        app.apply(cmd);
    }
}

fn node_row(
    app: &mut RobocadApp,
    ui: &mut egui::Ui,
    id: NodeId,
    pending: &mut Vec<Command>,
    select: &mut Option<(NodeId, bool)>,
    drop: &mut Option<(NodeId, DropTarget)>,
) {
    let Some(node) = app.editor.doc().get(id).cloned() else {
        return;
    };
    if node.is_group() {
        let state_id = ui.make_persistent_id(("tree", id));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
            .show_header(ui, |ui| {
                row_contents(app, ui, id, &node, pending, select, drop);
            })
            .body(|ui| {
                for c in node.children() {
                    node_row(app, ui, *c, pending, select, drop);
                }
            });
    } else {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            row_contents(app, ui, id, &node, pending, select, drop);
        });
    }
}

fn row_contents(
    app: &mut RobocadApp,
    ui: &mut egui::Ui,
    id: NodeId,
    node: &rc_doc::Node,
    pending: &mut Vec<Command>,
    select: &mut Option<(NodeId, bool)>,
    drop: &mut Option<(NodeId, DropTarget)>,
) {
    let eye = if node.visible { "👁" } else { "–" };
    if ui
        .add(egui::Button::new(eye).small().frame(false))
        .on_hover_text("Show / hide")
        .clicked()
    {
        pending.push(Command::SetVisible(id, !node.visible));
    }
    let lock = if node.locked { "🔒" } else { "🔓" };
    let lock_text = RichText::new(lock).weak();
    if ui
        .add(egui::Button::new(lock_text).small().frame(false))
        .on_hover_text("Lock / unlock")
        .clicked()
    {
        pending.push(Command::SetLocked(id, !node.locked));
    }
    let role_icon = match (node.role, &node.kind) {
        (Role::Hole, _) => "◻",
        (Role::Solid, NodeKind::Group { .. }) => "▣",
        (Role::Solid, NodeKind::Imported { .. }) => "■",
    };
    let [r, g, b] = node.color;
    let icon_color = if node.role == Role::Hole {
        ui.visuals().weak_text_color()
    } else {
        egui::Color32::from_rgb(r, g, b)
    };
    ui.label(RichText::new(role_icon).color(icon_color));

    if let Some((rid, text)) = app.scene_ui.renaming.as_mut()
        && *rid == id
    {
        let resp = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
        resp.request_focus();
        if resp.lost_focus() {
            let commit = !ui.input(|i| i.key_pressed(egui::Key::Escape));
            if commit && text.trim() != node.name {
                pending.push(Command::Rename(id, text.clone()));
            }
            app.scene_ui.renaming = None;
        }
        return;
    }

    let selected = app.editor.selection.contains(id);
    let mut label = RichText::new(&node.name);
    if node.role == Role::Hole {
        label = label.italics();
    }
    let warn = match node.kind {
        NodeKind::Imported { mesh, .. } => app
            .extras
            .get(&mesh)
            .is_some_and(|e| !e.topology.is_watertight()),
        NodeKind::Group { .. } => false,
    };
    if warn {
        label = RichText::new(format!("{} ⚠", node.name));
    }
    let resp = ui.add(egui::Button::selectable(selected, label).sense(Sense::click_and_drag()));
    resp.dnd_set_drag_payload(id);
    let resp = resp.on_hover_text(if warn {
        "Mesh is not watertight — see Properties"
    } else {
        "Click to select, double-click to rename, drag to move"
    });
    if resp.double_clicked() {
        app.scene_ui.renaming = Some((id, node.name.clone()));
    } else if resp.clicked() {
        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        *select = Some((id, additive));
    }
    resp.context_menu(|ui| {
        if !app.editor.selection.contains(id) {
            app.editor.selection.set([id]);
        }
        if ui.button("Rename").clicked() {
            app.scene_ui.renaming = Some((id, node.name.clone()));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            app.duplicate_selection();
            ui.close();
        }
        if ui.button("Toggle solid / hole").clicked() {
            app.toggle_hole_selection();
            ui.close();
        }
        if ui.button("Group").clicked() {
            app.group_selection();
            ui.close();
        }
        if node.is_group() && ui.button("Ungroup").clicked() {
            app.ungroup_selection();
            ui.close();
        }
        ui.separator();
        if ui.button("Delete").clicked() {
            app.delete_selection();
            ui.close();
        }
    });

    // Drop handling: onto a group = into it; onto anything else = before it.
    if let Some(hover) = resp.dnd_hover_payload::<NodeId>()
        && *hover != id
    {
        let stroke = egui::Stroke::new(2.0, ui.visuals().selection.bg_fill);
        if node.is_group() {
            ui.painter()
                .rect_stroke(resp.rect, 2.0, stroke, egui::StrokeKind::Outside);
        } else {
            ui.painter()
                .hline(resp.rect.x_range(), resp.rect.top(), stroke);
        }
    }
    if let Some(dragged) = resp.dnd_release_payload::<NodeId>()
        && *dragged != id
    {
        let target = if node.is_group() {
            DropTarget::Into(id)
        } else {
            DropTarget::Before(id)
        };
        *drop = Some((*dragged, target));
    }
}
