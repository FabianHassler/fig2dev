#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `-E utf-16` (fig2dev.c `-E`, `textconvert.c`): the texts are converted
//! with iconv's `UTF-16`; the header, the paper size among it, is not.
//! Expected values are what fig2dev 3.2.9a (C, libiconv) writes, except
//! where noted.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

fn eps(encoding: &str, text: &[u8]) -> Result<String, String> {
    let mut fig = b"#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
4 0 0 50 -1 0 12 0.0 4 150 900 600 600 "
        .to_vec();
    fig.extend_from_slice(text);
    fig.extend_from_slice(b"\\001\n");
    let mut ctx = Ctx {
        lang: "eps".into(),
        input_encoding: Some(encoding.into()),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("eps").unwrap();
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, &fig).map_err(|e| format!("read {e}"))?;
    let out = convert(&mut ctx, dev.as_mut(), &objects)?.output;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn shown(s: &str) -> String {
    s.lines().find(|l| l.ends_with(" show")).unwrap_or_default().to_string()
}

#[test]
fn utf16_header_is_not_decoded() {
    // the paper size "Letter" must stay "Letter" (it was read as UTF-16)
    assert!(eps("utf-16", b"abc").is_ok());
}

#[test]
fn ascii_text_is_not_converted() {
    // fig2dev converts only texts with a byte beyond ASCII
    assert!(shown(&eps("utf-16", b"N-N.").unwrap()).contains("(N-N.) show"));
}

#[test]
fn byte_order_mark_and_default_order() {
    // big endian without a mark: C4 E4 is U+C4E4
    assert!(shown(&eps("utf-16", b"\xc4\xe4").unwrap()).contains("(\u{c4e4}) show"));
    // BE mark FE FF, LE mark FF FE (the marks are removed)
    assert!(shown(&eps("UTF-16", b"\xfe\xff\xc4\xe4").unwrap()).contains("(\u{c4e4}) show"));
    assert!(shown(&eps("utf16", b"\xff\xfe\xe4\xc4").unwrap()).contains("(\u{c4e4}) show"));
}

#[test]
fn a_lone_trailing_byte_is_dropped() {
    // C also converts the string's NUL as a second byte and writes U+E400
    assert!(shown(&eps("utf-16", b"\xc4\xe4\xe5").unwrap()).contains("(\u{c4e4}) show"));
}

#[test]
fn surrogate_pairs_combine() {
    // U+1F600 is D83D DE00
    assert!(shown(&eps("utf-16", b"\xd8\x3d\xde\x01").unwrap()).contains("(\u{1f601}) show"));
}
