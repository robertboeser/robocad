//! `.rcad` project files: a ZIP container with the document as JSON plus mesh data.
//!
//! Layout:
//! - `document.json` — `{ "format_version": 1, "app_version": "...", "document": {...}, "view": {...} }`
//! - `meshes/<hash>.bin` — imported meshes in a raw indexed format (exact round trip, unlike STL,
//!   which cannot store vertex sharing and would undo separated touching parts)
//! - `thumbnail.png` — optional preview image
//!
//! Files are written atomically (temporary file + rename). Every format change bumps
//! [`FORMAT_VERSION`] and adds a step to [`migrate`]; old files must keep loading.

use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use rc_mesh::Mesh;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Document, NodeKind};

pub const FORMAT_VERSION: u64 = 1;
pub const EXTENSION: &str = "rcad";
const MESH_MAGIC: &[u8; 8] = b"RCMESH01";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a robocad project (invalid ZIP container): {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("the project file is damaged: {0}")]
    Corrupt(String),
    #[error("this project was made with a newer robocad (format {0}); please update")]
    TooNew(u64),
}

/// Camera state stored with a project.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub target: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub distance: f64,
    pub orthographic: bool,
}

/// A loaded project.
#[derive(Debug)]
pub struct Project {
    pub document: Document,
    pub view: Option<ViewState>,
}

#[derive(Serialize)]
struct ManifestOut<'a> {
    format_version: u64,
    app_version: &'a str,
    document: &'a Document,
    view: Option<ViewState>,
}

/// Encodes a mesh in the raw format.
pub fn encode_mesh(m: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + m.positions.len() * 12 + m.triangles.len() * 12);
    out.extend_from_slice(MESH_MAGIC);
    out.extend_from_slice(&(m.positions.len() as u32).to_le_bytes());
    out.extend_from_slice(&(m.triangles.len() as u32).to_le_bytes());
    for p in &m.positions {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for t in &m.triangles {
        for i in t {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    out
}

/// Decodes a mesh in the raw format, validating sizes and indices.
pub fn decode_mesh(b: &[u8]) -> Result<Mesh, ProjectError> {
    let bad = |m: &str| ProjectError::Corrupt(m.to_string());
    if b.len() < 16 || &b[..8] != MESH_MAGIC {
        return Err(bad("mesh data has an unknown format"));
    }
    let nv = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
    let nt = u32::from_le_bytes([b[12], b[13], b[14], b[15]]) as usize;
    let expected = 16 + nv * 12 + nt * 12;
    if b.len() != expected {
        return Err(bad("mesh data is truncated"));
    }
    let word = |o: usize| [b[o], b[o + 1], b[o + 2], b[o + 3]];
    let positions: Vec<[f32; 3]> = (0..nv)
        .map(|i| {
            let o = 16 + i * 12;
            [0, 4, 8].map(|k| f32::from_le_bytes(word(o + k)))
        })
        .collect();
    let base = 16 + nv * 12;
    let triangles: Vec<[u32; 3]> = (0..nt)
        .map(|i| {
            let o = base + i * 12;
            [0, 4, 8].map(|k| u32::from_le_bytes(word(o + k)))
        })
        .collect();
    if triangles.iter().flatten().any(|&i| i as usize >= nv) {
        return Err(bad("mesh data has invalid vertex indices"));
    }
    if positions.iter().flatten().any(|c| !c.is_finite()) {
        return Err(bad("mesh data has invalid coordinates"));
    }
    Ok(Mesh::new(positions, triangles))
}

/// Writes a project to any seekable writer.
pub fn write_to<W: Write + Seek>(
    doc: &Document,
    view: Option<ViewState>,
    thumbnail_png: Option<&[u8]>,
    w: W,
) -> Result<(), ProjectError> {
    let mut doc = doc.clone();
    doc.prune_meshes();
    let mut zip = zip::ZipWriter::new(w);
    let deflate = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let manifest = ManifestOut {
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION"),
        document: &doc,
        view,
    };
    zip.start_file("document.json", deflate)?;
    let json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| ProjectError::Corrupt(e.to_string()))?;
    zip.write_all(&json)?;
    let mut keys: Vec<(u64, &Arc<Mesh>)> = doc.meshes().collect();
    keys.sort_by_key(|(k, _)| *k);
    for (key, mesh) in keys {
        zip.start_file(format!("meshes/{key:016x}.bin"), deflate)?;
        zip.write_all(&encode_mesh(mesh))?;
    }
    if let Some(png) = thumbnail_png {
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("thumbnail.png", stored)?;
        zip.write_all(png)?;
    }
    zip.finish()?;
    Ok(())
}

/// Saves a project atomically.
pub fn save(
    doc: &Document,
    view: Option<ViewState>,
    thumbnail_png: Option<&[u8]>,
    path: impl AsRef<Path>,
) -> Result<(), ProjectError> {
    let path = path.as_ref();
    let tmp = path.with_extension("rcad.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut buf = std::io::BufWriter::new(file);
        write_to(doc, view, thumbnail_png, &mut buf)?;
        buf.flush()?;
        buf.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Brings an older manifest up to [`FORMAT_VERSION`].
pub fn migrate(mut manifest: Value) -> Result<Value, ProjectError> {
    let version = manifest
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| ProjectError::Corrupt("missing format version".into()))?;
    if version > FORMAT_VERSION {
        return Err(ProjectError::TooNew(version));
    }
    // Future migrations go here, one step per version, e.g.:
    // if version < 2 { manifest = v1_to_v2(manifest); }
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert("format_version".into(), Value::from(FORMAT_VERSION));
    }
    Ok(manifest)
}

/// Reads a project from any seekable reader.
pub fn read_from<R: Read + Seek>(r: R) -> Result<Project, ProjectError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let manifest: Value = {
        let mut f = zip
            .by_name("document.json")
            .map_err(|_| ProjectError::Corrupt("document.json is missing".into()))?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        serde_json::from_str(&s).map_err(|e| ProjectError::Corrupt(e.to_string()))?
    };
    let manifest = migrate(manifest)?;
    let doc_value = manifest
        .get("document")
        .cloned()
        .ok_or_else(|| ProjectError::Corrupt("document is missing".into()))?;
    let mut document: Document =
        serde_json::from_value(doc_value).map_err(|e| ProjectError::Corrupt(e.to_string()))?;
    let view = manifest
        .get("view")
        .cloned()
        .and_then(|v| serde_json::from_value::<Option<ViewState>>(v).ok())
        .flatten();

    // Load every referenced mesh and check its hash.
    let needed: Vec<u64> = document
        .nodes
        .values()
        .filter_map(|n| match n.kind {
            NodeKind::Imported { mesh, .. } => Some(mesh),
            _ => None,
        })
        .collect();
    for key in needed {
        if document.mesh(key).is_some() {
            continue;
        }
        let mut f = zip
            .by_name(&format!("meshes/{key:016x}.bin"))
            .map_err(|_| ProjectError::Corrupt(format!("mesh {key:016x} is missing")))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        let mesh = decode_mesh(&bytes)?;
        if mesh.content_hash() != key {
            return Err(ProjectError::Corrupt(format!(
                "mesh {key:016x} does not match its checksum"
            )));
        }
        document.add_mesh(Arc::new(mesh));
    }
    validate_tree(&document)?;
    Ok(Project { document, view })
}

