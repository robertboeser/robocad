# Performance measurements

## Summary against the targets in PLAN.md §1 (M9, release build, Apple Silicon)

| Metric | Target | Measured | Status |
|---|---|---|---|
| Start to interactive window | < 0.5 s | ~0.28 s (first launch after build ~0.5 s) | ✅ |
| Idle CPU | ~0 % | 0.0–0.7 % right after start, 0.0 % idle | ✅ |
| RAM, empty scene | < 120 MB | ~94 MB | ✅ |
| Load binary STL, 1M triangles (parse, safe repair, edges, BVH, analysis) | < 0.5 s | ~0.32 s | ✅ |
| Viewport, 5M triangles | ≥ 60 fps | ~2 ms/frame (~500 fps) at 2560×1600, 4× MSAA | ✅ |
| Boolean of two 100k-triangle meshes | < 150 ms | ~21 ms (difference), ~26 ms (union) | ✅ |
| Release binary size | < 25 MB | ~13.4 MB (`robocad`), DMG 8.9 MB | ✅ |

M9 performance pass: the import-time safe repair now skips steps the analysis shows are not
needed (clean meshes cost one analysis; 1M-triangle import fix-up 530 → 173 ms), and the BVH is
built in parallel (250 → 62 ms).

Machine: Apple Silicon (M-series), macOS, release/bench profile. Benchmarks: `cargo bench -p rc-mesh`.

## rc-mesh (1,000,000-triangle UV sphere)

| Operation | Time | Target |
|---|---|---|
| Binary STL parse + exact weld | ~30 ms | < 300 ms |
| Binary STL write | ~26 ms | — |
| Topology analysis | ~42 ms | — |
| BVH build (binned SAH; parallel since M9: ~62 ms) | ~250 ms | — (runs on worker thread) |
| Volume + area + bbox | ~23 ms | — |

## Viewer (M2)

| Metric | Measured | Target |
|---|---|---|
| Window visible after launch (release) | ~470 ms (incl. measuring overhead) | < 0.5 s |
| Idle CPU | 0.0 % | ~0 % |
| RSS, empty scene | ~66 MB | < 120 MB |
| RSS, 1M-triangle mesh loaded | ~160 MB | — |
| Load 1M-triangle binary STL (parse, weld, edges, BVH, analysis in parallel) | ~310 ms | < 0.5 s |
| Render 5M triangles, 2560×1600, 4× MSAA (`bench_render` example) | ~2 ms/frame | ≥ 60 fps |

Reproduce: `cargo run --release -p rc-render --example bench_render -- big.stl 5 120`.

## Booleans (M5)

`cargo bench -p rc-kernel --bench boolean` — two overlapping spheres, 101,760 triangles each.

| Operation | Time | Target |
|---|---|---|
| Mesh → solid | ~16 ms | — |
| Difference + back to mesh | ~21 ms | < 150 ms |
| Union + back to mesh | ~26 ms | < 150 ms |

Release app after M5: binary 11.8 MB (target < 25 MB), warm start ~85 ms, empty-scene RSS ~74 MB.

## Repair (M6)

| Operation | Time | Target |
|---|---|---|
| Full repair, 1M triangles with 1000 holes and every 7th face flipped | ~0.8 s | < 2 s |

## Patterns (M8)

| Operation | Time | Target |
|---|---|---|
| 110 × 110 mm plate minus a 10 × 10 grid of cylinders (release) | ~30 ms | < 500 ms |

## Round edges (M10)

`cargo run --release -p rc-kernel --example round_timing` (radius 0.5–2 mm):

| Shape | Input triangles | Time |
|---|---|---|
| Box 40 × 30 × 10 mm (convex fast path) | 12 | < 1 ms |
| Cylinder 20 mm (convex fast path) | 192 | 2–3 ms |
| Sphere 20 mm (convex fast path) | 2,208 | 20–30 ms |
| Plate with a through-hole (Minkowski path) | 144 | 0.4–0.7 s |
| Torus 30 mm (Minkowski path, 8-segment ball) | 3,584 | 3–4.5 s |

The general path grows with triangle count × ball vertices; it runs on the evaluation worker.

## M9 tools

- `cargo run --release -p rc-mesh --example repair_timing -- file.stl` — time per repair step
- `cargo run --release -p rc-mesh --example bvh_timing -- file.stl`
- CI: run the "CI" workflow manually with *bench* enabled to run all benchmarks.
