//! Times edge rounding on a few shapes: `cargo run --release -p rc-kernel --example round_timing`

use std::time::Instant;

use glam::{DMat4, DVec3};
use rc_kernel::{GeometryKernel, ManifoldKernel, PrimitiveKind, Solid};

fn prim(kind: PrimitiveKind, size: DVec3, at: DVec3) -> Solid {
    let k = ManifoldKernel;
    let mesh = kind.default_primitive().with_size(size).mesh();
    let s = k.solid_from_mesh(&mesh).expect("primitive is manifold");
    k.transform(&s, &DMat4::from_translation(at))
}

fn main() {
    let k = ManifoldKernel;
    let plate = k.difference(
        &prim(
            PrimitiveKind::Box,
            DVec3::new(40.0, 30.0, 10.0),
            DVec3::ZERO,
        ),
        &prim(
            PrimitiveKind::Cylinder,
            DVec3::new(10.0, 10.0, 30.0),
            DVec3::new(0.0, 0.0, -10.0),
        ),
    );
    let shapes = [
        (
            "box",
            prim(
                PrimitiveKind::Box,
                DVec3::new(40.0, 30.0, 10.0),
                DVec3::ZERO,
            ),
        ),
        (
            "cylinder",
            prim(PrimitiveKind::Cylinder, DVec3::splat(20.0), DVec3::ZERO),
        ),
        (
            "sphere",
            prim(PrimitiveKind::Sphere, DVec3::splat(20.0), DVec3::ZERO),
        ),
        ("plate with hole", plate),
        (
            "torus",
            prim(
                PrimitiveKind::Torus,
                DVec3::new(30.0, 30.0, 8.0),
                DVec3::ZERO,
            ),
        ),
    ];
    for r in [0.5, 1.0, 2.0] {
        for (name, s) in &shapes {
            let t = Instant::now();
            let out = k.round_edges(s, r);
            println!(
                "r {r:>3} {name:<16} {:>6} → {:>6} tris  {:>8.1} ms",
                k.triangle_count(s),
                k.triangle_count(&out),
                t.elapsed().as_secs_f64() * 1e3
            );
        }
    }
}
