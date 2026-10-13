//! Minimal Wavefront OBJ reader (geometry only: `v` and `f`; polygons are fan-triangulated).

use std::path::Path;

use crate::Mesh;
use crate::weld::weld_exact;

#[derive(Debug, thiserror::Error)]
pub enum ObjError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid OBJ at line {line}: {message}")]
    Invalid { line: usize, message: String },
    #[error("the file contains no faces")]
    NoFaces,
}

/// Reads an OBJ file. Returns the welded mesh and warnings.
pub fn read_file(path: impl AsRef<Path>) -> Result<(Mesh, Vec<String>), ObjError> {
    let bytes = std::fs::read(path)?;
    read_bytes(&bytes)
}

pub fn read_bytes(bytes: &[u8]) -> Result<(Mesh, Vec<String>), ObjError> {
    let text = String::from_utf8_lossy(bytes);
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut soup: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut skipped = 0usize;
    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let mut it = raw.split_whitespace();
        match it.next() {
            Some("v") => {
                let mut p = [0f32; 3];
                for c in &mut p {
                    *c = it.next().and_then(|t| t.parse().ok()).ok_or_else(|| {
                        ObjError::Invalid {
                            line,
                            message: "vertex needs 3 numbers".into(),
                        }
                    })?;
                }
                verts.push(p);
            }
            Some("f") => {
                let mut idx = Vec::new();
                for t in it {
                    let first = t.split('/').next().unwrap_or("");
                    let i: i64 = first.parse().map_err(|_| ObjError::Invalid {
                        line,
                        message: format!("bad face index '{t}'"),
                    })?;
                    // 1-based; negative indices count from the end.
                    let resolved = if i < 0 { verts.len() as i64 + i } else { i - 1 };
                    if resolved < 0 || resolved as usize >= verts.len() {
                        return Err(ObjError::Invalid {
                            line,
                            message: format!("face index {i} out of range"),
                        });
                    }
                    idx.push(resolved as usize);
                }
                if idx.len() < 3 {
                    skipped += 1;
                    continue;
                }
                for k in 1..idx.len() - 1 {
                    soup.push([verts[idx[0]], verts[idx[k]], verts[idx[k + 1]]]);
                }
            }
            _ => {} // normals, texture coordinates, groups, materials: ignored
        }
    }
    if soup.is_empty() {
        return Err(ObjError::NoFaces);
    }
    let (mesh, dropped) = weld_exact(&soup);
    let mut warnings = Vec::new();
    if skipped > 0 {
        warnings.push(format!(
            "skipped {skipped} faces with fewer than 3 vertices"
        ));
    }
    if dropped > 0 {
        warnings.push(format!("dropped {dropped} faces with invalid coordinates"));
    }
    Ok((mesh, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::analyze;

    #[test]
    fn cube_with_quads_and_slashes() {
        let src = b"# cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nvn 0 0 1\n\
f 1/1/1 4/1/1 3/1/1 2/1/1\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf -1 -5 -8 -4\n";
        let (m, w) = read_bytes(src).unwrap();
        assert!(w.is_empty());
        assert_eq!(m.triangle_count(), 12);
        let r = analyze(&m);
        assert!(r.is_clean_solid(), "{r:?}");
        assert!((m.volume() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn errors() {
        assert!(matches!(read_bytes(b"v 0 0 0\n"), Err(ObjError::NoFaces)));
        assert!(matches!(
            read_bytes(b"v 0 0 0\nf 1 2 3\n"),
            Err(ObjError::Invalid { line: 2, .. })
        ));
    }
}
