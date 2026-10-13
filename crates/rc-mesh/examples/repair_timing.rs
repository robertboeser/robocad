//! Times each repair step on an STL file: `cargo run --release -p rc-mesh --example repair_timing -- file.stl`

use std::time::Instant;

use rc_mesh::{repair, stl, topology};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: repair_timing <file.stl>");
    let t = Instant::now();
    let mut m = stl::read_file(&path).expect("read").mesh;
    println!(
        "read+weld        {:>7.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    let step = |name: &str, f: &mut dyn FnMut()| {
        let t = Instant::now();
        f();
        println!("{name:<16} {:>7.1} ms", t.elapsed().as_secs_f64() * 1e3);
    };
    step("analyze", &mut || {
        topology::analyze(&m);
    });
    let tol = f64::from(m.aabb().diagonal()) * 1e-7;
    step("weld_epsilon", &mut || {
        repair::weld_epsilon(&mut m, tol);
    });
    step("remove_degen", &mut || {
        repair::remove_degenerate(&mut m);
    });
    step("remove_dups", &mut || {
        repair::remove_duplicates(&mut m);
    });
    step("split_touching", &mut || {
        repair::split_touching(&mut m);
    });
    step("orient", &mut || {
        repair::orient_consistently(&mut m);
    });
    step("outward", &mut || {
        repair::orient_outward(&mut m);
    });
    step("compact", &mut || repair::compact(&mut m));
    let original = stl::read_file(&path).expect("read").mesh;
    for (name, opts) in [
        ("SAFE repair", repair::RepairOptions::SAFE),
        ("FULL repair", repair::RepairOptions::FULL),
    ] {
        let t = Instant::now();
        let (_, r) = repair::repair(&original, &opts);
        println!(
            "{name:<16} {:>7.1} ms  {:?}",
            t.elapsed().as_secs_f64() * 1e3,
            r.summary()
        );
    }
}
