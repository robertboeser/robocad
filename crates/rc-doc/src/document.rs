//! The document: a tree of nodes plus a content-addressed mesh store.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{DMat4, DVec3};
use rc_kernel::Primitive;
use rc_mesh::{Aabb, Mesh};
use serde::{Deserialize, Serialize};
use slotmap::{SlotMap, new_key_type};

use crate::Transform;

new_key_type! {
    /// Stable identifier of a node within a document.
    pub struct NodeId;
}

/// How a node participates in its parent group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Solid,
    /// Subtracted from the solids of its parent group.
    Hole,
}

/// How a group combines its children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupOp {
    /// Union of all solid children minus the union of all hole children.
    #[default]
    Combine,
    /// Intersection of all solid children (holes are still subtracted afterwards).
    Intersect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// A mesh imported from a file, referenced by content hash in the mesh store.
    Imported {
        mesh: u64,
        source: String,
    },
    /// A parametric primitive (box, cylinder, …).
    Primitive(Primitive),
    Group {
        children: Vec<NodeId>,
        op: GroupOp,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub transform: Transform,
    pub kind: NodeKind,
    pub role: Role,
    /// sRGB colour.
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
    pub parent: Option<NodeId>,
}

impl Node {
    pub fn new(name: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            name: name.into(),
            transform: Transform::IDENTITY,
            kind,
            role: Role::Solid,
            color: [74, 144, 226],
            visible: true,
            locked: false,
            parent: None,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, NodeKind::Group { .. })
    }

    pub fn children(&self) -> &[NodeId] {
        match &self.kind {
            NodeKind::Group { children, .. } => children,
            NodeKind::Imported { .. } | NodeKind::Primitive(_) => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocSettings {
    /// Build plate size in mm (x, y), centred on the origin.
    pub plate_size: [f64; 2],
    /// Translation snapping step in mm.
    pub grid_step: f64,
}

impl Default for DocSettings {
    fn default() -> Self {
        Self {
            plate_size: [220.0, 220.0],
            grid_step: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocError {
    #[error("node not found")]
    NotFound,
    #[error("a node cannot be moved into itself or its descendants")]
    Cycle,
    #[error("the target is not a group")]
    NotAGroup,
    #[error("{0}")]
    Invalid(String),
}

/// The document. Cloning is cheap enough for snapshot-based undo: nodes are small and meshes are
/// shared through `Arc`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    pub nodes: SlotMap<NodeId, Node>,
    pub roots: Vec<NodeId>,
    pub settings: DocSettings,
    #[serde(skip)]
    meshes: Arc<HashMap<u64, Arc<Mesh>>>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.roots == other.roots
            && self.settings == other.settings
            && self.nodes.len() == other.nodes.len()
            && self
                .nodes
                .iter()
                .all(|(k, n)| other.nodes.get(k) == Some(n))
            && self.meshes.len() == other.meshes.len()
            && self.meshes.keys().all(|k| other.meshes.contains_key(k))
    }
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------ meshes

    /// Adds a mesh to the store and returns its content hash.
    pub fn add_mesh(&mut self, mesh: Arc<Mesh>) -> u64 {
        let key = mesh.content_hash();
        if !self.meshes.contains_key(&key) {
            Arc::make_mut(&mut self.meshes).insert(key, mesh);
        }
        key
    }

    pub fn mesh(&self, key: u64) -> Option<&Arc<Mesh>> {
        self.meshes.get(&key)
    }

    pub fn meshes(&self) -> impl Iterator<Item = (u64, &Arc<Mesh>)> {
        self.meshes.iter().map(|(k, v)| (*k, v))
    }

    /// Removes meshes no node references (e.g. before saving).
    pub fn prune_meshes(&mut self) {
        let used: std::collections::HashSet<u64> = self
            .nodes
            .values()
            .filter_map(|n| match n.kind {
                NodeKind::Imported { mesh, .. } => Some(mesh),
                NodeKind::Group { .. } | NodeKind::Primitive(_) => None,
            })
            .collect();
        if self.meshes.keys().any(|k| !used.contains(k)) {
            Arc::make_mut(&mut self.meshes).retain(|k, _| used.contains(k));
        }
    }

    // ------------------------------------------------------------------ tree

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Children of `parent`, or the roots for `None`.
    pub fn child_list(&self, parent: Option<NodeId>) -> &[NodeId] {
        match parent {
            None => &self.roots,
            Some(p) => self.nodes.get(p).map_or(&[], |n| n.children()),
        }
    }

    fn child_list_mut(&mut self, parent: Option<NodeId>) -> Result<&mut Vec<NodeId>, DocError> {
        match parent {
            None => Ok(&mut self.roots),
            Some(p) => match &mut self.nodes.get_mut(p).ok_or(DocError::NotFound)?.kind {
                NodeKind::Group { children, .. } => Ok(children),
                NodeKind::Imported { .. } | NodeKind::Primitive(_) => Err(DocError::NotAGroup),
            },
        }
    }

    /// Inserts a node under `parent` at `index` (append if `None`).
    pub fn insert(
        &mut self,
        mut node: Node,
        parent: Option<NodeId>,
        index: Option<usize>,
    ) -> Result<NodeId, DocError> {
        // Validate the parent before inserting.
        self.child_list_mut(parent)?;
        node.parent = parent;
        let id = self.nodes.insert(node);
        let list = self.child_list_mut(parent)?;
        let i = index.unwrap_or(list.len()).min(list.len());
        list.insert(i, id);
        Ok(id)
    }

    /// Removes a node and all its descendants.
    pub fn remove(&mut self, id: NodeId) -> Result<(), DocError> {
        let parent = self.nodes.get(id).ok_or(DocError::NotFound)?.parent;
        self.child_list_mut(parent)?.retain(|c| *c != id);
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.remove(n) {
                stack.extend_from_slice(node.children());
            }
        }
        Ok(())
    }

    /// True if `ancestor` is `id` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.nodes.get(c).and_then(|n| n.parent);
        }
        false
    }

    /// Index of `id` within its parent's child list.
    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let parent = self.nodes.get(id)?.parent;
        self.child_list(parent).iter().position(|c| *c == id)
    }

    /// Moves `id` under `new_parent` at `index`, keeping its world transform.
    pub fn reparent(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: Option<usize>,
    ) -> Result<(), DocError> {
        if let Some(p) = new_parent
            && self.is_ancestor_or_self(id, p)
        {
            return Err(DocError::Cycle);
        }
        self.child_list_mut(new_parent)?;
        let world = self.world_matrix(id);
        let old_parent = self.nodes.get(id).ok_or(DocError::NotFound)?.parent;
        let old_index = self.index_in_parent(id);
        self.child_list_mut(old_parent)?.retain(|c| *c != id);
        let list = self.child_list_mut(new_parent)?;
        let mut i = index.unwrap_or(list.len());
        // Moving within the same list: account for the removed entry.
        if old_parent == new_parent
            && let (Some(oi), Some(_)) = (old_index, index)
            && oi < i
        {
            i -= 1;
        }
        let i = i.min(list.len());
        list.insert(i, id);
        let parent_world = new_parent.map_or(DMat4::IDENTITY, |p| self.world_matrix(p));
        let node = self.nodes.get_mut(id).ok_or(DocError::NotFound)?;
        node.parent = new_parent;
        node.transform = Transform::from_matrix(&(parent_world.inverse() * world));
        Ok(())
    }

    /// World matrix of a node (parent chain applied).
    pub fn world_matrix(&self, id: NodeId) -> DMat4 {
        let mut m = DMat4::IDENTITY;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.nodes.get(c) else { break };
            m = n.transform.matrix() * m;
            cur = n.parent;
        }
        m
    }

    /// Visible if the node and all its ancestors are visible.
    pub fn is_effectively_visible(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.nodes.get(c) {
                Some(n) if n.visible => cur = n.parent,
                _ => return false,
            }
        }
        true
    }

    /// Locked if the node or any ancestor is locked.
    pub fn is_effectively_locked(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.nodes.get(c) {
                Some(n) if n.locked => return true,
                Some(n) => cur = n.parent,
                None => return false,
            }
        }
        false
    }

    /// All nodes in depth-first display order with their depth.
    pub fn walk(&self) -> Vec<(NodeId, usize)> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<(NodeId, usize)> = self.roots.iter().rev().map(|r| (*r, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            out.push((id, depth));
            if let Some(n) = self.nodes.get(id) {
                stack.extend(n.children().iter().rev().map(|c| (*c, depth + 1)));
            }
        }
        out
    }

    /// Mesh-carrying descendants of `id` (or `id` itself).
    pub fn leaves(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.get(n) {
                if node.is_group() {
                    stack.extend(node.children().iter().rev());
                } else {
                    out.push(n);
                }
            }
        }
        out
    }

    /// Local-space bounds of a leaf's mesh.
    pub fn local_bounds(&self, id: NodeId) -> Aabb {
        match self.nodes.get(id).map(|n| &n.kind) {
            Some(NodeKind::Imported { mesh, .. }) => {
                self.mesh(*mesh).map_or(Aabb::EMPTY, |m| m.aabb())
            }
            Some(NodeKind::Primitive(p)) => {
                let s = p.size().as_vec3();
                Aabb::new(
                    glam::Vec3::new(-s.x * 0.5, -s.y * 0.5, 0.0),
                    glam::Vec3::new(s.x * 0.5, s.y * 0.5, s.z),
                )
            }
            _ => Aabb::EMPTY,
        }
    }

    /// True if `leaf` or a node between it and `top` (exclusive) is a hole.
    fn is_hole_below(&self, top: NodeId, leaf: NodeId) -> bool {
        let mut cur = Some(leaf);
        while let Some(c) = cur {
            if c == top {
                return false;
            }
            let Some(n) = self.nodes.get(c) else {
                return false;
            };
            if n.role == Role::Hole {
                return true;
            }
            cur = n.parent;
        }
        false
    }

    /// World-space axis-aligned bounds of a node and its descendants. Holes inside a group only
    /// remove material, so they are ignored unless the group consists of holes only.
    pub fn world_bounds(&self, id: NodeId) -> Aabb {
        let all = self.leaves(id);
        let solids: Vec<NodeId> = all
            .iter()
            .copied()
            .filter(|l| !self.is_hole_below(id, *l))
            .collect();
        let leaves = if solids.is_empty() { all } else { solids };
        let mut b = Aabb::EMPTY;
        for leaf in leaves {
            let local = self.local_bounds(leaf);
            if local.is_empty() {
                continue;
            }
            let m = self.world_matrix(leaf);
            for c in local.corners() {
                b.extend(m.transform_point3(c.as_dvec3()).as_vec3());
            }
        }
        b
    }

    /// World bounds of all visible root nodes.
    pub fn scene_bounds(&self) -> Aabb {
        self.roots
            .iter()
            .filter(|r| self.nodes.get(**r).is_some_and(|n| n.visible))
            .fold(Aabb::EMPTY, |b, r| b.union(&self.world_bounds(*r)))
    }

    /// Translation in world space applied to a node (converted to its parent space).
    pub fn translate_world(&mut self, id: NodeId, delta: DVec3) -> Result<(), DocError> {
        let parent = self.nodes.get(id).ok_or(DocError::NotFound)?.parent;
        let pm = parent.map_or(DMat4::IDENTITY, |p| self.world_matrix(p));
        let local_delta = pm.inverse().transform_vector3(delta);
        let node = self.nodes.get_mut(id).ok_or(DocError::NotFound)?;
        node.transform.translation += local_delta;
        Ok(())
    }

    /// Bounds of several nodes together.
    pub fn bounds_of(&self, ids: &[NodeId]) -> Aabb {
        ids.iter()
            .fold(Aabb::EMPTY, |b, id| b.union(&self.world_bounds(*id)))
    }

    /// Finds a spot on the plate (z = 0) near the origin where a footprint of `size` (x, y)
    /// does not overlap any visible root object. Candidates lie on a grid, nearest first.
    pub fn free_position(&self, size: DVec3) -> DVec3 {
        let occupied: Vec<Aabb> = self
            .roots
            .iter()
            .filter(|r| self.is_effectively_visible(**r))
            .map(|r| self.world_bounds(*r))
            .filter(|b| !b.is_empty())
            .collect();
        let step = ((size.x.max(size.y) + 5.0) / 5.0).ceil() * 5.0;
        let mut candidates: Vec<(i32, i32)> = (-8..=8)
            .flat_map(|i| (-8..=8).map(move |j| (i, j)))
            .collect();
        candidates.sort_by_key(|(i, j)| (i * i + j * j, *j, *i));
        let half = glam::Vec2::new(size.x as f32, size.y as f32) * 0.5;
        for (i, j) in candidates {
            let c = glam::Vec2::new(i as f32, j as f32) * step as f32;
            let (lo, hi) = (c - half, c + half);
            let free = occupied
                .iter()
                .all(|b| hi.x <= b.min.x || lo.x >= b.max.x || hi.y <= b.min.y || lo.y >= b.max.y);
            if free {
                return DVec3::new(f64::from(c.x), f64::from(c.y), 0.0);
            }
        }
        DVec3::ZERO
    }

    /// Generates a name not used by any node, e.g. "Box 3".
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.nodes.values().any(|n| n.name == name);
        if !taken(base) {
            return base.to_string();
        }
        // At most n + 1 candidates are needed with n nodes.
        (2..self.nodes.len() + 3)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| base.to_string())
    }
}
