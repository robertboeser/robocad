//! Small vector icons painted with egui shapes (no image assets or extra dependencies).

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Box,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    Wedge,
    Text,
    Move,
    Rotate,
    Scale,
    Measure,
    LayFlat,
    Round,
}

/// Paints `icon` into `rect` with stroke colour `c`.
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, c: Color32) {
    let s = Stroke::new(1.4, c);
    let r = rect.shrink(1.0);
    let ctr = r.center();
    let w = r.width().min(r.height());
    let at = |x: f32, y: f32| pos2(ctr.x + x * w * 0.5, ctr.y + y * w * 0.5);
    let fill = c.gamma_multiply(0.25);
    match icon {
        Icon::Box => {
            let front = [
                at(-0.8, -0.35),
                at(0.35, -0.35),
                at(0.35, 0.8),
                at(-0.8, 0.8),
            ];
            p.add(egui::Shape::convex_polygon(front.to_vec(), fill, s));
            p.line(
                vec![
                    at(-0.8, -0.35),
                    at(-0.35, -0.8),
                    at(0.8, -0.8),
                    at(0.35, -0.35),
                ],
                s,
            );
            p.line(vec![at(0.8, -0.8), at(0.8, 0.35), at(0.35, 0.8)], s);
        }
        Icon::Cylinder => {
            p.add(egui::Shape::ellipse_stroke(
                at(0.0, -0.6),
                vec2(w * 0.38, w * 0.16),
                s,
            ));
            p.line_segment([at(-0.76, -0.6), at(-0.76, 0.6)], s);
            p.line_segment([at(0.76, -0.6), at(0.76, 0.6)], s);
            half_ellipse(p, at(0.0, 0.6), vec2(w * 0.38, w * 0.16), s);
        }
        Icon::Cone => {
            p.line(vec![at(-0.76, 0.6), at(0.0, -0.85), at(0.76, 0.6)], s);
            half_ellipse(p, at(0.0, 0.6), vec2(w * 0.38, w * 0.16), s);
        }
        Icon::Sphere => {
            p.circle(ctr, w * 0.42, fill, s);
            p.add(egui::Shape::ellipse_stroke(
                ctr,
                vec2(w * 0.42, w * 0.14),
                Stroke::new(1.0, c),
            ));
        }
        Icon::Torus => {
            p.add(egui::Shape::ellipse_stroke(
                ctr,
                vec2(w * 0.45, w * 0.28),
                s,
            ));
            p.add(egui::Shape::ellipse_stroke(
                ctr,
                vec2(w * 0.18, w * 0.09),
                s,
            ));
        }
        Icon::Wedge => {
            p.add(egui::Shape::convex_polygon(
                vec![at(-0.8, -0.7), at(-0.8, 0.75), at(0.8, 0.75)],
                fill,
                s,
            ));
        }
        Icon::Text => {
            p.line(vec![at(-0.7, -0.75), at(0.7, -0.75)], Stroke::new(2.0, c));
            p.line_segment([at(0.0, -0.75), at(0.0, 0.8)], Stroke::new(2.0, c));
        }
        Icon::Move => {
            for d in [
                vec2(1.0, 0.0),
                vec2(-1.0, 0.0),
                vec2(0.0, 1.0),
                vec2(0.0, -1.0),
            ] {
                arrow(p, ctr, ctr + d * w * 0.45, s);
            }
        }
        Icon::Rotate => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.25 + 1.5 * i as f32 / 24.0);
                    ctr + Vec2::angled(a) * w * 0.38
                })
                .collect();
            let end = pts[pts.len() - 1];
            p.line(pts, s);
            let tip = end + vec2(w * 0.12, w * 0.02);
            p.line(vec![end + vec2(-w * 0.05, -w * 0.12), end, tip], s);
        }
        Icon::Scale => {
            p.rect_stroke(
                Rect::from_center_size(at(-0.25, 0.25), Vec2::splat(w * 0.5)),
                0.0,
                s,
                egui::StrokeKind::Middle,
            );
            arrow(p, at(-0.1, 0.1), at(0.8, -0.8), s);
        }
        Icon::Measure => {
            let a = at(-0.85, 0.35);
            let b = at(0.85, -0.35);
            p.line_segment([a, b], s);
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let q = a + (b - a) * t;
                let n = (b - a).normalized().rot90() * w * if t == 0.5 { 0.18 } else { 0.1 };
                p.line_segment([q - n, q + n], s);
            }
        }
        Icon::LayFlat => {
            p.line_segment([at(-0.9, 0.8), at(0.9, 0.8)], s);
            p.add(egui::Shape::convex_polygon(
                vec![at(-0.5, 0.55), at(0.5, 0.55), at(0.0, -0.2)],
                fill,
                s,
            ));
            arrow(p, at(0.0, -0.9), at(0.0, -0.35), s);
        }
        Icon::Round => {
            let r = Rect::from_center_size(ctr, Vec2::splat(w * 0.8));
            p.rect(r, w * 0.25, fill, s, egui::StrokeKind::Middle);
        }
    }
}

fn half_ellipse(p: &egui::Painter, c: Pos2, r: Vec2, s: Stroke) {
    let pts: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 16.0;
            pos2(c.x + r.x * a.cos(), c.y + r.y * a.sin())
        })
        .collect();
    p.line(pts, s);
}

fn arrow(p: &egui::Painter, from: Pos2, to: Pos2, s: Stroke) {
    p.line_segment([from, to], s);
    let d = (to - from).normalized();
    let n = d.rot90();
    let len = 4.0;
    p.line(
        vec![
            to - d * len + n * len * 0.7,
            to,
            to - d * len - n * len * 0.7,
        ],
        s,
    );
}

/// A toolbar button with a painted icon in front of the label.
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let id = ui.id().with(("icon", label));
    let out =
        egui::Button::selectable(selected, (egui::Atom::custom(id, Vec2::splat(14.0)), label))
            .sense(egui::Sense::click_and_drag())
            .atom_ui(ui);
    if let Some(rect) = out.rect(id) {
        let color = if selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().widgets.inactive.fg_stroke.color
        };
        paint(ui.painter(), rect, icon, color);
    }
    out.response
}
