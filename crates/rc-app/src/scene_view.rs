//! What the viewport shows: leaves, evaluated group results, ghosts — and picking on them.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{DMat4, DVec2, DVec3};
use rc_doc::eval::{EvalOutput, content_hash};
use rc_doc::{NodeId, Role};
use rc_mesh::Ray;
use rc_render::{DrawObject, DrawStyle};

use crate::app::{MeshExtras, RobocadApp};
use crate::interaction::Pick;

/// Salt separating evaluated-group display keys from other mesh keys.
const GROUP_SALT: u64 = 0x5851_f42d_4c95_7f2d;

/// One thing drawn in the viewport.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    /// Node selected when this item is clicked (always a root node).
    pub root: NodeId,
    /// Node this item represents (leaf or group).
    pub node: NodeId,
    pub key: u64,
    pub model: DMat4,
    pub color: [f32; 4],
    pub style: DrawStyle,
    pub pickable: bool,
}

/// Results of group evaluation kept by the app.
#[derive(Default)]
pub struct GroupResults {
    /// Display key by group content hash.
    pub ready: HashMap<u64, u64>,
    pub info: HashMap<u64, rc_mesh::MeshInfo>,
    pub errors: HashMap<u64, String>,
    /// Last good result per group node (shown while a newer one is computed).
    pub last: HashMap<NodeId, u64>,
    pub meshes: HashMap<u64, Arc<rc_mesh::Mesh>>,
    pub last_eval_ms: Option<f64>,
}

fn linear(c: [u8; 3]) -> [f32; 4] {
    let l = c.map(|v| rc_render::srgb_to_linear(f32::from(v) / 255.0));
    [l[0], l[1], l[2], 1.0]
}

const HOLE_COLOR: [f32; 4] = [0.35, 0.37, 0.40, 0.35];

impl RobocadApp {
    /// Receives finished evaluations and uploads their meshes.
    pub(crate) fn receive_evaluations(&mut self) {
        for msg in self.evaluator.poll() {
            match msg.result {
                Ok(out) => self.store_group_result(msg.node, msg.hash, &out),
                Err(e) if e.is_empty() => {} // superseded request
                Err(e) => {
                    self.groups.errors.insert(msg.hash, e);
                }
            }
        }
    }

    fn store_group_result(&mut self, node: NodeId, hash: u64, out: &EvalOutput) {
        let key = hash ^ GROUP_SALT;
        self.extras.entry(key).or_insert_with(|| MeshExtras {
            bvh: out.bvh.clone(),
            topology: rc_mesh::topology::analyze(&out.mesh),
            info: out.info,
            path: None,
            notes: Vec::new(),
        });
        if let Some(vp) = self.viewport.as_mut()
            && !vp.renderer().has_mesh(key)
        {
            let device = vp.device().clone();
            vp.renderer_mut()
                .upload_mesh(&device, key, &out.mesh, &out.edges);
        }
        self.groups.meshes.insert(key, out.mesh.clone());
        self.groups.ready.insert(hash, key);
        self.groups.info.insert(hash, out.info);
        self.groups.last.insert(node, key);
        self.groups.last_eval_ms = Some(out.elapsed.as_secs_f64() * 1e3);
    }

    /// Error message for a group's current state, if its evaluation failed.
    pub fn group_error(&self, id: NodeId) -> Option<&str> {
        let h = content_hash(self.editor.doc(), id)?;
        self.groups.errors.get(&h).map(String::as_str)
    }

    /// Mesh info of a group's current evaluated result.
    pub fn group_info(&self, id: NodeId) -> Option<rc_mesh::MeshInfo> {
        let h = content_hash(self.editor.doc(), id)?;
        self.groups.info.get(&h).copied()
    }

    /// Whether every group's current result is available (no evaluation pending).
    pub fn evaluation_idle(&self) -> bool {
        !self.evaluator.is_busy()
    }

    pub(crate) fn mesh_for_key(&self, key: u64) -> Option<&Arc<rc_mesh::Mesh>> {
        self.prim_meshes
            .get(&key)
            .or_else(|| self.groups.meshes.get(&key))
            .or_else(|| self.editor.doc().mesh(key))
    }

