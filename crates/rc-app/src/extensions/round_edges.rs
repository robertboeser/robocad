//! "Round edges": rounds all outward (convex) edges and corners of the selected objects.
//!
//! Each object is wrapped in a group carrying [`Modifier::RoundEdges`], so the object itself
//! stays editable and Ungroup removes the rounding again.

use eframe::egui::{self, RichText};
use rc_doc::{Command, Document, MAX_ROUND_RADIUS, Modifier, Node, NodeId};

use super::Extension;
use crate::app::RobocadApp;
use crate::icons::Icon;

pub const ID: &str = "round-edges";
const DEFAULT_RADIUS: f64 = 1.0;

pub struct RoundEdges;

impl Extension for RoundEdges {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Round edges"
    }

    fn description(&self) -> &'static str {
        "Rounds all outward edges and corners of the selected object with an editable radius. \
         Adds a Round button to the toolbar."
    }

    fn toolbar(&self, app: &mut RobocadApp, ui: &mut egui::Ui) {
        ui.add_enabled_ui(!app.editor.selection.is_empty(), |ui| {
            if crate::icons::button(ui, Icon::Round, "Round", false)
                .on_hover_text("Round all outward edges of the selected objects")
                .clicked()
            {
                app.round_selection();
            }
        });
    }

    fn selection_menu(&self, app: &mut RobocadApp, ui: &mut egui::Ui) {
        let has_sel = !app.editor.selection.is_empty();
        if ui
            .add_enabled(has_sel, egui::Button::new("Round edges"))
            .clicked()
        {
            app.round_selection();
            ui.close();
        }
    }

    fn node_properties(&self, app: &mut RobocadApp, ui: &mut egui::Ui, id: NodeId) {
        let Some(Modifier::RoundEdges { radius }) =
            app.editor.doc().get(id).and_then(Node::modifier)
        else {
            return;
        };
        ui.add_space(6.0);
        ui.label(RichText::new("Round edges").strong());
        let mut r = radius;
        let mut edited = None;
        egui::Grid::new("props_round")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Radius");
                let resp = ui.add(
                    crate::properties::number(&mut r, 0.05, " mm").range(0.01..=MAX_ROUND_RADIUS),
                );
                if resp.changed() {
                    edited = Some(resp.id.value());
                }
                ui.end_row();
            });
        ui.label(
            RichText::new(
                "Outward edges and corners are rounded, inner edges stay sharp. \
                 Parts thinner than twice the radius disappear.",
            )
            .weak()
            .small(),
        );
        if let Some(key) = edited
            && r != radius
        {
            let cmd = Command::SetModifier(id, Some(Modifier::RoundEdges { radius: r }));
            app.apply_merged(cmd, key);
        }
        if ui
            .button("Remove rounding")
            .on_hover_text("Ungroup: the object gets its sharp edges back")
            .clicked()
        {
            app.apply(Command::Ungroup(vec![id]));
        }
    }
}

impl RobocadApp {
    /// Rounds the outward edges of each selected object (objects that are already rounded are
    /// left alone). The new rounded groups become the selection.
    pub fn round_selection(&mut self) {
        let doc = self.editor.doc();
        let nodes: Vec<NodeId> = self
            .editor
            .top_level_selection()
            .into_iter()
            .filter(|id| {
                !matches!(
                    doc.get(*id).and_then(Node::modifier),
                    Some(Modifier::RoundEdges { .. })
                )
            })
            .collect();
        if nodes.is_empty() {
            return;
        }
        let radius = default_radius(doc, &nodes);
        self.apply(Command::AddModifier {
            nodes,
            modifier: Modifier::RoundEdges { radius },
        });
    }
}

/// 1 mm, but at most a fifth of the thinnest object's size so nothing disappears; a multiple
/// of 0.1 mm.
fn default_radius(doc: &Document, nodes: &[NodeId]) -> f64 {
    let thinnest = nodes
        .iter()
        .map(|id| doc.world_bounds(*id))
        .filter(|b| !b.is_empty())
        .map(|b| f64::from(b.size().min_element()))
        .fold(f64::INFINITY, f64::min);
    let r = DEFAULT_RADIUS.min(thinnest * 0.2);
    ((r * 10.0).floor() / 10.0).max(0.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use rc_doc::{Editor, PrimitiveKind};

    #[test]
    fn default_radius_fits_thin_objects() {
        let mut ed = Editor::default();
        let mut add = |size: DVec3| {
            ed.apply(Command::AddPrimitive {
                primitive: PrimitiveKind::Box.default_primitive().with_size(size),
                position: DVec3::ZERO,
                color: [0; 3],
            })
            .unwrap()
            .created[0]
        };
        let big = add(DVec3::splat(20.0));
        let thin = add(DVec3::new(20.0, 20.0, 2.0));
        let foil = add(DVec3::new(20.0, 20.0, 0.1));
        assert_eq!(default_radius(ed.doc(), &[big]), 1.0);
        assert!((default_radius(ed.doc(), &[big, thin]) - 0.4).abs() < 1e-12);
        assert!((default_radius(ed.doc(), &[foil]) - 0.1).abs() < 1e-12);
    }
}
