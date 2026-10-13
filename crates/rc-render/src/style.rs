//! Colours and display options for the viewport.

/// Converts an sRGB colour component (0..=1) to linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn rgb(hex: u32, alpha: f32) -> [f32; 4] {
    let f = |shift: u32| srgb_to_linear(((hex >> shift) & 0xff) as f32 / 255.0);
    [f(16), f(8), f(0), alpha]
}

/// Visual settings for a frame. Colours are linear RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneStyle {
    pub bg_top: [f32; 4],
    pub bg_bottom: [f32; 4],
    pub plate_color: [f32; 4],
    pub grid_color: [f32; 4],
    pub edge_color: [f32; 4],
    pub selection_color: [f32; 4],
    pub boundary_edge_color: [f32; 4],
    pub non_manifold_edge_color: [f32; 4],
    /// Build plate size in mm (x, y), centred on the origin.
    pub plate_size: [f32; 2],
    pub grid_minor: f32,
    pub grid_major: f32,
    pub show_grid: bool,
    pub show_edges: bool,
    /// Draw boundary and non-manifold edges in warning colours.
    pub show_diagnostics: bool,
}

impl SceneStyle {
    pub fn dark() -> Self {
        Self {
            bg_top: rgb(0x2b_2f_36, 1.0),
            bg_bottom: rgb(0x16_18_1c, 1.0),
            plate_color: rgb(0x30_35_3d, 0.55),
            grid_color: rgb(0x8a_93_a0, 0.6),
            edge_color: rgb(0x10_12_14, 0.85),
            selection_color: rgb(0xff_a5_1f, 1.0),
            boundary_edge_color: rgb(0xff_30_30, 1.0),
            non_manifold_edge_color: rgb(0xff_8c_00, 1.0),
            plate_size: [220.0, 220.0],
            grid_minor: 1.0,
            grid_major: 10.0,
            show_grid: true,
            show_edges: true,
            show_diagnostics: true,
        }
    }

    pub fn light() -> Self {
        Self {
            bg_top: rgb(0xf4_f6_f9, 1.0),
            bg_bottom: rgb(0xc9_cf_d8, 1.0),
            plate_color: rgb(0xe6_ea_f0, 0.6),
            grid_color: rgb(0x6c_75_82, 0.55),
            edge_color: rgb(0x20_24_2a, 0.75),
            selection_color: rgb(0xff_8c_00, 1.0),
            ..Self::dark()
        }
    }
}

impl Default for SceneStyle {
    fn default() -> Self {
        Self::dark()
    }
}
