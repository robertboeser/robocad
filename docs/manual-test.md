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
