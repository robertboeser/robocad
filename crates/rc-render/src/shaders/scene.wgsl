// robocad scene shader. Z up, reverse-Z depth, output in linear space to an sRGB target.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,      // xyz eye position
    camera_fwd: vec4<f32>,      // xyz forward, w = 1 for orthographic
    light_key: vec4<f32>,       // xyz direction towards the light
    light_fill: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    plate: vec4<f32>,           // x, y size of build plate, z grid minor, w grid major
    plate_color: vec4<f32>,
    grid_color: vec4<f32>,
    edge_params: vec4<f32>,     // x = depth offset factor
    viewport: vec4<f32>,        // x, y pixel size, z outline width in pixels, w overhangs on
    selection_color: vec4<f32>,
    hover_color: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    edge_color: vec4<f32>,
    // x = selected, y = hovered, z = style (0 solid, 1 hole, 2 ghost), w = unused
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> obj: Object;

// ---------------------------------------------------------------- background

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_background(@builtin(vertex_index) i: u32) -> BgOut {
    // Full-screen triangle.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: BgOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    o.t = uv.y * 0.5;
    return o;
}

@fragment
fn fs_background(in: BgOut) -> @location(0) vec4<f32> {
    return vec4<f32>(mix(g.bg_bottom.rgb, g.bg_top.rgb, clamp(in.t, 0.0, 1.0)), 1.0);
}

// ---------------------------------------------------------------- meshes

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_mesh(@location(0) position: vec3<f32>) -> MeshOut {
    let world = obj.model * vec4<f32>(position, 1.0);
    var o: MeshOut;
    o.world = world.xyz;
    o.pos = g.view_proj * world;
    return o;
}

fn to_camera(world: vec3<f32>) -> vec3<f32> {
    if g.camera_fwd.w > 0.5 {
        return -g.camera_fwd.xyz;
    }
    return normalize(g.camera_pos.xyz - world);
}

fn shade(world: vec3<f32>, front: bool) -> vec3<f32> {
    // Flat face normal from screen-space derivatives (no per-vertex normals needed).
    var n = normalize(cross(dpdx(world), dpdy(world)));
    let v = to_camera(world);
    if dot(n, v) < 0.0 {
        n = -n;
    }
    var base = obj.color.rgb;
    if !front {
        // Back faces are visible only for open or inside-out meshes: show them reddish.
        base = mix(base, vec3<f32>(0.85, 0.12, 0.10), 0.7);
    }
    // Overhangs: front faces pointing down more than 45° that do not rest on the plate.
    if front && g.viewport.w > 0.5 && n.z < -0.7071 && world.z > 0.05 {
        base = mix(base, vec3<f32>(0.95, 0.18, 0.12), 0.75);
    }
    if obj.flags.y > 0.5 {
        base = mix(base, vec3<f32>(1.0), 0.18);
    }

    let sky = vec3<f32>(0.30, 0.32, 0.36);
    let ground = vec3<f32>(0.12, 0.115, 0.11);
    let ambient = mix(ground, sky, n.z * 0.5 + 0.5);
    let key = max(dot(n, g.light_key.xyz), 0.0) * 0.85;
    let fill = max(dot(n, g.light_fill.xyz), 0.0) * 0.22;
    let h = normalize(g.light_key.xyz + v);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.12;
    var color = base * (ambient + key + fill) + vec3<f32>(spec);
    if obj.flags.x > 0.5 {
        color = mix(color, vec3<f32>(1.0), 0.06);
    }
    return color;
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(shade(in.world, front), 1.0);
}

@fragment
fn fs_mesh_transparent(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = shade(in.world, true);
    return vec4<f32>(c, obj.color.a);
}

// ---------------------------------------------------------------- edges

@vertex
fn vs_edge(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    let world = (obj.model * vec4<f32>(position, 1.0)).xyz;
    // Pull edges slightly towards the camera so they win the depth test against their faces.
    var toward: vec3<f32>;
    var dist: f32;
    if g.camera_fwd.w > 0.5 {
        toward = -g.camera_fwd.xyz;
        dist = g.camera_pos.w;
    } else {
        toward = normalize(g.camera_pos.xyz - world);
        dist = length(g.camera_pos.xyz - world);
    }
    let shifted = world + toward * dist * g.edge_params.x;
    return g.view_proj * vec4<f32>(shifted, 1.0);
}

