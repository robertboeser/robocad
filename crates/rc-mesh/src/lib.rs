//! Mesh data structures, STL I/O, spatial acceleration and analysis for robocad.
//!
//! Conventions: right-handed, Z up, millimetres, counter-clockwise triangles face outward.
//! Vertices are stored as `f32`; all accumulations (volume, area) use `f64`.

pub mod aabb;
pub mod bvh;
pub mod edges;
pub mod mesh;
pub mod repair;
pub mod stl;
pub mod topology;
pub mod weld;

pub use aabb::Aabb;
pub use bvh::{Bvh, Ray, RayHit};
pub use mesh::{Mesh, MeshInfo};
pub use topology::TopologyReport;
