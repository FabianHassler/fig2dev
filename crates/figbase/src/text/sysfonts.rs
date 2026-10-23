//! Installed fonts, found with `fontdb`.
//!
//! Each of xfig's PostScript fonts has an ordered list of candidate families:
//! the PostScript clones ghostscript installs on Linux (URW base35, Nimbus),
//! the macOS and Windows system faces, the TeX Gyre metric clones (found in
//! a TeX installation or the system font folders), then the nearest generic
//! serif, sans or monospace family. When nothing matches, figbase falls back
//! to the built-in Ubuntu Light and Hack. Everything is lazy, cached for the
//! process and never fails hard: any error means "use the built-in fonts".
//! On wasm32 the system is never searched.

use super::LoadedFace;
use fontdb::{Database, Family, Query, Stretch, Style, Weight, ID};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use ttf_parser::Face;

const SERIF: &[&str] = &[
    "Liberation Serif",
    "Tinos",
    "DejaVu Serif",
    "Noto Serif",
    "Times New Roman",
    "Georgia",
];
const SANS: &[&str] = &[
    "Liberation Sans",
    "Arimo",
    "DejaVu Sans",
    "Noto Sans",
    "Arial",
    "Helvetica",
    "Segoe UI",
];
const MONO: &[&str] = &[
    "Liberation Mono",
    "Cousine",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Menlo",
    "Consolas",
    "Courier New",
];

/// Families with broad symbol coverage, tried for glyphs the chosen face
/// lacks (arrows, maths, box drawing, geometric shapes) before the built-in
/// fonts.
pub const SYMBOL_FALLBACK: &[&str] = &[
    "Apple Symbols",
    "Menlo",
    "Arial Unicode MS",
    "DejaVu Sans",
    "Segoe UI Symbol",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Sans Math",
];

/// (exact PostScript clones and system faces, TeX Gyre) per font group.
/// Groups 0-7 are the eight four-style families; 8 Symbol, 9 Zapf
/// Chancery, 10 Zapf Dingbats.
const GROUPS: [(&[&str], &[&str]); 11] = [
    (
        &["Nimbus Roman", "Nimbus Roman No9 L", "Times", "Times New Roman"],
        &["TeX Gyre Termes"],
    ),
    (
        &[
            "URW Gothic",
            "URW Gothic L",
            "Avant Garde",
            "ITC Avant Garde Gothic",
            "Century Gothic",
        ],
        &["TeX Gyre Adventor"],
    ),
    (
        &["URW Bookman", "URW Bookman L", "Bookman", "Bookman Old Style"],
        &["TeX Gyre Bonum"],
    ),
    (
        &[
            "Nimbus Mono PS",
            "Nimbus Mono",
            "Nimbus Mono L",
            "Courier",
            "Courier New",
        ],
        &["TeX Gyre Cursor"],
    ),
    (
        &["Nimbus Sans", "Nimbus Sans L", "Helvetica", "Arial"],
        &["TeX Gyre Heros"],
    ),
    (
        &["Nimbus Sans Narrow", "Helvetica Narrow", "Arial Narrow"],
        &["TeX Gyre Heros Cn"],
    ),
    (
        &[
            "C059",
            "Century Schoolbook L",
            "New Century Schoolbook",
            "Century Schoolbook",
        ],
        &["TeX Gyre Schola"],
    ),
    (
        &["P052", "Palladio L", "Palatino", "Palatino Linotype"],
        &["TeX Gyre Pagella"],
    ),
    (&["Standard Symbols PS", "Standard Symbols L", "Symbol"], &[]),
    (
        &[
            "Z003",
            "URW Chancery L",
            "Zapf Chancery",
            "Apple Chancery",
            "Monotype Corsiva",
        ],
        &["TeX Gyre Chorus"],
    ),
    (&["D050000L", "Dingbats", "ITC Zapf Dingbats", "Zapf Dingbats"], &[]),
];

const LATEX_ROMAN: &[&str] = &["Latin Modern Roman", "LM Roman 10", "CMU Serif", "Computer Modern"];
const LATEX_SANS: &[&str] = &["Latin Modern Sans", "LM Sans 10", "CMU Sans Serif"];
const LATEX_MONO: &[&str] = &["Latin Modern Mono", "LM Mono 10", "CMU Typewriter Text"];

fn group_of(ps: i32) -> usize {
    match ps {
        0..=31 => (ps / 4) as usize,
        32 => 8,
        33 => 9,
        _ => 10,
    }
}

