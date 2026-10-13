//! Renders a cube offscreen and checks a few pixels. Skipped when no GPU adapter exists.

use glam::{DMat4, Vec3};
use rc_mesh::Mesh;
use rc_mesh::edges::feature_edges;
use rc_render::{
    DrawObject, DrawStyle, FrameDesc, OrbitCamera, Renderer, SceneStyle, StandardView,
};

#[test]
fn cube_renders_in_the_centre() {
    let Some((device, queue)) = rc_render::headless_device() else {
        eprintln!("no GPU adapter, skipping visual test");
        return;
    };
    let mesh = Mesh::cuboid(Vec3::splat(-10.0), Vec3::splat(10.0));
    let mut r = Renderer::new(&device);
    r.upload_mesh(&device, 7, &mesh, &feature_edges(&mesh, 30.0));
    let mut camera = OrbitCamera::default();
    camera.set_view(StandardView::Front);
    camera.frame(&mesh.aabb(), 1.0);
    let style = SceneStyle {
        show_grid: false,
        ..SceneStyle::dark()
    };
    let objects = [DrawObject {
        mesh: 7,
        model: DMat4::IDENTITY,
        color: [1.0, 0.0, 0.0, 1.0],
        style: DrawStyle::Solid,
        selected: false,
        hovered: false,
    }];
    r.render(
        &device,
        &queue,
        [64, 64],
        &FrameDesc {
            camera: &camera,
            objects: &objects,
            style: &style,
        },
    );
    let (w, h, px) = r.read_pixels(&device, &queue).expect("readback");
    assert_eq!((w, h), (64, 64));
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let centre = at(32, 32);
    assert!(
        centre[0] > 100 && centre[1] < 60 && centre[2] < 60,
        "centre {centre:?}"
    );
    let corner = at(1, 1);
    assert!(
        corner[0] < 80,
        "corner should be background, got {corner:?}"
    );
}
