# Changelog

## Unreleased

### M2 — Viewer
- `rc-render`: wgpu renderer with offscreen 4× MSAA target, reverse-Z depth, flat shading from screen-space derivatives, feature edges, diagnostic (open/non-manifold) edges, reddish back faces, procedural build plate grid with axes, background gradient.
- Orbit camera (perspective/orthographic, zoom to cursor, pan, frame, standard views) with tests.
- Headless rendering to PNG (`render_png` example), render benchmark, pixel test.
- `robocad` app: menu, scene and properties panels, status bar, background STL loading (parallel analysis), drag & drop, file dialog, picking, axis widget, keyboard views, trackpad support, light/dark theme.

### M1 — Mesh core & STL I/O
- `rc-mesh`: indexed `Mesh`, `Aabb`, exact vertex welding, volume/area/bbox, content hash.
- Binary and ASCII STL reading (robust format detection, parallel binary parsing, tolerant ASCII parser) and atomic writing.
- Topology analysis: boundary/non-manifold edges, non-manifold vertices, degenerate triangles, orientation consistency, components.
- SAH BVH with ray casting and box queries.
- `robocad-cli info` and `robocad-cli convert`.
- STL fixtures with generator script, criterion benchmarks (1M-triangle parse + weld ≈ 30 ms).

### M0 — Project skeleton & CI
- Cargo workspace with crates `rc-mesh`, `rc-kernel`, `rc-doc`, `rc-render`, `rc-app` (`robocad`), `rc-cli` (`robocad-cli`).
- Shared lints (clippy pedantic with selected allows, `unsafe_code = "deny"`), tuned dev/release profiles.
- GitHub Actions CI: fmt, clippy, tests on macOS, Linux and Windows.
