//! wgpu renderer for robocad.
//!
//! The renderer draws into its own offscreen texture (4× MSAA, reverse-Z depth). The host
//! application displays that texture (e.g. as an egui image) or reads it back (PNG export,
//! thumbnails, visual tests). It depends only on `rc-mesh`, never on the document model.

pub mod camera;
mod renderer;
mod style;

pub use camera::{OrbitCamera, Projection, StandardView};
pub use renderer::{
    DrawObject, DrawStyle, FrameDesc, MeshKey, Renderer, encode_png, headless_device,
};
pub use style::{SceneStyle, srgb_to_linear};
