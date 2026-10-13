# ADR 0001: Boolean kernel — Manifold via `manifold-csg`

Date: 2026-10-13 · Status: accepted

## Context
robocad needs robust, fast mesh booleans (union, difference, intersection) that always return
watertight results, for groups of solids and holes.

## Options
1. **Manifold** (C++, Apache-2.0) through Rust bindings.
   - `manifold-csg` 0.4 (+ `manifold-csg-sys` 3.5): safe API, f64 mesh I/O, batch booleans,
     transforms, cross sections / extrusion, hull, Minkowski; pinned to an upstream commit.
   - `manifold-rs` 0.7: thinner wrapper, fewer operations, Bevy-oriented extras.
2. Pure Rust (`csgrs`, BSP based): no C++ toolchain, but slow and numerically fragile.
3. Own implementation: out of scope.

## Decision
Use **`manifold-csg` 0.4** behind robocad's own `GeometryKernel` trait in `rc-kernel`.

Spike results (Apple Silicon, release): union + difference of two ~100k-triangle spheres in
~35 ms; mesh import of 100k triangles ~19 ms; open meshes are rejected with `NotManifold`.

## Consequences
- Building needs CMake, a C++17 compiler and `git` + network on the first build: the sys crate
  clones Manifold at a pinned commit and CMake fetches TBB and Clipper2. The first build takes
  ~2–3 minutes; later builds are cached in `target/`.
- macOS with a broken Command Line Tools install (stale `usr/include/c++/v1` without `cstdint`)
  fails to compile C++. Fix: reinstall the Command Line Tools, or build with
  `CXXFLAGS="-isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"` (see README).
- Non-manifold input meshes cannot take part in booleans; the UI reports this per object and
  the Repair command (M6) fixes most cases.
- The trait keeps a pure-Rust backend possible later (e.g. for a WASM build).
