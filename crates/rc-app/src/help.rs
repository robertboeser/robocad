//! Keyboard shortcut reference, first-run hints and the Help menu.

use eframe::egui::{self, RichText};

use crate::app::RobocadApp;

/// (keys, action) pairs grouped by section.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "View",
        &[
            ("Right drag · two-finger drag", "Orbit"),
            ("Middle drag · Shift + right drag", "Pan"),
            ("Shift + two-finger drag", "Pan (trackpad)"),
            ("Wheel · pinch", "Zoom to the cursor"),
            ("Alt + left drag", "Orbit (one-button mouse)"),
            ("F / Shift+F", "Frame selection / frame all"),
            ("1 · 3 · 7 · 0", "Front · right · top · isometric view"),
            ("5", "Perspective / orthographic"),
        ],
    ),
    (
        "Select & arrange",
        &[
            ("Click · Shift/⌘ + click", "Select · add to selection"),
            ("Drag on empty space", "Box selection"),
            ("Drag an object", "Slide it on the plate"),
            ("W · E · R", "Move · rotate · scale tool"),
            ("Alt while dragging", "Toggle snapping"),
            ("Arrows · Shift+arrows", "Nudge 1 mm · 10 mm"),
            ("PgUp · PgDn", "Nudge up · down"),
            ("D", "Drop to the plate"),
            ("Esc", "Cancel drag / tool, clear selection"),
        ],
    ),
    (
        "Edit",
        &[
            ("⌘Z · Shift+⌘Z", "Undo · redo"),
            ("⌘D", "Duplicate"),
            ("Del / Delete", "Delete"),
            ("⌘G · Shift+⌘G", "Group · ungroup"),
            ("H", "Toggle solid / hole"),
            ("⌘A", "Select all"),
            ("M · L", "Measure · lay flat"),
        ],
    ),
    (
        "Files",
        &[
            ("⌘N · ⌘O · ⌘S · Shift+⌘S", "New · open · save · save as"),
            ("⌘I", "Import STL / OBJ"),
            ("⌘,", "Preferences"),
            ("F1 · ?", "This list"),
        ],
    ),
];

/// The few shortcuts shown to new users.
const FIRST_STEPS: &[(&str, &str)] = &[
    ("Drop an STL", "or click a shape in the toolbar"),
    ("Right drag / two fingers", "orbit the view"),
    ("Wheel / pinch", "zoom"),
    ("Drag an object", "move it on the plate"),
    ("W · E · R", "move, rotate, scale handles"),
    ("H", "turn a shape into a hole"),
    ("⌘G", "group: holes cut the solids"),
    ("File > Export STL", "send it to your slicer"),
];

/// Adapts shortcut text to the platform: ⌘ on macOS, Ctrl+ elsewhere.
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.to_string()
    } else {
        s.replace('⌘', "Ctrl+")
    }
}

#[derive(Default)]
pub struct HelpState {
    pub show_shortcuts: bool,
}

impl RobocadApp {
    pub(crate) fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard shortcuts  F1").clicked() {
                self.help.show_shortcuts = true;
                ui.close();
            }
            if ui.button("Show first steps again").clicked() {
                self.settings.hints_shown = false;
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new(format!("robocad {}", env!("CARGO_PKG_VERSION"))).weak());
        });
    }

    pub(crate) fn help_windows(&mut self, ctx: &egui::Context) {
        // Ask before locking the input state: calling back into the context inside
        // `input_mut` would deadlock.
        let typing = ctx.egui_wants_keyboard_input();
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::F1)
                || (!typing && i.consume_key(egui::Modifiers::SHIFT, egui::Key::Slash))
                || (!typing && i.consume_key(egui::Modifiers::NONE, egui::Key::Questionmark))
        }) {
            self.help.show_shortcuts = !self.help.show_shortcuts;
        }
        if self.help.show_shortcuts {
            let mut open = true;
            egui::Window::new("Keyboard shortcuts")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            for (title, items) in SHORTCUTS {
                                ui.label(RichText::new(*title).strong().size(15.0));
                                egui::Grid::new(*title)
                                    .num_columns(2)
                                    .spacing([16.0, 3.0])
                                    .min_col_width(170.0)
                                    .show(ui, |ui| {
                                        for (k, a) in *items {
                                            ui.label(RichText::new(keys(k)).strong());
                                            ui.label(*a);
                                            ui.end_row();
                                        }
                                    });
                                ui.add_space(8.0);
                            }
                        });
                });
            if !open {
                self.help.show_shortcuts = false;
            }
        }
        if !self.settings.hints_shown && !self.project.recovery_offer {
            let mut close = false;
            egui::Window::new("Welcome to robocad")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::LEFT_BOTTOM, [250.0, -40.0])
                .show(ctx, |ui| {
                    egui::Grid::new("first_steps")
                        .num_columns(2)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            for (k, a) in FIRST_STEPS {
                                ui.label(RichText::new(keys(k)).strong());
                                ui.label(*a);
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Got it").clicked() {
                            close = true;
                        }
                        if ui.link("All shortcuts (F1)").clicked() {
                            self.help.show_shortcuts = true;
                        }
                    });
                });
            if close {
                self.settings.hints_shown = true;
            }
        }
    }
}
