//! Installed fonts in the output (RustFig plan F7): subsets of a font named by
//! a text's `rustfig-font:` comment, and the pieces drivers need to embed
//! them.

use figbase::text::LocalFont;
use std::sync::Arc;
use subsetter::GlyphRemapper;

/// The glyphs of one font that a document uses, numbered from 1 in order of
/// first use (0 is `.notdef`).
pub(crate) struct GlyphSet {
    pub font: Arc<LocalFont>,
    remap: GlyphRemapper,
    /// (new glyph id, character, advance in 1/1000 em)
    pub used: Vec<(u16, char, f64)>,
}

impl GlyphSet {
    pub fn new(font: Arc<LocalFont>) -> GlyphSet {
        let mut remap = GlyphRemapper::new();
        remap.remap(0);
        GlyphSet {
            font,
            remap,
            used: Vec::new(),
        }
    }

    /// The new glyph id and advance of `ch`, registering it. `None` when the
    /// font can't be read, or the set holds `max` glyphs already.
    pub fn code(&mut self, ch: char, max: usize) -> Option<(u16, f64)> {
        if let Some(&(g, _, a)) = self.used.iter().find(|u| u.1 == ch) {
            return Some((g, a));
        }
        if self.used.len() >= max {
            return None;
        }
        let face = self.font.face()?;
        let old = self.font.glyph(&face, ch);
        let adv = self.font.advance_1000(&face, old);
        let new = self.remap.remap(old);
        self.used.push((new, ch, adv));
        Some((new, adv))
    }

    /// The font reduced to the used glyphs (glyph ids renumbered, no `cmap`).
    pub fn subset(&self) -> Option<Vec<u8>> {
        subsetter::subset(&self.font.data, self.font.index, &self.remap).ok()
    }
}

/// The `sfnts` strings of a PostScript Type 42 font: the font split at table
/// boundaries. `None` when a table is too long for a PostScript string.
pub(crate) fn sfnts_strings(data: &[u8]) -> Option<Vec<&[u8]>> {
    let be16 = |o: usize| -> Option<usize> {
        let b = data.get(o..o + 2)?;
        Some(usize::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
    };
    let be32 = |o: usize| -> Option<usize> {
        let b = data.get(o..o + 4)?;
        usize::try_from(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?])).ok()
    };
    let n = be16(4)?;
    let dir_end = 12 + 16 * n;
    let mut offsets: Vec<usize> = Vec::new();
    for i in 0..n {
        offsets.push(be32(12 + 16 * i + 8)?);
    }
    offsets.sort_unstable();
    offsets.dedup();
    let mut cuts = vec![0, dir_end];
    cuts.extend(offsets.into_iter().filter(|&o| o > dir_end && o <= data.len()));
    cuts.push(data.len());
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (*w.first()?, *w.get(1)?);
        let piece = data.get(a..b)?;
        if piece.len() > 65_000 {
            return None;
        }
        out.push(piece);
    }
    Some(out)
}

/// A subset's tag for the PDF font name (`ABCDEF+`), from the used glyphs.
pub(crate) fn subset_tag(name: &str, used: &[(u16, char, f64)]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name
        .bytes()
        .chain(used.iter().flat_map(|u| u.1.to_string().into_bytes()))
    {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + u8::try_from((h >> (i * 5)) % 26).unwrap_or(0)))
        .collect()
}
