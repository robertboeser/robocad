//! 3MF export: one object per top-level solid, with names and colours, units in mm.

use std::fmt::Write as _;
use std::io::{Seek, Write};
use std::path::Path;

use glam::DMat4;
use rc_kernel::{GeometryKernel, ManifoldKernel};
use rc_mesh::Mesh;

use crate::eval::{SolidCache, eval_solid};
use crate::{Document, NodeId, Role};

/// One exported object.
pub struct Object3mf {
    pub name: String,
    pub color: [u8; 3],
    pub mesh: Mesh,
}

/// Evaluates the visible, non-hole top-level nodes among `ids` into world-space objects.
/// Nodes that cannot be evaluated are skipped and reported.
pub fn collect_objects(
    doc: &Document,
    ids: &[NodeId],
    cache: &mut SolidCache,
) -> (Vec<Object3mf>, Vec<String>) {
    let k = ManifoldKernel;
    let mut objects = Vec::new();
    let mut warnings = Vec::new();
    for id in crate::editor::top_level(doc, ids) {
        let Some(node) = doc.get(id) else { continue };
        if !doc.is_effectively_visible(id) || node.role == Role::Hole {
            continue;
        }
        let parent = node.parent.map_or(DMat4::IDENTITY, |p| doc.world_matrix(p));
        match eval_solid(doc, id, cache) {
            Ok(s) => {
                let mesh = k.solid_to_mesh(&k.transform(&s, &(parent * node.transform.matrix())));
                if !mesh.is_empty() {
                    objects.push(Object3mf {
                        name: node.name.clone(),
                        color: node.color,
                        mesh,
                    });
                }
            }
            Err(e) => warnings.push(format!("{e} — not exported")),
        }
    }
    (objects, warnings)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The 3D model XML part.
pub fn model_xml(objects: &[Object3mf]) -> String {
    let mut x = String::with_capacity(1 << 16);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str("<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n");
    x.push_str(" <metadata name=\"Application\">robocad</metadata>\n <resources>\n  <basematerials id=\"1\">\n");
    for o in objects {
        let [r, g, b] = o.color;
        let _ = writeln!(
            x,
            "   <base name=\"{}\" displaycolor=\"#{r:02X}{g:02X}{b:02X}FF\"/>",
            escape(&o.name)
        );
    }
    x.push_str("  </basematerials>\n");
    for (i, o) in objects.iter().enumerate() {
        let _ = writeln!(
            x,
            "  <object id=\"{}\" type=\"model\" name=\"{}\" pid=\"1\" pindex=\"{i}\">\n   <mesh>\n    <vertices>",
            i + 2,
            escape(&o.name)
        );
        for p in &o.mesh.positions {
            let _ = writeln!(
                x,
                "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                p[0], p[1], p[2]
            );
        }
        x.push_str("    </vertices>\n    <triangles>\n");
        for t in &o.mesh.triangles {
            let _ = writeln!(
                x,
                "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
                t[0], t[1], t[2]
            );
        }
        x.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
    x.push_str(" </resources>\n <build>\n");
    for i in 0..objects.len() {
        let _ = writeln!(x, "  <item objectid=\"{}\"/>", i + 2);
    }
    x.push_str(" </build>\n</model>\n");
    x
}

/// Writes a 3MF package.
pub fn write_3mf<W: Write + Seek>(objects: &[Object3mf], w: W) -> std::io::Result<()> {
    let mut zip = zip::ZipWriter::new(w);
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let e = |e: zip::result::ZipError| std::io::Error::other(e.to_string());
    zip.start_file("[Content_Types].xml", opt).map_err(e)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>
"#)?;
    zip.start_file("_rels/.rels", opt).map_err(e)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>
"#)?;
    zip.start_file("3D/3dmodel.model", opt).map_err(e)?;
    zip.write_all(model_xml(objects).as_bytes())?;
    zip.finish().map_err(e)?;
    Ok(())
}

/// Exports `ids` to a 3MF file atomically. Returns warnings.
pub fn export_3mf(
    doc: &Document,
    ids: &[NodeId],
    path: impl AsRef<Path>,
) -> std::io::Result<(usize, Vec<String>)> {
    let mut cache = SolidCache::default();
    let (objects, warnings) = collect_objects(doc, ids, &mut cache);
    if objects.is_empty() {
        return Err(std::io::Error::other(
            "nothing to export: no visible solid objects",
        ));
    }
    let path = path.as_ref();
    let tmp = path.with_extension("3mf.tmp");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        write_3mf(&objects, &mut f)?;
        f.flush()?;
    }
    std::fs::rename(tmp, path)?;
    Ok((objects.len(), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Editor, PrimitiveKind};
    use glam::DVec3;

    #[test]
    fn exports_objects_with_colours() {
        let mut ed = Editor::default();
        for (i, k) in [PrimitiveKind::Box, PrimitiveKind::Sphere]
            .into_iter()
            .enumerate()
        {
            ed.apply(Command::AddPrimitive {
                primitive: k.default_primitive(),
                position: DVec3::X * (i as f64 * 30.0),
                color: [255, 0, i as u8],
            })
            .unwrap();
        }
        let roots = ed.doc().roots.clone();
        let (objs, w) = collect_objects(ed.doc(), &roots, &mut SolidCache::default());
        assert!(w.is_empty());
        assert_eq!(objs.len(), 2);
        let xml = model_xml(&objs);
        assert!(xml.contains("unit=\"millimeter\""));
        assert!(xml.contains("displaycolor=\"#FF0001FF\""));
        assert_eq!(xml.matches("<object ").count(), 2);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.3mf");
        let (n, _) = export_3mf(ed.doc(), &roots, &p).unwrap();
        assert_eq!(n, 2);
        assert!(std::fs::metadata(&p).unwrap().len() > 500);
    }
}
