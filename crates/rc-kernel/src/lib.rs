//! Geometry kernel for robocad: parametric primitives, solid boolean operations and edge rounding.

pub mod primitive;
pub mod round;
pub mod solid;
pub mod text;

pub use primitive::{Primitive, PrimitiveKind};
pub use solid::{GeometryKernel, KernelError, ManifoldKernel, Solid};
