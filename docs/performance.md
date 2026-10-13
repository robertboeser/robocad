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
