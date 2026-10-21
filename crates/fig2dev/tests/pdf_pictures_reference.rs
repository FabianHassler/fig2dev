#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! PDF and EPS pictures against the C fig2dev (`/opt/homebrew/bin/fig2dev`,
//! or `FIG2DEV_REFERENCE`), with ghostscript and its `pdftops`: both
//! programs' PostScript, PDF and PNG are rasterised by ghostscript (the
//! PNGs are read directly) and compared pixel by pixel. Skipped when the
//! reference program or ghostscript is missing.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdf_pictures")
}

fn reference() -> Option<PathBuf> {
    let p = std::env::var_os("FIG2DEV_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/homebrew/bin/fig2dev"));
    p.is_file().then_some(p)
}

fn fig(file: &str, flipped: u8) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t {flipped} {file}\n\t 1200 1200 3600 1200 3600 3600 1200 3600 1200 1200\n"
    )
}

fn ours(lang: &str, src: &str) -> Vec<u8> {
    let mut ctx = Ctx {
        lang: lang.into(),
        dir: Some(fixtures()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver(lang).unwrap();
    let objects = readfp_fig(&mut ctx, src.as_bytes()).map_err(|_| "read").unwrap();
    convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
}

fn theirs(exe: &Path, lang: &str, src: &str, tmp: &Path) -> Vec<u8> {
    let f = tmp.join("in.fig");
    std::fs::write(&f, src).unwrap();
    let out = Command::new(exe)
        .current_dir(fixtures())
        .args(["-L", lang])
        .arg(&f)
        .output()
        .unwrap();
    out.stdout
}

/// ghostscript's rendering of a PS or PDF file at 40 dpi
fn gs_render(data: &[u8], ext: &str, tmp: &Path, name: &str) -> image::RgbaImage {
    let input = tmp.join(format!("{name}.{ext}"));
    let png = tmp.join(format!("{name}.png"));
    std::fs::write(&input, data).unwrap();
    let st = Command::new("gs")
        .args(["-q", "-dNOPAUSE", "-dBATCH", "-sDEVICE=png16m", "-r40", "-o"])
        .arg(&png)
        .arg(&input)
        .status()
        .unwrap();
    assert!(st.success());
    image::open(&png).unwrap().to_rgba8()
}

/// Mean absolute difference per channel, 0 to 255. PDF pages are measured
/// from the lower left corner and the programs round the page up
/// differently, so the images are laid on each other at the bottom.
fn diff(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    let (ya, yb) = (a.height() - h, b.height() - h);
    let mut sum = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.get_pixel(x, ya + y).0, b.get_pixel(x, yb + y).0);
            for k in 0..3 {
                sum += u64::from(p[k].abs_diff(q[k]));
            }
        }
    }
    sum as f64 / (u64::from(w) * u64::from(h) * 3) as f64
}

#[test]
fn pictures_look_like_the_c_programs() {
    let Some(exe) = reference() else {
        eprintln!("skipped: no reference fig2dev");
        return;
    };
    if Command::new("gs").arg("-v").output().is_err() {
        eprintln!("skipped: no ghostscript");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("pdfpic-ref-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // the C program looks for pictures next to the figure
    for f in ["rich.pdf", "shade.pdf", "rich.eps"] {
        std::fs::copy(fixtures().join(f), tmp.join(f)).unwrap();
    }
    let mut report = String::new();
    for file in ["rich.pdf", "shade.pdf", "rich.eps"] {
        for flipped in [0u8, 1] {
            let src = fig(file, flipped);
            for lang in ["ps", "pdf", "png"] {
                let (a, b) = (ours(lang, &src), theirs(&exe, lang, &src, &tmp));
                let (ra, rb) = if lang == "png" {
                    (
                        image::load_from_memory(&a).unwrap().to_rgba8(),
                        image::load_from_memory(&b).unwrap().to_rgba8(),
                    )
                } else {
                    (gs_render(&a, lang, &tmp, "ours"), gs_render(&b, lang, &tmp, "theirs"))
                };
                let d = diff(&ra, &rb);
                report.push_str(&format!("{file} flipped={flipped} {lang}: {d:.2}\n"));
                // a missing or wrongly placed picture changes the image by tens;
                // bitmaps differ in antialiasing and hinting from ghostscript's
                let limit = if lang == "png" { 25.0 } else { 6.0 };
                assert!(
                    d < limit,
                    "{file} flipped={flipped} {lang}: mean difference {d:.2}\n{report}"
                );
            }
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
    eprintln!("{report}");
}
