#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The figure's LaTeX preamble (`# RustFigLaTeXPreamble` comment lines,
//! written by RustFig): the output C fig2dev does not have (complete
//! documents of `tikz`, `pict2e`, `pstricks` with `-P`; `asy`) carries it,
//! everything C writes stays as C writes it.

mod tex_common;

use tex_common::run_with;

const PREAMBLE: &str = "# RustFigLaTeXPreamble \\usepackage{amsmath}\n\
# RustFigLaTeXPreamble\n\
# RustFigLaTeXPreamble \\newcommand{\\R}{\\mathbb{R}}\n";

const BODY: &str = "4 0 0 50 -1 0 12 0.0000 2 135 300 1200 1200 $x\\\\in\\\\R$\\001\n";

fn fig(preamble: bool) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n{}1200 2\n{BODY}",
        if preamble { PREAMBLE } else { "" }
    )
}

fn out(lang: &str, preamble: bool, opts: &[(char, Option<&str>)]) -> String {
    let (out, msgs, exit) = run_with(lang, fig(preamble).as_bytes(), None, opts, false).unwrap();
    assert_eq!(exit, 0, "{lang}: {msgs:?}");
    out
}

/// The preamble's lines, in order, between `after` and `\begin{document}`.
fn has_preamble(out: &str, after: &str) {
    let start = out.find(after).unwrap_or_else(|| panic!("{after} in {out}"));
    let doc = out.find("\\begin{document}").unwrap();
    let head = &out[start..doc];
    let a = head
        .find("\n\\usepackage{amsmath}\n")
        .unwrap_or_else(|| panic!("{head}"));
    let r = head
        .find("\n\\newcommand{\\R}{\\mathbb{R}}\n")
        .unwrap_or_else(|| panic!("{head}"));
    assert!(a < r, "{head}");
}

#[test]
fn complete_documents_carry_the_preamble() {
    has_preamble(&out("tikz", true, &[('P', None)]), "\\usepackage{tikz}");
    has_preamble(
        &out("pict2e", true, &[('P', None)]),
        "\\usepackage{pict2e,graphics,color}",
    );
    has_preamble(&out("pstricks", true, &[('P', None)]), "\\usepackage{pstricks");
}

#[test]
fn asy_gets_texpreamble() {
    let o = out("asy", true, &[]);
    assert!(o.contains("\ntexpreamble('\\\\usepackage{amsmath}');\n"), "{o}");
    assert!(
        o.contains("\ntexpreamble('\\\\newcommand{\\\\R}{\\\\mathbb{R}}');\n"),
        "{o}"
    );
    assert!(!o.contains("texpreamble('')"), "{o}");
    assert!(!out("asy", false, &[]).contains("texpreamble"));
}

/// What C fig2dev writes: the preamble lines are comments only (as C
/// prints every whole-figure comment), the rest is the output of the same
/// figure without them.
#[test]
fn c_outputs_only_show_the_comments() {
    for (lang, opts) in [
        ("tikz", &[][..]),
        ("pict2e", &[][..]),
        ("pstricks", &[][..]),
        ("latex", &[][..]),
        ("pstex_t", &[][..]),
        ("pdftex_t", &[][..]),
        ("epic", &[][..]),
        ("eepic", &[][..]),
        ("mp", &[][..]),
    ] {
        let with = out(lang, true, opts);
        let without = out(lang, false, opts);
        let kept: String = with
            .split_inclusive('\n')
            .filter(|l| !l.contains("RustFigLaTeXPreamble") && l.trim() != "%" && l.trim() != "--")
            .collect();
        let base: String = without
            .split_inclusive('\n')
            .filter(|l| l.trim() != "%" && l.trim() != "--")
            .collect();
        assert_eq!(kept, base, "{lang}");
    }
}
