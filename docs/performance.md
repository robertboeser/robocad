# Performance measurements

Machine: Apple Silicon (M-series), macOS, release/bench profile. Benchmarks: `cargo bench -p rc-mesh`.

## rc-mesh (1,000,000-triangle UV sphere)

| Operation | Time | Target |
|---|---|---|
| Binary STL parse + exact weld | ~30 ms | < 300 ms |
| Binary STL write | ~26 ms | — |
| Topology analysis | ~42 ms | — |
| BVH build (binned SAH) | ~250 ms | — (runs on worker thread) |
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