/// Loads a project file.
pub fn load(path: impl AsRef<Path>) -> Result<Project, ProjectError> {
    let file = std::fs::File::open(path)?;
    read_from(std::io::BufReader::new(file))
}

/// Reads only the thumbnail (for previews), if present.
pub fn read_thumbnail(path: impl AsRef<Path>) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    let mut f = zip.by_name("thumbnail.png").ok()?;
    let mut v = Vec::new();
    f.read_to_end(&mut v).ok()?;
    Some(v)
}

/// Checks parent/child links so a damaged file cannot create cycles or dangling ids.
fn validate_tree(doc: &Document) -> Result<(), ProjectError> {
    let bad = |m: String| Err(ProjectError::Corrupt(m));
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<(crate::NodeId, Option<crate::NodeId>)> =
        doc.roots.iter().map(|r| (*r, None)).collect();
    while let Some((id, parent)) = stack.pop() {
        let Some(n) = doc.get(id) else {
            return bad("a node reference is dangling".into());
        };
        if !seen.insert(id) {
            return bad("a node appears twice in the tree".into());
        }
        if n.parent != parent {
            return bad(format!("node '{}' has an inconsistent parent", n.name));
        }
        stack.extend(n.children().iter().map(|c| (*c, Some(id))));
    }
    if seen.len() != doc.nodes.len() {
        return bad("some nodes are not part of the tree".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Editor, PrimitiveKind, Transform};
    use glam::{DVec3, Vec3};

    fn sample() -> Editor {
        let mut ed = Editor::default();
        let a = ed
            .apply(Command::AddPrimitive {
                primitive: PrimitiveKind::Box.default_primitive(),
                position: DVec3::new(1.0, 2.0, 0.0),
                color: [10, 20, 30],
            })
            .unwrap()
            .created[0];
        let mut cube = Mesh::cuboid(Vec3::ZERO, Vec3::splat(5.0));
        // Bit-identical duplicate vertex that an STL round trip would weld away.
        cube.positions.push(cube.positions[0]);
        let b = ed
            .apply(Command::Import {
                name: "part.stl".into(),
                mesh: Arc::new(cube),
                transform: Transform::from_translation(DVec3::X * 30.0),
                color: [200, 0, 0],
            })
            .unwrap()
            .created[0];
        let g = ed.apply(Command::Group(vec![a, b])).unwrap().created[0];
        ed.apply(Command::Rename(g, "Bracket".into())).unwrap();
        ed.apply(Command::ToggleRole(vec![b])).unwrap();
        ed
    }

    #[test]
    fn round_trip_is_exact() {
        let ed = sample();
        let view = ViewState {
            target: [1.0, 2.0, 3.0],
            yaw: 0.5,
            pitch: 0.3,
            distance: 200.0,
            orthographic: true,
        };
        let mut buf = std::io::Cursor::new(Vec::new());
        write_to(ed.doc(), Some(view), Some(b"\x89PNG fake"), &mut buf).unwrap();
        buf.set_position(0);
        let p = read_from(buf).unwrap();
        assert_eq!(&p.document, ed.doc());
        assert_eq!(p.view, Some(view));
        for (k, m) in ed.doc().meshes() {
            assert_eq!(p.document.mesh(k).unwrap().as_ref(), m.as_ref());
        }
    }

    #[test]
    fn modifiers_round_trip_and_are_omitted_when_unused() {
        let mut ed = sample();
        let manifest = |ed: &Editor| {
            let mut buf = std::io::Cursor::new(Vec::new());
            write_to(ed.doc(), None, None, &mut buf).unwrap();
            buf.set_position(0);
            (read_from(buf.clone()).unwrap(), buf.into_inner())
        };
        let json = |zip: Vec<u8>| {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
            std::io::read_to_string(z.by_name("document.json").unwrap()).unwrap()
        };
        let (_, plain) = manifest(&ed);
        assert!(
            !json(plain).contains("modifier"),
            "old files stay unchanged"
        );

        let g = ed.doc().roots[0];
        ed.apply(Command::SetModifier(
            g,
            Some(crate::Modifier::RoundEdges { radius: 1.5 }),
        ))
        .unwrap();
        let (p, zip) = manifest(&ed);
        assert_eq!(&p.document, ed.doc());
        assert!(json(zip).contains("RoundEdges"));
    }

    #[test]
    fn save_load_file_and_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.rcad");
        save(sample().doc(), None, Some(b"png"), &path).unwrap();
        assert!(!path.with_extension("rcad.tmp").exists());
        assert_eq!(load(&path).unwrap().document, *sample().doc());
        assert_eq!(read_thumbnail(&path).unwrap(), b"png");
    }

    #[test]
    fn rejects_damaged_and_newer_files() {
        assert!(matches!(
            read_from(std::io::Cursor::new(b"not a zip".to_vec())),
            Err(ProjectError::Zip(_))
        ));
        let v = serde_json::json!({ "format_version": FORMAT_VERSION + 1 });
        assert!(matches!(migrate(v), Err(ProjectError::TooNew(_))));
        assert!(migrate(serde_json::json!({})).is_err());
        assert!(decode_mesh(b"RCMESH01\x01\0\0\0\x01\0\0\0").is_err());
        let mut m = encode_mesh(&Mesh::cuboid(Vec3::ZERO, Vec3::ONE));
        let n = m.len();
        m[n - 1] = 0xff; // index out of range
        assert!(decode_mesh(&m).is_err());
    }

    #[test]
    fn version_1_fixture_still_loads() {
        // Committed file written by format version 1. It must keep loading in every later
        // version (via `migrate`). Regenerate only if the fixture is missing.
        let path = format!(
            "{}/../../tests/fixtures/projects/v1_bracket.rcad",
            env!("CARGO_MANIFEST_DIR")
        );
        if !Path::new(&path).exists() {
            std::fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
            save(sample().doc(), None, None, &path).unwrap();
        }
        let p = load(&path).unwrap();
        let names: Vec<&str> = p.document.nodes.values().map(|n| n.name.as_str()).collect();
        assert!(
            names.contains(&"Bracket") && names.contains(&"part.stl") && names.contains(&"Box")
        );
        assert_eq!(p.document.roots.len(), 1);
    }
}
