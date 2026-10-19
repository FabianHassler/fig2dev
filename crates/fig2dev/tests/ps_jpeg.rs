#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! JPEG pictures in PostScript output: genps passes the DCT data through,
//! ASCII85-encoded (readjpg.c `JPEGtoPS`, encode.c), and reports a bad
//! header with readjpg.c's messages.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::PathBuf;

fn fig(file: &str, flipped: u8) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t {flipped} {file}\n\t 1200 1200 4200 1200 4200 3200 1200 3200 1200 1200\n"
    )
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ps-jpeg-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn jpeg(w: u32, h: u32, color: image::ExtendedColorType, samples: usize) -> Vec<u8> {
    let px: Vec<u8> = (0..w as usize * h as usize * samples)
        .map(|i| (i * 7 % 256) as u8)
        .collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .encode(&px, w, h, color)
        .unwrap();
    out
}

/// (output, messages)
fn run(dir: &std::path::Path, file: &str, flipped: u8) -> (String, Vec<String>) {
    let mut ctx = Ctx {
        lang: "eps".into(),
        dir: Some(dir.to_path_buf()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver("eps").unwrap();
    let objects = readfp_fig(&mut ctx, fig(file, flipped).as_bytes())
        .map_err(|_| "read")
        .unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    (String::from_utf8(out).unwrap(), ctx.messages)
}

/// The bytes the ASCII85 data between the DCT filter and `~>` stand for.
fn decode85(s: &str) -> Vec<u8> {
    let body: String = s
        .split("} exec\n")
        .nth(1)
        .unwrap()
        .split("~>")
        .next()
        .unwrap()
        .replace('\n', "");
    let mut out = Vec::new();
    let mut group: Vec<u32> = Vec::new();
    let flush = |g: &mut Vec<u32>, out: &mut Vec<u8>| {
        let n = g.len();
        while g.len() < 5 {
            g.push(84);
        }
        let w = g.iter().fold(0u32, |w, &d| w.wrapping_mul(85).wrapping_add(d));
        out.extend_from_slice(&w.to_be_bytes()[..n - 1]);
        g.clear();
    };
    for c in body.bytes() {
        if c == b'z' {
            out.extend_from_slice(&[0; 4]);
        } else {
            group.push(u32::from(c - b'!'));
            if group.len() == 5 {
                flush(&mut group, &mut out);
            }
        }
    }
    if group.len() > 1 {
        flush(&mut group, &mut out);
    }
    out
}

#[test]
fn the_dct_data_is_passed_through() {
    let d = dir("pass");
    for (w, h, color, samples, space) in [
        (37, 23, image::ExtendedColorType::Rgb8, 3, "RGB"),
        (5, 70, image::ExtendedColorType::L8, 1, "Gray"),
        (64, 3, image::ExtendedColorType::Rgb8, 3, "RGB"),
    ] {
        let data = jpeg(w, h, color, samples);
        std::fs::write(d.join("a.jpg"), &data).unwrap();
        let (ps, msgs) = run(&d, "a.jpg", 0);
        assert!(msgs.is_empty(), "{msgs:?}");
        assert!(ps.contains("% Begin Imported JPEG File: a.jpg\n\n"));
        assert!(ps.contains(&format!("% JPEG image follows:\n{w} {h} sc\n%!PS-Adobe-3.0 EPSF-3.0\n")));
        assert!(ps.contains("%%Creator: jpeg2ps by Thomas Merz in fig2dev 3.2.9a\n%%Title: a.jpg\n"));
        assert!(ps.contains(&format!("%%BoundingBox: 0 0 {w} {h}\n")));
        assert!(ps.contains(&format!("/Device{space} setcolorspace\n")));
        assert!(ps.contains(&format!(
            "\t/Width {w}\n\t/Height {h}\n\t/ImageMatrix [ {w} 0 0 -{h} 0 {h} ]\n"
        )));
        assert!(ps.contains("\t/BitsPerComponent 8\n"));
        assert!(ps.contains("/Data RawData << >> /DCTDecode filter def\n"));
        assert!(ps.contains("/RawData currentfile /ASCII85Decode filter def\n"));
        assert_eq!(decode85(&ps), data);
        assert!(ps.contains("~>\nrestore grestore\n"));
        // no pixels
        assert!(!ps.contains("colorimage"));
    }
}

#[test]
fn hostile_headers_give_readjpg_messages() {
    let d = dir("bad");
    let app0 = b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0";
    let sof = |c: u8, w: u8, h: u8| [0xff, 0xc0, 0, 8 + 3 * c, 8, 0, h, 0, w, c];
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (
            [&app0[..], b"\xff\xd9"].concat(),
            "Error: DNL marker not supported in PostScript Level 2!",
        ),
        (
            [&app0[..], &sof(2, 3, 3), b"\xff\xd9"].concat(),
            "Error: unknown color space (2 components)!\n",
        ),
        (
            [&app0[..], &sof(3, 0, 3), b"\xff\xd9"].concat(),
            "Error: DNL marker not supported in PostScript Level 2!",
        ),
        (
            [&app0[..], b"\xff\xdb\0\x43\x01"].concat(),
            "Error: unexpected end of JPEG file!",
        ),
        // C loops forever on these two
        ([&app0[..], b"abc"].concat(), "Error: unexpected end of JPEG file!"),
        (
            [&app0[..], b"\xff\xdb\0\0xx"].concat(),
            "Error: unexpected end of JPEG file!",
        ),
    ];
    for (data, msg) in cases {
        std::fs::write(d.join("b.jpg"), &data).unwrap();
        let (ps, msgs) = run(&d, "b.jpg", 0);
        assert_eq!(msgs, vec![msg.to_string(), "b.jpg: Bad JPEG format".to_string()]);
        assert!(!ps.contains("DCTDecode"));
    }
    // C accepts only JFIF and Exif headers
    std::fs::write(d.join("c.jpg"), b"\xff\xd8\xff\xdb\0\x02").unwrap();
    assert_eq!(run(&d, "c.jpg", 0).1, vec!["c.jpg: Unknown image format".to_string()]);
}

#[test]
fn cmyk_adobe_files_are_decoded_inverted() {
    let d = dir("cmyk");
    let adobe = [&b"\xff\xee\0\x0eAdobe\0d\0\0\0\0\0"[..]].concat();
    let sof = [
        0xff, 0xc0, 0, 20, 8, 0, 4, 0, 5, 4, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 4, 0x11, 0,
    ];
    let app0 = b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0";
    for (extra, decode) in [(&adobe[..], "[1 0 1 0 1 0 1 0]"), (&[][..], "[0 1 0 1 0 1 0 1]")] {
        std::fs::write(d.join("k.jpg"), [&app0[..], extra, &sof, b"\xff\xd9"].concat()).unwrap();
        let (ps, msgs) = run(&d, "k.jpg", 0);
        assert!(msgs.is_empty());
        assert!(ps.contains("/DeviceCMYK setcolorspace\n"));
        assert!(ps.contains(&format!("\t  /Decode {decode}\n")), "{ps}");
    }
}

#[test]
fn flipped_pictures_are_rotated_like_other_pictures() {
    let d = dir("flip");
    std::fs::write(d.join("a.jpg"), jpeg(6, 4, image::ExtendedColorType::Rgb8, 3)).unwrap();
    let (ps, _) = run(&d, "a.jpg", 1);
    assert!(ps.contains("4 0 translate\n270 rotate\n1 -1 scale\n"), "{ps}");
}
