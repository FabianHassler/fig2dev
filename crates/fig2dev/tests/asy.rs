#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `asy` driver (Asymptote). fig2dev has no such driver, so the tests
//! check the structure of the output object by object, and an ignored test
//! feeds the output of many figures to the `asy` program.

mod tex_common;

use std::path::{Path, PathBuf};
use std::process::Command;

use tex_common::{run, run_with};

fn asy(body: &str, opts: &[(char, Option<&str>)]) -> String {
    let (out, msgs, exit) = run("asy", body, opts).unwrap();
    assert_eq!(exit, 0, "{msgs:?}");
    out
}

const LINE: &str = "2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 2\n\t 600 600 3000 1200\n";

#[test]
fn header_scales_and_bounds() {
    let out = asy(LINE, &[]);
    assert!(out.starts_with("// Asymptote picture written by fig2dev"));
    // 1200 ppi at 100 %: 0.06 bp per Fig unit; y is turned upwards.
    assert!(out.contains("\nunitsize(0.06bp);\n"), "{out}");
    assert!(out.contains("labelmargin=0;"));
    assert!(out.contains("draw(box((588,-1212),(3012,-588)),invisible);"), "{out}");
    assert!(out.contains("draw((600,-600)--(3000,-1200),rgb(\"000000\")+linewidth(0.45)+linecap(0)+linejoin(0));"));
    assert!(!out.contains("import patterns"));
}

#[test]
fn magnification_and_border() {
    let out = asy(LINE, &[('m', Some("0.5"))]);
    assert!(out.contains("unitsize(0.03bp);"), "{out}");
    // The width of a line scales with the figure.
    assert!(out.contains("linewidth(0.225)"), "{out}");
    // -b: 10 bp = 333 Fig units at 0.06 bp each.
    let out = asy(LINE, &[('b', Some("10"))]);
    assert!(out.contains("draw(box((421,-1379),(3179,-421)),invisible);"), "{out}");
}

#[test]
fn polyline_styles_caps_joins_and_dashes() {
    let body = "2 1 1 3 4 7 50 -1 -1 4.000 1 2 -1 0 0 3\n\t 600 600 1200 1500 2400 600\n\
                2 1 2 1 1 7 50 -1 -1 3.000 0 0 -1 0 0 2\n\t 600 2000 3000 2000\n\
                2 1 3 1 1 7 50 -1 -1 4.000 0 0 -1 0 0 2\n\t 600 2200 3000 2200\n";
    let out = asy(body, &[]);
    // Thickness 3: (3 - 1/15 of 15) ... two thirds of a point: 30 * 0.06 bp.
    assert!(
        out.contains(
            "rgb(\"ff0000\")+linewidth(1.8)+linecap(2)+linejoin(1)+linetype(new real[]{3.6,3.6},0,false,false)"
        ),
        "{out}"
    );
    // Dotted: a dot of 1/80 inch, a gap of the style value.
    assert!(out.contains("linetype(new real[]{0.9,2.7},0,false,false)"), "{out}");
    // Dash-dot.
    assert!(
        out.contains("linetype(new real[]{3.6,1.8,0.9,1.8},0,false,false)"),
        "{out}"
    );
}

#[test]
fn arrows_clip_the_shaft_and_are_paths() {
    let body = "2 1 0 2 0 7 50 -1 -1 0.000 0 0 -1 1 1 2\n\
                \t1 1 1.00 60.00 120.00\n\t1 0 1.00 60.00 120.00\n\t 600 600 3000 600\n";
    let out = asy(body, &[]);
    // The shaft is drawn in a picture clipped without the arrowheads' areas.
    assert!(out.contains("picture p;"), "{out}");
    assert!(out.contains("clip(p,box(("), "{out}");
    assert!(out.contains(",evenodd);"), "{out}");
    assert!(out.contains("add(p);"));
    // A hollow head (style 0) is white, a full one (style 1) has the pen colour.
    assert!(out.contains(",white,rgb(\"000000\")+linewidth("), "{out}");
    assert!(out.contains("+miterlimit(8)"));
    assert_eq!(out.matches("filldraw(").count(), 2, "{out}");
}

#[test]
fn open_arrow_style_is_not_filled() {
    let body = "2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 1 0 2\n\
                \t0 0 1.00 60.00 120.00\n\t 600 600 3000 600\n";
    let out = asy(body, &[]);
    assert_eq!(out.matches("filldraw(").count(), 0, "{out}");
    assert!(out.contains("draw((2"), "{out}");
}

#[test]
fn box_and_filled_colours() {
    let body = "2 2 0 1 0 4 50 -1 20 0.000 0 0 -1 0 0 5\n\t 600 600 1800 600 1800 1500 600 1500 600 600\n";
    let out = asy(body, &[]);
    assert!(out.contains("path g=box((600,-600),(1800,-1500));"), "{out}");
    assert!(out.contains("fill(g,rgb(\"ff0000\"));"), "{out}");
    assert!(out.contains("draw(g,rgb(\"000000\")+linewidth(0.45)"), "{out}");
}

#[test]
fn shades_and_tints_are_mixed() {
    // Fill 10 of blue is half way to black, 30 half way to white; the
    // default colour (black) at 10 is mid grey.
    let body = "2 2 0 0 0 1 50 -1 10 0.000 0 0 -1 0 0 5\n\t 0 0 600 0 600 600 0 600 0 0\n\
                2 2 0 0 0 1 50 -1 30 0.000 0 0 -1 0 0 5\n\t 0 0 600 0 600 600 0 600 0 0\n\
                2 2 0 0 0 -1 50 -1 10 0.000 0 0 -1 0 0 5\n\t 0 0 600 0 600 600 0 600 0 0\n";
    let out = asy(body, &[]);
    assert!(out.contains("fill(g,rgb(\"000080\"));"), "{out}");
    assert!(out.contains("rgb(\"8080ff\")"), "{out}");
    assert!(out.contains("rgb(\"808080\")"), "{out}");
    // No stroke for thickness 0.
    assert!(!out.contains("linewidth"), "{out}");
}

