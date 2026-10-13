# Changelog

## Unreleased

### M7 — Projects & settings
- `.rcad` project files (ZIP: `document.json` + raw meshes + thumbnail), atomic writes, checksum and tree validation, versioned format with migration hook and a committed v1 fixture.
- App: New / Open (projects and STL) / Open recent / Save / Save As, background load and save, window title with file name and • for unsaved changes, unsaved-changes prompt on New, Open and Quit, autosave (default every 2 minutes) with crash recovery on next start.
- Settings in the OS config directory: theme, UI scale, mouse preset (robocad / Blender / Fusion 360), defaults for new projects, autosave interval, import options, recent files; Preferences window (⌘,).
- `robocad-cli info <project.rcad>` and `robocad-cli export <project.rcad> <out.stl>`.

### M6 — Inspect & Repair
- `rc-mesh::repair`: epsilon weld, degenerate and duplicate removal, separation of touching parts (bow-tie vertices, shared edges), orientation fix, outward orientation, hole filling with ear clipping, optional small-part removal; idempotent, with a change report.
- Safe fixes run automatically on import and are listed under the mesh info; the full Repair runs in the background, replaces the mesh (undoable) and reports what changed.
- Overhang view (> 45°), import units (mm/cm/m/inch), place-on-plate after import.
- `robocad-cli repair <in> <out> [--no-fill] [--min-part-triangles N]`.
- All broken fixtures repair to clean solids accepted by the boolean kernel; 1M-triangle repair ≈ 0.8 s.

### M5 — Booleans & export
- `rc-kernel`: `GeometryKernel` trait with the Manifold backend (`manifold-csg`, ADR 0001); property tests for volume identities and watertight results.
- `rc-doc::eval`: local-frame content hashes, solid cache, group evaluation (union − holes, intersection, hidden children ignored), background `Evaluator`, world-space `export_mesh`.
- App: groups display their combined result (stale result kept while recomputing), selected parts inside groups shown as ghosts, per-group errors in tree and properties, result info, "Combining…" status.
- Export STL (scene or selection) in the background with watertightness warnings; bounds of groups ignore holes.
- Example `plate_with_holes` export verified by PrusaSlicer (`manifold = yes`, one part).
- Benchmarks: 100k × 100k triangle difference ≈ 21 ms, union ≈ 26 ms.

### M4 — Primitives & direct manipulation
- `rc-kernel`: parametric box, cylinder/cone (top ratio), ellipsoid, torus and wedge with adaptive tessellation (0.025 mm chord tolerance); all watertight (property-tested).
- Document: primitive nodes, set-primitive, drop-to-plate, align and free-spot placement.
- Toolbar: click a shape to add it at the nearest free spot, or drag it onto the plate; move/rotate/scale tools; snapping toggle and grid step; drop, center, solid/hole.
- Own transform gizmo: move arrows and planes, rotation rings (15° snap), scale handles (1 mm size snap), Alt inverts snapping, Esc cancels, one undo step per drag.
- Drag objects directly to slide them over the plate; arrow keys nudge relative to the view (Shift = 10×), PgUp/PgDn move in Z.
- Properties: size edits primitive parameters, shape section (top size, tube width, segments), align buttons for multi-selection.

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
