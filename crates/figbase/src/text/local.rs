//! Installed fonts named by a text's comment (`# rustfig-font: <Family Style>`,
//! RustFig plan F7).
//!
//! Fig 3.2 knows fonts only as numbers; RustFig keeps the family of an
//! installed font beside the nearest PostScript font number. A driver that
//! can use the family looks it up here, and falls back to the Fig number
//! when it is not installed.

use super::{Builder, Metrics, PathOp, Shaped, FIG_PER_POINT, LINE_SPACING, MAX_CHARS};
use fontdb::{Database, Style, Weight};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ttf_parser::{Face, GlyphId, Tag};

/// The comment prefix (without the `# `).
pub const COMMENT_PREFIX: &str = "rustfig-font:";

/// Largest font file taken in (bytes).
const MAX_FONT_BYTES: usize = 128 << 20;

/// The font name a comment line carries, if it is a `rustfig-font:` line.
pub fn parse_comment(line: &str) -> Option<&str> {
    let l = line.trim();
    let l = l.strip_prefix('#').map_or(l, str::trim_start);
    let name = l.strip_prefix(COMMENT_PREFIX)?.trim();
    (!name.is_empty()).then_some(name)
}

/// An installed font face, with its file's bytes.
#[derive(Debug)]
pub struct LocalFont {
    /// The name asked for, as the installed fonts spell it (`Family Style`).
    pub name: String,
    pub family: String,
    pub post_script_name: String,
    pub data: Vec<u8>,
    /// Face number in a collection.
    pub index: u32,
    /// OpenType with CFF outlines (else TrueType `glyf`).
    pub cff: bool,
    /// The font's `fsType` lets a subset be embedded in a document, and the
    /// subsetter can read it.
    pub embeddable: bool,
    pub bold: bool,
    pub italic: bool,
}

impl LocalFont {
    pub fn face(&self) -> Option<Face<'_>> {
        Face::parse(&self.data, self.index).ok()
    }

    /// The glyph of `ch` (0, `.notdef`, when the font has none).
    pub fn glyph(&self, face: &Face<'_>, ch: char) -> u16 {
        face.glyph_index(ch).map_or(0, |g| g.0)
    }

    /// Advance of a glyph in 1/1000 em.
    pub fn advance_1000(&self, face: &Face<'_>, gid: u16) -> f64 {
        let upm = f64::from(face.units_per_em().max(1));
        f64::from(face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0)) * 1000.0 / upm
    }

    /// Glyph outlines and widths, laid out as figbase lays out any font
    /// (without kerning: embedded text uses the same advances).
    pub fn shape(&self, size_pt: f64, text: &str) -> Shaped {
        let size = if size_pt.is_finite() {
            size_pt.clamp(0.1, 5000.0)
        } else {
            12.0
        };
        let em = size * FIG_PER_POINT;
        let mut out = Shaped::default();
        let Some(face) = self.face() else {
            return out;
        };
        let scale = em / f64::from(face.units_per_em().max(1));
        let mut count = 0usize;
        let mut y = 0.0;
        let mut nlines = 0usize;
        for line in text.split('\n') {
            nlines += 1;
            let mut ops: Vec<PathOp> = Vec::new();
            let mut x = 0.0;
            for ch in line.chars() {
                count += 1;
                if count > MAX_CHARS {
                    break;
                }
                let gid = GlyphId(self.glyph(&face, ch));
                let mut b = Builder {
                    ops: &mut ops,
                    scale,
                    dx: x,
                    dy: y,
                    skew: 0.0,
                };
                let _ = face.outline_glyph(gid, &mut b);
                x += f64::from(face.glyph_hor_advance(gid).unwrap_or(0)) * scale;
            }
            out.line_widths.push(x);
            out.lines.push(ops);
            y += em * LINE_SPACING;
        }
        let nlines = nlines.max(1);
        out.metrics = Metrics {
            width: out.line_widths.iter().copied().fold(0.0, f64::max),
            ascent: f64::from(face.ascender()) * scale,
            descent: -f64::from(face.descender()) * scale + (nlines - 1) as f64 * em * LINE_SPACING,
            lines: nlines,
        };
        out
    }
}

type Cache = HashMap<String, Option<Arc<LocalFont>>>;

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static C: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn clear_cache() {
    cache().clear();
}