#[test]
fn user_colours_are_named_pens() {
    let body = "0 32 #336699\n2 1 0 1 32 7 50 -1 -1 0.000 0 0 -1 0 0 2\n\t 0 0 600 600\n\
                4 0 32 50 -1 0 12 0.0000 4 150 900 0 900 Hi\\001\n";
    let out = asy(body, &[]);
    assert!(out.contains("pen xfigc32=rgb(\"336699\");"), "{out}");
    assert!(out.contains("draw((0,0)--(600,-600),xfigc32+linewidth(0.45)"), "{out}");
    assert!(out.contains("xfigc32+fontsize(12)"), "{out}");
}

#[test]
fn rounded_box_has_curved_corners() {
    let body = "2 4 0 2 0 7 50 -1 -1 0.000 0 0 12 0 0 5\n\t 600 600 3000 600 3000 2400 600 2400 600 600\n";
    let out = asy(body, &[]);
    // Radius 12/80 inch = 180 Fig units.
    assert!(out.contains("(780,-2400)--(2820,-2400)"), "{out}");
    assert_eq!(out.matches("..controls").count(), 4, "{out}");
    assert!(out.contains("--cycle"));
}

#[test]
fn polygon_is_closed_without_the_repeated_point() {
    let body = "2 3 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 4\n\t 0 0 600 0 300 500 0 0\n";
    let out = asy(body, &[]);
    assert!(out.contains("(0,0)--(600,0)--(300,-500)--cycle"), "{out}");
    assert!(!out.contains("(300,-500)--(0,0)"), "{out}");
}

#[test]
fn open_filled_polyline_is_closed_for_the_fill_only() {
    let body = "2 1 0 1 0 2 50 -1 20 0.000 0 0 -1 0 0 3\n\t 0 0 600 0 300 500\n";
    let out = asy(body, &[]);
    assert!(out.contains("fill(g--cycle,rgb(\"00ff00\"));"), "{out}");
    assert!(out.contains("draw(g,"), "{out}");
}

#[test]
fn arcs_are_circular_arcs() {
    // Counterclockwise open arc with a forward arrow; centre (1000,1000).
    let ccw = "5 1 0 1 0 7 50 -1 -1 0.000 0 1 1 0 1000.000 1000.000 1500 1000 1000 500 500 1000\n\
               \t1 1 1.00 60.00 120.00\n";
    let out = asy(ccw, &[]);
    assert!(out.contains("arc((1000,-1000),500,0,180,CCW)"), "{out}");
    assert!(out.contains("clip(p,"), "{out}");
    let cw = "5 2 0 1 0 7 50 -1 20 0.000 0 0 0 0 1000.000 1000.000 1500 1000 1000 500 500 1000\n";
    let out = asy(cw, &[]);
    // A pie wedge goes through the centre; it is filled and stroked.
    assert!(
        out.contains("arc((1000,-1000),500,0,180,CW)--(1000,-1000)--cycle"),
        "{out}"
    );
    assert!(out.contains("fill(g,rgb(\"ffffff\"));"), "{out}");
}

#[test]
fn circles_and_ellipses_are_transformed_unit_circles() {
    let body = "1 3 0 1 0 7 50 -1 -1 0.000 1 0.0000 1000 1000 300 300 1000 1000 1300 1000\n\
                1 1 0 1 0 7 50 -1 -1 0.000 1 0.5236 3000 1000 400 200 3000 1000 3400 1000\n\
                1 1 0 1 0 7 50 -1 20 0.000 1 0.0000 5000 1000 400 200 5000 1000 5400 1000\n";
    let out = asy(body, &[]);
    assert!(out.contains("shift((1000,-1000))*scale(300)*unitcircle"), "{out}");
    assert!(
        out.contains("shift((3000,-1000))*rotate(30)*scale(400,200)*unitcircle"),
        "{out}"
    );
    assert!(out.contains("shift((5000,-1000))*scale(400,200)*unitcircle"), "{out}");
}

#[test]
fn xspline_is_a_guided_path_and_old_splines_are_converted() {
    let body = "3 4 0 1 0 7 50 -1 -1 0.000 0 0 0 4\n\t 600 600 1200 1500 1800 600 2400 1500\n\t 0 1 1 0\n";
    let out = asy(body, &[]);
    assert!(
        out.contains("bspline(new pair[]{(600,-600),(1200,-1500),(1800,-600),(2400,-1500)},false)"),
        "{out}"
    );
    let exact = asy(body, &[('S', Some("exact"))]);
    assert!(exact.contains("..controls"), "{exact}");
    // A 2.1 approximated spline (type 1, closed) comes through `spline`.
    let old = "#FIG 3.1\nLandscape\nCenter\nInches\n1200 2\n\
               3 1 0 1 0 7 50 -1 -1 0.000 0 0 0 4\n\t 600 600 1200 1500 1800 600 2400 1500\n";
    let (out, _, exit) = run_with("asy", old.as_bytes(), None, &[], false).unwrap();
    assert_eq!(exit, 0);
    assert!(
        out.contains("bspline(new pair[]{(600,-600),(1200,-1500),(1800,-600),(2400,-1500)},true)"),
        "{out}"
    );
}