/// Candidate family names for a request, best first.
pub(crate) fn family_list(ps: Option<i32>, generic: super::Generic) -> Vec<&'static str> {
    let generic_list = match generic {
        super::Generic::Serif => SERIF,
        super::Generic::Sans => SANS,
        super::Generic::Mono => MONO,
    };
    let mut v: Vec<&'static str> = Vec::new();
    match ps {
        Some(n) => {
            let g = group_of(n);
            if let Some((exact, gyre)) = GROUPS.get(g) {
                v.extend(exact.iter());
                v.extend(gyre.iter());
            }
            if g == 5 {
                // Narrow falls back to the regular-width Helvetica.
                if let Some((exact, gyre)) = GROUPS.get(4) {
                    v.extend(exact.iter());
                    v.extend(gyre.iter());
                }
            }
            if g == 8 || g == 10 {
                v.extend(SYMBOL_FALLBACK.iter());
            } else {
                v.extend(generic_list.iter());
            }
        }
        None => {
            v.extend(match generic {
                super::Generic::Serif => LATEX_ROMAN,
                super::Generic::Sans => LATEX_SANS,
                super::Generic::Mono => LATEX_MONO,
            });
            v.extend(match generic {
                super::Generic::Serif => &["TeX Gyre Termes"][..],
                super::Generic::Sans => &["TeX Gyre Heros"][..],
                super::Generic::Mono => &["TeX Gyre Cursor"][..],
            });
            v.extend(generic_list.iter());
        }
    }
    v
}

fn query_weight(ps: Option<i32>, bold: bool) -> Weight {
    match ps.map(group_of) {
        Some(1 | 2) if bold => Weight(600),
        Some(2) => Weight(300),
        _ if bold => Weight::BOLD,
        _ => Weight::NORMAL,
    }
}

fn query_style(ps: Option<i32>, italic: bool) -> Style {
    if !italic {
        return Style::Normal;
    }
    match ps.map(group_of) {
        Some(1 | 3 | 4 | 5) => Style::Oblique,
        _ => Style::Italic,
    }
}

/// The best installed face for a request, by the candidate order.
pub(crate) fn choose(db: &Database, req: &super::FontRequest) -> Option<ID> {
    let weight = query_weight(req.ps_font, req.bold);
    let style = query_style(req.ps_font, req.italic);
    let stretch = if req.ps_font.map(group_of) == Some(5) {
        Stretch::Condensed
    } else {
        Stretch::Normal
    };
    for fam in family_list(req.ps_font, req.generic) {
        let q = Query {
            families: &[Family::Name(fam)],
            weight,
            stretch,
            style,
        };
        if let Some(id) = db.query(&q) {
            return Some(id);
        }
    }
    None
}

/// A face picked for a request.
#[derive(Clone, Copy)]
pub(crate) struct Picked {
    pub face: &'static LoadedFace,
    pub synth_bold: bool,
    pub synth_italic: bool,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct Key {
    ps: i32,
    generic: u8,
    bold: bool,
    italic: bool,
}

/// A font database with lazily parsed (and leaked, so `'static`) faces.
pub(crate) struct Resolver {
    db: Database,
    loaded: HashMap<ID, Option<&'static LoadedFace>>,
    resolved: HashMap<Key, Option<Picked>>,
    chain: Option<Arc<Vec<&'static LoadedFace>>>,
}

impl Resolver {
    pub(crate) fn with_db(db: Database) -> Resolver {
        Resolver {
            db,
            loaded: HashMap::new(),
            resolved: HashMap::new(),
            chain: None,
        }
    }

    fn load(&mut self, id: ID) -> Option<&'static LoadedFace> {
        if let Some(l) = self.loaded.get(&id) {
            return *l;
        }
        let r = self.parse(id);
        self.loaded.insert(id, r);
        r
    }

