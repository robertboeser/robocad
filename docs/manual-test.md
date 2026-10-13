# Manual test checklist

Run before tagging a release. Use a release build: `cargo run --release -p rc-app`.

## Viewer (M2)
- [ ] Drag an STL file onto the window: it loads, the camera frames it, the status bar shows the load time.
- [ ] File → Open (⌘O / Ctrl+O) opens a native dialog; multi-select works.
- [ ] Right-drag orbits; Shift+right-drag and middle-drag pan; mouse wheel zooms towards the cursor.
- [ ] macOS trackpad: two-finger drag orbits, Shift+two-finger drag pans, pinch zooms.
- [ ] Alt+left-drag orbits, Alt+Shift+left-drag pans (for one-button mice).
- [ ] Keys 1/3/7/0 switch to front/right/top/iso, 5 toggles perspective/orthographic, F frames all.
- [ ] Clicking the axis widget's X/Y/Z (and the hollow negative circles) switches views.
- [ ] Clicking an object selects it (orange rim + edges); clicking empty space deselects.
- [ ] Open, inside-out or non-manifold meshes show red/orange edges and reddish back faces.
- [ ] Resize the window and move it between a Retina and a non-Retina display: image stays sharp.
- [ ] Switch the OS between light and dark mode: UI and viewport follow.
- [ ] Idle CPU in Activity Monitor is ~0 %.

## Document & editing (M3)
- [ ] Click selects, Shift/⌘-click toggles, drag on empty space box-selects, Esc clears.
- [ ] Delete, ⌘D, ⌘G, ⇧⌘G, H, ⌘A work; ⌘Z / ⇧⌘Z undo and redo every step.
- [ ] Dragging a number field changes the value live and undoes in one step.
- [ ] Typing `20/2+3` into a field gives 13; `2,5` gives 2.5.
- [ ] Scene tree: double-click renames, eye hides, lock prevents moving and selecting in the viewport.
- [ ] Drag a tree row onto a group (moves inside) and between rows (reorders).
- [ ] Window title shows • after changes.

## Primitives & manipulation (M4)
- [ ] Each toolbar shape adds a new object at a free spot; dragging a shape onto the plate places it under the cursor.
- [ ] W/E/R switch tools; arrows, planes, rings and scale handles highlight on hover and drag smoothly.
- [ ] Snapping: 1 mm moves, 15° rotations, 1 mm sizes; holding Alt drags freely.
- [ ] Esc during a drag restores the original placement.
- [ ] Dragging an object (not the gizmo) slides it on the plate.
- [ ] Arrow keys nudge relative to the view; Shift = 10 mm; PgUp/PgDn change height.
- [ ] D drops to the plate; Align buttons line up a multi-selection.

## Booleans & export (M5)
- [ ] Box + cylinder set to hole, grouped (⌘G): the hole cuts through; the status bar shows "combined in … ms".
- [ ] Moving the group is instant (no recomputation); moving a part inside it updates the result.
- [ ] Selecting a part in the tree shows it as a ghost inside the group.
- [ ] A group containing a non-watertight STL shows ⚠ with an explanation.
- [ ] File → Export STL… writes a file that PrusaSlicer / Bambu Studio / Cura open without repair warnings.

## Inspect & repair (M6)
- [ ] Importing a broken STL shows ⚠, the issues and "Fixed automatically" notes in Properties.
- [ ] "Repair mesh" fixes it in the background, shows a report, and ⌘Z restores the original.
- [ ] View → Show overhangs tints steep undersides red.
- [ ] File → Import options: inch units scale a 1-unit cube to 25.4 mm; place-on-plate centres imports.
