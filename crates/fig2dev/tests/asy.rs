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
fn xspline_is_a_smooth_polyline_and_old_splines_are_converted() {
    let body = "3 4 0 1 0 7 50 -1 -1 0.000 0 0 0 4\n\t 600 600 1200 1500 1800 600 2400 1500\n\t 0 1 1 0\n";
    let out = asy(body, &[]);
    assert!(out.matches("--").count() > 10, "{out}");
    // A 2.1 approximated spline (type 1, closed) comes through `spline`.
    let old = "#FIG 3.1\nLandscape\nCenter\nInches\n1200 2\n\
               3 1 0 1 0 7 50 -1 -1 0.000 0 0 0 4\n\t 600 600 1200 1500 1800 600 2400 1500\n";
    let (out, _, exit) = run_with("asy", old.as_bytes(), None, &[], false).unwrap();
    assert_eq!(exit, 0);
    assert!(out.contains("--cycle"), "{out}");
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
