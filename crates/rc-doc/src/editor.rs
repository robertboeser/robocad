//! Commands, selection and snapshot-based undo/redo.

use std::sync::Arc;

use glam::DVec3;
use rc_mesh::Mesh;

use rc_kernel::Primitive;

use crate::{DocError, Document, GroupOp, Node, NodeId, NodeKind, Role, Transform};

/// Where to align along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    Min,
    Center,
    Max,
}

/// A user-level edit. Every document change goes through one of these.
#[derive(Clone, Debug)]
pub enum Command {
    /// Adds an imported mesh as a new root node.
    Import {
        name: String,
        mesh: Arc<Mesh>,
        transform: Transform,
        color: [u8; 3],
    },
    AddNode {
        node: Node,
        parent: Option<NodeId>,
        index: Option<usize>,
    },
    Delete(Vec<NodeId>),
    SetTransforms(Vec<(NodeId, Transform)>),
    TranslateWorld(Vec<NodeId>, DVec3),
    Rename(NodeId, String),
    SetColor(Vec<NodeId>, [u8; 3]),
    SetVisible(NodeId, bool),
    SetLocked(NodeId, bool),
    SetRole(Vec<NodeId>, Role),
    /// Solid ↔ hole for each node.
    ToggleRole(Vec<NodeId>),
    Reparent {
        nodes: Vec<NodeId>,
        parent: Option<NodeId>,
        index: Option<usize>,
    },
    Group(Vec<NodeId>),
    Ungroup(Vec<NodeId>),
    SetGroupOp(NodeId, GroupOp),
    Duplicate(Vec<NodeId>),
    /// Adds a primitive as a new root node, positioned in world space.
    AddPrimitive {
        primitive: Primitive,
        position: DVec3,
        color: [u8; 3],
    },
    SetPrimitives(Vec<(NodeId, Primitive)>),
    /// Replaces the mesh of an imported node (e.g. after repair).
    ReplaceMesh {
        node: NodeId,
        mesh: Arc<Mesh>,
    },
    /// Moves nodes down/up so their lowest point sits on z = 0.
    DropToPlate(Vec<NodeId>),
    /// Aligns nodes to the combined bounds of all of them along `axis` (0 = x, 1 = y, 2 = z).
    Align {
        nodes: Vec<NodeId>,
        axis: usize,
        mode: AlignMode,
    },
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Import { .. } => "Import",
            Self::AddNode { .. } => "Add",
            Self::Delete(_) => "Delete",
            Self::SetTransforms(_) | Self::TranslateWorld(..) => "Transform",
            Self::Rename(..) => "Rename",
            Self::SetColor(..) => "Color",
            Self::SetVisible(..) => "Visibility",
            Self::SetLocked(..) => "Lock",
            Self::SetRole(..) | Self::ToggleRole(_) => "Solid/Hole",
            Self::Reparent { .. } => "Move in tree",
            Self::Group(_) => "Group",
            Self::Ungroup(_) => "Ungroup",
            Self::SetGroupOp(..) => "Group mode",
            Self::Duplicate(_) => "Duplicate",
            Self::AddPrimitive { .. } => "Add shape",
            Self::SetPrimitives(_) => "Shape",
            Self::ReplaceMesh { .. } => "Repair",
            Self::DropToPlate(_) => "Drop to plate",
            Self::Align { .. } => "Align",
        }
    }
}

/// Result of applying a command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Nodes created by the command (new selection for import/group/duplicate/ungroup).
    pub created: Vec<NodeId>,
}

/// Ordered set of selected nodes; the last one is the primary selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<NodeId>,
}

impl Selection {
    pub fn ids(&self) -> &[NodeId] {
        &self.ids
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn contains(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }
    pub fn primary(&self) -> Option<NodeId> {
        self.ids.last().copied()
    }
    pub fn clear(&mut self) {
        self.ids.clear();
    }
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.ids.clear();
        for id in ids {
            self.add(id);
        }
    }
    pub fn add(&mut self, id: NodeId) {
        self.ids.retain(|i| *i != id);
        self.ids.push(id);
    }
    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(id) {
            self.ids.retain(|i| *i != id);
        } else {
            self.ids.push(id);
        }
    }
    pub fn retain_valid(&mut self, doc: &Document) {
        self.ids.retain(|i| doc.contains(*i));
    }
}

