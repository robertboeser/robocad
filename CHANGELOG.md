# Changelog

## Unreleased

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
