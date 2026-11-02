#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Transparency of TIFF and PNG pictures (alpha channel, `tRNS`): partly
//! transparent pixels blend with what lies below them, exactly the
//! transparent pixels (not others of the same visible colour) let it show
//! through.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A light blue box (216, 238, 248) with the picture over it, 2400 x 1200.
fn fig(file: &str) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
0 32 #d8eef8\n\
2 2 0 0 0 32 60 -1 20 0.000 0 0 -1 0 0 5\n\t 0 0 2400 0 2400 1200 0 1200 0 0\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 {file}\n\t 0 0 2400 0 2400 1200 0 1200 0 0\n"
    )
}

fn run(lang: &str, dir: &Path, file: &str) -> Vec<u8> {
    let mut ctx = Ctx {
        lang: lang.into(),
        dir: Some(dir.to_path_buf()),
        ..Ctx::default()
    };
    let mut dev = drivers::driver(lang).unwrap();
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, fig(file).as_bytes()).map_err(|_| "read").unwrap();
    convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
}

/// (width, height, color type, RGB pixels) of a PNG.
fn decode(png_bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<[u8; 3]>) {
    let mut r = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let f = r.next_frame(&mut buf).unwrap();
    let color = r.info().color_type;
    let n = if color == png::ColorType::Rgb { 3 } else { 4 };
    let px = buf[..f.buffer_size()].chunks(n).map(|p| [p[0], p[1], p[2]]).collect();
    (f.width, f.height, color, px)
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 4)
}

/// The pixel in the middle of picture column `col` of `cols`.
fn column(png_bytes: &[u8], col: u32, cols: u32) -> [u8; 3] {
    let (w, h, _, px) = decode(png_bytes);
    px[((h / 2) * w + (col * 2 + 1) * w / (cols * 2)) as usize]
}

const BOX: [u8; 3] = [216, 238, 248];

fn over(c: [u8; 3], a: u32) -> [u8; 3] {
    [0, 1, 2].map(|k| ((u32::from(c[k]) * a + u32::from(BOX[k]) * (255 - a) + 127) / 255) as u8)
}

fn write_png(
    path: &Path,
    (w, h): (u32, u32),
    color: png::ColorType,
    depth: png::BitDepth,
    palette: Option<Vec<u8>>,
    trns: Option<Vec<u8>>,
    data: &[u8],
) {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    if let Some(p) = palette {
        enc.set_palette(p);
    }
    if let Some(t) = trns {
        enc.set_trns(t);
    }
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(data).unwrap();
    wr.finish().unwrap();
    std::fs::write(path, out).unwrap();
}

