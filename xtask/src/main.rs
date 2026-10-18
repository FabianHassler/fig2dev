//! Repository checks: `cargo xtask licences`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SKIP_DIRS: &[&str] = &["target", ".git", "old"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// Every Rust file with code ported from xfig or fig2dev points to
/// LICENSE-xfig, and LICENSE-xfig lists exactly those files.
fn licences() -> Result<(), String> {
    let root = root();
    let notice = std::fs::read_to_string(root.join("LICENSE-xfig")).map_err(|e| format!("LICENSE-xfig: {e}"))?;
    let listed: Vec<String> = notice
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter_map(|l| l.split_once(':').map(|(p, _)| p.trim().to_string()))
        .collect();
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut errors = Vec::new();
    let mut cited = Vec::new();
    for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "rs")) {
        let rel = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if rel.starts_with("xtask/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let ported = text.contains("Ported from xfig") || text.contains("Ported from fig2dev");
        let cites = text.contains("LICENSE-xfig");
        if ported && !cites {
            errors.push(format!(
                "{rel}: ported from xfig or fig2dev but doesn't point to LICENSE-xfig"
            ));
        }
        if cites {
            if !listed.contains(&rel) {
                errors.push(format!("{rel}: cites LICENSE-xfig but isn't listed there"));
            }
            cited.push(rel);
        }
    }
    for l in &listed {
        if !cited.contains(l) {
            errors.push(format!(
                "LICENSE-xfig lists {l}, which doesn't exist or doesn't cite it"
            ));
        }
    }
    if errors.is_empty() {
        println!("licences: ok ({} files derived from xfig and fig2dev)", cited.len());
        Ok(())
    } else {
        Err(format!("licence problems:\n  {}", errors.join("\n  ")))
    }
}

fn main() -> ExitCode {
    let result = match std::env::args().nth(1).as_deref() {
        Some("licences") => licences(),
        _ => Err("usage: cargo xtask licences".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
