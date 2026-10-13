//! Measures frame time for a scene of N copies of an STL (default: 5 copies).
//!
//! Usage: `cargo run --release -p rc-render --example bench_render -- <in.stl> [copies] [frames]`

use std::time::Instant;

use glam::{DMat4, DVec3};
use rc_mesh::{edges::feature_edges, stl};
use rc_render::{
    DrawObject, DrawStyle, FrameDesc, OrbitCamera, Renderer, SceneStyle, headless_device,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: bench_render <in.stl> [copies] [frames]");
        std::process::exit(2);
    };
    let copies: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let frames: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);
    let mesh = stl::read_file(path).expect("read stl").mesh;
    let (device, queue) = headless_device().expect("gpu");
    let mut r = Renderer::new(&device);
    r.upload_mesh(&device, 1, &mesh, &feature_edges(&mesh, 30.0));
    let size = mesh.aabb().size().as_dvec3();
    let objects: Vec<DrawObject> = (0..copies)
        .map(|i| DrawObject {
            mesh: 1,
            model: DMat4::from_translation(DVec3::new(i as f64 * size.x * 1.1, 0.0, 0.0)),
            color: [0.2, 0.4, 0.8, 1.0],
            style: DrawStyle::Solid,
            selected: false,
            hovered: false,
        })
        .collect();
    let mut cam = OrbitCamera::default();
    let total = mesh.aabb().union(&rc_mesh::Aabb::new(
        mesh.aabb().min,
        mesh.aabb().max + glam::Vec3::new((copies as f32) * size.x as f32 * 1.1, 0.0, 0.0),
    ));
    cam.frame(&total, 1.6);
    let style = SceneStyle::dark();
    let frame = FrameDesc {
        camera: &cam,
        objects: &objects,
        style: &style,
    };
    r.render(&device, &queue, [2560, 1600], &frame);
    let _ = r.read_pixels(&device, &queue);
    let start = Instant::now();
    for i in 0..frames {
        cam.orbit(glam::DVec2::new(2.0, 0.0));
        let frame = FrameDesc {
            camera: &cam,
            objects: &objects,
            style: &style,
        };
        r.render(&device, &queue, [2560, 1600], &frame);
        if i % 10 == 9 {
            device.poll(wgpu::PollType::wait_indefinitely()).ok();
        }
    }
    let _ = r.read_pixels(&device, &queue);
    let ms = start.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!(
        "{} triangles x {copies} = {} triangles, 2560x1600 4xMSAA: {ms:.2} ms/frame ({:.0} fps)",
        mesh.triangle_count(),
        mesh.triangle_count() * copies,
        1000.0 / ms
    );
}