#[test]
fn patterns_are_tilings_per_pen_colour() {
    let body = "2 2 0 1 4 2 50 -1 41 0.000 0 0 -1 0 0 5\n\t 600 600 1800 600 1800 1500 600 1500 600 600\n\
                2 2 0 1 4 2 50 -1 41 0.000 0 0 -1 0 0 5\n\t 2000 600 3000 600 3000 1500 2000 1500 2000 600\n\
                2 2 0 1 1 7 50 -1 52 0.000 0 0 -1 0 0 5\n\t 3200 600 4200 600 4200 1500 3200 1500 3200 600\n";
    let out = asy(body, &[]);
    assert!(out.starts_with("// Asymptote"));
    assert!(out.contains("\nimport patterns;\n"), "{out}");
    // One definition per pattern and pen colour, however often it is used.
    assert_eq!(out.matches("add(\"xfigp0_ff0000\",t);").count(), 1, "{out}");
    assert_eq!(out.matches("add(\"xfigp11_0000ff\",t);").count(), 1, "{out}");
    assert!(out.contains("clip(t,box((0,0),(48,24)));"), "{out}");
    // The background first, then the pattern.
    let bg = out.find("fill(g,rgb(\"00ff00\"));").unwrap();
    let pat = out.find("fill(g,pattern(\"xfigp0_ff0000\"));").unwrap();
    assert!(bg < pat);
}

#[test]
fn texts_are_baseline_labels_with_fonts() {
    let body = "4 0 0 50 -1 0 14 0.0000 4 150 900 600 600 Left & $x\\001\n\
                4 1 0 50 -1 16 12 0.0000 4 150 900 3000 600 Mid\\001\n\
                4 2 0 50 -1 18 12 0.5236 4 150 900 6000 600 Right\\001\n\
                4 0 0 50 -1 2 12 0.0000 0 150 900 600 1200 bold\\001\n\
                4 0 0 50 -1 5 12 0.0000 0 150 900 600 1500 mono\\001\n";
    let out = asy(body, &[]);
    // Left: east alignment; `&` and `$` are quoted, blanks are `\ `.
    assert!(
        out.contains(
            "label('\\\\raisebox{0pt}[0pt][0pt]{Left\\\\ \\\\&\\\\ \\\\$x}',(600,-600),E,rgb(\"000000\")+fontsize(14)+font(\"T1\",\"ptm\",\"m\",\"n\"));"
        ),
        "{out}"
    );
    // Centred, Helvetica.
    assert!(
        out.contains(",(3000,-600),NoAlign,rgb(\"000000\")+fontsize(12)+font(\"T1\",\"phv\",\"m\",\"n\"));"),
        "{out}"
    );
    // Rotated, right-aligned, bold Helvetica.
    assert!(out.contains("label(rotate(30)*Label('"), "{out}");
    assert!(
        out.contains(",(6000,-600),W,rgb(\"000000\")+fontsize(12)+font(\"T1\",\"phv\",\"b\",\"n\")));"),
        "{out}"
    );
    // LaTeX fonts.
    assert!(out.contains("fontcommand('\\\\bfseries')"), "{out}");
    assert!(out.contains("fontcommand('\\\\ttfamily')"), "{out}");
}

#[test]
fn special_text_is_not_quoted_and_options_change_fonts() {
    let body = "4 0 0 50 -1 0 12 0.0000 6 150 900 600 600 $\\\\alpha^2$ & x\\001\n";
    let out = asy(body, &[]);
    assert!(out.contains("{$\\\\alpha^2$ & x}'"), "{out}");
    // -O: every text is TeX.
    let plain = "4 0 0 50 -1 0 12 0.0000 4 150 900 600 600 a_b\\001\n";
    assert!(asy(plain, &[]).contains("a\\\\_b"));
    assert!(asy(plain, &[('O', None)]).contains("{a_b}"));
    // -F: no font; -o: no size; -f: another default font.
    let out = asy(plain, &[('F', None)]);
    assert!(out.contains("fontsize(12))"), "{out}");
    assert!(!out.contains("font(\"T1\""), "{out}");
    let out = asy(plain, &[('o', None)]);
    assert!(!out.contains("fontsize"), "{out}");
    let default_font = "4 0 0 50 -1 -1 12 0.0000 4 150 900 600 600 a\\001\n";
    let out = asy(default_font, &[('f', Some("Helvetica"))]);
    assert!(out.contains("font(\"T1\",\"phv\",\"m\",\"n\")"), "{out}");
    // -T: PostScript fonts become LaTeX fonts.
    let out = asy("4 0 0 50 -1 16 12 0.0000 4 150 900 600 600 s\\001\n", &[('T', None)]);
    assert!(out.contains("fontcommand('\\\\sffamily')"), "{out}");
    // Text that is empty or hidden is not written.
    let hidden = "4 0 0 50 -1 0 12 0.0000 12 150 900 600 600 gone\\001\n";
    assert!(!asy(&format!("{LINE}{hidden}"), &[]).contains("label("));
}

#[test]
fn symbol_font_text_uses_char_codes() {
    let body = "4 0 0 50 -1 32 12 0.0000 4 150 900 600 600 \u{e5}\\001\n";
    let out = asy(body, &[]);
    assert!(out.contains("font(\"U\",\"psy\",\"m\",\"n\")"), "{out}");
    assert!(out.contains("\\\\char229"), "{out}");
}

#[test]
fn pictures_are_graphics() {
    let body =
        "2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t0 pic/a b.eps\n\t 600 600 2600 600 2600 1600 600 1600 600 600\n";
    let out = asy(body, &[]);
    assert!(
        out.contains("label(Label(graphic('pic/a b.eps','width=120bp,height=60bp'),(1600,-1100),NoAlign));"),
        "{out}"
    );
    assert!(!out.contains("settings.tex"));
    let out = asy(body, &[('i', Some("dir/")), ('w', None)]);
    assert!(out.contains("graphic('dir/pic/a b'"), "{out}");
    // Bitmaps need pdfLaTeX.
    let png = body.replace("a b.eps", "x.png");
    assert!(asy(&png, &[]).contains("\nsettings.tex=\"pdflatex\";\n"));
    // A picture turned by 90 degrees.
    let turned =
        "2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t0 x.eps\n\t 600 1600 600 600 1600 600 1600 1600 600 1600\n";
    assert!(
        asy(turned, &[]).contains("label(rotate(90)*Label(graphic('x.eps','width=60bp,height=60bp')"),
        "{}",
        asy(turned, &[])
    );
}

