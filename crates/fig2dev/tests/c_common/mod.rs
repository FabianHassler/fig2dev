#![allow(dead_code)]
//! The options C fig2dev 3.2.9a accepts for each language, transcribed from its sources. Shared by `c_options.rs` and `differences_doc.rs` (`#[path]`).

/// `-G` and `-L` reach every driver (fig2dev.c passes them on) and each
/// driver takes both.
pub const EVERY: &str = "GL";

/// gen_ps_eps_option (genps.c): the options of ps, eps and pstex.
pub const PS_EPS: &str = "AbBCceFfgjlMnNOopPRTxYyz";
/// genpdf_option (genpdf.c): -P, -z and -Y itself, then gen_ps_eps_option.
pub const PDF: &str = PS_EPS;
/// genbitmaps_option (genbitmaps.c): bitmaps; -F goes to gen_ps_eps_option.
pub const BITMAPS: &str = "bFgNqtS";
/// genlatex_option (genlatex.c), which pstex_t and pdftex_t also use
/// (genpstex_t_option adds -p).
pub const LATEX: &str = "abdFflv";

/// (language, letters), each language of `drivers[]` in drivers.h and the
/// option function of its driver. Letters C only passes to `default:`
/// (a message) are not listed.
pub const C_OPTIONS: &[(&str, &str)] = &[
    ("box", ""),                   // genbox_option
    ("cgm", "ar"),                 // gencgm_option
    ("dxf", "acdflpPv"),           // gendxf_option
    ("epic", "dafLlPSvWwtFR"),     // genepic_option
    ("eepic", "dafLlPSvWwtFR"),    // genepic_option
    ("eepicemu", "dafLlPSvWwtFR"), // genepic_option
    ("emf", "rl"),                 // genemf_option
    ("eps", PS_EPS),               // geneps_option
    ("gbx", "dpgfiv"),             // gengbx_option
    ("ge", ""),                    // genge_option
    ("gif", BITMAPS),              // genbitmaps_option
    ("ibmgl", "acdfklFpPSvxyz"),   // genibmgl_option
    ("jpeg", BITMAPS),             // genbitmaps_option
    ("latex", LATEX),              // genlatex_option
    ("map", "b"),                  // genmap_option
    ("mf", "CnptxyXY"),            // genmf_option
    ("mp", "Modip"),               // genmp_option (others silently ignored)
    ("pcx", BITMAPS),              // genbitmaps_option
    ("pdf", PDF),                  // genpdf_option
    ("pdftex", PDF),               // genpdf_option
    ("pdftex_t", "pabdFflv"),      // genpstex_t_option, genlatex_option
    ("pic", "fp"),                 // genpic_option
    ("pict2e", "bCeFifoTOPRrvw"),  // genpict2e_option
    ("pictex", "flpr"),            // genpictex_option
    ("png", BITMAPS),              // genbitmaps_option
    ("ppm", BITMAPS),              // genbitmaps_option
    ("ps", PS_EPS),                // genps_option
    ("pstex", PS_EPS),             // geneps_option
    ("pstex_t", "pabdFflv"),       // genpstex_t_option, genlatex_option
    ("pstricks", "fLlnPpRStvxyz"), // genpstrx_option
    ("ptk", "lpPwz"),              // genptk_option
    ("shape", "n"),                // genshape_option
    ("sld", BITMAPS),              // genbitmaps_option
    ("svg", "z"),                  // gensvg_option
    ("textyl", "afl"),             // gentextyl_option
    ("tiff", BITMAPS),             // genbitmaps_option
    ("tikz", "bCFifoTOPvWw"),      // gentikz_option
    ("tk", "glpPz"),               // gentk_option
    ("tpic", "f"),                 // gentpic_option
    ("xbm", BITMAPS),              // genbitmaps_option
    ("xpm", BITMAPS),              // genbitmaps_option
];
