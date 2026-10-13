//! Benchmarks for STL parsing, welding, analysis and BVH construction on a ~1M triangle mesh.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rc_mesh::{Bvh, Mesh, stl, topology};

/// A closed UV sphere with `2 * rings * segments` triangles (minus pole degeneracy).
fn sphere(rings: u32, segments: u32, r: f32) -> Mesh {
    let mut positions = Vec::new();
    for i in 0..=rings {
        let theta = std::f32::consts::PI * i as f32 / rings as f32;
        for j in 0..segments {
            let phi = std::f32::consts::TAU * j as f32 / segments as f32;
            positions.push([
                r * theta.sin() * phi.cos(),
                r * theta.sin() * phi.sin(),
                r * theta.cos(),
            ]);
        }
    }
    let mut triangles = Vec::new();
    for i in 0..rings {
        for j in 0..segments {
            let a = i * segments + j;
            let b = i * segments + (j + 1) % segments;
            let c = a + segments;
            let d = b + segments;
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
    }
    Mesh::new(positions, triangles)
}

fn benches(c: &mut Criterion) {
    let mesh = sphere(500, 1000, 50.0); // 1,000,000 triangles
    let mut bytes = Vec::new();
    stl::write_binary(&mesh, "bench", &mut bytes).unwrap();
    let parsed = stl::read_bytes_slice(&bytes).unwrap().mesh;

    let mut g = c.benchmark_group("1M triangles");
    g.sample_size(10);
    g.bench_function("binary parse + weld", |b| {
        b.iter(|| stl::read_bytes_slice(black_box(&bytes)).unwrap());
    });
    g.bench_function("binary write", |b| {
        b.iter(|| {
            let mut out = Vec::with_capacity(bytes.len());
            stl::write_binary(black_box(&parsed), "bench", &mut out).unwrap();
            out
        });
    });
    g.bench_function("topology analyze", |b| {
        b.iter(|| topology::analyze(black_box(&parsed)));
    });
    g.bench_function("bvh build", |b| b.iter(|| Bvh::build(black_box(&parsed))));
    // A broken copy: drop every 1000th triangle (1000 holes) and flip every 7th.
    let mut broken = parsed.clone();
    let mut i = 0;
    broken.triangles.retain(|_| {
        i += 1;
        i % 1000 != 0
    });
    for (k, t) in broken.triangles.iter_mut().enumerate() {
        if k % 7 == 0 {
            t.swap(1, 2);
        }
    }
    g.bench_function("full repair (1000 holes, flipped faces)", |b| {
        b.iter(|| {
            rc_mesh::repair::repair(black_box(&broken), &rc_mesh::repair::RepairOptions::FULL)
        });
    });
    g.bench_function("volume + area", |b| {
        b.iter(|| black_box(&parsed).info());
    });
    g.finish();
}

criterion_group!(stl_benches, benches);
criterion_main!(stl_benches);
