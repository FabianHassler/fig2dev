//! LaTeX box: an empty `\makebox` of the figure's size, to reserve its space.
//
// Ported from fig2dev dev/genbox.c (xfig licence, see LICENSE-xfig).

use figbase::geom::{Rect, FIG_PER_INCH};

/// The box for `bounds` (Fig units) at `scale` (magnification, with the
/// metric correction), preceded by the whole-figure comments as
/// `print_comments("% ", comments, "")` writes them: `"%  {comment} "` per
/// comment, its bytes as read.
pub fn to_box(comments: &[Vec<u8>], bounds: &Rect, scale: f64) -> Vec<u8> {
    let mut out = Vec::new();
    for c in comments {
        out.extend_from_slice(b"%  ");
        out.extend_from_slice(c);
        out.extend_from_slice(b" \n");
    }
    let inches = |d: f64| d * scale / FIG_PER_INCH;
    out.extend_from_slice(
        format!(
            "\\makebox[{:.3}in]{{\\rule{{0in}}{{{:.3}in}}}}\n",
            inches(bounds.width()),
            inches(bounds.height())
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use figbase::geom::Point;

    #[test]
    fn box_of_two_by_one_inches() {
        let r = Rect::from_points([Point::new(0.0, 0.0), Point::new(2400.0, 1200.0)]);
        let s = to_box(&[b"figure".to_vec(), b"\xe9 notes".to_vec()], &r, 1.0);
        assert_eq!(
            s,
            b"%  figure \n%  \xe9 notes \n\\makebox[2.000in]{\\rule{0in}{1.000in}}\n"
        );
        assert!(String::from_utf8_lossy(&to_box(&[], &r, 0.5)).contains("[1.000in]"));
    }
}
