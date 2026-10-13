//! Geometry kernel for robocad: parametric primitives and (from M5) solid boolean operations.

pub mod primitive;
pub mod solid;
pub mod text;

pub use primitive::{Primitive, PrimitiveKind};
pub use solid::{GeometryKernel, KernelError, ManifoldKernel, Solid};