@fragment
fn fs_edge() -> @location(0) vec4<f32> {
    return obj.edge_color;
}

// ---------------------------------------------------------------- grid / build plate

struct GridOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) xy: vec2<f32>,
};

const GRID_EXTENT: f32 = 5000.0;

@vertex
fn vs_grid(@builtin(vertex_index) i: u32) -> GridOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let xy = corners[i] * GRID_EXTENT;
    var o: GridOut;
    o.xy = xy;
    o.pos = g.view_proj * vec4<f32>(xy, 0.0, 1.0);
    return o;
}

fn grid_line(coord: vec2<f32>, spacing: f32) -> f32 {
    let c = coord / spacing;
    let d = fwidth(c);
    let l = abs(fract(c - 0.5) - 0.5) / max(d, vec2<f32>(1e-6));
    let line = 1.0 - min(min(l.x, l.y), 1.0);
    // Fade out lines that would be denser than ~3 pixels to avoid moiré.
    let density = max(d.x, d.y);
    return line * clamp(1.0 - (density - 0.15) * 4.0, 0.0, 1.0);
}

@fragment
fn fs_grid(in: GridOut) -> @location(0) vec4<f32> {
    let half = g.plate.xy * 0.5;
    let inside = all(abs(in.xy) <= half);
    let minor = grid_line(in.xy, g.plate.z);
    let major = grid_line(in.xy, g.plate.w);
    var color = vec4<f32>(0.0);
    if inside {
        color = g.plate_color;
    }
    let line_alpha = max(minor * 0.35, major * 0.8) * select(0.35, 1.0, inside);
    color = mix(color, vec4<f32>(g.grid_color.rgb, 1.0), line_alpha * g.grid_color.a);
    color.a = max(color.a, line_alpha * g.grid_color.a);
    // Axis lines through the origin: X red, Y green.
    let w = fwidth(in.xy) * 1.5;
    if abs(in.xy.y) < w.y && in.xy.x >= 0.0 {
        color = vec4<f32>(0.85, 0.22, 0.2, 0.95);
    }
    if abs(in.xy.x) < w.x && in.xy.y >= 0.0 {
        color = vec4<f32>(0.3, 0.7, 0.25, 0.95);
    }
    // Plate border.
    let edge = abs(abs(in.xy) - half);
    if (edge.x < w.x * 1.2 && abs(in.xy.y) <= half.y) || (edge.y < w.y * 1.2 && abs(in.xy.x) <= half.x) {
        color = vec4<f32>(g.grid_color.rgb, 0.9);
    }
    // Fade with distance from the camera to hide the far horizon.
    let dist = length(g.camera_pos.xyz - vec3<f32>(in.xy, 0.0));
    let fade = clamp(1.0 - dist / (g.camera_pos.w * 6.0 + 600.0), 0.0, 1.0);
    return vec4<f32>(color.rgb, color.a * fade);
}

// ---------------------------------------------------------------- selection outline

@group(2) @binding(0) var mask_tex: texture_2d<f32>;

@fragment
fn fs_mask(in: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(obj.flags.x, obj.flags.y, 0.0, 1.0);
}

fn mask_at(p: vec2<i32>) -> vec2<f32> {
    let dims = vec2<i32>(textureDimensions(mask_tex));
    return textureLoad(mask_tex, clamp(p, vec2<i32>(0), dims - 1), 0).rg;
}

@fragment
fn fs_outline(in: BgOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let centre = mask_at(p);
    if centre.r > 0.5 {
        discard;
    }
    let r = max(g.viewport.z, 1.0);
    var sel = 0.0;
    var hov = 0.0;
    // Two rings of 12 samples give a smooth, cheap dilation.
    for (var i = 0; i < 12; i = i + 1) {
        let a = f32(i) * 0.5235988;
        let d = vec2<f32>(cos(a), sin(a));
        let s1 = mask_at(p + vec2<i32>(round(d * r)));
        let s2 = mask_at(p + vec2<i32>(round(d * r * 0.5)));
        sel = max(sel, max(s1.r, s2.r));
        hov = max(hov, max(s1.g, s2.g));
    }
    if sel > 0.5 {
        return g.selection_color;
    }
    if hov > 0.5 && centre.g < 0.5 {
        return g.hover_color;
    }
    discard;
}
