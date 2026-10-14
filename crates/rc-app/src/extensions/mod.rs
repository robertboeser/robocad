//! Optional extensions: features that bring their own buttons, menu entries and property
//! sections.
//!
//! Extensions are compiled in but **disabled by default**, so the interface stays simple; users
//! turn them on in Preferences → Extensions. Geometry an extension creates is stored in the
//! document (e.g. [`rc_doc::Modifier`]) and evaluated by the headless crates, so a project that
//! uses an extension opens, renders and exports the same whether or not it is enabled.
//!
//! To add an extension: implement [`Extension`] in a submodule and list it in [`ALL`]. Put its
//! geometry and commands in `rc-kernel` / `rc-doc` so they are testable and the CLI can use
//! them; the extension itself only maps UI to commands.

pub mod round_edges;

use eframe::egui::{self, RichText};
use rc_doc::NodeId;

use crate::app::RobocadApp;

/// An optional feature. All hooks have empty defaults; implement the ones you need.
pub trait Extension: Sync {
    /// Stable identifier stored in the settings; never change it once released.
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// One or two sentences for the preferences window.
    fn description(&self) -> &'static str;
    /// Toolbar widgets, shown while enabled.
    fn toolbar(&self, _app: &mut RobocadApp, _ui: &mut egui::Ui) {}
    /// Entries acting on the selection, shown in the Edit menu and the scene context menu while
    /// enabled. Call `ui.close()` after running an action.
    fn selection_menu(&self, _app: &mut RobocadApp, _ui: &mut egui::Ui) {}
    /// A properties section for a single selected node. Called even while disabled, so nodes
    /// that already use the extension stay editable; show nothing for other nodes.
    fn node_properties(&self, _app: &mut RobocadApp, _ui: &mut egui::Ui, _id: NodeId) {}
}

/// Every available extension, in display order.
pub static ALL: &[&dyn Extension] = &[&round_edges::RoundEdges];

impl RobocadApp {
    pub fn extension_enabled(&self, id: &str) -> bool {
        self.settings.extensions.contains(id)
    }

    /// Turns an extension on or off (saved with the settings).
    pub fn set_extension_enabled(&mut self, id: &str, on: bool) {
        if on {
            self.settings.extensions.insert(id.to_string());
        } else {
            self.settings.extensions.remove(id);
        }
    }

    fn enabled_extensions(&self) -> Vec<&'static dyn Extension> {
        ALL.iter()
            .copied()
            .filter(|e| self.extension_enabled(e.id()))
            .collect()
    }

    pub(crate) fn extensions_toolbar(&mut self, ui: &mut egui::Ui) {
        let enabled = self.enabled_extensions();
        if enabled.is_empty() {
            return;
        }
        ui.separator();
        for e in enabled {
            e.toolbar(self, ui);
        }
    }

    pub(crate) fn extensions_selection_menu(&mut self, ui: &mut egui::Ui) {
        let enabled = self.enabled_extensions();
        if enabled.is_empty() {
            return;
        }
        ui.separator();
        for e in enabled {
            e.selection_menu(self, ui);
        }
    }

    pub(crate) fn extensions_node_properties(&mut self, ui: &mut egui::Ui, id: NodeId) {
        for e in ALL {
            e.node_properties(self, ui, id);
        }
    }

    pub(crate) fn extensions_preferences(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Extensions").strong());
        ui.label(
            RichText::new(
                "Optional features. They are off by default to keep the interface simple.",
            )
            .weak(),
        );
        for e in ALL {
            let mut on = self.extension_enabled(e.id());
            if ui.checkbox(&mut on, e.name()).changed() {
                self.set_extension_enabled(e.id(), on);
            }
            ui.indent(e.id(), |ui| {
                ui.label(RichText::new(e.description()).weak());
            });
        }
    }
}
