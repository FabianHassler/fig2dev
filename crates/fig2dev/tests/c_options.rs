#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The options C fig2dev 3.2.9a accepts for each language, transcribed from
//! its sources, against what this crate takes. A letter C accepts must not
//! be unknown to the Rust driver, and in the option table (`options.rs`,
//! the one the scene languages and the docs use) it must be present and
//! `Support::Ported`.

use fig2dev::drivers::{driver, scene::options_lang};
use fig2dev::options::{option_spec, Support, OPTIONS};
use fig2dev::Ctx;

#[path = "c_common/mod.rs"]
mod c_common;
use c_common::{C_OPTIONS, EVERY};

/// The languages whose options the table in `options.rs` describes.
fn in_table(lang: &str) -> bool {
    matches!(
        lang,
        "ps" | "eps"
            | "pdf"
            | "pdftex"
            | "pstex"
            | "svg"
            | "gif"
            | "jpeg"
            | "pcx"
            | "png"
            | "ppm"
            | "sld"
            | "tiff"
            | "xbm"
            | "xpm"
    )
}

#[test]
fn every_c_option_is_taken() {
    let mut missing = Vec::new();
    for &(lang, letters) in C_OPTIONS {
        let letters = format!("{letters}{EVERY}");
        for c in letters.chars() {
            let mut d = driver(lang).unwrap_or_else(|| panic!("no driver for {lang}"));
            let mut ctx = Ctx::default();
            // Any argument: a bad value is another error than "unknown".
            let r = d.option(&mut ctx, c, Some("1"));
            if let Err(e) = r {
                if e.contains("unknown to") {
                    missing.push(format!("{lang} -{c}: {e}"));
                }
            }
            if in_table(lang) && !EVERY.contains(c) {
                match option_spec(options_lang(lang), &c.to_string()).filter(|s| s.letter == c) {
                    None => missing.push(format!("{lang} -{c}: not in OPTIONS")),
                    Some(s) if s.support != Support::Ported => {
                        missing.push(format!("{lang} -{c}: {:?}", s.support));
                    }
                    Some(_) => {}
                }
            }
        }
    }
    assert!(missing.is_empty(), "C options missing in Rust:\n{}", missing.join("\n"));
}

/// The converse for the table: no option listed for a language that C's
/// driver does not take (its letters would be a made-up option).
#[test]
fn table_has_no_option_c_lacks() {
    let mut extra = Vec::new();
    for o in OPTIONS {
        let langs: Vec<&str> = if o.langs.is_empty() {
            C_OPTIONS.iter().map(|(l, _)| *l).filter(|l| in_table(l)).collect()
        } else {
            o.langs.to_vec()
        };
        for lang in langs {
            // pspdf is RustFig's own (EPS and PDF together).
            if lang == "pspdf" {
                continue;
            }
            let Some((_, letters)) = C_OPTIONS.iter().find(|(l, _)| *l == lang) else {
                extra.push(format!("{lang}: not a C language"));
                continue;
            };
            // -D, -K, -m, -s, -Z and -G are fig2dev.c's own; the rest is the driver's.
            if "DKmsZG".contains(o.letter) {
                continue;
            }
            if !letters.contains(o.letter) {
                extra.push(format!("{lang} -{}", o.letter));
            }
        }
    }
    assert!(extra.is_empty(), "options C does not take: {}", extra.join(", "));
}

/// Every option of the table is implemented, none only planned.
#[test]
fn nothing_is_planned() {
    for o in OPTIONS {
        assert_eq!(o.support, Support::Ported, "-{} ({})", o.letter, o.key);
    }
}
