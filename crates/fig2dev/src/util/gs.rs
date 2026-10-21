//! Ghostscript, as an optional helper for EPS pictures (not on wasm).
//!
//! The C fig2dev runs ghostscript on everything it writes as PostScript:
//! for PDF and bitmap output an EPS picture is drawn by it. This port draws
//! no PostScript itself. For an EPS picture in PDF, bitmap and canvas
//! output it asks ghostscript, when the program is installed, to turn the
//! file into a one-page PDF (`pdfwrite`), which then goes the way of a PDF
//! picture ([`super::pdfpic`]). Without ghostscript the preview image of
//! the file is used (bitmaps), or the picture is left out.
//!
//! - The program is looked for on `PATH` at run time (`gs`; `gswin64c`,
//!   `gswin32c`, `gs` on Windows); it is never required.
//! - It runs without a shell, as `gs -dSAFER ... -` on the file's bytes
//!   through a pipe (no file names reach a command line), with a time limit
//!   ([`TIMEOUT`]) and a limit on its output ([`MAX_OUTPUT`]).
//! - Results are kept for the next request (the canvas draws the same
//!   picture again and again).
//
// Ported from fig2dev dev/readeps.c (`has_gs`, `gsexe`: calling
// ghostscript, the message when it is missing) (xfig licence, see
// LICENSE-xfig).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long ghostscript may run on one picture.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// The most PDF data taken from ghostscript, in bytes.
pub const MAX_OUTPUT: u64 = 256 << 20;
/// The most EPS data given to ghostscript, in bytes.
pub const MAX_INPUT: usize = 256 << 20;

/// fig2dev's message when ghostscript can't be run (`readeps.c`), the
/// text of the PDF case with "pdf files" for what is embedded.
pub const MISSING_MESSAGE: &str =
    "Could not call ghostscript executable, gs.\nPlease install ghostscript to embed eps files in pdf and bitmap output.";

/// The ghostscript program on `PATH`, if there is one.
pub fn find() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["gswin64c.exe", "gswin32c.exe", "gs.exe"]
    } else {
        &["gs"]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

type CacheEntry = (u64, usize, Arc<Vec<u8>>);

fn cache() -> &'static Mutex<Vec<CacheEntry>> {
    static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());
    &CACHE
}

/// Turns an EPS file into a one-page PDF (the page is the file's bounding
/// box). `Err` carries a message: ghostscript is not installed (with
/// [`MISSING_MESSAGE`]), failed, ran too long or gave no PDF.
pub fn eps_to_pdf(eps: &[u8]) -> Result<Arc<Vec<u8>>, String> {
    if eps.len() > MAX_INPUT {
        return Err("EPS file too large for ghostscript".into());
    }
    let mut h = DefaultHasher::new();
    eps.hash(&mut h);
    let key = (h.finish(), eps.len());
    if let Ok(c) = cache().lock() {
        if let Some(e) = c.iter().find(|e| (e.0, e.1) == key) {
            return Ok(e.2.clone());
        }
    }
    let exe = find().ok_or_else(|| MISSING_MESSAGE.to_string())?;
    let pdf = Arc::new(run(&exe, eps)?);
    if let Ok(mut c) = cache().lock() {
        if c.len() >= 16 {
            c.remove(0);
        }
        c.push((key.0, key.1, pdf.clone()));
    }
    Ok(pdf)
}

fn run(exe: &PathBuf, eps: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(exe)
        .args([
            "-q",
            "-dSAFER",
            "-dNOPAUSE",
            "-dBATCH",
            "-dEPSCrop",
            "-dAutoRotatePages=/None",
            "-sDEVICE=pdfwrite",
            "-dCompatibilityLevel=1.5",
            "-sstdout=%stderr",
            "-sOutputFile=-",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot run ghostscript to draw an EPS picture".to_string())?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let input = eps.to_vec();
    // The pipes are served by threads, so that a full pipe can't stop the
    // time limit from being checked.
    let writer = std::thread::spawn(move || {
        if let Some(mut s) = stdin {
            // ghostscript may stop reading early (an error); that is not ours
            let _ = s.write_all(&input);
        }
    });
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(s) = stdout {
            let _ = s.take(MAX_OUTPUT + 1).read_to_end(&mut out);
        }
        out
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                // too long, or the wait failed: stop it
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let _ = writer.join();
    let out = reader.join().map_err(|_| "Error calling ghostscript".to_string())?;
    match status {
        None => Err("ghostscript took too long to draw an EPS picture".into()),
        Some(s) if !s.success() => Err("ghostscript failed to draw an EPS picture".into()),
        Some(_) if out.len() as u64 > MAX_OUTPUT => Err("ghostscript's output is too large".into()),
        Some(_) if !out.starts_with(b"%PDF") => Err("ghostscript gave no PDF for an EPS picture".into()),
        Some(_) => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_when_installed() {
        if find().is_none() {
            return;
        }
        let eps =
            b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 50 40\n1 0 0 setrgbcolor 0 0 50 40 rectfill\nshowpage\n";
        let pdf = eps_to_pdf(eps).expect("pdf");
        let p = super::super::pdfpic::PdfPicture::open(pdf.to_vec()).expect("opens");
        assert_eq!(p.size(), (50.0, 40.0));
        // again from the cache
        assert_eq!(eps_to_pdf(eps).expect("pdf"), pdf);
    }

    #[test]
    fn garbage_gives_an_error_not_a_panic() {
        if find().is_none() {
            return;
        }
        let _ = eps_to_pdf(b"%!PS\n(((( unbalanced\n");
    }
}