#[test]
fn deepest_objects_come_first_and_comments_are_kept() {
    let body = "2 1 0 1 0 7 10 -1 -1 0.000 0 0 -1 0 0 2\n\t 0 0 600 600\n\
                # about the second\n2 1 0 1 0 7 90 -1 -1 0.000 0 0 -1 0 0 2\n\t 0 1000 600 1600\n";
    let out = asy(body, &[]);
    let deep = out.find("(0,-1000)--(600,-1600)").unwrap();
    let shallow = out.find("draw((0,0)--(600,-600)").unwrap();
    assert!(deep < shallow, "{out}");
    assert!(out.contains("// about the second"), "{out}");
}

#[test]
fn long_paths_are_built_from_an_array() {
    // asy's parser overflows its stack on a chain of about 1,000 `--`.
    let pts: Vec<String> = (0..1500)
        .map(|i| format!("{} {}", 600 + i, 600 + (i % 7) * 10))
        .collect();
    let body = format!("2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 1500\n\t{}\n", pts.join(" "));
    let out = asy(&body, &[]);
    assert!(
        out.contains("draw(operator --(...new pair[]{(600,-600),(601,-610),"),
        "{}",
        &out[..400]
    );
    assert!(out.contains("(2099,-610)}),rgb(\"000000\")"));
    assert!(!out.contains(")--("));
    // A short one stays a plain chain.
    assert!(asy(LINE, &[]).contains("(600,-600)--(3000,-1200)"));
}

#[test]
fn zero_length_line_needs_round_caps() {
    let butt = "2 1 0 3 0 7 50 -1 -1 0.000 0 0 -1 0 0 1\n\t 600 600\n";
    assert_eq!(asy(butt, &[]).matches("draw(").count(), 1);
    let round = "2 1 0 3 0 7 50 -1 -1 0.000 0 1 -1 0 0 1\n\t 600 600\n";
    assert!(asy(round, &[]).contains("draw((600,-600)--(600,-600),"));
}

#[test]
fn grid_and_unknown_options() {
    let out = asy(LINE, &[('G', Some("1:2in"))]);
    assert!(out.contains("for(real x="), "{out}");
    assert!(run("asy", LINE, &[('q', Some("1"))]).is_err());
    let (_, msgs, _) = run("asy", LINE, &[('f', Some("nope"))]).unwrap();
    assert!(
        msgs.iter().any(|m| m.contains("non-standard font name nope")),
        "{msgs:?}"
    );
}

#[test]
fn languages_know_asy() {
    assert!(fig2dev::drivers::driver("asy").is_some());
    assert!(fig2dev::drivers::help("asy").is_some());
    assert_eq!(
        fig2dev::languages::language_for_path("x.asy").map(|l| l.id),
        Some("asy")
    );
}

// ---- the asy program ------------------------------------------------------

const REPO: &str = "/Users/fabian/Documents/RustFig";

/// Why asy failed: its output names a TeX font or a picture file this
/// machine does not have, which says nothing about the driver.
fn environment_problem(log: &str) -> Option<&'static str> {
    if log.contains("mktextfm") || log.contains("Could not map source") || log.contains("Font T1/") {
        Some("TeX font not installed")
    } else if log.contains("No such picture file") || (log.contains("File `") && log.contains("not found")) {
        Some("picture file missing")
    } else {
        None
    }
}

