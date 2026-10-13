# robocad — notes for coding agents

Read `PLAN.md` first. It is the spec; milestones are worked in order and marked done there.

## Commands
- Build: `cargo build --workspace`
- Run app: `cargo run -p rc-app` (binary `robocad`)
- Run CLI: `cargo run -p rc-cli -- <args>` (binary `robocad-cli`)
- Full check (must pass before every commit):
  `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- Benchmarks: `cargo bench -p rc-mesh` (and other crates with `benches/`)

## Local build note (this Mac)
The Command Line Tools install has a stale C++ header dir. Export this before building, and run
`cargo clean -p manifold-csg-sys` once if a failed C++ build was cached:
`export CXXFLAGS="-isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"`

## Conventions
- Coordinate system: right-handed, **Z up**, units **millimetres**, CCW triangles face outward.
- `f64` for document values and transforms; `f32` only for stored mesh vertices and GPU data. Accumulate sums in `f64`.
- No `unwrap()`/`expect()` on data from files or user input. Library crates use `thiserror`; binaries use `anyhow`.
- No blocking work (> ~5 ms) on the UI thread: loading, evaluation, repair and export run on worker threads.
- Crate dependency direction: `rc-mesh` ← `rc-kernel` ← `rc-doc` ← `rc-app`; `rc-render` depends only on `rc-mesh`; `rc-cli` never depends on render/app.
- Logic belongs in the headless crates (`rc-mesh`, `rc-kernel`, `rc-doc`) so it is testable; the app maps input to `Command`s and draws state.
- Keep test fixtures tiny (< 100 KB); generate large meshes in code.
- Record significant decisions as ADRs in `docs/adr/NNNN-title.md`.
- Every new dependency must be justified in the commit message.
- egui/eframe 0.36: `eframe::App` uses `fn ui(&mut self, ui: &mut egui::Ui, frame)`; panels are `egui::Panel::left(id)` etc. and take `&mut Ui`.