fn weight_label(w: u16) -> &'static str {
    match w {
        0..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "",
        451..=550 => "Medium",
        551..=650 => "Semibold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    }
}

/// `Family` for the regular face, `Family Label` for the others (the name
/// RustFig's chooser stores).
fn face_names(info: &fontdb::FaceInfo) -> Vec<String> {
    let mut parts: Vec<&str> = Vec::new();
    let w = weight_label(info.weight.0);
    if !w.is_empty() {
        parts.push(w);
    }
    match info.stretch {
        fontdb::Stretch::Normal => {}
        fontdb::Stretch::UltraCondensed | fontdb::Stretch::ExtraCondensed => parts.push("ExtraCondensed"),
        fontdb::Stretch::Condensed | fontdb::Stretch::SemiCondensed => parts.push("Condensed"),
        _ => parts.push("Expanded"),
    }
    if info.style != Style::Normal {
        parts.push(if info.style == Style::Oblique {
            "Oblique"
        } else {
            "Italic"
        });
    }
    info.families
        .iter()
        .map(|(family, _)| {
            if parts.is_empty() {
                family.clone()
            } else {
                format!("{family} {}", parts.join(" "))
            }
        })
        .collect()
}

fn find_id(db: &Database, name: &str) -> Option<fontdb::ID> {
    let want = name.trim().to_lowercase();
    db.faces()
        .filter(|info| face_names(info).iter().any(|n| n.to_lowercase() == want))
        .min_by_key(|info| info.id.to_string())
        .map(|info| info.id)
}

fn fs_type(face: &Face<'_>) -> Option<u16> {
    let t = face.raw_face().table(Tag::from_bytes(b"OS/2"))?;
    let b = t.get(8..10)?;
    Some(u16::from_be_bytes([*b.first()?, *b.get(1)?]))
}

fn load(name: &str) -> Option<Arc<LocalFont>> {
    super::sysfonts::with_db(|db| {
        let id = find_id(db, name)?;
        let info = db.face(id)?;
        let (data, index) = db.with_face_data(id, |d, i| (d.to_vec(), i))?;
        if data.len() > MAX_FONT_BYTES {
            return None;
        }
        let face = Face::parse(&data, index).ok()?;
        let tables = face.raw_face();
        let cff = tables.table(Tag::from_bytes(b"CFF ")).is_some();
        let cff2 = tables.table(Tag::from_bytes(b"CFF2")).is_some();
        // fsType: 0x0002 restricted license, 0x0100 no subsetting, 0x0200 bitmaps only.
        let permitted = fs_type(&face).is_none_or(|t| t & 0x0302 == 0);
        let glyf = tables.table(Tag::from_bytes(b"glyf")).is_some();
        let embeddable = permitted && !cff2 && (cff || glyf);
        let family = info.families.first().map(|(f, _)| f.clone()).unwrap_or_default();
        let font = LocalFont {
            name: name.trim().to_string(),
            family,
            post_script_name: info.post_script_name.clone(),
            cff,
            embeddable,
            bold: info.weight >= Weight::SEMIBOLD,
            italic: info.style != Style::Normal,
            index,
            data,
        };
        Some(Arc::new(font))
    })
    .flatten()
}

/// The installed face a `rustfig-font:` name denotes (`None` when it is not
/// installed, or with the built-in fonts only).
pub fn find_local_font(name: &str) -> Option<Arc<LocalFont>> {
    if super::builtin_fonts() {
        return None;
    }
    let key = name.trim().to_lowercase();
    if let Some(hit) = cache().get(&key) {
        return hit.clone();
    }
    let found = load(name);
    let mut c = cache();
    if c.len() > 256 {
        c.clear();
    }
    c.insert(key, found.clone());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_comment() {
        assert_eq!(parse_comment("# rustfig-font: Foo Bold"), Some("Foo Bold"));
        assert_eq!(parse_comment("rustfig-font:Foo"), Some("Foo"));
        assert_eq!(parse_comment("rustfig-font:  "), None);
        assert_eq!(parse_comment("other"), None);
    }

    #[test]
    fn missing_family_is_none() {
        assert!(find_local_font("No Such Family Anywhere").is_none());
    }
}
