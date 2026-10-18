#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::approx_constant)]
//! FMA parity with the reference binary (audit pass 12): clang fuses
//!  in  and . The cases below are
//! ones where the fused and the unfused C give different results, recorded
//! from oracle programs (the C functions built with arm64 clang -O2,
//! ). The d tests compare against the complete
//! oracle outputs (412,800 ellipses, 20,000 splines) when
//!  names a directory with  and .

use fig2dev::bound::ellipse_bound;
use fig2dev::object::{Control, Ellipse, Point, Spline, T_CLOSED_XSPLINE, T_OPEN_XSPLINE};
use fig2dev::trans_spline::{create_line_with_spline, HIGH_PRECISION};
use fig2dev::Ctx;

const ANGLES: [f64; 8] = [0.0, 0.3, 0.5236, 1.0472, 0.7854, 1.5708, -0.2, 2.5];

fn ellipse(ai: usize, a: i32, b: i32) -> (i32, i32, i32, i32) {
    let e = Ellipse {
        thickness: 15,
        center: Point::new(6000, 3000),
        radiuses: Point::new(a, b),
        angle: ANGLES[ai],
        ..Ellipse::default()
    };
    ellipse_bound(&e)
}

fn checksum(pts: &[Point]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in pts {
        for v in [p.x, p.y] {
            h = (h ^ u64::from(v as u32)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

fn spline(type_: i32, pts: &[(i32, i32)], s: &[f64]) -> Spline {
    Spline {
        type_,
        points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        controls: s
            .iter()
            .map(|&s| Control {
                s,
                ..Control::default()
            })
            .collect(),
        ..Spline::default()
    }
}

fn convert(type_: i32, pts: &[(i32, i32)], s: &[f64]) -> Vec<Point> {
    let mut ctx = Ctx::default();
    let l = create_line_with_spline(&mut ctx, &spline(type_, pts, s));
    let _ = HIGH_PRECISION;
    l.map(|l| l.points).unwrap_or_default()
}

/// (angle index, a, b, oracle bounds) where the unfused C differs by 1.
type EllipseCase = (usize, i32, i32, (i32, i32, i32, i32));
const ELLIPSES: &[EllipseCase] = &[
    (0, 3, 99, (5989, 2894, 6011, 3106)),
    (0, 24, 99, (5968, 2894, 6032, 3106)),
    (0, 38, 15, (5954, 2978, 6046, 3022)),
    (0, 52, 99, (5940, 2894, 6060, 3106)),
    (0, 66, 589, (5926, 2404, 6074, 3596)),
    (0, 78, 568, (5914, 2426, 6086, 3574)),
    (0, 93, 29, (5899, 2964, 6101, 3036)),
    (0, 111, 225, (5881, 2768, 6119, 3232)),
    (0, 129, 29, (5863, 2964, 6137, 3036)),
    (0, 143, 36, (5849, 2957, 6151, 3043)),
    (0, 157, 211, (5835, 2782, 6165, 3218)),
    (0, 180, 127, (5812, 2865, 6188, 3135)),
    (0, 199, 183, (5793, 2810, 6207, 3190)),
    (0, 214, 43, (5778, 2950, 6222, 3050)),
    (0, 235, 71, (5757, 2922, 6243, 3078)),
    (0, 259, 22, (5733, 2971, 6267, 3029)),
    (0, 272, 120, (5720, 2873, 6280, 3127)),
    (0, 281, 218, (5711, 2776, 6289, 3224)),
    (0, 292, 260, (5700, 2734, 6300, 3266)),
    (0, 317, 134, (5675, 2859, 6325, 3141)),
    (0, 336, 36, (5656, 2958, 6344, 3042)),
    (0, 355, 50, (5637, 2943, 6363, 3057)),
    (0, 368, 15, (5624, 2978, 6376, 3022)),
    (0, 376, 92, (5616, 2901, 6384, 3099)),
    (0, 398, 183, (5594, 2810, 6406, 3190)),
    (0, 412, 29, (5580, 2963, 6420, 3037)),
    (0, 425, 456, (5567, 2538, 6433, 3462)),
    (0, 445, 337, (5547, 2655, 6453, 3345)),
    (0, 468, 323, (5524, 2670, 6476, 3330)),
    (0, 489, 274, (5503, 2720, 6497, 3280)),
    (0, 513, 78, (5479, 2915, 6521, 3085)),
    (0, 522, 323, (5470, 2669, 6530, 3331)),
    (0, 537, 302, (5455, 2692, 6545, 3308)),
    (0, 547, 337, (5445, 2656, 6555, 3344)),
    (0, 561, 295, (5431, 2698, 6569, 3302)),
    (0, 574, 162, (5418, 2832, 6582, 3168)),
    (0, 583, 29, (5409, 2964, 6591, 3036)),
    (0, 596, 29, (5396, 2963, 6604, 3037)),
    (1, 491, 491, (5501, 2501, 6499, 3499)),
    (3, 246, 246, (5746, 2748, 6254, 3252)),
    (4, 92, 92, (5900, 2900, 6100, 3100)),
    (4, 470, 470, (5522, 2522, 6478, 3478)),
];

/// (type, points, shape factors, point count, checksum) where the unfused C differs.
type SplineCase = (i32, &'static [(i32, i32)], &'static [f64], usize, u64);
const SPLINES: &[SplineCase] = &[
    (
        4,
        &[(7916, 2118), (5836, 15425), (3911, 2359), (7009, 1378), (-1955, -665)],
        &[1.0, 1.0, -0.9, 0.3, 1.0],
        323,
        0xd48fbfbf114aa6d7,
    ),
    (
        4,
        &[
            (-1908, 15045),
            (10331, 1984),
            (12439, 6339),
            (9481, 2228),
            (17035, 12777),
            (11847, 15644),
            (14403, 10162),
        ],
        &[0.3, 1.0, 0.0, 1.0, -0.7, 1.0, -0.9],
        460,
        0x18f3e268542f0050,
    ),
    (
        5,
        &[(12877, 5009), (2221, 2095), (2598, 9718), (8026, 7037)],
        &[1.0, -0.4, -1.0, -1.0],
        333,
        0x34609d502cb922f3,
    ),
    (
        5,
        &[
            (3227, -1801),
            (1467, 9981),
            (7351, 11313),
            (9797, 5193),
            (1689, 14480),
            (11775, 2356),
            (17650, -389),
        ],
        &[1.0, -0.6, 1.0, -1.0, 0.0, -0.7, -1.0],
        673,
        0x436696513a070cf4,
    ),
    (
        4,
        &[(3834, 17667), (7939, 8200), (9890, 1862)],
        &[-1.0, 1.0, -0.4],
        183,
        0x7d898e34f35bd956,
    ),
    (
        5,
        &[
            (-1735, 13463),
            (15796, 3067),
            (9893, 1405),
            (14070, 17898),
            (12123, 2854),
        ],
        &[1.0, -0.4, 1.0, 1.0, -1.0],
        491,
        0x61611fcbc0ef9c44,
    ),
    (
        5,
        &[(16526, 5637), (572, 3774), (17081, 2458), (7533, 1444), (3992, 4367)],
        &[0.0, 1.0, -0.7, -1.0, 0.6],
        481,
        0x9a246b54eb5c48a9,
    ),
    (
        4,
        &[(717, 6292), (11536, 492), (8785, -482), (16305, 8351), (11305, 7280)],
        &[1.0, -1.0, 1.0, -0.4, -1.0],
        331,
        0x7d31819d32fd4989,
    ),
    (
        4,
        &[(6147, -343), (17406, 8339), (3078, 4008), (6111, -604), (6312, 13433)],
        &[1.0, -0.6, -1.0, 1.0, 1.0],
        401,
        0xc69388d66ebd31ff,
    ),
    (
        5,
        &[(6570, 11352), (6146, 8870), (11833, 710), (6909, 15204), (12439, 14215)],
        &[-1.0, 1.0, -0.6, 1.0, 0.0],
        414,
        0x71403b22412c3873,
    ),
    (
        5,
        &[(15777, 14923), (10537, 4064), (12149, 5547)],
        &[-0.7, 0.5, 1.0],
        267,
        0x34d5700254910cd6,
    ),
    (
        4,
        &[
            (16307, 7323),
            (3326, 13388),
            (4872, 13099),
            (6758, 16714),
            (15765, -1607),
            (14531, 7858),
            (7336, 17637),
        ],
        &[1.0, -0.9, 0.0, 1.0, 1.0, -1.0, 0.0],
        555,
        0x2a7b2bf6d89629a3,
    ),
    (
        4,
        &[
            (6836, 16891),
            (7369, 16828),
            (14910, 7217),
            (7346, 12944),
            (3939, -1538),
        ],
        &[1.0, 1.0, 1.0, -0.9, 0.0],
        323,
        0x4c669ce7dfd05259,
    ),
    (
        5,
        &[
            (8347, 12294),
            (7487, 14524),
            (17422, -902),
            (-1643, 16598),
            (9264, 9345),
        ],
        &[0.0, 0.9, -1.0, 1.0, -0.4],
        460,
        0x2262809226077056,
    ),
    (
        4,
        &[(849, 13307), (-296, 6504), (5702, 6783), (12621, 11100)],
        &[-0.7, 1.0, -0.7, 1.0],
        230,
        0x5924e29670473715,
    ),
    (
        4,
        &[
            (15818, 9397),
            (-1051, 3603),
            (12454, -1159),
            (-189, 11624),
            (9393, 5866),
            (3012, 8296),
            (-545, 1776),
        ],
        &[1.0, -0.6, -0.5, 0.9, 1.0, 0.0, -0.4],
        562,
        0x2e171d74363de0f8,
    ),
    (
        5,
        &[(10511, 8583), (10473, 10701), (7168, 10315), (7238, 4195), (-869, 6122)],
        &[0.0, 1.0, -0.4, 1.0, 1.0],
        319,
        0xf9810bfd5f4a9201,
    ),
    (
        5,
        &[
            (7848, 12500),
            (6152, 5231),
            (14200, 11469),
            (6521, 5075),
            (5354, 13892),
            (11462, 11413),
        ],
        &[-1.0, -1.0, -0.5, 1.0, 1.0, -0.7],
        481,
        0x5619d3b79dbb6457,
    ),
    (
        5,
        &[
            (16580, 4098),
            (-907, 15675),
            (4823, 1565),
            (8611, 4000),
            (4255, 9851),
            (13107, 9325),
        ],
        &[0.0, 1.0, 1.0, -0.7, 1.0, -0.8],
        515,
        0x36f1ba162431264d,
    ),
    (
        5,
        &[
            (9666, 1517),
            (11257, 1362),
            (-341, 16335),
            (11875, 4983),
            (16750, 573),
            (6461, 3885),
            (-896, 14281),
        ],
        &[1.0, -0.6, 0.0, -0.9, 0.0, 1.0, 1.0],
        689,
        0x458ab3a6fa71c714,
    ),
    (
        5,
        &[
            (748, 6327),
            (10254, 13227),
            (466, 8317),
            (-1411, 9281),
            (5266, -133),
            (8752, 9266),
            (10253, 13251),
        ],
        &[1.0, -0.7, 1.0, -1.0, 1.0, -0.5, 0.0],
        571,
        0xe81ca99136e42a0b,
    ),
    (
        5,
        &[(4443, 12289), (11600, 9876), (10015, 13124), (12836, 8448)],
        &[-0.9, 1.0, -1.0, 1.0],
        274,
        0xdb69a9a4d1bcba24,
    ),
];

#[test]
fn ellipse_bound_is_fused_like_the_reference() {
    for &(ai, a, b, want) in ELLIPSES {
        assert_eq!(ellipse(ai, a, b), want, "angle {} a {a} b {b}", ANGLES[ai]);
    }
}

#[test]
fn splines_are_fused_like_the_reference() {
    for &(t, pts, s, n, sum) in SPLINES {
        let got = convert(t, pts, s);
        assert_eq!((got.len(), checksum(&got)), (n, sum), "{pts:?} {s:?}");
    }
}

#[test]
fn a_short_closed_spline_message_has_no_newline() {
    let mut ctx = Ctx::default();
    let s = spline(T_CLOSED_XSPLINE, &[(0, 0), (10, 10)], &[-1.0; 2]);
    assert!(create_line_with_spline(&mut ctx, &s).is_none());
    ctx.warn("Unable to convert spline to line at line 9.");
    assert_eq!(
        ctx.messages,
        ["A closed spline with less than three points.Unable to convert spline to line at line 9."]
    );
    ctx.warn("next");
    assert_eq!(ctx.messages.len(), 2);
}

fn oracle_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("FMA_ORACLE_DIR").map(Into::into)
}

#[test]
#[ignore = "needs FMA_ORACLE_DIR with on.txt"]
fn all_ellipses_match_the_oracle() {
    let dir = oracle_dir().expect("set FMA_ORACLE_DIR");
    let text = std::fs::read_to_string(dir.join("on.txt")).unwrap();
    let mut diffs = 0;
    let mut n = 0;
    for l in text.lines() {
        let f: Vec<i32> = l.split_whitespace().map(|v| v.parse().unwrap()).collect();
        let got = ellipse(f[0] as usize, f[1], f[2]);
        n += 1;
        if got != (f[3], f[4], f[5], f[6]) {
            diffs += 1;
        }
    }
    assert_eq!((n, diffs), (412_800, 0));
}

struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as u32
    }
}

#[test]
#[ignore = "needs FMA_ORACLE_DIR with ts_on.txt"]
fn all_splines_match_the_oracle() {
    let dir = oracle_dir().expect("set FMA_ORACLE_DIR");
    let text = std::fs::read_to_string(dir.join("ts_on.txt")).unwrap();
    let mut r = Xorshift(88_172_645_463_325_252);
    let sv = [-1.0, 0.0, 1.0];
    let mut diffs = 0;
    let mut n = 0;
    for l in text.lines() {
        let mut cnt = 2 + (r.next() % 6) as usize;
        let t = if !r.next().is_multiple_of(2) {
            T_OPEN_XSPLINE
        } else {
            T_CLOSED_XSPLINE
        };
        if t == T_CLOSED_XSPLINE && cnt < 3 {
            cnt = 3;
        }
        let (mut pts, mut ss) = (Vec::new(), Vec::new());
        for _ in 0..cnt {
            let x = (r.next() % 20000).wrapping_sub(2000) as i32;
            let y = (r.next() % 20000).wrapping_sub(2000) as i32;
            pts.push((x, y));
            ss.push(if r.next().is_multiple_of(4) {
                f64::from((r.next() % 21) as i32 - 10) / 10.0
            } else {
                sv[(r.next() % 3) as usize]
            });
        }
        let want: Vec<Point> = l
            .split_once(':')
            .unwrap()
            .1
            .split_whitespace()
            .map(|p| {
                let (x, y) = p.split_once(',').unwrap();
                Point::new(x.parse().unwrap(), y.parse().unwrap())
            })
            .collect();
        n += 1;
        if convert(t, &pts, &ss) != want {
            diffs += 1;
        }
    }
    assert_eq!((n, diffs), (20_000, 0));
}
