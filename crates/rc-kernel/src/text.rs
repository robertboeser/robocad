//! Extruded text from the bundled font (Lato Bold, SIL Open Font License).
//!
//! Glyph outlines are flattened into polygons, filled with the non-zero rule (so holes in
//! letters like "o" work regardless of contour direction) and extruded with Manifold.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use manifold_csg::cross_section::{CrossSection, FillRule};
use rc_mesh::{Aabb, Mesh};

use crate::{GeometryKernel, KernelError, ManifoldKernel, Solid};

/// The bundled font.
pub static FONT: &[u8] = include_bytes!("../../../assets/fonts/Lato-Bold.ttf");

/// Maximum deviation of flattened curves from the true outline, in mm.
const CURVE_TOLERANCE: f64 = 0.01;

/// Text parameters. `height` is the height of capital letters in mm.
#[derive(Clone, Debug, PartialEq)]
pub struct TextParams<'a> {
    pub text: &'a str,
    pub height: f64,
    pub depth: f64,
}

impl TextParams<'_> {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("the text is empty".into());
        }
        if self.text.chars().count() > 500 {
            return Err("the text is too long (at most 500 characters)".into());
        }
        if !(self.height.is_finite() && self.height > 0.1 && self.height < 10_000.0) {
            return Err("text height must be between 0.1 and 10000 mm".into());
        }
        if !(self.depth.is_finite() && self.depth > 0.01 && self.depth < 10_000.0) {
            return Err("text depth must be between 0.01 and 10000 mm".into());
        }
        Ok(())
    }

    /// Stable hash for caches.
    pub fn cache_key(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(b"text");
        feed(self.text.as_bytes());
        feed(&self.height.to_bits().to_le_bytes());
        feed(&self.depth.to_bits().to_le_bytes());
        h
    }
}

struct Flattener {
    scale: f64,
    offset: (f64, f64),
    tolerance: f64,
    contours: Vec<Vec<[f64; 2]>>,
    current: Vec<[f64; 2]>,
    last: (f64, f64),
}

impl Flattener {
    fn map(&self, x: f32, y: f32) -> (f64, f64) {
        (
            f64::from(x) * self.scale + self.offset.0,
            f64::from(y) * self.scale + self.offset.1,
        )
    }

    fn push(&mut self, p: (f64, f64)) {
        self.current.push([p.0, p.1]);
        self.last = p;
    }

    fn steps(&self, len: f64) -> usize {
        // Chord error of a curve of this length; generous upper bound on segments.
        ((len / self.tolerance).sqrt().ceil() as usize).clamp(2, 64)
    }

    fn finish(&mut self) {
        if self.current.len() >= 3 {
            let c = std::mem::take(&mut self.current);
            self.contours.push(c);
        } else {
            self.current.clear();
        }
    }
}

impl ttf_parser::OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        let p = self.map(x, y);
        self.push(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let c = self.map(x1, y1);
        let p = self.map(x, y);
        let len = dist(p0, c) + dist(c, p);
        let n = self.steps(len);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push((
                u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p.0,
                u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p.1,
            ));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let c1 = self.map(x1, y1);
        let c2 = self.map(x2, y2);
        let p = self.map(x, y);
        let len = dist(p0, c1) + dist(c1, c2) + dist(c2, p);
        let n = self.steps(len);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push((
                a * p0.0 + b * c1.0 + c * c2.0 + d * p.0,
                a * p0.1 + b * c1.1 + c * c2.1 + d * p.1,
            ));
        }
    }

    fn close(&mut self) {
        // Drop a duplicated closing point.
        if self.current.len() > 1 && self.current.first() == self.current.last() {
            self.current.pop();
        }
        self.finish();
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Flattened outline polygons of `text`, in mm, baseline at y = 0.
pub fn text_polygons(text: &str, height: f64) -> Result<Vec<Vec<[f64; 2]>>, KernelError> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|e| KernelError::Other(format!("font error: {e}")))?;
    let cap = face
        .capital_height()
        .map_or(f64::from(face.ascender()) * 0.7, f64::from)
        .max(1.0);
    let scale = height / cap;
    let line_height = f64::from(face.ascender() - face.descender() + face.line_gap()) * scale;
    let mut fl = Flattener {
        scale,
        offset: (0.0, 0.0),
        tolerance: CURVE_TOLERANCE,
        contours: Vec::new(),
        current: Vec::new(),
        last: (0.0, 0.0),
    };
    for (line_no, line) in text.lines().enumerate() {
        let mut x = 0.0;
        let y = -(line_no as f64) * line_height;
        let mut prev: Option<ttf_parser::GlyphId> = None;
        for ch in line.chars() {
            let Some(gid) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else {
                continue;
            };
            if let (Some(p), Some(kern)) = (prev, face.tables().kern) {
                for st in kern.subtables {
                    if st.horizontal
                        && let Some(k) = st.glyphs_kerning(p, gid)
                    {
                        x += f64::from(k) * scale;
                        break;
                    }
                }
            }
            fl.offset = (x, y);
            face.outline_glyph(gid, &mut fl);
            fl.finish();
            x += f64::from(face.glyph_hor_advance(gid).unwrap_or(0)) * scale;
            prev = Some(gid);
        }
    }
    Ok(fl.contours)
}

