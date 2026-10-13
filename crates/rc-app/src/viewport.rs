//! The 3D viewport widget: renders the scene with `rc-render` and handles camera input.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use glam::{DVec2, DVec3};
use rc_render::{
    DrawObject, FrameDesc, OrbitCamera, Projection, Renderer, SceneStyle, StandardView,
};

/// What happened in the viewport this frame.
pub struct ViewportOutput {
    /// Pointer position relative to the viewport (points), if hovered.
    pub hover: Option<DVec2>,
    /// Primary click without drag, relative to the viewport.
    pub clicked: Option<DVec2>,
    pub viewport_size: DVec2,
    /// Screen rectangle of the viewport.
    pub rect: Rect,
    pub drag_started_primary: bool,
    pub drag_stopped_primary: bool,
    /// Where the current primary press started, relative to the viewport.
    pub press_origin: Option<DVec2>,
    /// Pointer position relative to the viewport, even outside it.
    pub pointer_local: Option<DVec2>,
}

pub struct Viewport {
    render_state: RenderState,
    renderer: Renderer,
    texture: Option<(egui::TextureId, u64)>,
    pub camera: OrbitCamera,
}

impl Viewport {
    pub fn new(render_state: RenderState) -> Self {
        let renderer = Renderer::new(&render_state.device);
        Self {
            render_state,
            renderer,
            texture: None,
            camera: OrbitCamera::default(),
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.render_state.device
    }

    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Lays out, renders and handles input for the viewport, filling the available space.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        objects: &[DrawObject],
        style: &SceneStyle,
        tool_busy: bool,
    ) -> ViewportOutput {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let size = DVec2::new(f64::from(rect.width()), f64::from(rect.height()));
        let mut out = ViewportOutput {
            hover: None,
            clicked: None,
            viewport_size: size,
            rect,
            drag_started_primary: false,
            drag_stopped_primary: false,
            press_origin: None,
            pointer_local: None,
        };
        let to_local =
            |p: Pos2| DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));

        if !tool_busy {
            self.handle_input(ui, &response, rect, size);
        }
        if response.clicked_by(egui::PointerButton::Primary) {
            out.clicked = response.interact_pointer_pos().map(to_local);
        }
        out.hover = response.hover_pos().map(to_local);
        out.drag_started_primary = response.drag_started_by(egui::PointerButton::Primary);
        out.drag_stopped_primary = response.drag_stopped_by(egui::PointerButton::Primary);
        out.press_origin = ui.input(|i| i.pointer.press_origin()).map(to_local);
        out.pointer_local = ui.input(|i| i.pointer.latest_pos()).map(to_local);

        let ppp = ui.ctx().pixels_per_point();
        let px = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        self.camera.scene_radius = self.camera.scene_radius.max(f64::from(style.plate_size[0]));
        self.renderer.render(
            &self.render_state.device,
            &self.render_state.queue,
            px,
            &FrameDesc {
                camera: &self.camera,
                objects,
                style,
            },
        );
        self.sync_texture();
        if let Some((id, _)) = self.texture {
            ui.painter().image(
                id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        self.axis_widget(ui, rect);
        out
    }

    /// Registers the renderer's output texture with egui, re-registering after a resize.
    fn sync_texture(&mut self) {
        let generation = self.renderer.generation();
        let Some(view) = self.renderer.output_view() else {
            return;
        };
        let mut egui_renderer = self.render_state.renderer.write();
        match self.texture {
            Some((_, g)) if g == generation => {}
            Some((id, _)) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &self.render_state.device,
                    view,
                    wgpu::FilterMode::Nearest,
                    id,
                );
                self.texture = Some((id, generation));
            }
            None => {
                let id = egui_renderer.register_native_texture(
                    &self.render_state.device,
                    view,
                    wgpu::FilterMode::Nearest,
                );
                self.texture = Some((id, generation));
            }
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, size: DVec2) {
        let modifiers = ui.input(|i| i.modifiers);
        let delta = response.drag_delta();
        let d = DVec2::new(f64::from(delta.x), f64::from(delta.y));
        let secondary = response.dragged_by(egui::PointerButton::Secondary);
        let middle = response.dragged_by(egui::PointerButton::Middle);
        let alt_primary = response.dragged_by(egui::PointerButton::Primary) && modifiers.alt;
        if (secondary && modifiers.shift) || middle || (alt_primary && modifiers.shift) {
            self.camera.pan(d, size.y);
        } else if secondary || alt_primary {
            self.camera.orbit(d);
        }

        if !response.hovered() {
            return;
        }
        let cursor = response
            .hover_pos()
            .map(|p| DVec2::new(f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y)));
        let events = ui.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
            {
                if modifiers.command || modifiers.ctrl {
                    continue; // handled as zoom by egui (see zoom_delta below)
                }
                match unit {
                    egui::MouseWheelUnit::Point => {
                        // Trackpad: two-finger drag orbits, with Shift it pans.
                        let d = DVec2::new(f64::from(delta.x), f64::from(delta.y));
                        if modifiers.shift {
                            self.camera.pan(d, size.y);
                        } else {
                            self.camera.orbit(d);
                        }
                    }
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                        let steps = f64::from(delta.y + delta.x);
                        self.camera
                            .zoom(0.88f64.powf(steps), cursor.map(|c| (c, size)));
                    }
                }
            }
        }
        let zoom = f64::from(ui.input(egui::InputState::zoom_delta));
        if (zoom - 1.0).abs() > 1e-4 {
            self.camera.zoom(1.0 / zoom, cursor.map(|c| (c, size)));
        }
    }

    /// Small clickable axis indicator in the top-right corner (acts as a view cube).
    fn axis_widget(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let center = Pos2::new(rect.max.x - 56.0, rect.min.y + 56.0);
        let len = 34.0;
        let right = self.camera.right();
        let up = self.camera.up();
        let fwd = self.camera.forward();
        let axes = [
            (
                DVec3::X,
                "X",
                Color32::from_rgb(230, 70, 60),
                StandardView::Right,
            ),
            (
                DVec3::Y,
                "Y",
                Color32::from_rgb(90, 190, 70),
                StandardView::Back,
            ),
            (
                DVec3::Z,
                "Z",
                Color32::from_rgb(70, 130, 240),
                StandardView::Top,
            ),
            (
                -DVec3::X,
                "",
                Color32::from_rgb(230, 70, 60),
                StandardView::Left,
            ),
            (
                -DVec3::Y,
                "",
                Color32::from_rgb(90, 190, 70),
                StandardView::Front,
            ),
            (
                -DVec3::Z,
                "",
                Color32::from_rgb(70, 130, 240),
                StandardView::Bottom,
            ),
        ];
        let mut items: Vec<_> = axes
            .iter()
            .map(|(a, label, color, view)| {
                let p = center + Vec2::new(a.dot(right) as f32, -a.dot(up) as f32) * len;
                (a.dot(fwd), p, *label, *color, *view)
            })
            .collect();
        // Draw far axes first.
        items.sort_by(|a, b| b.0.total_cmp(&a.0));
        let painter = ui.painter_at(rect);
        painter.circle_filled(center, len + 14.0, Color32::from_black_alpha(40));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let clicked = ui.input(|i| i.pointer.primary_clicked());
        let mut chosen = None;
        for (_, p, label, color, view) in items {
            let hovered = pointer.is_some_and(|q| q.distance(p) < 10.0);
            if label.is_empty() {
                painter.circle(p, 6.0, color.gamma_multiply(0.35), Stroke::new(1.5, color));
            } else {
                painter.line_segment([center, p], Stroke::new(2.5, color));
                painter.circle_filled(p, 9.0, color);
                painter.text(
                    p,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
            if hovered {
                painter.circle_stroke(p, 11.0, Stroke::new(1.5, Color32::WHITE));
                if clicked {
                    chosen = Some(view);
                }
            }
        }
        if let Some(view) = chosen {
            self.camera.set_view(view);
        }
        let label = match self.camera.projection {
            Projection::Perspective => "Persp",
            Projection::Orthographic => "Ortho",
        };
        let r = Rect::from_center_size(center + Vec2::new(0.0, len + 26.0), Vec2::new(48.0, 18.0));
        let resp = ui.put(
            r,
            egui::Button::new(egui::RichText::new(label).small()).small(),
        );
        if resp
            .on_hover_text("Toggle perspective / orthographic (5)")
            .clicked()
        {
            self.toggle_projection();
        }
    }

    pub fn toggle_projection(&mut self) {
        self.camera.projection = match self.camera.projection {
            Projection::Perspective => Projection::Orthographic,
            Projection::Orthographic => Projection::Perspective,
        };
    }
}
