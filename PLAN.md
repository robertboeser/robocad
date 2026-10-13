# robocad — Implementation Plan

> A simple, fast, lightweight desktop tool for building 3D-printable designs and working with STL files.
> This document is the build spec for an implementing agent (Claude Opus 5.5). Read it fully before writing code.

---

## 0. Open questions & assumptions

The plan was written without answers to the questions below. Each has a **default assumption** that the plan follows.
If the owner answers differently, adjust the affected sections before starting the matching milestone.

| # | Question | Default assumption used in this plan | Affects |
|---|----------|--------------------------------------|---------|
| Q1 | What kind of modeling? | **Tinkercad-style solid modeling**: combine primitives and imported STLs with move/rotate/scale, solids and "holes", groups (= booleans). Not sculpting, not sketch-based parametric CAD. | Everything |
| Q2 | Rust or Go? | **Rust** (reasons in §2). | Everything |
| Q3 | Target platforms? | **macOS (Apple Silicon) first**, Linux and Windows also supported. Web/WASM is a later option. | M0, M9 |
| Q4 | Is a C++ dependency acceptable for robust booleans? | **Yes**: the Manifold library via Rust bindings, behind a trait so it can be swapped. | M5 |
| Q5 | Which file formats besides STL? | STL import/export (ASCII + binary) in MVP. **3MF export** and **OBJ import** later. | M1, M8 |
| Q6 | Anything robot-specific (name "robocad")? | **No** robotics features (joints, kinematics). Focus is 3D-printable parts. | Scope |
| Q7 | License / open source? | **MIT OR Apache-2.0** dual license (compatible with Manifold's Apache-2.0). | M0 |
| Q8 | UI language? | **English** UI, strings kept in one module so translation (e.g. German) is easy later. | M3 |

Your defaults fit well. Just stick to them.
---

## 1. Product vision

**robocad** is a desktop app where a hobbyist or engineer can:

1. Open an STL (drag & drop), look at it, measure it, check if it is printable, fix small problems.
2. Build new parts from primitives (box, cylinder, sphere, cone, torus, wedge, text) by placing, sizing and combining them.
3. Cut, drill and merge shapes: any shape can be a **solid** or a **hole**; grouping shapes unions the solids and subtracts the holes.
4. Export a clean, watertight STL (and later 3MF) ready for a slicer.

### Design principles
- **Easy**: every common action is one click or one key. Numbers can always be typed exactly (mm). No modal dialogs for normal work.
- **Fast**: instant startup, 60 fps viewport, booleans fast enough to feel live.
- **Light**: small binary, low RAM, zero CPU when idle.
- **Non-destructive**: the document is a tree of parametric nodes. Meshes are *results*, recomputed from the tree. Changing a cylinder's diameter after it was subtracted just works.
- **Honest**: show when a mesh is not watertight and why, instead of silently producing bad output.

### Non-goals (for v1)
- Sketch/constraint-based parametric CAD (FreeCAD/Fusion style), B-rep/STEP, fillets/chamfers on arbitrary edges.
- Sculpting, texturing, animation, rendering.
- Slicing / G-code.
- Collaboration, cloud, accounts.

### Performance targets (measured on an Apple M1 / 8 GB, release build)

| Metric | Target |
|---|---|
| Cold start to interactive window | < 0.5 s |
| Idle CPU | ~0 % (repaint only on input/events) |
| RAM, empty scene | < 120 MB |
| Load binary STL, 1 M triangles | < 0.5 s (parse + weld + GPU upload) |
| Viewport, 5 M triangles total | ≥ 60 fps |
| Boolean of two 100 k-triangle meshes | < 150 ms |
| Release binary size (stripped) | < 25 MB |

---

## 2. Technology decision: Rust

Rust is chosen over Go because:

- **Graphics/GUI**: Rust has mature, native, cross-platform GPU and immediate-mode UI crates (`wgpu`, `egui`/`eframe`). Go's options (Fyne, Gio, raw GL bindings via cgo) are weaker for a 3D viewport plus tool panels.
- **Geometry ecosystem**: Rust has `glam`, robust-mesh-boolean bindings (Manifold), and good SIMD/parallelism (`rayon`). Go would need cgo for most of this.
- **Performance & memory**: no GC pauses during interaction, tight control over memory layout for large meshes (zero-copy GPU uploads via `bytemuck`).
- **Single static binary** on all three desktop OSes, and a realistic WASM path later.

Toolchain present on the dev machine: `rustc/cargo 1.98`. Use **edition 2024**, stable toolchain only.

### Dependencies (verify current versions with `cargo search` / crates.io at implementation time; pin in `Cargo.toml`)

| Purpose | Crate | Notes |
|---|---|---|
| Window + UI | `eframe`, `egui` | Use the **wgpu** backend (`eframe` feature `wgpu`), not glow. |
| GPU | `wgpu` (via `egui-wgpu`) | Use the same wgpu version eframe re-exports. Custom 3D pass via egui paint callback. |
| Math | `glam` | `f64` (`DVec3`, `DQuat`, `DMat4`) for document/transforms, `f32` for GPU and mesh storage. |
| Byte casting | `bytemuck` | Zero-copy vertex buffers and binary STL parsing. |
| Large files | (none) | Files are read with `std::fs::read`; memory mapping needs `unsafe` and reading is far cheaper than parsing. |
| Float parsing | `fast-float` (or `fast-float2`) | ASCII STL. |
| Parallelism | `rayon` | STL parse, welding, analysis. |
| Booleans/kernel | Manifold Rust bindings (candidates: `manifold3d`, `manifold-rs`) | **Verify** which crate is maintained. If none is acceptable, write a thin `-sys` crate with `bindgen` against Manifold's C API (`manifoldc`) built via `cmake` crate. Hidden behind `trait GeometryKernel`. |
| IDs | `slotmap` | Stable node IDs in the document. |
| Serialization | `serde`, `serde_json` | Project file manifest. |
| Project container | `zip` | `.rcad` files. |
| File dialogs | `rfd` | Native open/save dialogs. |
| Errors | `thiserror` (libs), `anyhow` (app/cli) | |
| Logging | `log` + `env_logger` | |
| CLI | `clap` (derive) | Headless tool. |
| Fonts → text solids | `ttf-parser` | M8. Bundle one OFL font (e.g. Inter or Noto Sans). |
| Transform gizmo (optional) | `transform-gizmo-egui` | Evaluate in M4; write our own if it does not fit the custom wgpu viewport cleanly. |
| Dev | `criterion`, `proptest`, `approx`, `insta` | Benchmarks, property tests, snapshots. |

**Dependency rule**: every new dependency must be justified in the PR/commit message. Prefer small, focused crates. No async runtime (tokio) — not needed.

---

## 3. Architecture

### 3.1 Cargo workspace layout

```
robocad/
├── Cargo.toml                 # [workspace], shared lints, release profile
├── PLAN.md                    # this file
├── CLAUDE.md                  # conventions for the implementing agent (create in M0)
├── crates/
│   ├── rc-mesh/               # Mesh data structures, STL/3MF/OBJ IO, BVH, analysis, repair. No GUI, no kernel.
│   ├── rc-kernel/             # GeometryKernel trait + Manifold backend, primitive tessellation, text.
│   ├── rc-doc/                # Document model (node tree), commands, undo/redo, evaluation + cache, project files.
│   ├── rc-render/             # wgpu renderer: pipelines, shaders (WGSL), camera, GPU mesh cache. No egui types in public API except the callback adapter.
│   ├── rc-app/                # eframe app: panels, tools, gizmo, input, settings. Binary name `robocad`.
│   └── rc-cli/                # Headless CLI: info, convert, repair, eval project → STL. Binary name `robocad-cli`.
├── assets/                    # icons (SVG→embedded), font, sample models
├── tests/fixtures/            # STL fixtures (small!), golden outputs
└── .github/workflows/ci.yml
```

Dependency direction (strict, no cycles): `rc-kernel` depends on `rc-mesh`; `rc-doc` depends on `rc-kernel` and `rc-mesh`.
`rc-render` depends only on `rc-mesh` (+ glam), never on the document. `rc-app` depends on all. `rc-cli` depends on `rc-mesh`, `rc-kernel`, `rc-doc` (never on render/app).

```
rc-mesh ──► rc-kernel ──► rc-doc ──► rc-app
   │                         ▲          ▲
   └────────► rc-render ─────┼──────────┘
                             └── rc-cli
```

### 3.2 Core data structures

```rust
// rc-mesh
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,   // welded, indexed
    pub triangles: Vec<[u32; 3]>,   // CCW = outward
}
// Derived data computed lazily and cached alongside (not inside) the mesh:
pub struct MeshInfo { aabb: Aabb, volume: f64, area: f64, tri_count: usize, topology: TopologyReport }
pub struct TopologyReport { boundary_edges: usize, non_manifold_edges: usize, degenerate_tris: usize,
                            inconsistent_orientation: bool, components: usize, is_watertight: bool }
pub struct Bvh { /* flat node array, SAH-binned build, ray + AABB queries */ }
```

- Storage is `f32` (STL is f32 anyway). All accumulations (volume, area, centroid) use `f64`.
- No half-edge structure is stored permanently; build a transient edge map (`HashMap<(u32,u32), …>` or sorted edge list) when analysis/repair needs it.
- Meshes are shared as `Arc<Mesh>`; they are immutable once built.

```rust
// rc-doc
pub struct Document {
    pub nodes: SlotMap<NodeId, Node>,
    pub roots: Vec<NodeId>,            // top-level order = scene tree order
    pub settings: DocSettings,         // units (mm), build plate size, grid size
}
pub struct Node {
    pub name: String,
    pub transform: Transform,          // translation DVec3, rotation DQuat, scale DVec3 (non-uniform allowed)
    pub kind: NodeKind,
    pub role: Role,                    // Solid | Hole
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
}
pub enum NodeKind {
    Primitive(Primitive),              // parametric, see below
    Imported { mesh: MeshRef, source_name: String },  // MeshRef = content hash → Arc<Mesh> in a MeshStore
    Group { children: Vec<NodeId>, op: GroupOp },     // GroupOp::Combine (solids ∪, minus holes) | Intersect
    Mirror { child: NodeId, plane: Axis },            // M8
    Array { child: NodeId, kind: ArrayKind },          // M8: linear / circular
}
pub enum Primitive {
    Box { size: DVec3 },
    Cylinder { radius_bottom: f64, radius_top: f64, height: f64, segments: u32 }, // cone = different radii
    Sphere { radius: f64, segments: u32 },
    Torus { major: f64, minor: f64, segments: u32 },
    Wedge { size: DVec3 },
    Text { text: String, size: f64, depth: f64, font: FontRef },  // M8
}
```

**Group semantics (Tinkercad model — simple and powerful):**
`Combine` result = (∪ of all Solid children) − (∪ of all Hole children). `Intersect` = ∩ of Solid children (holes still subtracted afterwards).
A group's own `role` decides how *it* participates in its parent. This gives union, difference and intersection with one concept the user already understands.

**Primitive local frames:** every primitive sits on z = 0 with its base, centered in x/y (so "drop to plate" and sizing are intuitive). Z is up, units are millimetres.

### 3.3 Evaluation pipeline

```
Document (params) ──► Evaluator (worker thread) ──► EvalResult per node: Arc<Mesh> + MeshInfo ──► GPU cache
```

- Each node gets a **content hash** = hash(kind params, role, children hashes) — *transform excluded for leaves*, included when baked into a parent boolean.
- `EvalCache: HashMap<u64, Arc<Mesh>>` with LRU eviction by byte budget (default 512 MB, configurable). Unchanged subtrees are never recomputed.
- Evaluation runs on a dedicated worker thread (`std::thread` + channels; `rayon` inside heavy ops). The UI never blocks. While a node is recomputing, the UI keeps showing the previous result and a small spinner in the tree.
- Requests carry a document **revision number**; stale results are dropped.
- **Rendering does not wait for booleans**: top-level leaf nodes are drawn directly from their local mesh + transform (GPU applies the model matrix). Only groups need kernel evaluation.
- During a **gizmo drag inside a group**: re-evaluate live if the last evaluation of that group took < 30 ms, otherwise show children as ghosts (translucent) and evaluate on release (with 150 ms debounce). Measure and store per-node eval time.

### 3.4 Geometry kernel abstraction

```rust
pub trait GeometryKernel: Send + Sync {
    type Solid: Clone + Send + Sync;             // backend's native solid (e.g. Manifold handle)
    fn from_mesh(&self, m: &Mesh) -> Result<Self::Solid, KernelError>; // fails on non-manifold input
    fn to_mesh(&self, s: &Self::Solid) -> Mesh;
    fn transform(&self, s: &Self::Solid, m: DMat4) -> Self::Solid;
    fn union(&self, parts: &[Self::Solid]) -> Self::Solid;        // batch op, not pairwise
    fn difference(&self, a: &Self::Solid, b: &Self::Solid) -> Self::Solid;
    fn intersection(&self, parts: &[Self::Solid]) -> Self::Solid;
    fn primitive(&self, p: &Primitive) -> Self::Solid;           // may fall back to own tessellation
}
```

- **Backend 1 (default): Manifold.** Robust, fast, guaranteed manifold output. Keep native solids cached inside the evaluator (avoid mesh↔solid round-trips between nested groups).
- Primitive tessellation is our own code in `rc-kernel` (deterministic, testable), with `segments` defaulting to an adaptive value from size (chord error ≤ 0.05 mm, clamped 16..256).
- Imported meshes that fail `from_mesh` (not manifold even after auto-repair) can be **displayed, measured and exported**, but participating in a boolean shows a clear error on the group: *"'bracket.stl' is not watertight (12 open edges). Run Repair or exclude it."*

### 3.5 Rendering (rc-render)

- Integrate with egui through `egui_wgpu::CallbackTrait` (paint callback in the central panel). Renderer owns its pipelines and a depth texture sized to the viewport rect; render into an offscreen color+depth target at native pixel scale with **4× MSAA**, then composite into egui's pass (or render directly if the callback pass supports depth — check the current egui-wgpu API).
- **Flat shading without vertex duplication**: compute the face normal in the fragment shader from screen-space derivatives (`normalize(cross(dpdx(world_pos), dpdy(world_pos)))`). Keeps indexed meshes at their compact size.
- Lighting: simple two-light + hemispheric ambient (or a matcap); readable, cheap.
- **Feature edges**: computed once per mesh on CPU (dihedral angle > 30° or boundary), drawn as a line list on top with depth bias. Toggle in view menu.
- **Holes**: drawn after opaque objects, translucent grey with depth test, no depth write (Tinkercad look).
- **Selection**: tinted color + accent-colored feature edges; hover highlight.
- **Non-manifold diagnostics overlay**: boundary edges in red, non-manifold edges in orange.
- Build plate: configurable size (default 220 × 220 mm), grid with 1 mm/10 mm lines drawn procedurally in a fragment shader (one quad, fades with distance). Axis indicator and a small **view cube** (clickable top/front/right/iso).
- Camera: orbit around pivot, pan, zoom-to-cursor; perspective ↔ orthographic toggle; "frame selection" (F). Pivot = point under cursor on mouse-down (via BVH ray cast), falling back to scene center.
- GPU mesh cache keyed by mesh hash; upload once, free on eviction. One draw call per visible object (instancing for Array nodes later).
- **Reactive repaint**: request repaint only on input, camera animation, or evaluator results. Idle = 0 % CPU.

### 3.6 Picking & snapping

- CPU ray casting against per-mesh BVH (in mesh-local space, ray transformed by the inverse model matrix). No GPU readback.
- Picking returns node, triangle index, hit point, face normal → used for selection, measure tool, "place on face" and the orbit pivot.
- Snapping: translation snaps to grid (default 1 mm, toggle with modifier key), rotation snaps to 15°, scale snaps to 1 mm on the box size.

### 3.7 Undo/redo

- **Snapshot-based** undo with structural sharing: `Document` is cheap to clone because meshes are `Arc`, and nodes are small. Each user command produces a new snapshot pushed to the undo stack (cap: 200 steps or memory budget).
- Continuous edits (gizmo drag, slider drag, typing in a field) are **coalesced** into one undo step (begin/commit transaction API).
- All mutations go through `Document::apply(Command)` so the CLI/tests can drive the same code path.

### 3.8 Project file format (`.rcad`)

- A ZIP container:
  - `document.json` — versioned schema (`"format_version": 1`), nodes, settings, camera.
  - `meshes/<hash>.stl` — imported meshes in binary STL (deduplicated by content hash).
  - `thumbnail.png` — 256×256 viewport render (for OS file previews later).
- Writing is atomic (write temp file, then rename). Autosave to the OS data dir every 2 minutes when dirty; offer recovery on next start.
- Migration functions for each `format_version` bump; never break old files.

### 3.9 STL handling details

- **Detection**: binary vs ASCII must not trust the `solid` header (many binary files start with "solid"). Rule: if file size == 84 + 50 × tri_count, it is binary; otherwise try ASCII.
- **Binary parsing**: read the file into memory, parse records in parallel chunks with rayon, ignore the stored normals (recompute; many exporters write garbage normals), ignore attribute bytes (optionally read VisCAM/SolidView color bits later).
- **ASCII parsing**: hand-written tokenizer over bytes, `fast-float`, tolerant of whitespace/case variations, multiple `solid` blocks.
- **Welding**: exact weld using a hash map on the bit patterns of the f32 triple (fast, lossless). Optional epsilon weld (grid-quantized, default 1e-5 × bbox diagonal) used by Repair.
- **Export**: binary STL by default (ASCII optional), normals computed from geometry, header `"robocad <version>"`. Units are mm (STL has no units; offer "imported file is in inches/metres" scale option on import).
- Must handle: empty files, truncated files (clear error), NaN/inf vertices (drop triangle, report), 0-area triangles, > 4 GB files (u32 index limit → clear error above 2^32 vertices).

### 3.10 Mesh analysis & repair (rc-mesh)

Analysis (`TopologyReport`, shown in an "Inspect" panel): triangle/vertex count, bbox, volume, surface area, components, boundary edges, non-manifold edges, degenerate triangles, flipped/inconsistent orientation, watertight yes/no, optional overhang visualization (faces whose normal is > 45° from +Z facing down, colored).

Repair pipeline (each step individually testable, run in order, reporting what it changed):
1. Drop NaN/inf and degenerate (zero-area) triangles.
2. Epsilon weld vertices.
3. Remove duplicate triangles.
4. Make orientation consistent per connected component (BFS over edges), then orient outward (signed volume > 0).
5. Fill simple holes (boundary loops) with ear-clipping fan triangulation for planar-ish loops; skip complicated ones and report.
6. Optional: remove tiny disconnected components (< N triangles or < X mm³).
7. Final check; if still not manifold, report remaining issues.

---

## 4. User interface

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [Open] [Import] [Export] │ [Box][Cyl][Sphere][Cone][Torus][Wedge][Text] │ [Group][Ungroup][Solid/Hole] [Undo][Redo] │
├───────────────┬──────────────────────────────────────────────┬───────────────┤
│ Scene         │                                              │ Properties    │
│  ▾ Bracket    │                                              │ Name  [    ]  │
│    ▪ Base     │            3D viewport                       │ Role  Solid ▾ │
│    ▫ Hole 1   │        (build plate, grid, gizmo)            │ Pos  x y z    │
│  ▪ part.stl ⚠ │                                    [viewcube]│ Rot  x y z    │
│               │                                              │ Size x y z    │
│               │                                              │ Params…       │
│               │                                              │ Color ■       │
├───────────────┴──────────────────────────────────────────────┴───────────────┤
│ 2 selected · 12,480 tris · 42.0 × 30.0 × 12.5 mm · watertight ✓ · 3 ms eval │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Scene panel**: tree with drag-to-reparent, rename (double-click/F2), visibility eye, lock, solid/hole icon, warning icon for non-manifold.
- **Properties panel**: exact numeric fields for everything (position, rotation in degrees, size in mm, primitive parameters). Fields accept simple math (`20/2+3`) — tiny expression parser.
- **"Size" editing** scales the object so its bounding box matches the typed size (more intuitive than scale factors); lock-aspect toggle.
- **Viewport tools**: Select (default), Move/Rotate/Scale gizmo, Measure (point-to-point, point-to-face distance, angle between faces), Place-on-face (click a face → object rotates so that face sits on the plate).
- **Align** tool for multi-selection (min/center/max per axis), **Drop to plate**, **Center on plate**.
- **Drag & drop** STL/.rcad files onto the window.
- Light and dark theme (follow OS), UI scale setting.
- First-run hint overlay with the 8 most important shortcuts; `?` shows the full list.

### Default input mapping

| Action | Mouse / trackpad | Key |
|---|---|---|
| Orbit | Right drag · two-finger drag (trackpad) | |
| Pan | Middle drag · Shift + right drag · Shift + two-finger drag | |
| Zoom (to cursor) | Wheel · pinch | |
| Select / add to selection | Click / Shift+click, box-select by left drag on empty space | Ctrl/Cmd+A |
| Move / Rotate / Scale gizmo | | W / E / R |
| Frame selection / all | | F / Shift+F |
| Standard views | View cube | 1 front, 3 right, 7 top, 5 persp/ortho, 0 iso |
| Duplicate | | Ctrl/Cmd+D |
| Delete | | Del / Backspace |
| Group / Ungroup | | Ctrl/Cmd+G / Ctrl/Cmd+Shift+G |
| Toggle Solid/Hole | | H |
| Drop to plate | | D |
| Undo / Redo | | Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z |
| Nudge selection | | Arrow keys (1 mm; Shift = 10 mm; PgUp/PgDn = Z) |
| Snap off while dragging | hold Alt | |

On macOS use Cmd instead of Ctrl everywhere (egui `Modifiers::command`). Mac trackpads are a first-class input device — test them.

---

## 5. Milestones

Each milestone ends with: all tests green, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean, a short entry in `CHANGELOG.md`, and a commit. Do not start a milestone before the previous one meets its acceptance criteria.

### M0 — Project skeleton & CI — ✅ DONE
- Create the workspace and all six crates (empty but compiling), shared `[workspace.lints]` (`clippy::pedantic` as warn with a few allowed, `unsafe_code = "deny"` everywhere except an FFI crate if one becomes needed).
- Release profile: `lto = "fat"`, `codegen-units = 1`, `strip = true`. (Unwinding is kept instead of `panic = "abort"` so a panic inside a worker thread can be caught and reported instead of killing the app.) Dev profile: `opt-level = 1` for our crates, `opt-level = 3` for dependencies (geometry is unusable in debug otherwise).
- `CLAUDE.md` with build/test commands and the conventions in §7.
- GitHub Actions CI: fmt, clippy, test on macOS, Linux, Windows; cache with `Swatinem/rust-cache`.
- License files, update `README.md` (build instructions).
- **Accept**: `cargo build --workspace` and `cargo test --workspace` pass locally and in CI; `cargo run -p rc-app` opens an empty window titled "robocad".

### M1 — Mesh core & STL I/O (headless) — ✅ DONE
- `rc-mesh`: `Mesh`, `Aabb`, binary + ASCII STL read/write (§3.9), exact welding, `MeshInfo` (volume, area, bbox), `TopologyReport`, BVH with ray cast.
- `rc-cli`: `robocad-cli info <file.stl>` (prints counts, bbox, volume, watertight status) and `robocad-cli convert <in> <out> [--ascii]`.
- Fixtures (each < 100 KB): cube (12 tris), ASCII cube, binary file whose header starts with "solid", cube with a missing triangle, two overlapping cubes, a non-manifold "bowtie", flipped-normal cube, truncated file, empty file.
- Generate a 1 M-triangle test mesh in the benchmark (don't commit large files).
- **Accept**: unit tests for every fixture; round-trip read→write→read is identical; cube volume = 1000 for a 10 mm cube within 1e-9; criterion benchmark shows 1 M-triangle binary parse+weld < 300 ms on M1.

### M2 — Viewer
- `rc-render`: camera (orbit/pan/zoom-to-cursor, persp/ortho), mesh pipeline with derivative flat shading, MSAA, grid/build plate shader, feature edges, GPU mesh cache.
- `rc-app`: eframe app with the three-panel layout (panels can be empty stubs), viewport callback, open STL via menu, file dialog and drag & drop, frame-all on load, status bar with tri count & bbox.
- Reactive repaint (verify idle CPU ≈ 0 % with Activity Monitor).
- **Accept**: open a 1 M-triangle STL in < 0.5 s and orbit at 60 fps; resizing and HiDPI work; dark/light theme; trackpad orbit/pan/pinch work on macOS.

### M3 — Document, selection, properties, undo
- `rc-doc`: `Document`, `Node`, `Command` enum, `apply`, snapshot undo/redo with transactions, `MeshStore`.
- Imported STLs become `Imported` nodes. Scene tree panel (select, rename, visibility, lock, delete, reorder). Picking via BVH; hover + selection highlight; box select.
- Properties panel with numeric transform fields (incl. expression parser), color picker.
- **Accept**: import 3 STLs, rename, move numerically, delete, undo/redo every step correctly (unit tests drive `Command`s headlessly); selection works in viewport and tree in both directions.

### M4 — Primitives & direct manipulation
- Primitive tessellation for Box, Cylinder/Cone, Sphere (UV with poles welded; or icosphere — pick one that is watertight and has an adaptive segment count), Torus, Wedge. All watertight, outward-oriented (tests!).
- Toolbar buttons and shortcut to add primitives; new objects appear at the plate center (or under the cursor when dragged from the toolbar onto the plate).
- Transform gizmo (move/rotate/scale, local/world toggle) with snapping; "Size" editing with bbox semantics; Drop to plate; Align; Duplicate; Nudge.
- **Accept**: every primitive passes `is_watertight` and volume tests (e.g. sphere volume within 1 % of 4/3·π·r³ at default segments); a user can build a "box with a hole marker" layout purely with mouse + keyboard; gizmo drags are a single undo step.

### M5 — Booleans (Solid/Hole/Group) & export
- `rc-kernel`: `GeometryKernel` trait and the Manifold backend (§3.4). Spike first: one day max to pick the binding crate; document the decision in `docs/adr/0001-boolean-kernel.md`.
- Evaluator on worker thread with content-hash cache, revisions, per-node eval timing (§3.3).
- Solid/Hole role toggle, Group/Ungroup, Intersect group, hole rendering.
- Export STL: whole scene, or selection; exported mesh = evaluated, transformed, merged result; warn if result is not watertight.
- **Accept**: property tests (proptest, random boxes/cylinders with random transforms): `vol(A∪B) + vol(A∩B) ≈ vol(A) + vol(B)` (rel. 1e-6), `vol(A−B) = vol(A) − vol(A∩B)`, all results watertight; boolean of two 100 k-tri meshes < 150 ms; UI stays at 60 fps while a slow boolean computes; exported STLs open cleanly in PrusaSlicer/Bambu Studio/Cura without repair warnings (manual check, note in CHANGELOG).

### M6 — Inspect & Repair
- Inspect panel (TopologyReport, volume, area), diagnostic overlay (boundary/non-manifold edges), overhang shading toggle.
- Repair pipeline (§3.10) as a command (undoable), with a report dialog ("Removed 14 degenerate triangles, filled 2 holes, flipped 1 component").
- Auto-run steps 1–4 silently on import (they are lossless/safe); offer full repair when issues remain.
- Import unit option (mm / inch / m) and "place on plate after import" setting.
- `robocad-cli repair <in> <out>`.
- **Accept**: all broken fixtures are repaired to watertight where theoretically possible; repaired results are accepted by the kernel for booleans; repair of a 1 M-triangle mesh < 2 s.

### M7 — Projects & settings
- `.rcad` save/load (§3.8), Save/Save As/Open/Recent files, dirty indicator in title, "unsaved changes" prompt on quit, autosave + crash recovery.
- Settings stored in OS config dir (`directories` crate or eframe storage): theme, UI scale, build plate size, grid, default segment quality, mouse mapping preset (default / "Blender-like" / "Fusion-like").
- `robocad-cli export <project.rcad> <out.stl>` evaluates headlessly.
- **Accept**: save → load round trip equals the original document (test); loading a v1 file in later versions works (migration test harness in place); killing the app mid-work and restarting offers recovery.

### M8 — Power features
- Measure tool (point-point, point-face, face-face angle, edge length), results shown as overlay labels.
- Place-on-face.
- Mirror node, linear and circular array nodes (parametric).
- Text primitive: `ttf-parser` outlines → flatten curves → polygons with holes → extrude (use the kernel's 2D cross-section extrusion if Manifold binding exposes it, else own ear-clipping triangulation for caps + side walls). Bundled font + system font selection later.
- 3MF export (single mesh per object, units mm, colors). OBJ import.
- **Accept**: text "robocad" exports as one watertight solid; arrays of 10×10 cylinders subtracted from a plate evaluate in < 500 ms; 3MF opens in PrusaSlicer and Bambu Studio.

### M9 — Polish, performance, packaging
- Profiling pass (`cargo flamegraph`, `tracy` optional) against §1 targets; fix regressions. Add a `perf` CI job that runs criterion benchmarks and fails on > 20 % regression (optional, can be manual).
- Keyboard shortcut help overlay, first-run hints, tooltips on every button, consistent icons (embedded SVG, e.g. Lucide/Phosphor, MIT-licensed).
- Error handling review: no panics on user input; every failure is a toast or panel message.
- Packaging: macOS `.app` bundle + DMG (`cargo-bundle` or a script; universal binary optional; codesigning/notarization documented but optional), Windows `.exe` (+ optional MSI via `cargo-wix`), Linux AppImage/tarball. File association for `.rcad` and `.stl` (optional).
- Release workflow in GitHub Actions that builds artifacts on tags.
- **Accept**: all §1 targets met and recorded in `docs/performance.md`; a new user can follow README to download, open an STL, add a hole, and export in under 5 minutes.

### Later (not scheduled)
- Sketch + extrude/revolve (2D polygon editor on a plane), simple chamfers on boxes/cylinders as primitive parameters, screw threads generator, hull/Minkowski, decimation (quadric simplification), plane cut/split for printing, WASM/web build, scripting (Rhai or Lua) for parametric parts, STEP import (via OpenCascade — heavy, only if needed).

---

## 6. Testing strategy

| Level | What | Where |
|---|---|---|
| Unit | Parsers, welding, BVH, analysis, repair steps, tessellation, expression parser, commands/undo | each crate, `#[cfg(test)]` |
| Property | Boolean volume identities, watertightness of all generated primitives, STL round-trips, repair idempotence | `proptest` in `rc-kernel`, `rc-mesh` |
| Golden | Evaluated sample projects → hash or volume/bbox snapshot | `insta` snapshots in `rc-doc` |
| CLI | `robocad-cli` commands on fixtures | `tests/` integration tests (`assert_cmd`) |
| Benchmarks | STL parse, weld, BVH build, boolean, repair | `criterion` in `benches/` |
| Manual | UI checklist per milestone (in `docs/manual-test.md`): trackpad, HiDPI, themes, slicer import | before tagging |

Keep the GUI layer thin: logic lives in `rc-doc`/`rc-mesh`/`rc-kernel`, which are fully testable headlessly. The app only maps input → `Command`s and draws state.

---

## 7. Conventions for the implementing agent

- **Work milestone by milestone.** At the start of each, re-read this plan's section for it, then list concrete tasks. At the end, run the full check (`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`) and commit.
- **Verify crate APIs and versions** before relying on them (crates.io, docs.rs); this plan names crates but APIs (especially `egui-wgpu` callbacks, `wgpu`) change between versions. Use the wgpu version that matches the chosen eframe.
- **Coordinate system**: right-handed, **Z up**, units **mm**, CCW triangles face outward. Write it in `CLAUDE.md` and never deviate.
- **Precision**: `f64` for document values and transforms; `f32` only for stored mesh vertices and GPU data. Accumulate sums in `f64`.
- **No `unwrap()`/`expect()`** on anything derived from files or user input. Library crates return typed errors (`thiserror`).
- **No blocking work on the UI thread** above ~5 ms: file loading, evaluation, repair and export run on worker threads and report progress.
- Keep functions small, public APIs documented (`///`), and modules focused. Prefer plain data + functions over trait hierarchies; the kernel trait is the main deliberate abstraction.
- Record significant decisions as short ADRs in `docs/adr/NNNN-title.md` (kernel choice, gizmo choice, file format).
- Keep test fixtures tiny; generate large meshes in code.
- When a target in §1 cannot be met, measure, document why in `docs/performance.md`, and continue — do not silently drop it.

---

## 8. Risks & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Manifold Rust bindings unmaintained / hard to build (needs C++ toolchain + CMake) | M5 blocked | Time-boxed spike; fallback is own `-sys` crate via `bindgen` on the C API; `GeometryKernel` trait allows a pure-Rust backend later (e.g. `csgrs`, slower). Document required toolchain in README. |
| Imported STLs are often broken → booleans fail | Bad UX | Auto-repair safe steps on import, clear per-node warnings, full Repair command, never crash. |
| egui/wgpu API churn | Rework | Pin versions; keep the renderer behind our own small API; isolate the egui callback adapter in one file. |
| Slow booleans on large meshes make interaction laggy | Feels slow | Worker thread, caching by content hash, ghost preview during drags, native-solid caching between nested groups. |
| Huge files (tens of millions of triangles) exhaust memory | Crash | Report size before full load, refuse > configured limit with a clear message, compact indexed storage (~24 B/triangle). |
| Precision issues from f32 meshes far from origin | Artifacts | Keep geometry near origin in local space; transforms in f64; kernel works in f64. |
| Scope creep toward full CAD | Never ships | Non-goals in §1 are binding for v1. |

---

## 9. Definition of done for v1.0

- A user can: open/drag-drop an STL, inspect and repair it, add primitives and text, position them precisely, combine them with holes/groups, mirror and array them, measure, save a project, and export a watertight STL or 3MF that slicers accept without warnings.
- All §1 performance targets met (or documented exceptions).
- CI green on macOS, Linux, Windows; release artifacts for all three.
- README with screenshots, shortcut list, and build instructions.