    /// Builds the list of things to draw this frame, requesting group evaluations as needed.
    pub(crate) fn build_items(&mut self) -> Vec<Item> {
        let roots: Vec<NodeId> = self.editor.doc().roots.clone();
        let mut items = Vec::new();
        let mut requests = Vec::new();
        for root in roots {
            let Some(node) = self.editor.doc().get(root).cloned() else {
                continue;
            };
            if !node.visible {
                continue;
            }
            if !node.is_group() {
                self.push_leaf_items(root, root, false, &mut items);
                continue;
            }
            let hash = content_hash(self.editor.doc(), root);
            let ready = hash.and_then(|h| self.groups.ready.get(&h).copied());
            let failed = hash.is_some_and(|h| self.groups.errors.contains_key(&h));
            let shown = match ready {
                Some(k) => Some(k),
                None if failed => None,
                None => {
                    if let Some(h) = hash {
                        requests.push((root, h));
                    }
                    self.groups.last.get(&root).copied()
                }
            };
            match shown {
                Some(key) => {
                    let (color, style) = if node.role == Role::Hole {
                        (HOLE_COLOR, DrawStyle::Hole)
                    } else {
                        (linear(node.color), DrawStyle::Solid)
                    };
                    items.push(Item {
                        root,
                        node: root,
                        key,
                        model: self.editor.doc().world_matrix(root),
                        color,
                        style,
                        pickable: true,
                    });
                    // Selected parts inside the group are shown as ghosts.
                    let inner: Vec<NodeId> = self
                        .editor
                        .selection
                        .ids()
                        .iter()
                        .copied()
                        .filter(|&s| s != root && self.editor.doc().is_ancestor_or_self(root, s))
                        .collect();
                    for s in inner {
                        self.push_leaf_items(root, s, true, &mut items);
                    }
                }
                // No result yet (first evaluation) or evaluation failed: show the parts.
                None => self.push_leaf_items(root, root, false, &mut items),
            }
        }
        if !requests.is_empty() {
            let doc = self.editor.doc().clone();
            self.evaluator.request(&doc, &requests);
        }
        let used: Vec<u64> = items.iter().map(|i| i.key).collect();
        self.evict_unused(&used);
        items
    }

    /// Items for the visible leaves under `id`.
    fn push_leaf_items(&mut self, root: NodeId, id: NodeId, ghost: bool, items: &mut Vec<Item>) {
        let leaves = self.editor.doc().leaves(id);
        for leaf in leaves {
            if !self.editor.doc().is_effectively_visible(leaf) {
                continue;
            }
            let Some(key) = self.ensure_leaf(leaf) else {
                continue;
            };
            let doc = self.editor.doc();
            let mut cur = Some(leaf);
            let mut hole = false;
            while let Some(c) = cur {
                let n = doc.get(c);
                hole |= n.is_some_and(|n| n.role == Role::Hole);
                cur = n.and_then(|n| n.parent);
            }
            let Some(node) = doc.get(leaf) else { continue };
            let (color, style) = if ghost {
                let mut c = linear(node.color);
                c[3] = 0.3;
                (c, DrawStyle::Ghost)
            } else if hole {
                (HOLE_COLOR, DrawStyle::Hole)
            } else {
                (linear(node.color), DrawStyle::Solid)
            };
            items.push(Item {
                root,
                node: leaf,
                key,
                model: doc.world_matrix(leaf),
                color,
                style,
                pickable: !ghost && !doc.is_effectively_locked(leaf),
            });
        }
    }

    pub(crate) fn draw_list(&self, items: &[Item]) -> Vec<DrawObject> {
        let sel = &self.editor.selection;
        items
            .iter()
            .map(|it| DrawObject {
                mesh: it.key,
                model: it.model,
                color: it.color,
                style: it.style,
                selected: it.style != DrawStyle::Ghost
                    && (sel.contains(it.root) || sel.contains(it.node)),
                hovered: self.hovered == Some(it.root) && it.style != DrawStyle::Ghost,
            })
            .collect()
    }

    /// Closest pickable item under a viewport pixel.
    pub(crate) fn pick(&self, pixel: DVec2, size: DVec2) -> Option<Pick> {
        let vp = self.viewport.as_ref()?;
        let (origin, dir) = vp.camera.ray(pixel, size);
        let mut best: Option<(f64, Pick)> = None;
        for it in self.items.iter().filter(|i| i.pickable) {
            if self.editor.doc().is_effectively_locked(it.root) {
                continue;
            }
            let (Some(m), Some(extra)) = (self.mesh_for_key(it.key), self.extras.get(&it.key))
            else {
                continue;
            };
            let inv = it.model.inverse();
            let ray = Ray::new(
                inv.transform_point3(origin).as_vec3(),
                inv.transform_vector3(dir).as_vec3(),
            );
            if let Some(hit) = extra.bvh.raycast(m, &ray, f32::INFINITY) {
                let wp = it.model.transform_point3(hit.point.as_dvec3());
                let dist = (wp - origin).dot(dir);
                if best.is_none_or(|(d, _)| dist < d) {
                    let triangle = m
                        .triangle(hit.triangle as usize)
                        .map(|v| it.model.transform_point3(v.as_dvec3()));
                    let normal = (triangle[1] - triangle[0])
                        .cross(triangle[2] - triangle[0])
                        .normalize_or(DVec3::Z);
                    best = Some((
                        dist,
                        Pick {
                            node: it.node,
                            point: wp,
                            normal,
                            triangle,
                        },
                    ));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// Triangle count of everything currently drawn.
    pub fn drawn_triangles(&self) -> usize {
        self.items
            .iter()
            .filter_map(|i| self.mesh_for_key(i.key))
            .map(|m| m.triangle_count())
            .sum()
    }
}
