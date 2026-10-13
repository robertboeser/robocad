# Changelog

## Unreleased

### M3 — Document, selection, properties, undo
- `rc-doc`: document tree (slotmap IDs, groups, solid/hole roles, content-addressed mesh store), transforms with Euler degrees, world matrices and bounds.
- Command-based editing with snapshot undo/redo (200 steps), merging of continuous edits, selection handling, typed-arithmetic expression parser.
- Commands: import, add, delete, transform, rename, color, visibility, lock, solid/hole, reparent (keeps world transform), group, ungroup, group mode, duplicate.
- App: hierarchical scene tree (rename, visibility, lock, context menu, drag-to-reparent), properties panel (position/rotation/size with math input, keep proportions, role, group mode, color, mesh info), Edit menu, shortcuts, viewport picking with hover, box selection, dirty indicator.
- Renderer: screen-space selection and hover outline.
- Headless UI tests with egui_kittest (click/box select, delete/undo/redo, group, rename, typed math).

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