/// 4x2 RGBA TIFF: left half opaque white, right half (95, 124, 0, 102);
/// with `premultiplied` the samples are stored premultiplied and the
/// ExtraSamples tag says so.
fn write_tiff(path: &Path, premultiplied: bool) {
    use tiff::encoder::{colortype::RGB8, TiffEncoder};
    let mut data = Vec::new();
    for _ in 0..2 {
        for x in 0..4 {
            data.extend_from_slice(&if x < 2 {
                [255, 255, 255, 255]
            } else if premultiplied {
                [
                    ((95u32 * 102 + 127) / 255) as u8,
                    ((124u32 * 102 + 127) / 255) as u8,
                    0,
                    102,
                ]
            } else {
                [95, 124, 0, 102]
            });
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    let mut enc = TiffEncoder::new(&mut out).unwrap();
    let mut image = enc.new_image::<RGB8>(4, 2).unwrap();
    let kind = if premultiplied {
        tiff::tags::ExtraSamples::AssociatedAlpha
    } else {
        tiff::tags::ExtraSamples::UnassociatedAlpha
    };
    image.extra_samples(&[kind]).unwrap();
    image.write_data(&data).unwrap();
    let bytes = out.into_inner();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn tiff_alpha_blends_in_bitmaps_and_pdf() {
    for premultiplied in [false, true] {
        let d = dir(if premultiplied { "tiff_alpha_pre" } else { "tiff_alpha" });
        write_tiff(&d.join("a.tif"), premultiplied);
        let png = run("png", &d, "a.tif");
        assert_eq!(decode(&png).2, png::ColorType::Rgb);
        let blended = column(&png, 3, 4);
        assert!(close(blended, over([95, 124, 0], 102)), "{premultiplied} {blended:?}");
        assert!(close(column(&png, 0, 4), [255, 255, 255]));
        let pdf = run("pdf", &d, "a.tif");
        assert!(String::from_utf8_lossy(&pdf).contains("/SMask"));
        // PostScript has no alpha: composited over white, no soft mask
        let eps = run("eps", &d, "a.tif");
        assert!(!String::from_utf8_lossy(&eps).contains("/SMask"));
    }
}

#[test]
fn tiff_without_alpha_is_unchanged() {
    use tiff::encoder::{colortype::RGB8, TiffEncoder};
    let d = dir("tiff_opaque");
    let mut out = std::io::Cursor::new(Vec::new());
    let data: Vec<u8> = (0..8).flat_map(|_| [95u8, 124, 0]).collect();
    TiffEncoder::new(&mut out)
        .unwrap()
        .write_image::<RGB8>(4, 2, &data)
        .unwrap();
    std::fs::write(d.join("o.tif"), out.into_inner()).unwrap();
    let png = run("png", &d, "o.tif");
    assert!(close(column(&png, 2, 4), [95, 124, 0]));
    assert!(!String::from_utf8_lossy(&run("pdf", &d, "o.tif")).contains("/SMask"));
}

#[test]
fn palette_trns_is_per_entry_alpha() {
    let d = dir("png_palette_trns");
    // entries 0 and 1 are both red; 0 is transparent, 1 opaque; 2 is half
    // transparent green
    let plte = vec![255, 0, 0, 255, 0, 0, 0, 160, 0];
    write_png(
        &d.join("p.png"),
        (4, 1),
        png::ColorType::Indexed,
        png::BitDepth::Eight,
        Some(plte),
        Some(vec![0, 255, 128]),
        &[0, 1, 2, 1],
    );
    for lang in ["png", "pdf"] {
        let out = run(lang, &d, "p.png");
        if lang == "pdf" {
            assert!(String::from_utf8_lossy(&out).contains("/SMask"));
            continue;
        }
        assert!(
            close(column(&out, 0, 4), BOX),
            "transparent entry {:?} {:?} {:?}",
            column(&out, 0, 4),
            column(&out, 1, 4),
            column(&out, 2, 4)
        );
        assert!(close(column(&out, 1, 4), [255, 0, 0]), "opaque red stays opaque");
        assert!(close(column(&out, 2, 4), over([0, 160, 0], 128)), "half transparent");
        assert!(close(column(&out, 3, 4), [255, 0, 0]));
    }
}

#[test]
fn grey_and_rgb_trns_make_exactly_that_colour_transparent() {
    // 16-bit grey: 0x1234 is transparent, 0x1235 is not, though both show
    // as grey 0x12
    let d = dir("png_grey_trns");
    let mut data = Vec::new();
    for v in [0x1234u16, 0x1235, 0x1234, 0x8000] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_png(
        &d.join("g.png"),
        (4, 1),
        png::ColorType::Grayscale,
        png::BitDepth::Sixteen,
        None,
        Some(vec![0x12, 0x34]),
        &data,
    );
    let out = run("png", &d, "g.png");
    assert!(close(column(&out, 0, 4), BOX));
    let opaque = column(&out, 1, 4);
    assert!(opaque.iter().all(|&v| v < 40), "{opaque:?}");
    assert!(close(column(&out, 2, 4), BOX));
    assert!(column(&out, 3, 4)[0] > 100 && column(&out, 3, 4)[0] < 160);

    // 8-bit RGB: pure red transparent, near red and blue opaque
    let d = dir("png_rgb_trns");
    let data = [255, 0, 0, 254, 0, 0, 0, 0, 255, 255, 0, 0];
    write_png(
        &d.join("c.png"),
        (4, 1),
        png::ColorType::Rgb,
        png::BitDepth::Eight,
        None,
        Some(vec![0, 255, 0, 0, 0, 0]),
        &data,
    );
    let out = run("png", &d, "c.png");
    assert!(close(column(&out, 0, 4), BOX));
    assert!(close(column(&out, 1, 4), [254, 0, 0]));
    assert!(close(column(&out, 2, 4), [0, 0, 255]));
    assert!(close(column(&out, 3, 4), BOX));
}

#[test]
fn sixteen_bit_png_with_alpha_blends() {
    let d = dir("png_rgba16");
    let mut data = Vec::new();
    for _ in 0..4 {
        for v in [95u16 * 257, 124 * 257, 0, 102 * 257] {
            data.extend_from_slice(&v.to_be_bytes());
        }
    }
    write_png(
        &d.join("q.png"),
        (4, 1),
        png::ColorType::Rgba,
        png::BitDepth::Sixteen,
        None,
        None,
        &data,
    );
    let out = run("png", &d, "q.png");
    let c = column(&out, 2, 4);
    assert!(close(c, over([95, 124, 0], 102)), "{c:?}");
    // grey + alpha, 16 bit
    let mut data = Vec::new();
    for _ in 0..4 {
        for v in [0u16, 102 * 257] {
            data.extend_from_slice(&v.to_be_bytes());
        }
    }
    write_png(
        &d.join("ga.png"),
        (4, 1),
        png::ColorType::GrayscaleAlpha,
        png::BitDepth::Sixteen,
        None,
        None,
        &data,
    );
    let c = column(&run("png", &d, "ga.png"), 1, 4);
    assert!(close(c, over([0, 0, 0], 102)), "{c:?}");
}
