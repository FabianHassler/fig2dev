//! PDF files as picture objects: the first page of the file, kept as a
//! vector picture.
//!
//! fig2dev's C program has no PDF reader of its own: `genps.c` has
//! `pdftops` (readeps.c) turn the file into EPS, and the PDF and bitmap
//! languages get the result through ghostscript. Here the file is read with
//! the `hayro` crates (pure Rust, no unsafe code):
//!
//! - [`PdfPicture::xobject`]: the page as a Form XObject for the PDF writer
//!   (`hayro-write`; stays vector, fonts and images are copied);
//! - [`PdfPicture::render`]: the page as pixels for the bitmap languages
//!   and the canvas;
//! - [`PdfPicture::to_eps`]: the page as EPS for the PostScript writers
//!   (the `pdftops` step of C; see [`super::pdfps`]).
//!
//! Every call into the PDF crates is guarded: a hostile file gives `None`,
//! never a panic (on wasm a panic aborts, so the crates must not panic
//! there; the guard is the extra line of defence on native).
//
// Ported from fig2dev dev/readeps.c (`read_pdf`, `pdftops`) (xfig licence,
// see LICENSE-xfig); the page box comes from the PDF reader instead of
// scanning for `/MediaBox` or running ghostscript.

use figbase::image::RasterImage;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use hayro_write::pdf_writer::{Chunk, Ref};
use hayro_write::{ChunkSettings, ExtractionQuery};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Largest edge of a rendered page, in pixels (the pixmap's size is a
/// `u16`).
pub const MAX_RENDER_EDGE: u32 = 16_000;
/// Largest rendered page, in pixels.
pub const MAX_RENDER_PIXELS: u64 = 1 << 27;

/// Run `f`, turning a panic of the PDF crates into `None`.
pub(crate) fn guarded<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok().flatten()
}

/// The first page of a PDF file.
#[derive(Clone)]
pub struct PdfPicture {
    data: Arc<Vec<u8>>,
    /// A hash of the file, to tell pictures apart.
    id: u64,
    /// The page's size in points (rotation and crop box applied).
    width: f64,
    height: f64,
    /// Rendered in grey (fig2dev's `-N`); the file is the same.
    gray: bool,
}

impl std::fmt::Debug for PdfPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PdfPicture({} bytes, {} x {} pt)",
            self.data.len(),
            self.width,
            self.height
        )
    }
}

impl PartialEq for PdfPicture {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) || self.data == other.data
    }
}