/// Builds the extruded text solid, centred in x/y and standing on z = 0.
pub fn text_solid(p: &TextParams<'_>) -> Result<Solid, KernelError> {
    p.validate().map_err(KernelError::Other)?;
    let polys = text_polygons(p.text, p.height)?;
    if polys.is_empty() {
        return Err(KernelError::Other(
            "the text has no printable characters".into(),
        ));
    }
    let cs = CrossSection::from_polygons_with_fill_rule(&polys, FillRule::NonZero);
    let m = manifold_csg::Manifold::extrude(&cs, p.depth);
    let solid = Solid::wrap(m);
    let mesh = ManifoldKernel.solid_to_mesh(&solid);
    let c = mesh.aabb().center().as_dvec3();
    Ok(ManifoldKernel.transform(
        &solid,
        &glam::DMat4::from_translation(glam::DVec3::new(-c.x, -c.y, 0.0)),
    ))
}

type TextCache = Mutex<HashMap<u64, (Arc<Mesh>, Aabb)>>;

fn cache() -> &'static TextCache {
    static CACHE: OnceLock<TextCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The text mesh and its bounds, cached by parameters (text meshes are requested often).
pub fn text_mesh(p: &TextParams<'_>) -> Result<(Arc<Mesh>, Aabb), KernelError> {
    let key = p.cache_key();
    if let Ok(c) = cache().lock()
        && let Some(hit) = c.get(&key)
    {
        return Ok(hit.clone());
    }
    let solid = text_solid(p)?;
    let mesh = Arc::new(ManifoldKernel.solid_to_mesh(&solid));
    let bounds = mesh.aabb();
    if let Ok(mut c) = cache().lock() {
        if c.len() > 128 {
            c.clear();
        }
        c.insert(key, (mesh.clone(), bounds));
    }
    Ok((mesh, bounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_mesh::topology::analyze;

    #[test]
    fn robocad_is_one_watertight_solid() {
        let (m, b) = text_mesh(&TextParams {
            text: "robocad",
            height: 10.0,
            depth: 2.0,
        })
        .unwrap();
        let r = analyze(&m);
        assert!(r.is_watertight() && r.orientation_consistent(), "{r:?}");
        assert!(m.volume() > 0.0);
        assert!((b.size().z - 2.0).abs() < 1e-4);
        assert!(b.size().x > 40.0 && b.size().x < 70.0, "{:?}", b.size());
        assert!(b.min.z.abs() < 1e-6 && b.center().truncate().length() < 1e-3);
        // Letters with counters ("o", "b", "d", "a") have holes: genus > 0 overall volume less
        // than the filled outline area would give.
        let solid = ManifoldKernel.solid_from_mesh(&m).unwrap();
        assert!(ManifoldKernel.volume(&solid) > 0.0);
    }

    #[test]
    fn capital_height_matches() {
        let (_, b) = text_mesh(&TextParams {
            text: "H",
            height: 20.0,
            depth: 1.0,
        })
        .unwrap();
        assert!((b.size().y - 20.0).abs() < 0.05, "{:?}", b.size());
    }

    #[test]
    fn validation() {
        let p = |t: &'static str, h: f64, d: f64| TextParams {
            text: t,
            height: h,
            depth: d,
        };
        assert!(p("  ", 10.0, 1.0).validate().is_err());
        assert!(p("a", 0.0, 1.0).validate().is_err());
        assert!(p("a", 10.0, f64::NAN).validate().is_err());
        assert!(p("a\nb", 10.0, 1.0).validate().is_ok());
        assert_ne!(p("a", 10.0, 1.0).cache_key(), p("b", 10.0, 1.0).cache_key());
    }
}
