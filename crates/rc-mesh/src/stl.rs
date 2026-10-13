//! STL reading and writing (binary and ASCII).
//!
//! Stored facet normals are ignored on import (many exporters write garbage) and recomputed on
//! export. STL has no units; robocad treats values as millimetres.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use rayon::prelude::*;

use crate::Mesh;
use crate::weld::weld_exact;

const HEADER_LEN: usize = 80;
const RECORD_LEN: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum StlError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("the file is empty")]
    Empty,
    #[error("the file is truncated: header announces {expected} triangles but only {found} fit")]
    Truncated { expected: u64, found: u64 },
    #[error("invalid ASCII STL at line {line}: {message}")]
    InvalidAscii { line: usize, message: String },
    #[error("the file contains no triangles")]
    NoTriangles,
    #[error("the mesh is too large ({0} vertices exceed the 32-bit index limit)")]
    TooLarge(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StlFormat {
    Binary,
    Ascii,
}

/// Result of reading an STL file.
#[derive(Clone, Debug)]
pub struct StlFile {
    pub mesh: Mesh,
    pub format: StlFormat,
    /// Binary header text or ASCII solid name (trimmed, lossy UTF-8).
    pub name: String,
    /// Triangles in the file before cleaning.
    pub triangles_in_file: usize,
    /// Triangles dropped because they contained NaN or infinite coordinates.
    pub dropped_non_finite: usize,
    /// Non-fatal notes, e.g. trailing bytes after the last binary record.
    pub warnings: Vec<String>,
}

/// Reads an STL file from disk.
///
/// The file is read into memory in one call; reading is I/O bound and far cheaper than parsing,
/// and it avoids `unsafe` memory mapping (another process could truncate a mapped file).
pub fn read_file(path: impl AsRef<Path>) -> Result<StlFile, StlError> {
    let bytes = std::fs::read(path)?;
    read_bytes_slice(&bytes)
}

/// Detects the format of STL data.
///
/// The `solid` prefix is not trusted: many binary files start with it. A file is binary when its
/// size matches the triangle count in the header exactly; otherwise ASCII is assumed if it starts
/// with `solid`, else binary.
pub fn detect_format(bytes: &[u8]) -> StlFormat {
    if bytes.len() >= HEADER_LEN + 4 {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as u64;
        if (HEADER_LEN as u64 + 4 + count * RECORD_LEN as u64) == bytes.len() as u64 {
            return StlFormat::Binary;
        }
    }
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let head = &bytes[start..bytes.len().min(start + 5)];
    if head.eq_ignore_ascii_case(b"solid") && looks_textual(&bytes[start..]) {
        StlFormat::Ascii
    } else {
        StlFormat::Binary
    }
}

fn looks_textual(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .take(1024)
        .all(|&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
}

/// Parses STL data held in memory.
pub fn read_bytes_slice(bytes: &[u8]) -> Result<StlFile, StlError> {
    if bytes.is_empty() {
        return Err(StlError::Empty);
    }
    let (soup, format, name, mut warnings) = match detect_format(bytes) {
        StlFormat::Binary => {
            let (soup, name, warnings) = parse_binary(bytes)?;
            (soup, StlFormat::Binary, name, warnings)
        }
        StlFormat::Ascii => {
            let (soup, name) = parse_ascii(bytes)?;
            (soup, StlFormat::Ascii, name, Vec::new())
        }
    };
    if soup.is_empty() {
        return Err(StlError::NoTriangles);
    }
    let triangles_in_file = soup.len();
    let (mesh, dropped_non_finite) = weld_exact(&soup);
    if mesh.positions.len() > u32::MAX as usize {
        return Err(StlError::TooLarge(mesh.positions.len()));
    }
    if dropped_non_finite > 0 {
        warnings.push(format!(
            "dropped {dropped_non_finite} triangles with invalid (NaN/infinite) coordinates"
        ));
    }
    if mesh.is_empty() {
        return Err(StlError::NoTriangles);
    }
    Ok(StlFile {
        mesh,
        format,
        name,
        triangles_in_file,
        dropped_non_finite,
        warnings,
    })
}

type Soup = Vec<[[f32; 3]; 3]>;

fn parse_binary(bytes: &[u8]) -> Result<(Soup, String, Vec<String>), StlError> {
    if bytes.len() < HEADER_LEN + 4 {
        return Err(StlError::Truncated {
            expected: 0,
            found: 0,
        });
    }
    let name = String::from_utf8_lossy(&bytes[..HEADER_LEN])
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as u64;
    let body = &bytes[HEADER_LEN + 4..];
    let fit = (body.len() / RECORD_LEN) as u64;
    if fit < count {
        return Err(StlError::Truncated {
            expected: count,
            found: fit,
        });
    }
    let mut warnings = Vec::new();
    let used = count as usize * RECORD_LEN;
    if body.len() > used {
        warnings.push(format!(
            "ignored {} trailing bytes after the last triangle",
            body.len() - used
        ));
    }
    let soup: Soup = body[..used]
        .par_chunks_exact(RECORD_LEN)
        .with_min_len(4096)
        .map(|rec| {
            let f = |o: usize| f32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
            // Skip the 12-byte normal; read the 3 vertices.
            [
                [f(12), f(16), f(20)],
                [f(24), f(28), f(32)],
                [f(36), f(40), f(44)],
            ]
        })
        .collect();
    Ok((soup, name, warnings))
}

fn parse_ascii(bytes: &[u8]) -> Result<(Soup, String), StlError> {
    let mut soup = Vec::new();
    let mut name = String::new();
    let mut facet: Vec<[f32; 3]> = Vec::with_capacity(3);
    let mut in_facet = false;
    for (line_no, raw) in bytes.split(|&b| b == b'\n').enumerate() {
        let line_no = line_no + 1;
        let mut tokens = raw.split(u8::is_ascii_whitespace).filter(|t| !t.is_empty());
        let Some(first) = tokens.next() else { continue };
        let kw = |k: &[u8]| first.eq_ignore_ascii_case(k);
        if kw(b"vertex") {
            let mut v = [0.0f32; 3];
            for c in &mut v {
                let tok = tokens.next().ok_or_else(|| StlError::InvalidAscii {
                    line: line_no,
                    message: "vertex needs 3 coordinates".into(),
                })?;
                *c = fast_float2::parse(tok).map_err(|_| StlError::InvalidAscii {
                    line: line_no,
                    message: format!("invalid number '{}'", String::from_utf8_lossy(tok)),
                })?;
            }
            if !in_facet {
                return Err(StlError::InvalidAscii {
                    line: line_no,
                    message: "vertex outside of a facet".into(),
                });
            }
            facet.push(v);
        } else if kw(b"facet") {
            in_facet = true;
            facet.clear();
        } else if kw(b"endfacet") {
            // Fan-triangulate polygons with more than 3 vertices; ignore degenerate facets.
            for i in 1..facet.len().saturating_sub(1) {
                soup.push([facet[0], facet[i], facet[i + 1]]);
            }
            facet.clear();
            in_facet = false;
        } else if kw(b"solid") && name.is_empty() {
            let rest = raw.get(raw.len().min(first.len() + leading_ws(raw))..);
            name = String::from_utf8_lossy(rest.unwrap_or_default())
                .trim()
                .to_string();
        }
        // outer loop / endloop / endsolid are structural and ignored.
    }
    Ok((soup, name))
}

fn leading_ws(raw: &[u8]) -> usize {
    raw.iter().take_while(|b| b.is_ascii_whitespace()).count()
}

/// Computes the unit normal of a triangle for export.
fn normal(t: [[f32; 3]; 3]) -> [f32; 3] {
    let [a, b, c] = t.map(|p| glam::Vec3::from_array(p).as_dvec3());
    (b - a)
        .cross(c - a)
        .normalize_or_zero()
        .as_vec3()
        .to_array()
}

/// Writes a binary STL to any writer.
pub fn write_binary<W: Write>(mesh: &Mesh, header: &str, mut w: W) -> Result<(), StlError> {
    let mut head = [0u8; HEADER_LEN];
    // A binary header must not start with "solid" or naive readers think it is ASCII.
    let header = if header
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("solid")
    {
        format!("robocad {header}")
    } else {
        header.to_string()
    };
    let n = header.len().min(HEADER_LEN);
    head[..n].copy_from_slice(&header.as_bytes()[..n]);
    w.write_all(&head)?;
    let count = u32::try_from(mesh.triangles.len()).map_err(|_| StlError::TooLarge(0))?;
    w.write_all(&count.to_le_bytes())?;
    let mut buf = Vec::with_capacity(RECORD_LEN * 4096);
    for i in 0..mesh.triangles.len() {
        let t = mesh.triangle_positions(i);
        for f in normal(t).iter().chain(t.iter().flatten()) {
            buf.extend_from_slice(&f.to_le_bytes());
        }
        buf.extend_from_slice(&[0, 0]);
        if buf.len() >= RECORD_LEN * 4096 {
            w.write_all(&buf)?;
            buf.clear();
        }
    }
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

/// Writes an ASCII STL to any writer. Coordinates are written with round-trip precision.
pub fn write_ascii<W: Write>(mesh: &Mesh, name: &str, w: W) -> Result<(), StlError> {
    let mut w = BufWriter::new(w);
    let name = name.replace(['\n', '\r'], " ");
    writeln!(w, "solid {name}")?;
    for i in 0..mesh.triangles.len() {
        let t = mesh.triangle_positions(i);
        let n = normal(t);
        writeln!(w, "  facet normal {} {} {}", n[0], n[1], n[2])?;
        writeln!(w, "    outer loop")?;
        for v in t {
            writeln!(w, "      vertex {} {} {}", v[0], v[1], v[2])?;
        }
        writeln!(w, "    endloop")?;
        writeln!(w, "  endfacet")?;
    }
    writeln!(w, "endsolid {name}")?;
    w.flush()?;
    Ok(())
}

/// Writes an STL file atomically (temporary file + rename).
pub fn write_file(
    mesh: &Mesh,
    path: impl AsRef<Path>,
    format: StlFormat,
    name: &str,
) -> Result<(), StlError> {
    let path = path.as_ref();
    let tmp = path.with_extension("stl.tmp");
    {
        let file = BufWriter::new(File::create(&tmp)?);
        match format {
            StlFormat::Binary => write_binary(mesh, name, file)?,
            StlFormat::Ascii => write_ascii(mesh, name, file)?,
        }
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn cube() -> Mesh {
        Mesh::cuboid(Vec3::ZERO, Vec3::splat(10.0))
    }

    #[test]
    fn binary_round_trip() {
        let mut buf = Vec::new();
        write_binary(&cube(), "test", &mut buf).unwrap();
        assert_eq!(buf.len(), 84 + 12 * 50);
        let f = read_bytes_slice(&buf).unwrap();
        assert_eq!(f.format, StlFormat::Binary);
        assert_eq!(f.mesh, crate::weld::reweld_exact(&cube()));
        assert_eq!(f.name, "test");
    }

    #[test]
    fn ascii_round_trip() {
        let mut buf = Vec::new();
        write_ascii(&cube(), "cube", &mut buf).unwrap();
        let f = read_bytes_slice(&buf).unwrap();
        assert_eq!(f.format, StlFormat::Ascii);
        assert_eq!(f.mesh, crate::weld::reweld_exact(&cube()));
        assert_eq!(f.name, "cube");
    }

    #[test]
    fn binary_header_with_solid_is_prefixed() {
        let mut buf = Vec::new();
        write_binary(&cube(), "solid thing", &mut buf).unwrap();
        assert!(buf.starts_with(b"robocad solid thing"));
    }

    #[test]
    fn empty_is_error() {
        assert!(matches!(read_bytes_slice(&[]), Err(StlError::Empty)));
    }

    #[test]
    fn ascii_polygon_facet_is_fanned() {
        let src = b"solid q\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 1 1 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid q\n";
        let f = read_bytes_slice(src).unwrap();
        assert_eq!(f.mesh.triangles.len(), 2);
    }

    #[test]
    fn ascii_bad_number() {
        let src = b"solid q\nfacet normal 0 0 1\nouter loop\nvertex 0 x 0\n";
        assert!(matches!(
            read_bytes_slice(src),
            Err(StlError::InvalidAscii { line: 4, .. })
        ));
    }
}