impl PdfPicture {
    /// Reads `data` as a PDF file; `None` when it is not one the PDF
    /// reader can open or its first page has no usable size.
    pub fn open(data: Vec<u8>) -> Option<PdfPicture> {
        if data.len() > super::readpics::MAX_PICFILE_BYTES {
            return None;
        }
        let data = Arc::new(data);
        let (width, height) = guarded(|| {
            let pdf = Pdf::new(data.clone()).ok()?;
            let page = pdf.pages().first()?;
            let (w, h) = page.render_dimensions();
            let (w, h) = (f64::from(w), f64::from(h));
            (w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0 && w <= 1.0e6 && h <= 1.0e6).then_some((w, h))
        })?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&*data, &mut h);
        let id = std::hash::Hasher::finish(&h);
        Some(PdfPicture {
            data,
            id,
            width,
            height,
            gray: false,
        })
    }

    /// The same page, rendered in grey levels (`-N`).
    pub fn grayed(&self) -> PdfPicture {
        PdfPicture {
            gray: true,
            id: self.id ^ 0x6772_6179,
            ..self.clone()
        }
    }

    /// The page's size in points.
    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    /// A hash of the file's bytes (equal files, equal ids).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The file's bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Runs `f` on the parsed file.
    pub(crate) fn with_pdf<T>(&self, f: impl FnOnce(&Pdf) -> Option<T>) -> Option<T> {
        guarded(|| {
            let pdf = Pdf::new(self.data.clone()).ok()?;
            f(&pdf)
        })
    }

    /// The page as a Form XObject: the objects to add to the output
    /// (numbered from `first_id` on) and the number of the last one
    /// used, with the XObject's reference. Its coordinate system is the
    /// page's size with the origin at the lower left corner.
    pub fn xobject(&self, first_id: i32) -> Option<(Chunk, Ref, i32)> {
        self.with_pdf(|pdf| {
            let mut next = first_id.max(1);
            let result = hayro_write::extract(
                pdf,
                Box::new(|| {
                    let r = Ref::new(next);
                    next = next.saturating_add(1).min(i32::MAX - 1);
                    r
                }),
                ChunkSettings::default(),
                &[ExtractionQuery::new_xobject(0)],
            )
            .ok()?;
            let root = result.root_refs.first()?.as_ref().ok().copied()?;
            Some((result.chunk, root, next))
        })
    }

    /// The page as `width` x `height` pixels (straight alpha, transparent
    /// where nothing is drawn); `None` when it can't be rendered or the
    /// size is out of range.
    pub fn render(&self, width: u32, height: u32) -> Option<RasterImage> {
        if width == 0
            || height == 0
            || width > MAX_RENDER_EDGE
            || height > MAX_RENDER_EDGE
            || u64::from(width) * u64::from(height) > MAX_RENDER_PIXELS
        {
            return None;
        }
        self.with_pdf(|pdf| {
            let page = pdf.pages().first()?;
            let (pw, ph) = page.render_dimensions();
            let settings = PixmapSettings {
                x_scale: (width as f32 + 0.999) / pw,
                y_scale: (height as f32 + 0.999) / ph,
                ..PixmapSettings::default()
            };
            let cache = RenderCache::new();
            let pm = hayro::render(
                page,
                &cache,
                &InterpreterSettings::default(),
                &RenderSettings::default(),
                &settings,
            );
            let (w, h) = (u32::from(pm.width()), u32::from(pm.height()));
            if w == 0 || h == 0 {
                return None;
            }
            let mut rgba = pm.take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::Alpha);
            if self.gray {
                for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
                    // the luminance of fig2dev's `-N` colours
                    let v = (0.3 * f64::from(px[0]) + 0.59 * f64::from(px[1]) + 0.11 * f64::from(px[2])).round() as u8;
                    px[..3].fill(v);
                }
            }
            (rgba.len() == (w as usize) * (h as usize) * 4).then_some(RasterImage {
                width: w,
                height: h,
                rgba,
            })
        })
    }

    /// The page as EPS (the `pdftops` of fig2dev's C code), its bounding
    /// box `0 0 W H` with W and H the page's size rounded up; `None` when
    /// the page can't be converted.
    pub fn to_eps(&self) -> Option<super::pdfps::PdfEps> {
        super::pdfps::to_eps(self)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-page PDF with a red triangle, written by hand.
    pub(crate) fn sample_pdf() -> Vec<u8> {
        let content = b"1 0 0 rg 10 10 m 90 10 l 50 90 l f 0 0 1 RG 4 w 10 10 80 80 re S";
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        let objs: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << >> >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
                content,
                b"\nendstream",
            ]
            .concat(),
        ];
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn grayed_page_renders_in_grey() {
        let p = PdfPicture::open(sample_pdf()).expect("sample opens");
        let color = p.render(20, 20).expect("renders");
        assert!(color.rgba.as_chunks::<4>().0.iter().any(|px| px[0] != px[2]));
        let gray = p.grayed().render(20, 20).expect("renders");
        assert!(gray
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[0] == px[1] && px[1] == px[2]));
        assert_ne!(p.id(), p.grayed().id());
    }

    #[test]
    fn opens_and_renders() {
        let p = PdfPicture::open(sample_pdf()).expect("sample opens");
        assert_eq!(p.size(), (100.0, 100.0));
        let img = p.render(50, 50).expect("renders");
        assert!(img.width >= 50 && img.height >= 50);
        // the triangle's middle is red, the corner is empty
        let at = |x: u32, y: u32| {
            let i = ((y * img.width + x) * 4) as usize;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
        };
        assert_eq!(at(25, 35), [255, 0, 0, 255]);
        assert_eq!(at(1, 1)[3], 0);
    }

    #[test]
    fn xobject_is_extracted() {
        let p = PdfPicture::open(sample_pdf()).expect("sample opens");
        let (chunk, _root, next) = p.xobject(10).expect("xobject");
        assert!(chunk.len() > 0 && next > 10);
    }

    #[test]
    fn hostile_data_is_refused() {
        assert!(PdfPicture::open(Vec::new()).is_none());
        assert!(PdfPicture::open(b"%PDF-1.4\n garbage".to_vec()).is_none());
        let mut bad = sample_pdf();
        bad.truncate(bad.len() / 2);
        let _ = PdfPicture::open(bad);
    }
}
