//! Document model for robocad: a tree of nodes with transforms, plus commands and undo/redo.
//!
//! All mutations go through [`Editor::apply`] so the app, the CLI and tests share one code path.

pub mod document;
pub mod editor;
pub mod expr;
pub mod transform;

pub use document::{DocError, DocSettings, Document, GroupOp, Node, NodeId, NodeKind, Role};
pub use editor::{Command, Editor, Selection};
pub use transform::Transform;
