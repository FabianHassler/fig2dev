//! fig2dev's output drivers, ported to Rust (plan B9 and friends), with
//! the parts of fig2dev they stand on: its object model (`object`), its
//! reader (`read`), its bounds (`bound`), its spline conversion
//! (`trans_spline`), the driver interface and the main flow (`gendev`).
//!
//! EPS, PDF, SVG, the bitmaps and the LaTeX box are drawn from a display
//! list (`scene`: RustFig's writers and rasteriser, which RustFig's canvas
//! shares); the other languages are ported drivers. A figure reaches a
//! driver as xfig hands it to fig2dev: as Fig text, read by the ported
//! `read.c`. `options` and `languages` describe every language and option.
//!
//! Output goes to an [`Out`] buffer with [`fprintf!`], so ported code
//! reads like the C it came from; messages fig2dev prints on stderr are
//! collected in [`Ctx::messages`]; where fig2dev exits with an error, the
//! port returns `Err`.

pub mod bound;
pub mod drivers;
pub mod gendev;
pub mod languages;
pub mod object;
pub mod options;
pub mod read;
pub mod scene;
pub mod trans_spline;
pub mod util;

pub use gendev::{convert, Ctx, Driver, Status};

/// The output file (`tfp`): bytes, since some languages are binary or use
/// an 8-bit encoding.
#[derive(Debug, Default)]
pub struct Out {
    pub buf: Vec<u8>,
}

impl Out {
    pub fn new() -> Out {
        Out::default()
    }
    /// `fputs`.
    pub fn puts(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }
    /// `fwrite`, `putc`.
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn print(&mut self, args: std::fmt::Arguments<'_>) {
        // Writing to a Vec can't fail.
        let _ = std::fmt::Write::write_fmt(self, args);
    }
}

impl std::fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// `fprintf(tfp, …)` with Rust's format syntax: `fprintf!(out, "{:.3}", x)`.
#[macro_export]
macro_rules! fprintf {
    ($out:expr, $($arg:tt)*) => {
        $out.print(format_args!($($arg)*))
    };
}

/// C's `round()` macro from fig2dev.h: halves away from zero, to `i32`
/// (saturating; NaN gives 0).
pub fn round(x: f64) -> i32 {
    (if x >= 0.0 { x + 0.5 } else { x - 0.5 }) as i32
}

#[cfg(test)]
mod round_tests {
    use super::round;

    /// fig2dev.h: `(int)((x) + ((x >= 0)? 0.5: -0.5))`, truncating, with
    /// the double rounding of the sum (0.49999999999999994 gives 1) and
    /// arm64's saturating, NaN-to-0 conversion.
    #[test]
    fn round_is_the_c_macro() {
        assert_eq!(round(0.5), 1);
        assert_eq!(round(-0.5), -1);
        assert_eq!(round(2.4999), 2);
        assert_eq!(round(-2.5), -3);
        assert_eq!(round(-0.0), 0);
        assert_eq!(round(0.499_999_999_999_999_94), 1);
        assert_eq!(round(1e30), i32::MAX);
        assert_eq!(round(-1e30), i32::MIN);
        assert_eq!(round(f64::NAN), 0);
    }
}
