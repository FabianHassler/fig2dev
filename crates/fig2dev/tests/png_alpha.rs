#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! PNG pictures with an alpha channel are drawn with straight-alpha
//! "over" compositing on what lies below them (bitmaps, PDF), and PNG
//! exports are RGB unless a pixel is transparent.

use fig2dev::util::pdfpic::PdfPicture;
use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::{Path, PathBuf};

/// A 4x2 RGBA picture: the left half opaque white, the right half the
/// half transparent (95, 124, 0, 102).
fn picture_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 4, 2);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let mut data = Vec::new();
    for _ in 0..2 {
        for x in 0..4 {
            data.extend_from_slice(if x < 2 {
                &[255, 255, 255, 255]
            } else {
                &[95, 124, 0, 102]
            });
        }
    }
    w.write_image_data(&data).unwrap();
    w.finish().unwrap();
    std::fs::write(dir.join("alpha.png"), out).unwrap();
    dir
}

/// A light blue box with the picture over it, both 2 inches wide.
const FIG: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
0 32 #d8eef8\n\
2 2 0 0 0 32 60 -1 20 0.000 0 0 -1 0 0 5\n\t 0 0 2400 0 2400 1200 0 1200 0 0\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 alpha.png\n\t 0 0 2400 0 2400 1200 0 1200 0 0\n";

fn run(lang: &str, dir: &Path, options: &[(char, Option<&str>)]) -> Vec<u8> {
    let mut ctx = Ctx {
        lang: lang.into(),
        dir: Some(dir.to_path_buf()),
        ..Ctx::default()
    };
    let mut dev = drivers::driver(lang).unwrap();
    for (c, a) in options {
        if !ctx.general_option(*c, *a).unwrap() {
            dev.option(&mut ctx, *c, *a).unwrap();
        }
    }
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, FIG.as_bytes()).map_err(|_| "read").unwrap();
    convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
}

/// (width, height, color type, RGBA pixels) of a PNG.
fn decode(png_bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
    let mut r = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let f = r.next_frame(&mut buf).unwrap();
    let color = r.info().color_type;
    let n = if color == png::ColorType::Rgb { 3 } else { 4 };
    let mut rgba = Vec::new();
    for p in buf[..f.buffer_size()].chunks(n) {
        rgba.extend_from_slice(&[p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]);
    }
    (f.width, f.height, color, rgba)
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

#[test]
fn half_transparent_pixels_blend_with_what_lies_below() {
    let dir = picture_dir("png_alpha_raster");
    let (w, h, color, px) = decode(&run("png", &dir, &[]));
    assert_eq!(color, png::ColorType::Rgb, "an opaque export has no alpha channel");
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    // straight alpha "over" the box: c * a + box * (1 - a), box (216, 238, 248)
    let over = at(w * 3 / 4, h / 2);
    assert!(close(over, [168, 193, 149]), "{over:?}");
    // an opaque white pixel of a picture with alpha stays white (it was
    // taken for the transparent colour)
    let white = at(w / 4, h / 2);
    assert!(close(white, [255, 255, 255]), "{white:?}");
}

#[test]
fn pdf_picture_alpha_is_a_soft_mask() {
    let dir = picture_dir("png_alpha_pdf");
    let pdf = run("pdf", &dir, &[]);
    assert!(String::from_utf8_lossy(&pdf).contains("/SMask"));
    let (w, h) = (96u32, 48u32);
    let img = PdfPicture::open(pdf).unwrap().render(w, h).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    };
    let over = at(w * 3 / 4, h / 2);
    assert!(close(over, [168, 193, 149]), "{over:?}");
    let white = at(w / 4, h / 2);
    assert!(close(white, [255, 255, 255]), "{white:?}");
}

#[test]
fn png_export_is_rgb_and_deterministic() {
    let dir = picture_dir("png_alpha_export");
    let opaque = run("png", &dir, &[]);
    assert_eq!(decode(&opaque).2, png::ColorType::Rgb);
    // deterministic
    assert_eq!(opaque, run("png", &dir, &[]));
}
