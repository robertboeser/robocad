//! Renders an STL file to a PNG without opening a window.
//!
//! Usage: `cargo run -p rc-render --example render_png -- <in.stl> <out.png> [width height] [view]`
//! where `view` is one of iso, front, top, right (default iso).

use std::sync::Arc;

use rc_mesh::{edges::feature_edges, stl};
use rc_render::{
    DrawObject, DrawStyle, FrameDesc, OrbitCamera, Renderer, SceneStyle, StandardView,
    headless_device, srgb_to_linear,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: render_png <in.stl> <out.png> [width height] [iso|front|top|right]");
        std::process::exit(2);
    }
    let file = stl::read_file(&args[1]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[1]);
        std::process::exit(1);
    });
    let w: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1200);
    let h: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(800);
    let view = match args.get(5).map(String::as_str) {
        Some("front") => StandardView::Front,
        Some("top") => StandardView::Top,
        Some("right") => StandardView::Right,
        _ => StandardView::Iso,
    };
    let Some((device, queue)) = headless_device() else {
        eprintln!("no GPU adapter available");
        std::process::exit(1);
    };
    let mesh = Arc::new(file.mesh);
    let mut renderer = Renderer::new(&device);
    renderer.upload_mesh(&device, 1, &mesh, &feature_edges(&mesh, 30.0));
    let mut camera = OrbitCamera::default();
    camera.set_view(view);
    camera.frame(&mesh.aabb(), f64::from(w) / f64::from(h));
    let color = [0.29, 0.56, 0.89].map(srgb_to_linear);
    let objects = [DrawObject {
        mesh: 1,
        model: glam::DMat4::IDENTITY,
        color: [color[0], color[1], color[2], 1.0],
        style: DrawStyle::Solid,
        selected: false,
        hovered: false,
    }];
    let style = SceneStyle::dark();
    renderer.render(
        &device,
        &queue,
        [w, h],
        &FrameDesc {
            camera: &camera,
            objects: &objects,
            style: &style,
        },
    );
    renderer
        .save_png(&device, &queue, &args[2])
        .unwrap_or_else(|e| {
            eprintln!("cannot write {}: {e}", args[2]);
            std::process::exit(1);
        });
    println!("wrote {}", args[2]);
}
