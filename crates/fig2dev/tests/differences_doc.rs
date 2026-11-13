#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `docs/differences.md` lists every option this program takes that C
//! fig2dev 3.2.9a does not. The options are derived from the drivers: for
//! each language, every letter the driver takes (it does not answer
//! "unknown to ... driver") that C's option function for that language
//! (`c_common`) does not take. The test fails when a row is missing from
//! the page and prints the rows to paste.

use fig2dev::drivers::driver;
use fig2dev::languages::LANGUAGES;
use fig2dev::options::OPTIONS;
use fig2dev::Ctx;

#[path = "c_common/mod.rs"]
mod c_common;
use c_common::{C_OPTIONS, EVERY};

/// Drivers that take every letter without complaint, as C's do (`mp`
/// ignores the others silently): their letters beyond C can't be told.
const TAKE_ALL: &[&str] = &["mp"];

/// (language, letter) pairs the program takes beyond C.
fn beyond_c() -> Vec<(String, char)> {
    let mut langs: Vec<&str> = LANGUAGES.iter().map(|l| l.id).collect();
    for (l, _) in C_OPTIONS {
        if !langs.contains(l) {
            langs.push(l);
        }
    }
    let mut found = Vec::new();
    for lang in langs {
        if TAKE_ALL.contains(&lang) {
            continue;
        }
        let Some(mut d) = driver(lang) else { continue };
        let c = C_OPTIONS.iter().find(|(l, _)| *l == lang).map(|(_, s)| *s);
        for ch in ('0'..='9').chain('a'..='z').chain('A'..='Z') {
            // A bad value is another error than "unknown".
            let mut ctx = Ctx::default();
            // The bitmap drivers only warn about a letter they don't take.
            let taken = match d.option(&mut ctx, ch, Some("1")) {
                Ok(()) => !ctx.messages.iter().any(|m| m.contains("unknown to")),
                Err(e) => !e.contains("unknown to"),
            };
            // -D, -E, -K, -m, -s and -Z are fig2dev.c's own, taken by every language.
            let in_c = c.is_some_and(|s| s.contains(ch)) || EVERY.contains(ch) || "DEKmsZ".contains(ch);
            if taken && !in_c {
                found.push((lang.to_string(), ch));
            }
        }
    }
    found
}

/// The table's options for languages C's driver lacks. `pspdf` is the
/// library's EPS-and-PDF-together language, not one of the program's.
fn table_beyond_c() -> Vec<(String, char)> {
    let mut v = Vec::new();
    for o in OPTIONS {
        for l in o.langs {
            if *l != "pspdf" && !C_OPTIONS.iter().any(|(c, _)| c == l) {
                v.push((l.to_string(), o.letter));
            }
        }
    }
    v
}

#[test]
fn differences_page_lists_every_option_beyond_c() {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/differences.md"))
        .expect("docs/differences.md");
    let mut all = beyond_c();
    for p in table_beyond_c() {
        if !all.contains(&p) {
            all.push(p);
        }
    }
    all.sort();
    // A row is "| `lang` | `-X` | ... |".
    let missing: Vec<String> = all
        .iter()
        .filter(|(l, c)| !page.contains(&format!("| `{l}` | `-{c}` |")))
        .map(|(l, c)| format!("| `{l}` | `-{c}` |  |"))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/differences.md lacks these rows (options beyond C); add them:\n{}",
        missing.join("\n")
    );
    // Every row of the page names an option the program takes beyond C.
    let known: Vec<String> = all.iter().map(|(l, c)| format!("| `{l}` | `-{c}` |")).collect();
    let stale: Vec<&str> = page
        .lines()
        .filter(|l| l.starts_with("| `") && l.contains("| `-") && !known.iter().any(|k| l.starts_with(k.as_str())))
        .collect();
    assert!(
        stale.is_empty(),
        "rows of docs/differences.md for options that are not beyond C:\n{}",
        stale.join("\n")
    );
}
