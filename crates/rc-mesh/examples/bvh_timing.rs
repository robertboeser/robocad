//! Times BVH construction: `cargo run --release -p rc-mesh --example bvh_timing -- file.stl`
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: bvh_timing <file.stl>");
    let m = rc_mesh::stl::read_file(&path).expect("read").mesh;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let b = rc_mesh::Bvh::build(&m);
        println!(
            "bvh build {:.1} ms ({:?})",
            t.elapsed().as_secs_f64() * 1e3,
            b.bounds().size()
        );
    }
}
