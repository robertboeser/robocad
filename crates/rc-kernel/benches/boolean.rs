//! Boolean performance on two overlapping ~100k-triangle spheres.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::{DMat4, DVec3};
use rc_kernel::{GeometryKernel, ManifoldKernel, Primitive};

fn benches(c: &mut Criterion) {
    let k = ManifoldKernel;
    let p = Primitive::Sphere {
        size: DVec3::splat(40.0),
        segments: Some(320),
    };
    let mesh = p.mesh();
    let a = k.solid_from_mesh(&mesh).unwrap();
    let b = k.transform(&a, &DMat4::from_translation(DVec3::new(15.0, 5.0, 3.0)));
    let mut g = c.benchmark_group(format!("{} triangles each", mesh.triangle_count()));
    g.sample_size(10);
    g.bench_function("mesh -> solid", |bch| {
        bch.iter(|| k.solid_from_mesh(black_box(&mesh)).unwrap());
    });
    g.bench_function("difference + to_mesh", |bch| {
        bch.iter(|| k.solid_to_mesh(&k.difference(black_box(&a), black_box(&b))));
    });
    g.bench_function("union + to_mesh", |bch| {
        bch.iter(|| k.solid_to_mesh(&k.union(&[a.clone(), b.clone()])));
    });
    g.finish();
}

criterion_group!(boolean, benches);
criterion_main!(boolean);