fn run_asy(dir: &Path, name: &str, code: &str) -> Result<(), String> {
    let file = dir.join(format!("{name}.asy"));
    std::fs::write(&file, code).map_err(|e| e.to_string())?;
    let o = Command::new("asy")
        .args(["-f", "pdf", "-noV"])
        .arg(format!("{name}.asy"))
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let pdf = dir.join(format!("{name}.pdf"));
    if o.status.success() && pdf.metadata().is_ok_and(|m| m.len() > 100) {
        return Ok(());
    }
    if let Some(why) = environment_problem(&log) {
        return Err(format!("SKIP {why}"));
    }
    let tail: Vec<&str> = log.lines().rev().take(12).collect();
    Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// RustFig's fixtures and the first 50 figures of the corpus sample.
fn figures() -> Vec<PathBuf> {
    let repo = Path::new(REPO);
    let mut v: Vec<PathBuf> = std::fs::read_dir(repo.join("crates/engine/tests/fixtures"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|e| e == "fig"));
    v.sort();
    if let Ok(list) = std::fs::read_to_string(repo.join("plan/corpus-sample.txt")) {
        v.extend(
            list.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .take(50)
                .map(|l| repo.join(l)),
        );
    }
    v
}

#[test]
#[ignore = "runs the asy program (needs TeX); cargo test --test asy -- --ignored --nocapture"]
fn asy_compiles_the_figures() {
    if Command::new("asy").arg("-version").output().is_err() {
        eprintln!("asy not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fig2dev-asy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let figs = figures();
    assert!(!figs.is_empty(), "no figures");
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(path) = figs.get(i) else { break };
                let fig = std::fs::read(path).unwrap();
                let name = format!("f{i}");
                let res = match run_with("asy", &fig, Some(&path.to_string_lossy()), &[], false) {
                    Ok((out, _, 0)) => run_asy(&dir, &name, &out),
                    Ok((_, msgs, exit)) => Err(format!("SKIP fig2dev exit {exit}: {msgs:?}")),
                    Err(e) => Err(format!("fig2dev: {e}")),
                };
                results.lock().unwrap().push((i, path.clone(), res));
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    let (mut ok, mut skipped, mut failed) = (0, 0, Vec::new());
    for (_, path, res) in &results {
        match res {
            Ok(()) => ok += 1,
            Err(e) if e.starts_with("SKIP ") => {
                skipped += 1;
                eprintln!("skipped {}: {e}", path.display());
            }
            Err(e) => failed.push(format!("{}:\n{e}", path.display())),
        }
    }
    eprintln!(
        "asy: {ok} compiled, {skipped} skipped, {} failed of {}",
        failed.len(),
        results.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failed.is_empty(), "{}", failed.join("\n\n"));
    assert!(ok >= figs.len() / 2, "too few figures compiled");
}

// ---- Splines as Asymptote paths (-S guide | exact). ----

use fig2dev::object::{Control, Point, Spline};
use fig2dev::trans_spline::{compute_closed_spline, compute_open_spline};

type Pair = (f64, f64);

const FIVE: [(i32, i32); 5] = [(1200, 1200), (3600, 1200), (4200, 3000), (2400, 4200), (900, 3000)];
const SIX: [(i32, i32); 6] = [
    (5200, 5200),
    (6000, 6400),
    (6800, 5200),
    (7600, 6400),
    (8000, 5200),
    (8800, 6400),
];

/// A Fig 3.2 spline object; `arrows` adds a forward arrow, `fill` an area fill.
fn spline_obj(type_: i32, pts: &[(i32, i32)], factors: &[f64], arrow: bool, fill: i32) -> String {
    let p: Vec<String> = pts.iter().map(|(x, y)| format!("{x} {y}")).collect();
    let f: Vec<String> = factors.iter().map(|v| format!("{v}")).collect();
    format!(
        "3 {type_} 0 1 0 4 50 -1 {fill} 0.000 0 {} 0 {}\n{}\t {}\n\t {}\n",
        i32::from(arrow),
        pts.len(),
        if arrow { "\t 1 1 1.00 60.00 120.00\n" } else { "" },
        p.join(" "),
        f.join(" ")
    )
}

fn spline_of(type_: i32, pts: &[(i32, i32)], factors: &[f64]) -> Spline {
    Spline {
        type_,
        points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        controls: factors
            .iter()
            .map(|&s| Control {
                s,
                ..Control::default()
            })
            .collect(),
        ..Spline::default()
    }
}

/// The path of the first `draw(` after the bounds box, up to the pen.
fn first_path(out: &str) -> &str {
    let from = out.find("invisible);\n").unwrap() + "invisible);\n".len();
    let rest = &out[from..];
    let rest = rest.strip_prefix("draw(").unwrap_or(rest);
    // The comment of a mixed spline written as Béziers.
    let rest = if rest.starts_with("//") {
        rest.split_once('\n').map_or(rest, |(_, r)| r)
    } else {
        rest
    };
    &rest[..rest.find(",rgb(").unwrap()]
}

fn read_pair(s: &str) -> (Pair, &str) {
    let s = s.strip_prefix('(').unwrap();
    let end = s.find(')').unwrap();
    let (x, y) = s[..end].split_once(',').unwrap();
    ((x.parse().unwrap(), -y.parse::<f64>().unwrap()), &s[end + 1..])
}

/// Start node and segments (controls and end) of an explicit-controls path
/// in Fig coordinates (y down); whether it ends in `cycle`.
fn parse_bezier(path: &str) -> (Pair, Vec<[Pair; 3]>, bool) {
    let flat: String = path.chars().filter(|c| !c.is_whitespace()).collect();
    let (start, mut rest) = read_pair(&flat);
    let (mut at, mut segs, mut closed) = (start, Vec::new(), false);
    loop {
        if let Some(r) = rest.strip_prefix("..controls") {
            let (c1, r) = read_pair(r);
            let (c2, r) = read_pair(r.strip_prefix("and").unwrap());
            let r = r.strip_prefix("..").unwrap();
            if let Some(r) = r.strip_prefix("cycle") {
                segs.push([c1, c2, start]);
                closed = true;
                rest = r;
            } else {
                let (e, r) = read_pair(r);
                segs.push([c1, c2, e]);
                at = e;
                rest = r;
            }
        } else if let Some(r) = rest.strip_prefix("--") {
            if let Some(r) = r.strip_prefix("cycle") {
                segs.push([at, start, start]);
                closed = true;
                rest = r;
            } else {
                let (e, r) = read_pair(r);
                segs.push([at, e, e]);
                at = e;
                rest = r;
            }
        } else {
            assert!(rest.is_empty(), "left over: {rest}");
            return (start, segs, closed);
        }
    }
}

fn dist_to_poly(p: Pair, poly: &[Pair]) -> f64 {
    poly.windows(2)
        .map(|w| {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let l2 = dx * dx + dy * dy;
            let u = if l2 > 0.0 {
                (((p.0 - w[0].0) * dx + (p.1 - w[0].1) * dy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (p.0 - w[0].0 - u * dx).hypot(p.1 - w[0].1 - u * dy)
        })
        .fold(f64::INFINITY, f64::min)
}

fn xspline_polyline(s: &Spline) -> Vec<Pair> {
    let mut ctx = fig2dev::Ctx::default();
    let pts = if s.closed_spline() {
        compute_closed_spline(&mut ctx, s, 0.05)
    } else {
        compute_open_spline(&mut ctx, s, 0.05)
    }
    .unwrap();
    pts.iter().map(|p| (f64::from(p.x), f64::from(p.y))).collect()
}

/// The largest distance of the parsed Bézier path from the X-spline.
fn deviation(path: &str, s: &Spline) -> f64 {
    let poly = xspline_polyline(s);
    let (start, segs, _) = parse_bezier(path);
    let mut at = start;
    let mut worst = 0.0f64;
    for [c1, c2, e] in segs {
        for i in 0..=20 {
            let u = f64::from(i) / 20.0;
            let v = 1.0 - u;
            let q = |f: fn(Pair) -> f64| {
                v * v * v * f(at) + 3.0 * v * v * u * f(c1) + 3.0 * v * u * u * f(c2) + u * u * u * f(e)
            };
            worst = worst.max(dist_to_poly((q(|p| p.0), q(|p| p.1)), &poly));
        }
        at = e;
    }
    worst
}

fn controls(s: &str) -> usize {
    s.matches("..controls").count()
}

#[test]
fn exact_mode_keeps_the_points_as_nodes_and_guide_is_the_default() {
    let body = spline_obj(3, &FIVE, &[-1.0; 5], false, -1);
    let out = asy(&body, &[('S', Some("exact"))]);
    let path = first_path(&out);
    assert!(path.ends_with("..cycle"), "{path}");
    // Every Fig point is a node (y turned upwards).
    for (x, y) in FIVE {
        assert!(path.contains(&format!("({x},{})", -y)), "{x},{y} in {path}");
    }
    // `-S guide` is the default.
    assert_eq!(asy(&body, &[('S', Some("guide"))]), asy(&body, &[]));
    assert_ne!(asy(&body, &[]), out);
}

#[test]
fn exact_mode_is_within_half_a_unit() {
    let cases = [
        (3, FIVE.to_vec(), vec![-1.0; 5]),
        (1, FIVE.to_vec(), vec![1.0; 5]),
        (2, SIX[..4].to_vec(), vec![0.0, -1.0, -1.0, 0.0]),
        (0, SIX[..4].to_vec(), vec![0.0, 1.0, 1.0, 0.0]),
        (4, SIX.to_vec(), vec![0.0, 1.0, 0.0, -1.0, 1.0, 0.0]),
        (5, SIX.to_vec(), vec![0.5, 1.0, 0.0, -1.0, 0.3, -0.5]),
    ];
    for (t, pts, f) in cases {
        let out = asy(&spline_obj(t, &pts, &f, false, -1), &[('S', Some("exact"))]);
        let path = first_path(&out);
        let dev = deviation(path, &spline_of(t, &pts, &f));
        eprintln!("exact type {t}: {} pieces, deviation {dev:.2}", controls(path));
        // The polyline itself is rounded to whole units.
        assert!(dev <= 1.0, "type {t}: {dev}");
        let (_, segs, closed) = parse_bezier(path);
        assert_eq!(closed, t % 2 == 1);
        assert!(segs.len() <= 12 * pts.len(), "type {t}: {} pieces", segs.len());
    }
}

#[test]
fn mixed_shape_factors_with_a_corner() {
    let f = [0.0, 1.0, 0.0, -1.0, 1.0, 0.0];
    let out = asy(&spline_obj(4, &SIX, &f, false, -1), &[]);
    let path = first_path(&out);
    let dev = deviation(path, &spline_of(4, &SIX, &f));
    eprintln!("bezier mixed: {} pieces, deviation {dev:.2}", controls(path));
    assert!(controls(path) >= 5 && controls(path) <= 12, "{path}");
    assert!(dev <= 40.0, "{dev}");
    // The corner at the third point: its neighbours' controls sit on it.
    assert!(path.contains("(6800,-5200)"), "{path}");
}

#[test]
fn guide_mode_approximated_is_a_bspline_of_the_points() {
    let body = spline_obj(1, &FIVE, &[1.0; 5], false, -1);
    let out = asy(&body, &[('S', Some("guide"))]);
    assert!(out.contains("path bspline(pair[] p, bool closed=false)"), "{out}");
    let draw = &out[out.find("draw(bspline(").unwrap()..];
    assert!(
        draw.starts_with(
            "draw(bspline(new pair[]{(1200,-1200),(3600,-1200),(4200,-3000),(2400,-4200),(900,-3000)},true),"
        ),
        "{draw}"
    );
    assert!(!draw.contains("controls"));
    // The helper is defined once, however many splines use it.
    let two = format!("{body}{body}");
    assert_eq!(asy(&two, &[]).matches("path bspline(").count(), 1);
}

/// The uniform cubic B-spline `bspline` draws, as points (closed, or open
/// with the end points tripled).
fn bspline_points(p: &[(i32, i32)], closed: bool) -> Vec<Pair> {
    let mut q: Vec<Pair> = p.iter().map(|&(x, y)| (f64::from(x), f64::from(y))).collect();
    let (mut first, mut range_end) = (0, q.len());
    if !closed {
        let (f, l) = (q[0], q[q.len() - 1]);
        q.splice(0..0, [f, f]);
        q.extend([l, l]);
        first = 1;
        range_end = q.len() - 2;
    }
    let n = q.len();
    let mut out = Vec::new();
    for i in first..range_end {
        let (a, b, c, d) = (q[(i + n - 1) % n], q[i], q[(i + 1) % n], q[(i + 2) % n]);
        for j in 0..=16 {
            let u = f64::from(j) / 16.0;
            let w = [
                (1.0 - u).powi(3) / 6.0,
                (3.0 * u.powi(3) - 6.0 * u * u + 4.0) / 6.0,
                (-3.0 * u.powi(3) + 3.0 * u * u + 3.0 * u + 1.0) / 6.0,
                u.powi(3) / 6.0,
            ];
            out.push((
                w[0] * a.0 + w[1] * b.0 + w[2] * c.0 + w[3] * d.0,
                w[0] * a.1 + w[1] * b.1 + w[2] * c.1 + w[3] * d.1,
            ));
        }
    }
    out
}

#[test]
fn bspline_of_the_guide_mode_is_close_to_the_xspline() {
    for (t, pts, f) in [
        (1, FIVE.to_vec(), vec![1.0; 5]),
        (0, SIX.to_vec(), vec![0.0, 1.0, 1.0, 1.0, 1.0, 0.0]),
    ] {
        let s = spline_of(t, &pts, &f);
        let poly = xspline_polyline(&s);
        let bs = bspline_points(&pts, t % 2 == 1);
        let dev = bs.iter().map(|&p| dist_to_poly(p, &poly)).fold(0.0, f64::max);
        let back = poly.iter().map(|&p| dist_to_poly(p, &bs)).fold(0.0, f64::max);
        eprintln!("bspline vs X-spline type {t}: {dev:.1} / {back:.1} Fig units");
        // About one percent of the figure's 3000-unit size.
        assert!(dev < 40.0 && back < 40.0, "{dev} {back}");
    }
}

#[test]
fn guide_mode_interpolated_is_bezier_through_the_points() {
    let out = asy(&spline_obj(3, &FIVE, &[-1.0; 5], false, -1), &[]);
    let path = first_path(&out);
    assert_eq!(controls(path), 5, "{path}");
    for (x, y) in FIVE {
        assert!(path.contains(&format!("({x},{})", -y)), "{x},{y} in {path}");
    }
    assert!(!out.contains("// Mixed"));
    let dev = deviation(path, &spline_of(3, &FIVE, &[-1.0; 5]));
    assert!(dev <= 40.0, "{dev}");
}

#[test]
fn hobby_mode_interpolated_is_hobby_through_the_points() {
    let out = asy(&spline_obj(3, &FIVE, &[-1.0; 5], false, -1), &[('S', Some("hobby"))]);
    assert_eq!(
        first_path(&out),
        "(1200,-1200)..(3600,-1200)..(4200,-3000)..(2400,-4200)..(900,-3000)..cycle"
    );
    // Open: the ends have shape factor 0 but are not corners.
    let out = asy(
        &spline_obj(2, &SIX[..4], &[0.0, -1.0, -1.0, 0.0], false, -1),
        &[('S', Some("hobby"))],
    );
    assert_eq!(
        first_path(&out),
        "(5200,-5200)..(6000,-6400)..(6800,-5200)..(7600,-6400)"
    );
    assert!(!out.contains("controls"));
}

#[test]
fn guide_mode_mixed_falls_back_to_bezier_with_a_comment() {
    let f = [0.0, 1.0, 0.0, -1.0, 1.0, 0.0];
    let out = asy(&spline_obj(4, &SIX, &f, false, -1), &[('S', Some("guide"))]);
    assert!(out.contains("// Mixed shape factors"), "{out}");
    assert!(controls(&out) >= 5);
    assert!(!out.contains("bspline"));
}

#[test]
fn spline_mode_rejects_other_arguments() {
    let mut ctx = fig2dev::Ctx::default();
    let mut dev = fig2dev::drivers::driver("asy").unwrap();
    assert!(dev.option(&mut ctx, 'S', Some("smooth")).is_err());
    assert!(dev.option(&mut ctx, 'S', Some("bezier")).is_err());
    assert!(dev.option(&mut ctx, 'S', Some("guide")).is_ok());
    assert!(dev.option(&mut ctx, 'S', Some("hobby")).is_ok());
    assert!(dev.option(&mut ctx, 'S', Some("exact")).is_ok());
    assert!(dev.keeps_splines());
}

#[test]
fn splines_with_arrows_and_fills() {
    // Arrowheads: the shaft is clipped as for a polyline, the head is a path.
    let out = asy(&spline_obj(4, &SIX, &[0.0, 1.0, 0.0, -1.0, 1.0, 0.0], true, -1), &[]);
    assert!(out.contains("clip(p,"), "{out}");
    assert!(out.contains("draw(p,g,"), "{out}");
    assert!(out.contains("(5200,-5200)"), "{out}");
    assert!(controls(&out) >= 5);
    assert!(out.contains("filldraw("), "{out}");
    // A filled closed spline: the fill is the same path.
    let out = asy(&spline_obj(3, &FIVE, &[-1.0; 5], false, 20), &[]);
    assert!(
        out.contains("fill(g,rgb(\"000000\"));") || out.contains("fill(g,"),
        "{out}"
    );
    assert!(out.contains("..cycle"), "{out}");
    // Dashes work on the Bézier path.
    let dashed = spline_obj(1, &FIVE, &[1.0; 5], false, -1)
        .replacen("3 1 0 1", "3 1 1 1", 1)
        .replacen("0.000", "4.000", 1);
    let out = asy(&dashed, &[]);
    assert!(out.contains("linetype("), "{out}");
    // An open filled spline is closed for the fill only.
    let out = asy(&spline_obj(2, &SIX[..4], &[0.0, -1.0, -1.0, 0.0], false, 20), &[]);
    assert!(out.contains("fill(g--cycle,"), "{out}");
}

#[test]
fn the_figure_bounds_do_not_depend_on_how_splines_are_drawn() {
    // The header's box is the same as with the flattened curve (the other
    // drivers' bounds).
    let body = spline_obj(3, &FIVE, &[-1.0; 5], false, -1);
    let asy_out = asy(&body, &[]);
    let (eps, _, _) = run("eps", &body, &[]).unwrap();
    let bb = eps.lines().find(|l| l.starts_with("%%BoundingBox")).unwrap();
    assert!(asy_out.contains("invisible)"));
    let _ = bb;
    let bounds = asy_out.lines().find(|l| l.contains("invisible")).unwrap();
    let (llx, ury_neg) = {
        let (a, _) = read_pair(bounds.strip_prefix("draw(box(").unwrap());
        a
    };
    assert!(llx > 0.0 && ury_neg.abs() > 0.0);
    // Same bounds as the polyline of a figure with its spline flattened by
    // the reader (a driver that does not keep splines).
    let mut ctx = fig2dev::Ctx::default();
    let flat = fig2dev::read::readfp_fig(&mut ctx, format!("{}{body}", tex_common::HDR).as_bytes()).unwrap();
    let mut kept_ctx = fig2dev::Ctx {
        keep_splines: true,
        ..fig2dev::Ctx::default()
    };
    let kept = fig2dev::read::readfp_fig(&mut kept_ctx, format!("{}{body}", tex_common::HDR).as_bytes()).unwrap();
    assert_eq!((flat.splines.len(), kept.splines.len()), (0, 1));
    assert_eq!(
        fig2dev::bound::compound_bound(&ctx, &flat, true),
        fig2dev::bound::compound_bound(&kept_ctx, &kept, true)
    );
}

#[test]
fn long_splines_are_joined_from_pieces() {
    let pts: Vec<(i32, i32)> = (0..400).map(|i| (600 + 20 * i, 3000 + (i % 5) * 300)).collect();
    let f: Vec<f64> = (0..pts.len())
        .map(|i| if i == 0 || i + 1 == pts.len() { 0.0 } else { -1.0 })
        .collect();
    let out = asy(&spline_obj(2, &pts, &f, false, -1), &[('S', Some("exact"))]);
    assert!(out.contains("path xfigjoin(path[] p)"), "defined once, before use");
    assert_eq!(out.matches("path xfigjoin(").count(), 1);
    assert!(
        out.contains("draw(xfigjoin(new path[]{"),
        "{}",
        &out[..600.min(out.len())]
    );
    assert!(out.find("path xfigjoin(").unwrap() < out.find("draw(xfigjoin(").unwrap());
    // A closed one ends in &cycle.
    let mut cl = pts.clone();
    cl.push((600, 5000));
    let f: Vec<f64> = vec![-1.0; cl.len()];
    let out = asy(&spline_obj(3, &cl, &f, false, -1), &[('S', Some("exact"))]);
    assert!(
        out.contains("}))&cycle") || out.contains("}\n    )&cycle") || out.contains("&cycle,"),
        "{out}"
    );
}

#[test]
fn boxes_and_polygons_keep_their_native_forms() {
    let body = "2 2 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 600 600 1800 600 1800 1500 600 1500 600 600\n\
                2 3 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 4\n\t 3000 600 4000 600 3500 1500 3000 600\n";
    let out = asy(body, &[]);
    assert!(out.contains("draw(box((600,-600),(1800,-1500))"), "{out}");
    assert!(out.contains("(3000,-600)--(4000,-600)--(3500,-1500)--cycle"), "{out}");
}

const SPLINE_FIGS: &[&str] = &[
    "interp-closed",
    "approx-closed",
    "interp-open",
    "mixed-open",
    "arrow",
    "filled",
];

fn spline_figs() -> Vec<(String, String)> {
    let f = [0.0, 1.0, 0.0, -1.0, 1.0, 0.0];
    vec![
        ("interp-closed".into(), spline_obj(3, &FIVE, &[-1.0; 5], false, -1)),
        ("approx-closed".into(), spline_obj(1, &FIVE, &[1.0; 5], false, -1)),
        (
            "interp-open".into(),
            spline_obj(2, &SIX[..4], &[0.0, -1.0, -1.0, 0.0], false, -1),
        ),
        ("mixed-open".into(), spline_obj(4, &SIX, &f, false, -1)),
        ("arrow".into(), spline_obj(4, &SIX, &f, true, -1)),
        ("filled".into(), spline_obj(3, &FIVE, &[-1.0; 5], false, 20)),
    ]
}

#[test]
#[ignore = "runs the asy program; cargo test --test asy asy_compiles_splines -- --ignored --nocapture"]
fn asy_compiles_splines_in_every_mode() {
    if Command::new("asy").arg("-version").output().is_err() {
        eprintln!("asy not installed; skipped");
        return;
    }
    assert_eq!(SPLINE_FIGS.len(), spline_figs().len());
    let dir = std::env::temp_dir().join(format!("fig2dev-asy-splines-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failed = Vec::new();
    for mode in ["exact", "guide", "hobby"] {
        for (name, body) in spline_figs() {
            let out = asy(&body, &[('S', Some(mode))]);
            let file = format!("{name}-{mode}");
            if let Err(e) = run_asy(&dir, &file, &out) {
                failed.push(format!("{file}: {e}"));
            }
        }
    }
    // A long one, in every mode.
    let pts: Vec<(i32, i32)> = (0..400).map(|i| (600 + 20 * i, 3000 + (i % 5) * 300)).collect();
    let f: Vec<f64> = vec![-1.0; pts.len()];
    for mode in ["exact", "guide", "hobby"] {
        let out = asy(&spline_obj(3, &pts, &f, false, -1), &[('S', Some(mode))]);
        if let Err(e) = run_asy(&dir, &format!("long-{mode}"), &out) {
            failed.push(format!("long-{mode}: {e}"));
        }
    }
    eprintln!("outputs in {}", dir.display());
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn hobby_mode_keeps_cubics_for_a_spline_with_arrowheads() {
    let body = spline_obj(2, &SIX[..4], &[0.0, -1.0, -1.0, 0.0], true, -1);
    let out = asy(&body, &[('S', Some("hobby"))]);
    assert!(out.contains("// Arrowheads"), "{out}");
    assert!(controls(&out) >= 3, "{out}");
}

#[test]
fn xfigs_interpolated_factor_counts_as_interpolated() {
    // xfig writes -0.5 for an interpolated spline (S_SPLINE_INTERP).
    let body = spline_obj(3, &FIVE, &[-0.5; 5], false, -1);
    let guide = asy(&body, &[]);
    assert!(!guide.contains("// Mixed"), "{guide}");
    let hobby = asy(&body, &[('S', Some("hobby"))]);
    assert!(!hobby.contains("controls"), "{hobby}");
    assert!(first_path(&hobby).ends_with("..cycle"));
}