struct Entry {
    doc: Document,
    selection: Selection,
    label: &'static str,
}

const UNDO_LIMIT: usize = 200;

/// Owns the document, the selection and the undo history.
pub struct Editor {
    doc: Document,
    pub selection: Selection,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    merge_key: Option<u64>,
    revision: u64,
    saved_revision: u64,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Document::new())
    }
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("nodes", &self.doc.nodes.len())
            .field("undo", &self.undo.len())
            .field("redo", &self.redo.len())
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            selection: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            merge_key: None,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Increments on every document change (including undo/redo).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// Replaces the whole document (e.g. after loading a file) and clears history.
    pub fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
        self.selection.clear();
        self.merge_key = None;
        self.revision += 1;
        self.saved_revision = self.revision;
    }

    /// Changes document settings without creating an undo step.
    pub fn set_settings(&mut self, settings: crate::DocSettings) {
        if self.doc.settings != settings {
            self.doc.settings = settings;
            self.revision += 1;
        }
    }

    /// Applies a command as its own undo step.
    pub fn apply(&mut self, cmd: Command) -> Result<Outcome, DocError> {
        self.merge_key = None;
        self.apply_inner(cmd, None)
    }

    /// Applies a command, merging it into the previous undo step if that step used the same
    /// `key` (continuous edits such as dragging a slider or a gizmo).
    pub fn apply_merged(&mut self, cmd: Command, key: u64) -> Result<Outcome, DocError> {
        self.apply_inner(cmd, Some(key))
    }

    /// Ends the current merge sequence, so the next merged command starts a new undo step.
    pub fn end_merge(&mut self) {
        self.merge_key = None;
    }

    fn apply_inner(&mut self, cmd: Command, key: Option<u64>) -> Result<Outcome, DocError> {
        let label = cmd.label();
        let before = self.doc.clone();
        let before_sel = self.selection.clone();
        match execute(&mut self.doc, &mut self.selection, cmd) {
            Ok(outcome) => {
                if self.doc == before {
                    return Ok(outcome);
                }
                let merging = key.is_some() && key == self.merge_key;
                if !merging {
                    self.undo.push(Entry {
                        doc: before,
                        selection: before_sel,
                        label,
                    });
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                }
                self.merge_key = key;
                self.redo.clear();
                self.revision += 1;
                self.selection.retain_valid(&self.doc);
                Ok(outcome)
            }
            Err(e) => {
                self.doc = before;
                self.selection = before_sel;
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|e| e.label)
    }
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|e| e.label)
    }

    pub fn undo(&mut self) -> bool {
        self.merge_key = None;
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.doc, entry.doc);
        let sel = std::mem::replace(&mut self.selection, entry.selection);
        self.redo.push(Entry {
            doc: current,
            selection: sel,
            label: entry.label,
        });
        self.selection.retain_valid(&self.doc);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.merge_key = None;
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.doc, entry.doc);
        let sel = std::mem::replace(&mut self.selection, entry.selection);
        self.undo.push(Entry {
            doc: current,
            selection: sel,
            label: entry.label,
        });
        self.selection.retain_valid(&self.doc);
        self.revision += 1;
        true
    }

    /// Selected nodes without those whose ancestor is also selected.
    pub fn top_level_selection(&self) -> Vec<NodeId> {
        top_level(&self.doc, self.selection.ids())
    }
}

/// Filters out nodes whose ancestor is also in the list, and missing nodes.
pub fn top_level(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter()
        .copied()
        .filter(|&id| doc.contains(id))
        .filter(|&id| {
            let parent = doc.get(id).and_then(|n| n.parent);
            !ids.iter().any(|&other| {
                other != id && parent.is_some_and(|p| doc.is_ancestor_or_self(other, p))
            })
        })
        .collect()
}