    fn parse(&self, id: ID) -> Option<&'static LoadedFace> {
        let info = self.db.face(id)?;
        let (data, index) = self.db.with_face_data(id, |d, i| (d.to_vec(), i))?;
        let bytes: &'static [u8] = Box::leak(data.into_boxed_slice());
        let face = Face::parse(bytes, index).ok()?;
        let mut key = String::new();
        for (name, _) in &info.families {
            key.push_str(name);
            key.push(' ');
        }
        key.push_str(&info.post_script_name);
        let key = key.to_lowercase();
        let bold = info.weight.0 >= 600;
        let italic = info.style != Style::Normal;
        Some(Box::leak(Box::new(LoadedFace {
            face,
            key,
            bold,
            italic,
            system: true,
        })))
    }

    pub(crate) fn resolve(&mut self, req: &super::FontRequest) -> Option<Picked> {
        let key = Key {
            ps: req.ps_font.unwrap_or(-1),
            generic: req.generic as u8,
            bold: req.bold,
            italic: req.italic,
        };
        if let Some(p) = self.resolved.get(&key) {
            return *p;
        }
        let picked = choose(&self.db, req).and_then(|id| self.load(id)).map(|face| Picked {
            face,
            synth_bold: req.bold && !face.bold,
            synth_italic: req.italic && !face.italic,
        });
        self.resolved.insert(key, picked);
        picked
    }

    /// Faces with broad symbol coverage that are installed.
    pub(crate) fn chain(&mut self) -> Arc<Vec<&'static LoadedFace>> {
        if let Some(c) = &self.chain {
            return c.clone();
        }
        let mut ids: Vec<ID> = Vec::new();
        for fam in SYMBOL_FALLBACK {
            let q = Query {
                families: &[Family::Name(fam)],
                ..Query::default()
            };
            if let Some(id) = self.db.query(&q) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        let faces: Vec<&'static LoadedFace> = ids.into_iter().filter_map(|id| self.load(id)).collect();
        let c = Arc::new(faces);
        self.chain = Some(c.clone());
        c
    }
}

// ---- TeX Gyre discovery ----

const GYRE_SUBDIR: &str = "fonts/opentype/public/tex-gyre";

/// TeX Gyre font folders under the given TeX roots (`texmf-dist`-style
/// directories). A root may also be a folder of year directories (MacTeX's
/// `/usr/local/texlive`, TeX Live's `/usr/share/texlive`): each child with a
/// `texmf-dist` is searched too. Only existing folders are returned.
pub(crate) fn texgyre_dirs_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    for r in roots {
        push(r.join(GYRE_SUBDIR));
        push(r.join("texmf-dist").join(GYRE_SUBDIR));
        if let Ok(rd) = std::fs::read_dir(r) {
            let mut kids: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            kids.sort();
            for k in kids {
                push(k.join("texmf-dist").join(GYRE_SUBDIR));
            }
        }
    }
    out
}

/// The usual TeX roots of this machine (no process is run).
pub(crate) fn default_tex_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "/usr/local/texlive",
        "/usr/share/texlive",
        "/usr/share/texmf",
        "/usr/share/texmf-dist",
        "/usr/local/share/texmf",
        "/opt/homebrew/share/texmf",
        "/opt/texlive",
        "C:/texlive",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        v.push(PathBuf::from(home).join("texmf"));
    }
    v
}

/// The folder `kpsewhich` finds a TeX Gyre font in, if TeX is on the PATH.
#[cfg(not(target_arch = "wasm32"))]
fn kpsewhich_dir() -> Option<PathBuf> {
    use std::process::{Command, Stdio};
    let out = Command::new("kpsewhich")
        .arg("texgyretermes-regular.otf")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let p = PathBuf::from(s.lines().next()?.trim());
    p.parent().map(Path::to_path_buf).filter(|d| d.is_dir())
}

/// Load a folder of fonts (no-op on wasm32).
#[cfg(not(target_arch = "wasm32"))]
fn load_dir(db: &mut Database, dir: &Path) {
    db.load_fonts_dir(dir);
}
#[cfg(target_arch = "wasm32")]
fn load_dir(_db: &mut Database, _dir: &Path) {}

/// The database of this machine: system fonts, TeX Gyre, extra folders.
fn build_db(extra: &[PathBuf]) -> Database {
    let mut db = Database::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        db.load_system_fonts();
        let mut dirs = texgyre_dirs_in(&default_tex_roots());
        if let Some(d) = kpsewhich_dir() {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        for d in &dirs {
            load_dir(&mut db, d);
        }
    }
    for d in extra {
        load_dir(&mut db, d);
    }
    db
}

// ---- process-wide state ----

struct Global {
    builtin: Option<bool>,
    extra: Vec<PathBuf>,
    resolver: Option<Resolver>,
}

