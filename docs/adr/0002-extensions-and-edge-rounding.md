# ADR 0002: Optional extensions, and rounding as the first one

Date: 2026-10-14 · Status: accepted

## Context
New features keep adding buttons and menu entries. The owner wants an extension system whose
features are **off by default** to keep the UI clean, and as the first extension a way to
round all outward edges of the selected object.

## Decision: extensions
- Extensions are **compiled in** and listed in a static registry (`rc-app/src/extensions`).
  Each implements the `Extension` trait: id, name, description, and optional UI hooks
  (toolbar, selection menu used by the Edit and scene context menus, properties section).
- The enabled set is a list of ids in the user settings (`Settings::extensions`), empty by
  default, toggled in Preferences → Extensions. Unknown ids are ignored.
- Extensions only add **UI**. Their geometry lives in the document and the headless crates
  (`rc-doc::Modifier`, `rc-kernel`), so projects that use an extension evaluate, render and
  export identically whether or not it is enabled, including in `robocad-cli`. Properties of
  nodes that already use an extension stay visible while it is disabled so they remain
  editable.
- Not chosen: runtime-loaded plugins (dynamic libraries, WASM or a scripting language). They
  need a stable ABI or sandbox and much more surface for what is currently one feature. The
  trait keeps the door open: a scripting host could later be one `Extension`.

## Decision: rounding edges
- Rounding is a **non-destructive group modifier** `Modifier::RoundEdges { radius }`, like
  patterns: "Round" wraps each selected object in its own group, so the object stays editable
  and Ungroup removes the rounding. The group takes over the object's solid/hole role, so a
  rounded hole cuts a pocket with rounded edges. Order inside a group: combine → modifier →
  pattern.
- Geometry: **morphological opening** by a ball, `(S ⊖ B) ⊕ B` (`rc-kernel/src/round.rs`).
  It rounds exactly the convex edges and corners with the radius, keeps flat faces in place
  (the same polyhedral ball is used for both steps) and never adds material, so concave edges
  stay sharp. Parts thinner than 2 × radius disappear; the evaluator reports that as an error
  instead of producing an empty object.
- Convex solids (most single primitives) take a fast path: face planes are moved inward by the
  ball's support distance and intersected via polar duality (one hull), then one convex
  Minkowski hull. Other solids use Manifold's Minkowski difference and sum (per-triangle hulls
  plus a batch union).
- The ball resolution adapts to the radius (chord tolerance 0.025 mm, 8–32 segments) and is
  reduced for large non-convex inputs to bound the cost.
- `format_version` stays 1: the field is optional and omitted when unused, so existing files are
  unchanged and still load. Older robocad versions ignore the field (they show the object
  unrounded).

## Consequences
- Release timings (`cargo run --release -p rc-kernel --example round_timing`): box < 1 ms,
  cylinder ~3 ms, 2k-triangle sphere ~25 ms, plate with a hole 0.4–0.7 s, 3.6k-triangle torus
  3–4.5 s. Slow cases run on the evaluation worker; the UI stays responsive and shows
  "Combining…".
- Scaling a rounded group non-uniformly stretches its fillets; resize the object inside instead.
- Rounding *selected* edges only (true fillets/chamfers on chosen edges) is still out of scope.