fn execute(doc: &mut Document, sel: &mut Selection, cmd: Command) -> Result<Outcome, DocError> {
    let mut out = Outcome::default();
    match cmd {
        Command::Import {
            name,
            mesh,
            transform,
            color,
        } => {
            let key = doc.add_mesh(mesh);
            let mut node = Node::new(
                doc.unique_name(&name),
                NodeKind::Imported {
                    mesh: key,
                    source: name,
                },
            );
            node.transform = transform;
            node.color = color;
            let id = doc.insert(node, None, None)?;
            sel.set([id]);
            out.created.push(id);
        }
        Command::AddNode {
            node,
            parent,
            index,
        } => {
            let id = doc.insert(node, parent, index)?;
            sel.set([id]);
            out.created.push(id);
        }
        Command::Delete(ids) => {
            for id in top_level(doc, &ids) {
                if !doc.is_effectively_locked(id) {
                    doc.remove(id)?;
                }
            }
        }
        Command::SetTransforms(list) => {
            for (id, t) in list {
                if doc.is_effectively_locked(id) {
                    continue;
                }
                validate_transform(&t)?;
                doc.get_mut(id).ok_or(DocError::NotFound)?.transform = t;
            }
        }
        Command::TranslateWorld(ids, delta) => {
            for id in top_level(doc, &ids) {
                if !doc.is_effectively_locked(id) {
                    doc.translate_world(id, delta)?;
                }
            }
        }
        Command::Rename(id, name) => {
            let name = name.trim();
            if name.is_empty() {
                return Err(DocError::Invalid("a name cannot be empty".into()));
            }
            doc.get_mut(id).ok_or(DocError::NotFound)?.name = name.to_string();
        }
        Command::SetColor(ids, color) => {
            for id in ids {
                for leaf in doc.leaves(id) {
                    doc.get_mut(leaf).ok_or(DocError::NotFound)?.color = color;
                }
                doc.get_mut(id).ok_or(DocError::NotFound)?.color = color;
            }
        }
        Command::SetVisible(id, v) => doc.get_mut(id).ok_or(DocError::NotFound)?.visible = v,
        Command::SetLocked(id, v) => doc.get_mut(id).ok_or(DocError::NotFound)?.locked = v,
        Command::SetRole(ids, role) => {
            for id in ids {
                doc.get_mut(id).ok_or(DocError::NotFound)?.role = role;
            }
        }
        Command::ToggleRole(ids) => {
            for id in ids {
                let n = doc.get_mut(id).ok_or(DocError::NotFound)?;
                n.role = match n.role {
                    Role::Solid => Role::Hole,
                    Role::Hole => Role::Solid,
                };
            }
        }
        Command::Reparent {
            nodes,
            parent,
            index,
        } => {
            let mut index = index;
            for id in top_level(doc, &nodes) {
                doc.reparent(id, parent, index)?;
                index = index.map(|_| doc.index_in_parent(id).map_or(0, |i| i + 1));
            }
        }
        Command::Group(ids) => {
            let ids = top_level(doc, &ids);
            let Some(&first) = ids.first() else {
                return Ok(out);
            };
            let parent = doc.get(first).ok_or(DocError::NotFound)?.parent;
            let index = ids
                .iter()
                .filter(|id| doc.get(**id).is_some_and(|n| n.parent == parent))
                .filter_map(|id| doc.index_in_parent(*id))
                .min();
            let mut group = Node::new(
                doc.unique_name("Group"),
                NodeKind::Group {
                    children: Vec::new(),
                    op: GroupOp::Combine,
                },
            );
            group.color = doc.get(first).map_or(group.color, |n| n.color);
            let gid = doc.insert(group, parent, index)?;
            for id in ids {
                doc.reparent(id, Some(gid), None)?;
            }
            sel.set([gid]);
            out.created.push(gid);
        }
        Command::Ungroup(ids) => {
            let mut new_sel = Vec::new();
            for gid in ids {
                let Some(g) = doc.get(gid) else { continue };
                if !g.is_group() {
                    continue;
                }
                let parent = g.parent;
                let children = g.children().to_vec();
                let mut index = doc.index_in_parent(gid);
                for c in children {
                    doc.reparent(c, parent, index)?;
                    index = doc.index_in_parent(c).map(|i| i + 1);
                    new_sel.push(c);
                }
                doc.remove(gid)?;
            }
            sel.set(new_sel.iter().copied());
            out.created = new_sel;
        }
        Command::SetGroupOp(id, new_op) => {
            match &mut doc.get_mut(id).ok_or(DocError::NotFound)?.kind {
                NodeKind::Group { op, .. } => *op = new_op,
                NodeKind::Imported { .. } | NodeKind::Primitive(_) => {
                    return Err(DocError::NotAGroup);
                }
            }
        }
        Command::AddPrimitive {
            primitive,
            position,
            color,
        } => {
            primitive.validate().map_err(DocError::Invalid)?;
            let mut node = Node::new(
                doc.unique_name(primitive.kind().name()),
                NodeKind::Primitive(primitive),
            );
            node.transform = Transform::from_translation(position);
            node.color = color;
            let id = doc.insert(node, None, None)?;
            sel.set([id]);
            out.created.push(id);
        }
        Command::SetPrimitives(list) => {
            for (id, p) in list {
                if doc.is_effectively_locked(id) {
                    continue;
                }
                p.validate().map_err(DocError::Invalid)?;
                match &mut doc.get_mut(id).ok_or(DocError::NotFound)?.kind {
                    NodeKind::Primitive(old) => *old = p,
                    _ => return Err(DocError::Invalid("not a primitive".into())),
                }
            }
        }
        Command::ReplaceMesh { node, mesh } => {
            let key = doc.add_mesh(mesh);
            match &mut doc.get_mut(node).ok_or(DocError::NotFound)?.kind {
                NodeKind::Imported { mesh, .. } => *mesh = key,
                _ => {
                    return Err(DocError::Invalid(
                        "only imported meshes can be replaced".into(),
                    ));
                }
            }
        }
        Command::DropToPlate(ids) => {
            for id in top_level(doc, &ids) {
                let b = doc.world_bounds(id);
                if !b.is_empty() && !doc.is_effectively_locked(id) {
                    doc.translate_world(id, DVec3::new(0.0, 0.0, -f64::from(b.min.z)))?;
                }
            }
        }
        Command::Align { nodes, axis, mode } => {
            let ids = top_level(doc, &nodes);
            let all = doc.bounds_of(&ids);
            if all.is_empty() || axis > 2 {
                return Ok(out);
            }
            let pick = |b: &rc_mesh::Aabb| match mode {
                AlignMode::Min => f64::from(b.min[axis]),
                AlignMode::Center => f64::from(b.center()[axis]),
                AlignMode::Max => f64::from(b.max[axis]),
            };
            let target = pick(&all);
            for id in ids {
                let b = doc.world_bounds(id);
                if b.is_empty() || doc.is_effectively_locked(id) {
                    continue;
                }
                let mut d = DVec3::ZERO;
                d[axis] = target - pick(&b);
                doc.translate_world(id, d)?;
            }
        }
        Command::Duplicate(ids) => {
            let mut created = Vec::new();
            for id in top_level(doc, &ids) {
                let parent = doc.get(id).ok_or(DocError::NotFound)?.parent;
                let index = doc.index_in_parent(id).map(|i| i + 1);
                let copy = copy_subtree(doc, id, parent, index)?;
                created.push(copy);
            }
            sel.set(created.iter().copied());
            out.created = created;
        }
    }
    Ok(out)
}

fn validate_transform(t: &Transform) -> Result<(), DocError> {
    let finite = t.translation.is_finite() && t.rotation_deg.is_finite() && t.scale.is_finite();
    if !finite {
        return Err(DocError::Invalid(
            "transform contains invalid numbers".into(),
        ));
    }
    if t.scale.abs().min_element() < 1e-9 {
        return Err(DocError::Invalid("scale must not be zero".into()));
    }
    Ok(())
}

fn copy_subtree(
    doc: &mut Document,
    id: NodeId,
    parent: Option<NodeId>,
    index: Option<usize>,
) -> Result<NodeId, DocError> {
    let mut node = doc.get(id).ok_or(DocError::NotFound)?.clone();
    let children = node.children().to_vec();
    if let NodeKind::Group { children: c, .. } = &mut node.kind {
        c.clear();
    }
    node.name = doc.unique_name(&node.name);
    let new_id = doc.insert(node, parent, index)?;
    for c in children {
        copy_subtree(doc, c, Some(new_id), None)?;
    }
    Ok(new_id)
}