fn global() -> &'static Mutex<Global> {
    static G: OnceLock<Mutex<Global>> = OnceLock::new();
    G.get_or_init(|| {
        Mutex::new(Global {
            builtin: None,
            extra: Vec::new(),
            resolver: None,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Global> {
    global().lock().unwrap_or_else(|e| e.into_inner())
}

fn env_builtin() -> bool {
    std::env::var("FIG2DEV_BUILTIN_FONTS").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn builtin_in(g: &Global) -> bool {
    cfg!(target_arch = "wasm32") || g.builtin.unwrap_or_else(env_builtin)
}

/// Force (or release) the built-in fonts for the whole process. The default
/// follows the environment variable `FIG2DEV_BUILTIN_FONTS` (`1` forces).
pub fn set_builtin_fonts(on: bool) {
    lock().builtin = Some(on);
}

/// Whether only the built-in fonts are used (always on wasm32).
pub fn builtin_fonts() -> bool {
    builtin_in(&lock())
}

/// Add folders to search for fonts. Takes effect for text laid out later.
pub fn add_font_dirs(dirs: &[PathBuf]) {
    let mut g = lock();
    for d in dirs {
        if !g.extra.contains(d) {
            g.extra.push(d.clone());
        }
    }
    // The cached choices may change: look again.
    g.resolver = None;
}

/// The installed face for a request (None: use the built-in fonts).
pub(crate) fn pick_system(req: &super::FontRequest) -> Option<Picked> {
    let mut g = lock();
    if builtin_in(&g) {
        return None;
    }
    if g.resolver.is_none() {
        let db = build_db(&g.extra);
        g.resolver = Some(Resolver::with_db(db));
    }
    g.resolver.as_mut()?.resolve(req)
}

/// Installed faces with broad symbol coverage (empty with the built-in fonts).
pub(crate) fn symbol_chain() -> Arc<Vec<&'static LoadedFace>> {
    let mut g = lock();
    if builtin_in(&g) {
        return Arc::new(Vec::new());
    }
    if g.resolver.is_none() {
        let db = build_db(&g.extra);
        g.resolver = Some(Resolver::with_db(db));
    }
    match g.resolver.as_mut() {
        Some(r) => r.chain(),
        None => Arc::new(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FontRequest;
    use fontdb::{FaceInfo, Language, Source};

    fn fake_db(entries: &[(&str, u16, Style)]) -> Database {
        let mut db = Database::new();
        for (fam, w, st) in entries {
            db.push_face_info(FaceInfo {
                id: ID::dummy(),
                source: Source::Binary(Arc::new(epaint_default_fonts::UBUNTU_LIGHT.to_vec())),
                index: 0,
                families: vec![((*fam).to_string(), Language::English_UnitedStates)],
                post_script_name: format!("{fam}-{w}"),
                style: *st,
                weight: Weight(*w),
                stretch: Stretch::Normal,
                monospaced: false,
            });
        }
        db
    }

    fn fam(db: &Database, id: Option<ID>) -> Option<String> {
        db.face(id?).and_then(|f| f.families.first().map(|(n, _)| n.clone()))
    }

    #[test]
    fn exact_beats_gyre_beats_generic() {
        let db = fake_db(&[
            ("Liberation Serif", 400, Style::Normal),
            ("TeX Gyre Termes", 400, Style::Normal),
            ("Times New Roman", 400, Style::Normal),
        ]);
        let req = FontRequest::for_fig(0, true);
        assert_eq!(fam(&db, choose(&db, &req)).as_deref(), Some("Times New Roman"));
        let db = fake_db(&[
            ("Liberation Serif", 400, Style::Normal),
            ("TeX Gyre Termes", 400, Style::Normal),
        ]);
        assert_eq!(fam(&db, choose(&db, &req)).as_deref(), Some("TeX Gyre Termes"));
        let db = fake_db(&[("Liberation Serif", 400, Style::Normal)]);
        assert_eq!(fam(&db, choose(&db, &req)).as_deref(), Some("Liberation Serif"));
        let db = fake_db(&[("Unrelated", 400, Style::Normal)]);
        assert!(choose(&db, &req).is_none());
    }

    #[test]
    fn per_platform_names() {
        let mac = fake_db(&[("Helvetica", 400, Style::Normal), ("Courier", 400, Style::Normal)]);
        assert_eq!(
            fam(&mac, choose(&mac, &FontRequest::for_fig(16, true))).as_deref(),
            Some("Helvetica")
        );
        assert_eq!(
            fam(&mac, choose(&mac, &FontRequest::for_fig(12, true))).as_deref(),
            Some("Courier")
        );
        let linux = fake_db(&[("Nimbus Sans", 400, Style::Normal), ("P052", 400, Style::Normal)]);
        assert_eq!(
            fam(&linux, choose(&linux, &FontRequest::for_fig(16, true))).as_deref(),
            Some("Nimbus Sans")
        );
        assert_eq!(
            fam(&linux, choose(&linux, &FontRequest::for_fig(28, true))).as_deref(),
            Some("P052")
        );
        let win = fake_db(&[
            ("Palatino Linotype", 400, Style::Normal),
            ("Symbol", 400, Style::Normal),
        ]);
        assert_eq!(
            fam(&win, choose(&win, &FontRequest::for_fig(30, true))).as_deref(),
            Some("Palatino Linotype")
        );
        assert_eq!(
            fam(&win, choose(&win, &FontRequest::for_fig(32, true))).as_deref(),
            Some("Symbol")
        );
        // Narrow falls back to regular-width Helvetica.
        assert_eq!(
            fam(&mac, choose(&mac, &FontRequest::for_fig(20, true))).as_deref(),
            Some("Helvetica")
        );
    }

    #[test]
    fn weight_and_style_follow_the_postscript_font() {
        let db = fake_db(&[
            ("Times", 400, Style::Normal),
            ("Times", 700, Style::Normal),
            ("Times", 400, Style::Italic),
            ("Times", 700, Style::Italic),
        ]);
        let pick = |n: i32| {
            let id = choose(&db, &FontRequest::for_fig(n, true)).unwrap();
            let f = db.face(id).unwrap();
            (f.weight.0, f.style)
        };
        assert_eq!(pick(0), (400, Style::Normal));
        assert_eq!(pick(1), (400, Style::Italic));
        assert_eq!(pick(2), (700, Style::Normal));
        assert_eq!(pick(3), (700, Style::Italic));
    }

    #[test]
    fn resolver_loads_and_flags_synthetic_styles() {
        let db = fake_db(&[("Times", 400, Style::Normal)]);
        let mut r = Resolver::with_db(db);
        let p = r.resolve(&FontRequest::for_fig(3, true)).unwrap();
        assert!(p.face.system && p.synth_bold && p.synth_italic);
        let p = r.resolve(&FontRequest::for_fig(0, true)).unwrap();
        assert!(!p.synth_bold && !p.synth_italic);
        // Cached.
        assert!(r.resolve(&FontRequest::for_fig(0, true)).is_some());
        let none =
            Resolver::with_db(fake_db(&[("Unrelated", 400, Style::Normal)])).resolve(&FontRequest::for_fig(0, true));
        assert!(none.is_none());
    }

    #[test]
    fn symbol_chain_lists_installed_families() {
        let db = fake_db(&[("Menlo", 400, Style::Normal), ("DejaVu Sans", 400, Style::Normal)]);
        let mut r = Resolver::with_db(db);
        assert_eq!(r.chain().len(), 2);
    }

    #[test]
    fn texgyre_found_in_a_fake_texmf_tree() {
        let base = std::env::temp_dir().join(format!("fig2dev-texmf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let year = base.join("texlive/2025/texmf-dist").join(GYRE_SUBDIR);
        let flat = base.join("texmf").join(GYRE_SUBDIR);
        std::fs::create_dir_all(&year).unwrap();
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::create_dir_all(base.join("texlive/2025/other")).unwrap();
        let dirs = texgyre_dirs_in(&[base.join("texlive"), base.join("texmf"), base.join("missing")]);
        assert_eq!(dirs, vec![year, flat]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn builtin_switch() {
        let mk = |b| Global {
            builtin: b,
            extra: Vec::new(),
            resolver: None,
        };
        assert!(builtin_in(&mk(Some(true))));
        assert_eq!(builtin_in(&mk(Some(false))), cfg!(target_arch = "wasm32"));
        // The process-wide switch (never switched off here: tests run in parallel).
        set_builtin_fonts(true);
        assert!(builtin_fonts());
        assert!(pick_system(&FontRequest::for_fig(0, true)).is_none());
        assert!(symbol_chain().is_empty());
        assert!(!crate::text::select(&FontRequest::for_fig(0, true)).is_some_and(|(f, _, _)| f.system));
    }
}
