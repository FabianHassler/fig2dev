//! Reading imported pictures for the drivers that embed them (emf, tk,
//! ptk): find the file on disk (also compressed, `name.gz` for `name`),
//! uncompress it, recognise its type, and read its size and pixels into a
//! [`Pic`] the way fig2dev's `read_*` functions fill `F_pic`.
//!
//! - [`open_stream`] (`open_stream`, `file_on_disk`): relative names are
//!   taken relative to [`Ctx::dir`] (the directory of the Fig file; C uses
//!   the working directory). `.gz`, `.Z` and `.z` files are uncompressed
//!   here (gzip and Unix `compress` LZW, recognised by their magic
//!   numbers), where C pipes them through `gunzip -c`. `.zip`, `.bz2`,
//!   `.bz` and `.xz` files are found but reported as unsupported: RustFig
//!   does not run external programs.
//! - [`pic_type`]: the file signatures genps.c recognises.
//! - [`read_pic`]: fills `Pic`, with fig2dev's messages ("NAME: Bad PNG
//!   format", "NAME: Unknown image format", libpng's own lines). PNG goes
//!   through [`read_png`] (readpng.c: file palette order, libpng's gamma
//!   and alpha arithmetic with `gAMA`/`bKGD`, `tRNS` as `num_transp` and
//!   `transp_cols`). GIF, PPM, TIFF and XPM go through `giftopnm`,
//!   `ppmtopcx` and the like in C and so arrive as PCX pictures: a picture
//!   with ≤ 256 colours is an indexed bitmap (one byte per pixel, `cmap`,
//!   `numcols`, colours in order of first use where netpbm's order is a
//!   hash), one with more an RGB bitmap (`numcols` = 2^24); alpha is
//!   dropped (XPM `None` is black); a GIF's transparent colour is found in
//!   the colour map again and its index kept in `transp_cols[0]`. Other
//!   formats' alpha is composited over the background (white, or `-g`).
//!   `-N` (`grayonly`) maps colours to grey. XBM gives the packed bits
//!   (`numcols` 0), EPS and PDF only their bounding box.
//!
//! PNG is decoded by the `png` crate, GIF by `gif`, JPEG and TIFF by the
//! `image` crate; PCX, XBM and PPM are ports of readpcx.c, readxbm.c and
//! readppm.c, XPM a reader written for RustFig (C converts it with external
//! programs). Sizes are capped ([`MAX_PICFILE_BYTES`], [`MAX_PIXELS`]);
//! hostile files give an error, never a panic.
//
// Ported from fig2dev dev/readpics.c, dev/readpics.h, dev/readeps.c,
// dev/readpcx.c, dev/readpng.c, dev/readppm.c, dev/readxbm.c, dev/readgif.c
// and the picture signatures of dev/genps.c (xfig licence, see LICENSE-xfig).

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use figbase::image::RasterImage;

use super::readjpg;
use crate::gendev::Ctx;
use crate::object::{Pic, Point, P_EPS, P_GIF, P_JPEG, P_PCX, P_PNG, P_PPM, P_TIF, P_XBM, P_XPM};

/// Largest picture file read, after uncompressing (256 MiB).
pub const MAX_PICFILE_BYTES: usize = 256 << 20;
/// Largest picture, in pixels (32 Mi).
pub const MAX_PIXELS: u64 = 1 << 25;
/// Largest side of a picture, in pixels.
pub const MAX_SIDE: u32 = 1 << 16;

/// `num_transp` values (object.h): no transparency, or the transparent
/// colour given in `transp_col`.
pub const NO_TRANSPARENCY: i32 = 0;
pub const TRANSP_COLOR: i32 = -1;
/// `numcols` of an RGB bitmap.
pub const RGB_NUMCOLS: i32 = 1 << 24;

/// How a picture file on disk is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    /// `.gz`, `.Z`, `.z`: gzip or `compress`, told apart by content.
    Gzip,
    Zip,
    Bzip2,
    Xz,
}

/// Compression suffixes, in fig2dev's order (`filetypes`).
const SUFFIXES: [(&str, Compression); 7] = [
    (".gz", Compression::Gzip),
    (".Z", Compression::Gzip),
    (".z", Compression::Gzip),
    (".zip", Compression::Zip),
    (".bz2", Compression::Bzip2),
    (".bz", Compression::Bzip2),
    (".xz", Compression::Xz),
];

/// An opened picture (`struct xfig_stream`), its content uncompressed.
#[derive(Clone, Debug)]
pub struct PicStream {
    /// The name in the Fig file.
    pub name: String,
    /// The file read, e.g. `image.ppm.gz` for `image.ppm`.
    pub path: PathBuf,
    pub compression: Compression,
    /// The uncompressed content.
    pub data: Vec<u8>,
}

/// The path of picture `name`: relative names relative to `ctx.dir`.
pub fn pic_path(ctx: &Ctx, name: &str) -> PathBuf {
    let p = Path::new(name);
    match &ctx.dir {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p.to_path_buf(),
    }
}

fn compression_of(name: &str) -> Compression {
    let suffix = name.rfind('.').and_then(|i| name.get(i..)).unwrap_or("");
    SUFFIXES
        .iter()
        .find(|(s, _)| *s == suffix)
        .map_or(Compression::None, |&(_, c)| c)
}

/// `file_on_disk`: the file that holds picture `path`: the file itself, or
/// with a compression suffix added, or — if `path` has a compression
/// suffix but only the uncompressed file exists — that one.
pub fn file_on_disk(path: &Path) -> Option<(PathBuf, Compression)> {
    let name = path.to_string_lossy();
    if path.exists() {
        return Some((path.to_path_buf(), compression_of(&name)));
    }
    for (suffix, c) in SUFFIXES {
        let mut s = path.as_os_str().to_os_string();
        s.push(suffix);
        let p = PathBuf::from(s);
        if p.exists() {
            return Some((p, c));
        }
    }
    let i = name.rfind('.')?;
    let suffix = name.get(i..)?;
    if SUFFIXES.iter().any(|(s, _)| *s == suffix) {
        let p = PathBuf::from(name.get(..i)?);
        if p.exists() {
            return Some((p, Compression::None));
        }
    }
    None
}

/// `open_stream`: finds, reads and uncompresses picture `name`.
pub fn open_stream(ctx: &Ctx, name: &str) -> Result<PicStream, String> {
    open_path(ctx, pic_path(ctx, name), name)
}

/// [`open_stream`] for a picture of the figure: the file is found by the
/// name's raw bytes (a Latin-1 name in a legacy file opens on Unix).
pub fn open_stream_pic(ctx: &Ctx, pic: &Pic) -> Result<PicStream, String> {
    let p = Path::new(&pic.os_name()).to_path_buf();
    let wanted = match &ctx.dir {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p,
    };
    open_path(ctx, wanted, &pic.file)
}

fn open_path(_ctx: &Ctx, wanted: PathBuf, name: &str) -> Result<PicStream, String> {
    let (path, compression) = file_on_disk(&wanted).ok_or_else(|| format!("{name}: No such picture file"))?;
    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut raw = Vec::new();
    file.take(MAX_PICFILE_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if raw.len() > MAX_PICFILE_BYTES {
        return Err(format!("{}: picture file too large", path.display()));
    }
    let data = uncompress(&raw, compression).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(PicStream {
        name: name.to_string(),
        path,
        compression,
        data,
    })
}

/// Uncompresses `data`.
pub fn uncompress(data: &[u8], c: Compression) -> Result<Vec<u8>, String> {
    match c {
        Compression::None => Ok(data.to_vec()),
        Compression::Gzip => match data {
            [0x1f, 0x8b, ..] => gunzip(data),
            [0x1f, 0x9d, ..] => uncompress_lzw(data),
            _ => Err("not in gzip or compress format".into()),
        },
        Compression::Zip => Err("zip-compressed pictures are not supported".into()),
        Compression::Bzip2 => Err("bzip2-compressed pictures are not supported".into()),
        Compression::Xz => Err("xz-compressed pictures are not supported".into()),
    }
}

/// Uncompresses a gzip file (its first member).
pub fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    const FHCRC: u8 = 2;
    const FEXTRA: u8 = 4;
    const FNAME: u8 = 8;
    const FCOMMENT: u8 = 16;
    let bad = || "corrupt gzip header".to_string();
    if data.get(..3) != Some(&[0x1f, 0x8b, 8][..]) {
        return Err(bad());
    }
    let flags = *data.get(3).ok_or_else(bad)?;
    let mut pos = 10usize;
    if flags & FEXTRA != 0 {
        let lo = *data.get(pos).ok_or_else(bad)?;
        let hi = *data.get(pos + 1).ok_or_else(bad)?;
        pos += 2 + usize::from(u16::from_le_bytes([lo, hi]));
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let len = data
                .get(pos..)
                .ok_or_else(bad)?
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(bad)?;
            pos += len + 1;
        }
    }
    if flags & FHCRC != 0 {
        pos += 2;
    }
    let body = data.get(pos..).ok_or_else(bad)?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_PICFILE_BYTES)
        .map_err(|e| format!("corrupt gzip data ({:?})", e.status))
}

/// Uncompresses a Unix `compress` (`.Z`) file, as gzip's `unlzw` does.
pub fn uncompress_lzw(data: &[u8]) -> Result<Vec<u8>, String> {
    const INIT_BITS: u32 = 9;
    const CLEAR: usize = 256;
    const FIRST: usize = 257;
    let corrupt = || "corrupt compress data".to_string();
    if data.get(..2) != Some(&[0x1f, 0x9d][..]) {
        return Err("not in compress format".into());
    }
    let flags = *data.get(2).ok_or_else(corrupt)?;
    let maxbits = u32::from(flags & 0x1f);
    let block_mode = flags & 0x80 != 0;
    if !(INIT_BITS..=16).contains(&maxbits) {
        return Err(format!("compressed with {maxbits} bits, can only handle 9 to 16"));
    }
    let body = data.get(3..).unwrap_or(&[]);
    let total_bits = body.len() as u64 * 8;
    let maxmaxcode = 1usize << maxbits;

    let mut prefix = vec![0u16; maxmaxcode];
    let mut suffix = vec![0u8; maxmaxcode];
    for (i, s) in suffix.iter_mut().enumerate().take(256) {
        *s = i as u8;
    }
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let mut n_bits = INIT_BITS;
    let mut maxcode = (1usize << n_bits) - 1;
    let mut free_ent = if block_mode { FIRST } else { 256 };
    let mut oldcode: Option<usize> = None;
    let mut finchar = 0u8;
    let mut pos: u64 = 0;
    let mut group_start: u64 = 0;

    // Codes come in groups of eight; a new code width or a clear code
    // skips the rest of the group.
    let align = |pos: u64, group_start: u64, n_bits: u32| -> u64 {
        let n = u64::from(n_bits) * 8;
        let rel = pos - group_start;
        group_start + rel.div_ceil(n) * n
    };

    while pos + u64::from(n_bits) <= total_bits {
        if free_ent > maxcode {
            pos = align(pos, group_start, n_bits);
            group_start = pos;
            n_bits += 1;
            maxcode = if n_bits == maxbits {
                maxmaxcode
            } else {
                (1usize << n_bits) - 1
            };
            continue;
        }
        let byte = (pos / 8) as usize;
        let mut word = 0u32;
        for k in 0..3 {
            word |= u32::from(body.get(byte + k).copied().unwrap_or(0)) << (8 * k);
        }
        let code = ((word >> (pos % 8)) & ((1 << n_bits) - 1)) as usize;
        pos += u64::from(n_bits);

        let Some(old) = oldcode else {
            if code >= 256 {
                return Err(corrupt());
            }
            finchar = code as u8;
            out.push(finchar);
            oldcode = Some(code);
            continue;
        };
        if code == CLEAR && block_mode {
            free_ent = FIRST - 1;
            pos = align(pos, group_start, n_bits);
            group_start = pos;
            n_bits = INIT_BITS;
            maxcode = (1usize << n_bits) - 1;
            continue;
        }
        let incode = code;
        let mut code = code;
        stack.clear();
        if code >= free_ent {
            // The KwKwK case.
            if code > free_ent {
                return Err(corrupt());
            }
            stack.push(finchar);
            code = old;
        }
        while code >= 256 {
            stack.push(*suffix.get(code).ok_or_else(corrupt)?);
            code = usize::from(*prefix.get(code).ok_or_else(corrupt)?);
            if stack.len() > maxmaxcode {
                return Err(corrupt());
            }
        }
        finchar = *suffix.get(code).ok_or_else(corrupt)?;
        stack.push(finchar);
        if out.len() + stack.len() > MAX_PICFILE_BYTES {
            return Err("uncompressed picture too large".into());
        }
        out.extend(stack.iter().rev());
        if free_ent < maxmaxcode {
            if let (Some(p), Some(s)) = (prefix.get_mut(free_ent), suffix.get_mut(free_ent)) {
                *p = old as u16;
                *s = finchar;
            }
            free_ent += 1;
        }
        oldcode = Some(incode);
    }
    Ok(out)
}

/// Picture file types fig2dev recognises (genps.c `headers[]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicType {
    Gif,
    Pcx,
    Eps,
    /// EPS with a DOS binary header.
    Epsi,
    Pdf,
    Ppm,
    Tiff,
    Xbm,
    Png,
    Jpeg,
    Xpm,
}

impl PicType {
    /// The name fig2dev gives the type.
    pub fn name(self) -> &'static str {
        match self {
            PicType::Gif => "GIF",
            PicType::Pcx => "PCX",
            PicType::Eps => "EPS",
            PicType::Epsi => "EPSI",
            PicType::Pdf => "PDF",
            PicType::Ppm => "PPM",
            PicType::Tiff => "TIFF",
            PicType::Xbm => "XBM",
            PicType::Png => "PNG",
            PicType::Jpeg => "JPEG",
            PicType::Xpm => "XPM",
        }
    }

    /// The `F_pic` subtype a picture of this type gets.
    pub fn subtype(self) -> i32 {
        match self {
            PicType::Gif => P_GIF,
            PicType::Pcx => P_PCX,
            PicType::Eps | PicType::Epsi | PicType::Pdf => P_EPS,
            PicType::Ppm => P_PPM,
            PicType::Tiff => P_TIF,
            PicType::Xbm => P_XBM,
            PicType::Png => P_PNG,
            PicType::Jpeg => P_JPEG,
            PicType::Xpm => P_XPM,
        }
    }
}

/// The type of a picture from its first bytes (genps.c's `headers`): PPM
/// is `P3` or `P6` only, JPEG a JFIF or Exif header.
pub fn pic_type(data: &[u8]) -> Option<PicType> {
    const HEADERS: [(&[u8], PicType); 14] = [
        (b"GIF", PicType::Gif),
        (b"\x0a\x05\x01", PicType::Pcx),
        (b"%!", PicType::Eps),
        (b"\xc5\xd0\xd3\xc6", PicType::Epsi),
        (b"%PDF", PicType::Pdf),
        (b"P3", PicType::Ppm),
        (b"P6", PicType::Ppm),
        (b"II*\0", PicType::Tiff),
        (b"MM\0*", PicType::Tiff),
        (b"#define", PicType::Xbm),
        (b"\x89PNG\r\n\x1a\n", PicType::Png),
        (b"\xff\xd8\xff\xe0", PicType::Jpeg),
        (b"\xff\xd8\xff\xe1", PicType::Jpeg),
        (b"/* XPM */", PicType::Xpm),
    ];
    HEADERS.iter().find(|(h, _)| data.starts_with(h)).map(|&(_, t)| t)
}

/// Reads picture `stream` into `pic` (the `read_*` functions): sets
/// `subtype`, `bit_size`, `hw_ratio` and, for bitmaps, `bitmap`, `cmap`,
/// `numcols`, `num_transp`, `transp_col`. Returns the lower left corner of
/// the bounding box (`llx`, `lly`; 0 except for EPS and PDF).
pub fn read_pic(ctx: &mut Ctx, stream: &PicStream, pic: &mut Pic) -> Result<(i32, i32), String> {
    let file = if pic.file.is_empty() {
        stream.name.clone()
    } else {
        pic.file.clone()
    };
    let t = pic_type(&stream.data).ok_or_else(|| format!("{file}: Unknown image format"))?;
    let mut msgs = Vec::new();
    let r = match t {
        PicType::Eps | PicType::Epsi => read_eps(&stream.data, pic),
        PicType::Pdf => read_pdf(&stream.data, pic),
        PicType::Xbm => read_xbm(&stream.data, pic).map(|()| (0, 0)),
        PicType::Xpm => read_xpm(ctx, &stream.data, pic).map(|()| (0, 0)),
        PicType::Pcx => read_pcx(ctx, &stream.data, pic).map(|()| (0, 0)),
        PicType::Gif => read_gif(ctx, &stream.data, pic).map(|()| (0, 0)),
        PicType::Ppm => read_ppm(ctx, &stream.data, pic).map(|()| (0, 0)),
        PicType::Tiff => read_raster(ctx, &stream.data, image::ImageFormat::Tiff, t, pic),
        PicType::Png => read_png(ctx, &stream.data, pic, &mut msgs).map(|()| (0, 0)),
        PicType::Jpeg => read_jpg(&stream.data, pic, &mut msgs).map(|()| (0, 0)),
    };
    for m in msgs {
        ctx.warn(m);
    }
    // genps.c: "%s: Bad %s format", after any message of the library
    r.map_err(|e| {
        let bad = format!("{file}: Bad {} format", t.name());
        if e.is_empty() || !e.starts_with("libpng ") {
            bad
        } else {
            format!("{e}\n{bad}")
        }
    })
}

/// `open_stream` and `read_pic` in one.
pub fn read_picture(ctx: &mut Ctx, pic: &mut Pic) -> Result<(i32, i32), String> {
    let stream = open_stream_pic(ctx, pic)?;
    read_pic(ctx, &stream, pic)
}

fn check_size(w: u64, h: u64) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    if w > u64::from(MAX_SIDE) || h > u64::from(MAX_SIDE) || w * h > MAX_PIXELS {
        return Err(format!("image of {w}×{h} pixels is too large"));
    }
    Ok(())
}

fn set_size(pic: &mut Pic, w: u32, h: u32) {
    pic.bit_size = Point::new(w as i32, h as i32);
    pic.hw_ratio = h as f32 / w as f32;
}

/// `rgb2luminance` (colors.c) on 8-bit channels.
fn luminance(c: [u8; 3]) -> u8 {
    let [r, g, b] = c.map(|v| f64::from(v) / 255.0);
    ((0.3 * r + 0.59 * g + 0.11 * b) * 255.0) as u8
}

/// The background alpha is composited over: `-g`, or white.
fn background(ctx: &Ctx) -> [u8; 3] {
    if ctx.bgspec {
        let b = ctx.background;
        [(b.red >> 8) as u8, (b.green >> 8) as u8, (b.blue >> 8) as u8]
    } else {
        [255; 3]
    }
}

/// Fills `pic`'s bitmap from RGBA pixels: indexed with ≤ 256 colours, RGB
/// otherwise; alpha composited over the background.
pub fn fill_from_rgba(ctx: &Ctx, pic: &mut Pic, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    check_size(u64::from(w), u64::from(h))?;
    let n = (w as usize) * (h as usize);
    if rgba.len() < n * 4 {
        return Err("short image data".into());
    }
    let bg = background(ctx);
    let mut transparent = false;
    let mut pixels: Vec<[u8; 3]> = Vec::with_capacity(n);
    for px in rgba.as_chunks::<4>().0.iter().take(n) {
        let (c, a) = ([px[0], px[1], px[2]], u32::from(px[3]));
        let c = if a == 255 {
            c
        } else {
            transparent = true;
            let mix = |v: u8, b: u8| ((u32::from(v) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
            [mix(c[0], bg[0]), mix(c[1], bg[1]), mix(c[2], bg[2])]
        };
        pixels.push(if ctx.grayonly { [luminance(c); 3] } else { c });
    }
    pic.num_transp = if transparent { TRANSP_COLOR } else { NO_TRANSPARENCY };
    pic.transp_col = if transparent {
        if ctx.grayonly {
            [luminance(bg); 3]
        } else {
            bg
        }
    } else {
        [0; 3]
    };

    index_pixels(pic, pixels, w, h);
    Ok(())
}

/// What `ppmtopcx` does with the pixels: a picture of at most 256 colours
/// becomes an indexed bitmap (colour map in order of first use; netpbm's
/// is in hash order), one of more an RGB bitmap.
fn index_pixels(pic: &mut Pic, pixels: Vec<[u8; 3]>, w: u32, h: u32) {
    let n = pixels.len();
    let mut index: HashMap<[u8; 3], u8> = HashMap::new();
    let mut cmap: Vec<[u8; 3]> = Vec::new();
    let mut indexed = Vec::with_capacity(n);
    for &c in &pixels {
        let i = match index.get(&c) {
            Some(&i) => i,
            None => {
                if cmap.len() == 256 {
                    cmap.clear();
                    break;
                }
                let i = cmap.len() as u8;
                index.insert(c, i);
                cmap.push(c);
                i
            }
        };
        indexed.push(i);
    }
    if indexed.len() == n {
        pic.bitmap = indexed;
        pic.numcols = cmap.len() as i32;
        pic.cmap = cmap;
    } else {
        pic.bitmap = pixels.concat();
        pic.cmap = Vec::new();
        pic.numcols = RGB_NUMCOLS;
    }
    set_size(pic, w, h);
}

/// Fills `pic` from RGB pixels the way the readers that go through
/// `giftopnm | ppmtopcx` do, with the colour `transparent` standing for
/// transparency: C finds it again in the colour map (the first entry with
/// that colour, after `-N` has made the map grey) and keeps its index in
/// `transp_cols`; if no entry has it, there is no transparency.
fn fill_indexed(ctx: &Ctx, pic: &mut Pic, w: u32, h: u32, rgb: Vec<[u8; 3]>, transparent: Option<[u8; 3]>) {
    let pixels: Vec<[u8; 3]> = if ctx.grayonly {
        rgb.iter().map(|&c| [luminance(c); 3]).collect()
    } else {
        rgb
    };
    index_pixels(pic, pixels, w, h);
    pic.num_transp = NO_TRANSPARENCY;
    pic.transp_col = [0; 3];
    pic.transp_cols = Vec::new();
    if let Some(t) = transparent {
        if let Some(i) = pic.cmap.iter().position(|&c| c == t) {
            pic.num_transp = 1;
            pic.transp_cols = vec![i as u8];
        }
    }
}

/// `read_gif`: the first image of a GIF file, with its colours (C runs
/// `giftopnm | ppmtopcx`). The transparent colour of the graphic control
/// extension becomes `transp_cols[0]`, the index of that colour in the
/// colour map; see [`fill_indexed`].
fn read_gif(ctx: &Ctx, data: &[u8], pic: &mut Pic) -> Result<(), String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        std::num::NonZeroU64::new(MAX_PIXELS * 4).unwrap_or(std::num::NonZeroU64::MIN),
    ));
    let mut decoder = options.read_info(Cursor::new(data)).map_err(|e| e.to_string())?;
    let global: Option<Vec<u8>> = decoder.global_palette().map(<[u8]>::to_vec);
    let frame = decoder
        .read_next_frame()
        .map_err(|e| e.to_string())?
        .ok_or("no image")?;
    let (w, h) = (u32::from(frame.width), u32::from(frame.height));
    check_size(u64::from(w), u64::from(h))?;
    let palette = frame.palette.as_deref().or(global.as_deref()).unwrap_or(&[]);
    let colour = |i: u8| -> [u8; 3] {
        let at = usize::from(i) * 3;
        match palette.get(at..at + 3) {
            Some(&[r, g, b]) => [r, g, b],
            _ => [0; 3],
        }
    };
    let rgb: Vec<[u8; 3]> = frame.buffer.iter().map(|&i| colour(i)).collect();
    if rgb.len() != (w as usize) * (h as usize) {
        return Err("short image data".into());
    }
    let transparent = frame.transparent.map(colour);
    fill_indexed(ctx, pic, w, h, rgb, transparent);
    // `_read_pcx` sets the subtype
    pic.subtype = P_PCX;
    Ok(())
}

/// `skip_comments_whitespace`: white space and `#` comments up to the end of
/// the line.
fn ppm_skip(data: &[u8], pos: &mut usize) {
    while let Some(&c) = data.get(*pos) {
        if c == b'#' {
            while data.get(*pos).is_some_and(|&c| c != b'\n') {
                *pos += 1;
            }
        } else if c.is_ascii_whitespace() || c == 0x0b {
            *pos += 1;
        } else {
            break;
        }
    }
}

/// `fscanf("%u")`: white space, then decimal digits (wrapping like an
/// unsigned `int`).
fn ppm_uint(data: &[u8], pos: &mut usize) -> Option<u32> {
    while data.get(*pos).is_some_and(|c| c.is_ascii_whitespace() || *c == 0x0b) {
        *pos += 1;
    }
    let start = *pos;
    let mut v = 0u32;
    while let Some(&c) = data.get(*pos).filter(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add(u32::from(c - b'0'));
        *pos += 1;
    }
    (*pos > start).then_some(v)
}

/// `read_ppm` (readppm.c): a binary (`P6`) or ASCII (`P3`) PPM of any
/// `maxval` up to 65535, scaled to 0-255 as `_read_ppm` does (a value
/// above `maxval` is clamped in ASCII files); C reads it through
/// `ppmtopcx`, so the picture is then indexed when it has at most 256
/// colours.
fn read_ppm(ctx: &Ctx, data: &[u8], pic: &mut Pic) -> Result<(), String> {
    let bad = || "bad PPM".to_string();
    let magic = match data.get(..2) {
        Some(b"P6") => 6,
        Some(b"P3") => 3,
        _ => return Err(bad()),
    };
    let mut pos = 2usize;
    ppm_skip(data, &mut pos);
    let width = ppm_uint(data, &mut pos).ok_or_else(bad)?;
    ppm_skip(data, &mut pos);
    let height = ppm_uint(data, &mut pos).ok_or_else(bad)?;
    if width == 0 || height == 0 {
        return Err(bad());
    }
    ppm_skip(data, &mut pos);
    let maxval = ppm_uint(data, &mut pos).ok_or_else(bad)?;
    if maxval == 0 || maxval > 65535 {
        return Err(bad());
    }
    if magic == 6 {
        // exactly one white space character ends the header
        pos += 1;
    }
    check_size(u64::from(width), u64::from(height))?;
    let n = (width as usize) * (height as usize) * 3;
    let rnd = maxval / 2;
    let scale = |v: u32| (v.wrapping_mul(255).wrapping_add(rnd)) / maxval;
    let mut samples: Vec<u8> = Vec::with_capacity(n);
    if magic == 6 {
        let wide = maxval >= 256;
        let bytes = data.get(pos..).unwrap_or(&[]);
        if bytes.len() < n * if wide { 2 } else { 1 } {
            return Err(bad());
        }
        if wide {
            for c in bytes.as_chunks::<2>().0.iter().take(n) {
                let v = scale((u32::from(c[0]) << 8) + u32::from(c[1]));
                samples.push(v.min(255) as u8);
            }
        } else {
            // C scales the byte in place: a value above `maxval` wraps
            for &b in bytes.iter().take(n) {
                samples.push(if maxval == 255 { b } else { scale(u32::from(b)) as u8 });
            }
        }
    } else {
        for _ in 0..n {
            let v = ppm_uint(data, &mut pos).ok_or_else(bad)?;
            samples.push(if maxval == 255 { v.min(255) } else { scale(v).min(255) } as u8);
        }
    }
    let pixels: Vec<[u8; 3]> = samples.as_chunks::<3>().0.to_vec();
    fill_indexed(ctx, pic, width, height, pixels, None);
    // `_read_pcx` sets the subtype
    pic.subtype = P_PCX;
    Ok(())
}

/// `read_jpg`: genps does not decode a JPEG, it reads the header (size and
/// colour space) and later passes the data on; `pic` gets the size and no
/// bitmap. The message of a bad header goes to `msgs`.
fn read_jpg(data: &[u8], pic: &mut Pic, msgs: &mut Vec<String>) -> Result<(), String> {
    match readjpg::analyze_jpeg(data) {
        Ok(info) => {
            pic.subtype = P_JPEG;
            set_size(pic, info.width as u32, info.height as u32);
            Ok(())
        }
        Err(m) => {
            msgs.push(m);
            Err(String::new())
        }
    }
}

/// TIFF and (for display) JPEG through the `image` crate.
fn read_raster(
    ctx: &Ctx,
    data: &[u8],
    format: image::ImageFormat,
    t: PicType,
    pic: &mut Pic,
) -> Result<(i32, i32), String> {
    let mut reader = image::ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_PIXELS * 8);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    check_size(u64::from(img.width()), u64::from(img.height()))?;
    if t == PicType::Tiff {
        // `ppmtopcx` / `tifftopnm | ppmtopcx`: the pixels without an alpha
        // channel; `_read_pcx` sets the subtype
        let rgb = img.to_rgb8();
        let pixels: Vec<[u8; 3]> = rgb.as_raw().as_chunks::<3>().0.to_vec();
        fill_indexed(ctx, pic, rgb.width(), rgb.height(), pixels, None);
        pic.subtype = P_PCX;
        return Ok((0, 0));
    }
    let rgba = img.to_rgba8();
    fill_from_rgba(ctx, pic, rgba.width(), rgba.height(), rgba.as_raw())?;
    pic.subtype = t.subtype();
    Ok((0, 0))
}

/// libpng's fixed-point gamma arithmetic (pngrtran.c, pngset.c): values in
/// units of 1/100000.
const PNG_FP_1: i64 = 100_000;
/// The screen gamma `read_png` passes to `png_set_gamma`.
const SCREEN_GAMMA: i64 = 220_000;
/// The file gamma `read_png` assumes without a `gAMA` chunk.
const DEFAULT_FILE_GAMMA: i64 = 45_455;

/// `png_reciprocal`: 1/a, rounded.
fn png_reciprocal(a: i64) -> i64 {
    if a == 0 {
        return 0;
    }
    ((PNG_FP_1 as f64 * PNG_FP_1 as f64) / a as f64 + 0.5).floor() as i64
}

/// `png_reciprocal2`: 1/(a*b), rounded.
fn png_reciprocal2(a: i64, b: i64) -> i64 {
    if a == 0 || b == 0 {
        return 0;
    }
    let p = (PNG_FP_1 as f64).powi(3) / (a as f64 * b as f64);
    (p + 0.5).floor() as i64
}

/// `png_build_8bit_table`: the identity unless the gamma is significant
/// (more than 5 % from 1), else `png_gamma_8bit_correct` for 1..=254.
fn png_table(gamma: i64) -> [u8; 256] {
    let significant = !(95_000..=105_000).contains(&gamma);
    let mut t = [0u8; 256];
    for (i, e) in t.iter_mut().enumerate() {
        *e = if significant && i > 0 && i < 255 {
            (255.0 * (i as f64 / 255.0).powf(gamma as f64 * 0.000_01) + 0.5).floor() as u8
        } else {
            i as u8
        };
    }
    t
}

/// `png_build_16bit_table` flattened: libpng looks a 16-bit value `v` up
/// as `table[(v & 0xff) >> shift][v >> 8]`; with `png_set_scale_16` the
/// shift is at least 5, so only the top 11 bits of `v` count, scaled back
/// to 16 bits before the gamma function is applied.
fn png_table16(gamma: i64) -> Vec<u16> {
    const SHIFT: u32 = 5;
    let significant = !(95_000..=105_000).contains(&gamma);
    let max = (1u64 << (16 - SHIFT)) - 1;
    let max_by_2 = 1u64 << (15 - SHIFT);
    (0..=65535u64)
        .map(|v| {
            let ig = ((v >> SHIFT) * 65535 + max_by_2) / max;
            if significant && ig > 0 && ig < 65535 {
                (65535.0 * (ig as f64 / 65535.0).powf(gamma as f64 * 0.000_01) + 0.5).floor() as u16
            } else {
                ig as u16
            }
        })
        .collect()
}

/// `png_scale_16_to_8`'s arithmetic (`PNG_DIV257`).
fn scale16(v: u16) -> u8 {
    ((u32::from(v) * 255 + 32895) >> 16) as u8
}

/// How `read_png` makes libpng composite an alpha channel.
#[derive(Clone, Copy, PartialEq)]
enum PngBackground {
    /// `bKGD` chunk, `PNG_BACKGROUND_GAMMA_FILE`.
    File,
    /// `-g` colour or white, `PNG_BACKGROUND_GAMMA_SCREEN`.
    Screen,
}

/// libpng's 8-bit gamma and composition tables for `png_set_gamma(2.2,
/// file_gamma)`.
struct PngGamma {
    /// File to screen (`gamma_table`), for pixels without alpha.
    table: [u8; 256],
    /// File to linear (`gamma_to_1`).
    to1: [u8; 256],
    /// Linear to screen (`gamma_from_1`).
    from1: [u8; 256],
    /// Screen to linear, for a background given in screen gamma.
    screen_to1: [u8; 256],
}

impl PngGamma {
    fn new(file_gamma: i64) -> Self {
        Self {
            table: png_table(png_reciprocal2(file_gamma, SCREEN_GAMMA)),
            to1: png_table(png_reciprocal(file_gamma)),
            from1: png_table(png_reciprocal(SCREEN_GAMMA)),
            screen_to1: png_table(SCREEN_GAMMA),
        }
    }

    /// `png_do_compose` for one 8-bit sample with 8-bit alpha `a` over
    /// background `bg`.
    fn compose(&self, c: u8, a: u8, bg: u8, mode: PngBackground) -> u8 {
        let at = |t: &[u8; 256], i: u8| t.get(usize::from(i)).copied().unwrap_or(i);
        match a {
            255 => at(&self.table, c),
            0 => match mode {
                PngBackground::File => at(&self.table, bg),
                PngBackground::Screen => bg,
            },
            _ => {
                let bl = match mode {
                    PngBackground::File => at(&self.to1, bg),
                    PngBackground::Screen => at(&self.screen_to1, bg),
                };
                let t = u32::from(at(&self.to1, c)) * u32::from(a) + u32::from(bl) * (255 - u32::from(a)) + 128;
                let w = ((t + (t >> 8)) >> 8).min(255) as u8;
                at(&self.from1, w)
            }
        }
    }
}

/// libpng's 16-bit tables: images with 16 bits per sample are gamma
/// corrected and composited at 16 bits, then scaled to 8.
struct PngGamma16 {
    table: Vec<u16>,
    to1: Vec<u16>,
    from1: Vec<u16>,
    screen_to1: Vec<u16>,
}

impl PngGamma16 {
    fn new(file_gamma: i64) -> Self {
        Self {
            table: png_table16(png_reciprocal2(file_gamma, SCREEN_GAMMA)),
            to1: png_table16(png_reciprocal(file_gamma)),
            from1: png_table16(png_reciprocal(SCREEN_GAMMA)),
            screen_to1: png_table16(SCREEN_GAMMA),
        }
    }

    /// Gamma correction of a sample without alpha, as 8 bits.
    fn correct(&self, v: u16) -> u8 {
        scale16(self.table.get(usize::from(v)).copied().unwrap_or(v))
    }

    /// `png_do_compose` for one 16-bit sample, as 8 bits.
    fn compose(&self, c: u16, a: u16, bg: u16, mode: PngBackground) -> u8 {
        let at = |t: &[u16], i: u16| t.get(usize::from(i)).copied().unwrap_or(i);
        let w = match a {
            65535 => at(&self.table, c),
            0 => match mode {
                PngBackground::File => at(&self.table, bg),
                PngBackground::Screen => bg,
            },
            _ => {
                let bl = match mode {
                    PngBackground::File => at(&self.to1, bg),
                    PngBackground::Screen => at(&self.screen_to1, bg),
                };
                let t = u64::from(at(&self.to1, c)) * u64::from(a) + u64::from(bl) * (65535 - u64::from(a)) + 32768;
                let w = ((t + (t >> 16)) >> 16).min(65535) as u16;
                at(&self.from1, w)
            }
        };
        scale16(w)
    }
}

/// The text libpng prints (`libpng error: …`) for a decoding failure, where
/// the `png` crate's error identifies the same fault.
fn libpng_error(data: &[u8], e: &png::DecodingError) -> Option<String> {
    let s = e.to_string();
    if matches!(e, png::DecodingError::IoError(_)) || s.contains("end of file") || s.contains("EndOfFile") {
        return Some("libpng error: Read Error".into());
    }
    if s.contains("Missing palette") {
        return Some("libpng error: IDAT: Missing PLTE before IDAT".into());
    }
    if s.contains("before IHDR") {
        // libpng names the first chunk when its type is not a valid one
        let name = data.get(12..16)?;
        let valid = name.iter().all(u8::is_ascii_alphabetic) && name.get(2).is_some_and(u8::is_ascii_uppercase);
        if !valid {
            let n: String = name.iter().map(|&b| char::from(b)).collect();
            return Some(format!("libpng error: {n}: bad header (invalid type)"));
        }
    }
    None
}

/// `read_png` (readpng.c): like libpng with packing, 16 to 8 bit scaling
/// and `png_set_gamma(2.2, file gamma)`. Unlike `read_pic`'s other readers
/// it keeps the file's palette (in file order) and gives grey images a ramp
/// colour map of `1 << bit_depth` entries; RGB images have 3 bytes per
/// pixel and `numcols == RGB_NUMCOLS`. Transparency: a palette with `tRNS`
/// gets `num_transp` entries in `transp_cols` (the alpha values, as C
/// stores them), a grey or RGB image with `tRNS` the colour in
/// `transp_col`; an alpha channel is composited (libpng's gamma
/// arithmetic) over the `bKGD` colour, the `-g` colour or white, which
/// becomes `transp_col`. libpng's warnings go to `msgs`; the `Err` carries
/// libpng's own error message, if any (else an empty string).
pub fn read_png(ctx: &Ctx, data: &[u8], pic: &mut Pic, msgs: &mut Vec<String>) -> Result<(), String> {
    let limits = png::Limits {
        bytes: (MAX_PIXELS as usize).saturating_mul(8),
    };
    let fail = |e: png::DecodingError| libpng_error(data, &e).unwrap_or_default();
    let decoder = png::Decoder::new_with_limits(Cursor::new(data), limits);
    let mut reader = decoder.read_info().map_err(fail)?;
    let (w, h, color, depth) = {
        let i = reader.info();
        (i.width, i.height, i.color_type, i.bit_depth as u8)
    };
    check_size(u64::from(w), u64::from(h)).map_err(|_| String::new())?;
    let info = reader.info();
    let palette: Option<Vec<u8>> = info.palette.as_ref().map(|p| p.to_vec());
    let mut trns: Option<Vec<u8>> = info.trns.as_ref().map(|p| p.to_vec());
    let bkgd: Option<Vec<u8>> = info.bkgd.as_ref().map(|p| p.to_vec());
    let file_gamma = match &info.gama_chunk {
        Some(g) if g.into_scaled() != 0 => i64::from(g.into_scaled()),
        _ => DEFAULT_FILE_GAMMA,
    };
    let interlaced = info.interlaced;
    let size = reader.output_buffer_size().ok_or_else(String::new)?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(fail)?;
    let line = frame.line_size;
    let channels = match color {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    };
    // Sample `k` of row `y`, 16 bit scaled to 8 (libpng's png_set_scale_16).
    let sample = |y: usize, k: usize| -> u8 {
        let row = buf.get(y * line..).unwrap_or(&[]);
        match depth {
            16 => {
                let v = row
                    .get(2 * k..2 * k + 2)
                    .map_or(0, |b| u32::from(b[0]) << 8 | u32::from(b[1]));
                ((v * 255 + 32895) >> 16) as u8
            }
            8 => row.get(k).copied().unwrap_or(0),
            d => {
                let bit = k * usize::from(d);
                let byte = row.get(bit / 8).copied().unwrap_or(0);
                let shift = 8 - usize::from(d) - bit % 8;
                (byte >> shift) & ((1u8 << d) - 1)
            }
        }
    };
    // the 16-bit sample `k` of row `y`
    let raw16 = |y: usize, k: usize| -> u16 {
        let row = buf.get(y * line..).unwrap_or(&[]);
        row.get(2 * k..2 * k + 2)
            .map_or(0, |b| u16::from(b[0]) << 8 | u16::from(b[1]))
    };
    let (w, h) = (w as usize, h as usize);
    let gamma = PngGamma::new(file_gamma);
    let g16 = (depth == 16).then(|| PngGamma16::new(file_gamma));
    let gray_depth = if depth == 16 { 8 } else { u32::from(depth) };
    let has_alpha = matches!(color, png::ColorType::GrayscaleAlpha | png::ColorType::Rgba);
    // libpng ignores a tRNS chunk next to an alpha channel, or one with more
    // entries than the palette
    let mut warn: Vec<&str> = Vec::new();
    if interlaced {
        // png_read_image turns the handling on itself
        warn.push("libpng warning: Interlace handling should be turned on when using png_read_image");
    }
    if has_alpha {
        trns = None;
    }
    let num_palette = palette.as_ref().map_or(0, |p| p.len() / 3);
    if color == png::ColorType::Indexed && trns.as_ref().is_some_and(|t| t.len() > num_palette) {
        warn.push("libpng warning: tRNS: invalid");
        trns = None;
    }
    // the background alpha is composited over
    let bg16: [u16; 3];
    let (bg, mode) = if let Some(b) = bkgd.as_ref().filter(|_| has_alpha) {
        let v = |i: usize| {
            b.get(2 * i..2 * i + 2)
                .map_or(0, |p| u16::from(p[0]) << 8 | u16::from(p[1]))
        };
        let scale = |x: u16| {
            if depth == 16 {
                ((u32::from(x) * 255 + 32895) >> 16) as u8
            } else {
                x as u8
            }
        };
        let c = if color == png::ColorType::Rgba {
            [v(0), v(1), v(2)]
        } else {
            [v(0); 3]
        };
        // C copies the 16-bit values into the unsigned char `transp_col`
        if has_alpha {
            pic.transp_col = c.map(|x| x as u8);
        }
        bg16 = c;
        (c.map(scale), PngBackground::File)
    } else {
        let b = background(ctx);
        if has_alpha {
            pic.transp_col = b;
        }
        bg16 = b.map(|v| u16::from(v) * 257);
        (b, PngBackground::Screen)
    };
    let mut bitmap: Vec<u8> = Vec::with_capacity(w.saturating_mul(h).saturating_mul(3));
    pic.cmap = Vec::new();
    pic.num_transp = NO_TRANSPARENCY;
    pic.transp_cols = Vec::new();
    if !has_alpha {
        pic.transp_col = [0; 3];
    }
    let mut palette_overrun = false;
    let at = |t: &[u8; 256], i: u8| t.get(usize::from(i)).copied().unwrap_or(i);
    match color {
        png::ColorType::Indexed => {
            pic.transp_col = [0; 3];
            let pal = palette.ok_or_else(|| "libpng error: IDAT: Missing PLTE before IDAT".to_string())?;
            pic.cmap = pal.as_chunks::<3>().0.iter().take(256).copied().collect();
            let n = pic.cmap.len();
            if ctx.grayonly {
                let cmap: Vec<u8> = pic.cmap.iter().map(|&c| luminance(c)).collect();
                for y in 0..h {
                    for x in 0..w {
                        let i = usize::from(sample(y, x));
                        palette_overrun |= i >= n;
                        bitmap.push(cmap.get(i).copied().unwrap_or(0));
                    }
                }
                pic.cmap = (0..=255u8).map(|i| [i; 3]).collect();
                pic.numcols = 256;
            } else {
                for y in 0..h {
                    for x in 0..w {
                        let i = sample(y, x);
                        palette_overrun |= usize::from(i) >= n;
                        bitmap.push(i);
                    }
                }
                pic.numcols = n as i32;
            }
            if let Some(t) = &trns {
                // `transp_cols[i] = trans_alpha[i]`: the alpha values, which
                // genps then uses as the MaskColor
                pic.transp_cols = t.clone();
                pic.num_transp = t.len() as i32;
            }
        }
        png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha => {
            let n = 1usize << gray_depth;
            pic.cmap = (0..n).map(|i| [(i * 255 / (n - 1)) as u8; 3]).collect();
            pic.numcols = n as i32;
            let shift = 8 - gray_depth;
            for y in 0..h {
                for x in 0..w {
                    let g = sample(y, x * channels);
                    let v = if let (Some(g16), true) = (&g16, depth == 16) {
                        if has_alpha {
                            g16.compose(raw16(y, x * channels), raw16(y, x * channels + 1), bg16[0], mode)
                        } else {
                            g16.correct(raw16(y, x))
                        }
                    } else if has_alpha {
                        let a = sample(y, x * channels + 1);
                        let g8 = (u32::from(g) * 255 / (n as u32 - 1)) as u8;
                        let m = gamma.compose(g8, a, bg[0], mode);
                        (u32::from(m) * (n as u32 - 1) / 255) as u8
                    } else if depth < 8 {
                        // libpng gammas the value replicated to 8 bits
                        let rep = (u32::from(g) * 255 / (n as u32 - 1)) as u8;
                        at(&gamma.table, rep) >> shift
                    } else {
                        at(&gamma.table, g)
                    };
                    bitmap.push(v);
                }
            }
            if has_alpha {
                pic.num_transp = TRANSP_COLOR;
            } else if let Some(t) = &trns {
                // the `png` crate keeps one byte for depths below 16
                let g = if depth == 16 {
                    t.get(..2).map_or(0, |b| u32::from(b[0]) << 8 | u32::from(b[1])) >> 8
                } else {
                    u32::from(t.first().copied().unwrap_or(0)) * (255 / ((1 << depth) - 1))
                };
                pic.transp_col = [g as u8; 3];
                pic.num_transp = TRANSP_COLOR;
            }
        }
        png::ColorType::Rgb | png::ColorType::Rgba => {
            pic.numcols = RGB_NUMCOLS;
            if ctx.grayonly {
                pic.cmap = (0..=255u8).map(|i| [i; 3]).collect();
                pic.numcols = 256;
            }
            for y in 0..h {
                for x in 0..w {
                    let mut c = [0, 1, 2].map(|k| sample(y, x * channels + k));
                    if let Some(g16) = &g16 {
                        let r = [0, 1, 2].map(|k| raw16(y, x * channels + k));
                        c = if has_alpha {
                            let a = raw16(y, x * channels + 3);
                            [0, 1, 2].map(|k| g16.compose(r[k], a, bg16[k], mode))
                        } else {
                            r.map(|v| g16.correct(v))
                        };
                    } else if has_alpha {
                        let a = sample(y, x * channels + 3);
                        for (k, v) in c.iter_mut().enumerate() {
                            *v = gamma.compose(*v, a, bg[k], mode);
                        }
                    } else {
                        c = c.map(|v| at(&gamma.table, v));
                    }
                    if ctx.grayonly {
                        bitmap.push(luminance(c));
                    } else {
                        bitmap.extend(c);
                    }
                }
            }
            if has_alpha {
                pic.num_transp = TRANSP_COLOR;
            } else if let Some(t) = &trns {
                // C reads the colour from `trans_color` (for 8 bit images
                // the low byte; for 16 bit ones the rounded high byte)
                let v = |i: usize| -> u32 {
                    if depth == 16 {
                        t.get(2 * i..2 * i + 2)
                            .map_or(0, |b| u32::from(b[0]) << 8 | u32::from(b[1]))
                    } else {
                        u32::from(t.get(i).copied().unwrap_or(0))
                    }
                };
                pic.transp_col = [0, 1, 2].map(|i| {
                    if depth == 16 {
                        ((v(i) + 0x80) >> 8) as u8
                    } else {
                        v(i) as u8
                    }
                });
                pic.num_transp = TRANSP_COLOR;
            }
        }
    }
    if palette_overrun {
        warn.push("libpng warning: IDAT: Read palette index exceeding num_palette");
    }
    msgs.extend(warn.into_iter().map(String::from));
    pic.bitmap = bitmap;
    pic.subtype = P_PNG;
    set_size(pic, w as u32, h as u32);
    Ok(())
}

/// The PostScript part of an EPS file, without a DOS binary header.
pub fn eps_section(data: &[u8]) -> &[u8] {
    if data.starts_with(b"\xc5\xd0\xd3\xc6") {
        let word = |i: usize| -> Option<usize> {
            let b = data.get(i..i + 4)?;
            usize::try_from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok()
        };
        if let (Some(start), Some(len)) = (word(4), word(8)) {
            let end = start.saturating_add(len).min(data.len());
            return data.get(start..end).unwrap_or(&[]);
        }
        return &[];
    }
    data
}

/// The preview of an EPS file, if it has one, as RGBA pixels: the TIFF of
/// a DOS EPS header, else EPSI's `%%BeginPreview` data (hex, `depth` bits,
/// larger values darker as fig2dev's `-A` and ghostscript's `ps2epsi`
/// write it). `w` x `h` is the bounding box; a larger preview is cut to
/// it at its lower left corner (fig2dev's previews are one pixel larger
/// each way).
pub fn eps_preview(data: &[u8], w: i32, h: i32) -> Option<RasterImage> {
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let img = if data.starts_with(b"\xc5\xd0\xd3\xc6") {
        let word = |i: usize| -> Option<usize> {
            let b: [u8; 4] = data.get(i..i + 4)?.try_into().ok()?;
            usize::try_from(u32::from_le_bytes(b)).ok()
        };
        let (start, len) = (word(20)?, word(24)?);
        let tif = data.get(start..start.checked_add(len)?.min(data.len()))?;
        let mut reader = image::ImageReader::with_format(Cursor::new(tif), image::ImageFormat::Tiff);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        limits.max_alloc = Some(MAX_PIXELS * 8);
        reader.limits(limits);
        let rgba = reader.decode().ok()?.to_rgba8();
        let (iw, ih) = rgba.dimensions();
        if u64::from(iw) * u64::from(ih) > MAX_PIXELS {
            return None;
        }
        RasterImage {
            width: iw,
            height: ih,
            rgba: rgba.into_raw(),
        }
    } else {
        epsi_preview(eps_section(data))?
    };
    // Cut to the bounding box from the lower left.
    if img.width < w || img.height < h || (img.width == w && img.height == h) {
        return Some(img);
    }
    let (x0, y0) = (0usize, (img.height - h) as usize);
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y0..y0 + h as usize {
        let from = (row * img.width as usize + x0) * 4;
        rgba.extend_from_slice(img.rgba.get(from..from + w as usize * 4)?);
    }
    Some(RasterImage {
        width: w,
        height: h,
        rgba,
    })
}

/// EPSI's `%%BeginPreview: width height depth lines` and its hex lines.
fn epsi_preview(ps: &[u8]) -> Option<RasterImage> {
    let mut lines = ps.split(|&b| b == b'\n');
    let head = lines.find_map(|l| l.strip_prefix(b"%%BeginPreview:"))?;
    let head = String::from_utf8_lossy(head);
    let v: Vec<u32> = head.split_whitespace().take(4).map_while(|t| t.parse().ok()).collect();
    let &[w, h, depth, _] = v.as_slice() else {
        return None;
    };
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || !matches!(depth, 1 | 2 | 4 | 8) {
        return None;
    }
    let mut bytes = Vec::new();
    let mut half: Option<u8> = None;
    for line in lines {
        if line.starts_with(b"%%EndPreview") {
            break;
        }
        let Some(rest) = line.strip_prefix(b"%") else {
            break;
        };
        for &c in rest {
            let Some(d) = (c as char).to_digit(16) else {
                continue;
            };
            match half.take() {
                Some(hi) => bytes.push(hi << 4 | d as u8),
                None => half = Some(d as u8),
            }
        }
        if bytes.len() > (MAX_PIXELS as usize) {
            return None;
        }
    }
    let row_bytes = (w as usize * depth as usize).div_ceil(8);
    let max = (1u32 << depth) - 1;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let row = bytes.get(y * row_bytes..(y + 1) * row_bytes)?;
        for x in 0..w as usize {
            let bit = x * depth as usize;
            let byte = u32::from(*row.get(bit / 8)?);
            let v = (byte >> (8 - depth as usize - bit % 8)) & max;
            // 1 = black at depth 1; larger values are darker.
            let g = (255 - 255 * v / max) as u8;
            rgba.extend_from_slice(&[g, g, g, 255]);
        }
    }
    Some(RasterImage {
        width: w,
        height: h,
        rgba,
    })
}

/// The first `n` numbers in `s` (as `sscanf("%lf %lf …")`).
fn numbers(s: &str, n: usize) -> Option<Vec<f64>> {
    let v: Vec<f64> = s
        .split_whitespace()
        .take(n)
        .map_while(|t| t.parse::<f64>().ok())
        .collect();
    (v.len() == n && v.iter().all(|x| x.is_finite())).then_some(v)
}

/// `read_eps`: the bounding box of an EPS file (or EPSI). Without a
/// `%%BoundingBox`, the picture is 10 × 10 points at the origin.
pub fn read_eps(data: &[u8], pic: &mut Pic) -> Result<(i32, i32), String> {
    let ps = eps_section(data);
    pic.subtype = P_EPS;
    pic.bit_size = Point::new(10, 10);
    let mut ll = (0, 0);
    let mut nested = 0i32;
    for line in ps.split(|&b| b == b'\n') {
        if nested == 0 && line.starts_with(b"%%BoundingBox:") {
            let s = String::from_utf8_lossy(line);
            if s.contains("(atend)") {
                continue;
            }
            let v = s
                .split_once(':')
                .and_then(|(_, r)| numbers(r, 4))
                .ok_or("Bad EPS file")?;
            ll = (v[0].floor() as i32, v[1].floor() as i32);
            pic.bit_size = Point::new((v[2] - v[0]) as i32, (v[3] - v[1]) as i32);
            break;
        } else if line.starts_with(b"%%Begin") {
            nested += 1;
        } else if nested > 0 && line.starts_with(b"%%End") {
            nested -= 1;
        }
    }
    Ok(ll)
}

/// readeps.c `scan_mediabox`: `fgets` reads lines of at most 511 bytes
/// (a longer one comes in pieces, a carriage return is no line end); the
/// first piece holding `/MediaBox` decides: the four numbers after the
/// next `[` in that piece, or failure.
fn scan_mediabox(data: &[u8]) -> Option<[f64; 4]> {
    const KEY: &[u8] = b"/MediaBox";
    let mut rest = data;
    while !rest.is_empty() {
        let mut n = rest.len().min(511);
        if let Some(nl) = rest.get(..n).and_then(|c| c.iter().position(|&b| b == b'\n')) {
            n = nl + 1;
        }
        let (line, tail) = rest.split_at(n);
        rest = tail;
        // C's strings end at a NUL byte.
        let line = line.split(|&b| b == 0).next().unwrap_or(line);
        if let Some(at) = line.windows(KEY.len()).position(|w| w == KEY) {
            let from = line.get(at..)?;
            let open = from.iter().position(|&b| b == b'[')?;
            let s = String::from_utf8_lossy(from.get(open + 1..)?);
            // `%lf %lf %lf %lf` stops at the ']', as the number would.
            let v = numbers(s.split(']').next().unwrap_or(""), 4)?;
            return Some([*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?]);
        }
    }
    None
}

/// `read_pdf`: the first `/MediaBox` of a PDF, found as `scan_mediabox`
/// does. When that fails, C runs ghostscript to ask for the page's box;
/// RustFig runs no external programs and looks further instead: a `[`
/// on a later line is accepted, as gs would find it. Otherwise it fails.
pub fn read_pdf(data: &[u8], pic: &mut Pic) -> Result<(i32, i32), String> {
    const KEY: &[u8] = b"/MediaBox";
    pic.subtype = P_EPS;
    let fail = || "no /MediaBox found in the PDF".to_string();
    let v = match scan_mediabox(data) {
        Some(v) => v,
        None => {
            let at = data.windows(KEY.len()).position(|w| w == KEY).ok_or_else(fail)?;
            let rest = data.get(at + KEY.len()..).ok_or_else(fail)?;
            let rest = rest.get(..rest.len().min(512)).unwrap_or(rest);
            let s = String::from_utf8_lossy(rest);
            let s = s.trim_start().strip_prefix('[').ok_or_else(fail)?;
            let v = numbers(s.split(']').next().unwrap_or(""), 4).ok_or_else(fail)?;
            [v[0], v[1], v[2], v[3]]
        }
    };
    let (llx, lly) = (v[0].floor() as i32, v[1].floor() as i32);
    pic.bit_size = Point::new(
        (v[2].ceil() as i32).saturating_sub(llx),
        (v[3].ceil() as i32).saturating_sub(lly),
    );
    Ok((llx, lly))
}

/// An X bitmap: `height` rows of `(width + 7) / 8` bytes, leftmost pixel
/// in the most significant bit (C reverses the bits of the file's bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XBitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Skips ASCII white space, as `sscanf` does for a blank in its format.
fn skip_blanks(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|b| b.is_ascii_whitespace()).count();
    s.get(n..).unwrap_or(&[])
}

/// `%s`: skips white space, then takes the non-blank characters.
fn scan_word(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let s = skip_blanks(s);
    let n = s.iter().take_while(|b| !b.is_ascii_whitespace()).count();
    (n > 0).then(|| (s.get(..n).unwrap_or(&[]), s.get(n..).unwrap_or(&[])))
}

/// `%d`: white space, an optional sign, decimal digits.
fn scan_decimal(s: &[u8]) -> Option<i64> {
    let s = skip_blanks(s);
    let (neg, s) = match s.split_first() {
        Some((b'-', r)) => (true, r),
        Some((b'+', r)) => (false, r),
        _ => (false, s),
    };
    let digits = s.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let v = s
        .iter()
        .take(digits)
        .fold(0i64, |v, &d| v.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    Some(if neg { v.wrapping_neg() } else { v })
}

/// `NextInt` (readxbm.c): the next hex value of the data, its low byte
/// with the bits reversed. Characters that are neither hex digits nor
/// delimiters (`x`, `;`, CR) are skipped; digits of a decimal number are
/// read as hex. At the end of the file C's `value` is -1, which comes out
/// as 0xff, so the missing data are ones, not an error.
fn next_int(data: &[u8], pos: &mut usize) -> u8 {
    let mut value: i32 = 0;
    let mut gotone = false;
    loop {
        let Some(&ch) = data.get(*pos) else {
            value = -1;
            break;
        };
        *pos += 1;
        if let Some(d) = char::from(ch).to_digit(16) {
            value = value.wrapping_shl(4).wrapping_add(d as i32);
            gotone = true;
        } else if matches!(ch, b' ' | b',' | b'}' | b'\n' | b'\t') && gotone {
            break;
        }
    }
    (value as u8).reverse_bits()
}

/// `ReadFromBitmapFile`: reads an X11 (or X10) bitmap file the way C does
/// (readxbm.c, from X11's `XReadBitmapFromFile`): lines of at most 253
/// characters, `#define NAME_width N` and `NAME_height N`, then the data
/// after a `static [unsigned] char`/`short NAME_bits[] = {` line. X10
/// bitmaps (16-bit values) lose their high bytes in C and here.
pub fn read_bitmap_file(data: &[u8]) -> Option<XBitmap> {
    const MAX_SIZE: usize = 255;
    let after_underscore = |name: &[u8]| -> Vec<u8> {
        match name.iter().rposition(|&b| b == b'_') {
            Some(i) => name.get(i + 1..).unwrap_or(&[]).to_vec(),
            None => name.to_vec(),
        }
    };
    let (mut ww, mut hh) = (0u32, 0u32);
    let mut bitmap: Option<Vec<u8>> = None;
    let mut pos = 0usize;
    while pos < data.len() {
        // fgets(line, MAX_SIZE, file)
        let rest = data.get(pos..).unwrap_or(&[]);
        let n = rest
            .iter()
            .position(|&b| b == b'\n')
            .map_or(rest.len(), |i| i + 1)
            .min(MAX_SIZE - 1);
        let line = rest.get(..n).unwrap_or(&[]);
        pos += n;
        if line.len() == MAX_SIZE - 1 {
            return None;
        }
        if let Some(r) = line.strip_prefix(b"#define") {
            if let Some((name, r)) = scan_word(r) {
                if let Some(value) = scan_decimal(r) {
                    match after_underscore(name).as_slice() {
                        b"width" => ww = value as u32,
                        b"height" => hh = value as u32,
                        _ => {}
                    }
                    continue;
                }
            }
        }
        // `static short %s`, `static unsigned char %s`, `static char %s`
        let version10 = if let Some(r) = line.strip_prefix(b"static").map(skip_blanks) {
            if let Some(r) = r.strip_prefix(b"short") {
                scan_word(r).map(|(w, _)| (w, true))
            } else if let Some(r) = r.strip_prefix(b"unsigned").map(skip_blanks) {
                r.strip_prefix(b"char").and_then(scan_word).map(|(w, _)| (w, false))
            } else {
                r.strip_prefix(b"char").and_then(scan_word).map(|(w, _)| (w, false))
            }
        } else {
            None
        };
        let Some((name, version10)) = version10 else {
            continue;
        };
        if after_underscore(name).as_slice() != b"bits[]" {
            continue;
        }
        if ww == 0 || hh == 0 {
            return None;
        }
        check_size(u64::from(ww), u64::from(hh)).ok()?;
        let padding = usize::from(ww % 16 != 0 && ww % 16 < 9 && version10);
        let bytes_per_line = (ww as usize).div_ceil(8) + padding;
        let size = bytes_per_line * hh as usize;
        let mut out = Vec::with_capacity(size);
        if version10 {
            // two bytes per value; the second is `value >> 8` of a byte,
            // so 0, and the padding byte at the end of a line is dropped
            let mut bytes = 0;
            while bytes < size {
                out.push(next_int(data, &mut pos));
                if padding == 0 || (bytes + 2) % bytes_per_line != 0 {
                    out.push(0);
                }
                bytes += 2;
            }
            out.truncate(size);
        } else {
            for _ in 0..size {
                out.push(next_int(data, &mut pos));
            }
        }
        bitmap = Some(out);
    }
    Some(XBitmap {
        width: ww,
        height: hh,
        data: bitmap?,
    })
}

/// `read_xbm`.
pub fn read_xbm(data: &[u8], pic: &mut Pic) -> Result<(), String> {
    let b = read_bitmap_file(data).ok_or("not a valid X bitmap")?;
    pic.subtype = P_XBM;
    pic.numcols = 0;
    pic.bitmap = b.data;
    set_size(pic, b.width, b.height);
    Ok(())
}

/// The string literals of a C source (XPM), comments skipped.
fn c_strings(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        _ => s.push(c),
                    }
                }
                out.push(s);
                if out.len() >= max {
                    break;
                }
            }
            _ => {}
        }
    }
    out
}

/// The colour of an XPM colour definition (after the pixel characters):
/// the `c` key, else `g`, `g4`, `m`; `None` is black.
fn xpm_color(def: &str) -> Option<[u8; 4]> {
    let mut values: Vec<(&str, String)> = Vec::new();
    for t in def.split_whitespace() {
        if matches!(t, "c" | "m" | "g" | "g4" | "s") {
            values.push((t, String::new()));
        } else if let Some((_, v)) = values.last_mut() {
            if !v.is_empty() {
                v.push(' ');
            }
            v.push_str(t);
        }
    }
    let value = ["c", "g", "g4", "m"].iter().find_map(|k| {
        values
            .iter()
            .find(|(key, v)| key == k && !v.is_empty())
            .map(|(_, v)| v.as_str())
    })?;
    // xpmtoppm has no transparency: None comes out black
    if value.eq_ignore_ascii_case("none") {
        return Some([0, 0, 0, 255]);
    }
    let c = figbase::color::x_color(value)?;
    Some([c.r, c.g, c.b, 255])
}

/// Reads an XPM (version 3) image (C converts it with netpbm or
/// ImageMagick). Unknown colour names are errors, as in `xpmtoppm`.
pub fn read_xpm(ctx: &Ctx, data: &[u8], pic: &mut Pic) -> Result<(), String> {
    let text = String::from_utf8_lossy(data);
    let header = c_strings(&text, 1).into_iter().next().ok_or("not an XPM file")?;
    let v: Vec<u64> = header
        .split_whitespace()
        .take(4)
        .map_while(|t| t.parse().ok())
        .collect();
    let &[w, h, ncolors, cpp] = v.as_slice() else {
        return Err("bad XPM header".into());
    };
    check_size(w, h)?;
    if ncolors == 0 || ncolors > MAX_PIXELS || !(1..=8).contains(&cpp) {
        return Err("bad XPM header".into());
    }
    let (w, h, ncolors, cpp) = (w as usize, h as usize, ncolors as usize, cpp as usize);
    let strings = c_strings(&text, 1 + ncolors + h);
    if strings.len() < 1 + ncolors + h {
        return Err("XPM file too short".into());
    }
    let mut colors: HashMap<&[u8], [u8; 4]> = HashMap::with_capacity(ncolors);
    for def in strings.get(1..1 + ncolors).unwrap_or(&[]) {
        let key = def.as_bytes().get(..cpp).ok_or("bad XPM colour")?;
        let rest = def.get(cpp..).unwrap_or("");
        // xpmtoppm fails on a colour it cannot interpret
        colors.insert(key, xpm_color(rest).ok_or("bad XPM colour")?);
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in strings.get(1 + ncolors..1 + ncolors + h).unwrap_or(&[]) {
        let row = row.as_bytes();
        for x in 0..w {
            let key = row.get(x * cpp..(x + 1) * cpp).ok_or("XPM row too short")?;
            let c = colors.get(key).ok_or("undefined XPM pixel")?;
            rgba.extend_from_slice(c);
        }
    }
    // `xpmtoppm | ppmtopcx`: no transparency, and `_read_pcx` sets the subtype
    let rgb: Vec<[u8; 3]> = rgba.as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2]]).collect();
    fill_indexed(ctx, pic, w as u32, h as u32, rgb, None);
    pic.subtype = P_PCX;
    Ok(())
}

/// The decoder state of `_read_pcx`: C's `dispbyte` arguments.
struct PcxState {
    x: usize,
    y: usize,
    plane: usize,
    pmask: u32,
}

/// `dispbyte` (readpcx.c): stores byte `c` of the run-length decoded data.
/// C writes into one array of `w * h` pixels for all planes, and its
/// indices run past the line (and the array) for odd line lengths and
/// long runs; writes beyond `buf` are dropped here.
fn pcx_dispbyte(buf: &mut [u8], st: &mut PcxState, c: u8, w: usize, h: usize, real_bpp: u32, byteline: usize) {
    match real_bpp {
        1..=4 => {
            if st.y >= h {
                return;
            }
            // the pointer runs on linearly even where x wraps to the next plane
            let start = st.y * w + st.x;
            let line = byteline * 8;
            for f in 0..8 {
                if let Some(b) = buf.get_mut(start + f) {
                    if c & (0x80 >> f) != 0 {
                        *b |= st.pmask as u8;
                    }
                }
                st.x += 1;
                if st.x >= line {
                    if real_bpp == 1 {
                        st.x = 0;
                        st.y += 1;
                        return;
                    }
                    st.x = 0;
                    st.plane += 1;
                    st.pmask <<= 1;
                    if st.plane == real_bpp as usize {
                        st.y += 1;
                        return;
                    }
                }
                if st.y >= h {
                    return;
                }
            }
        }
        8 => {
            if let Some(b) = buf.get_mut(st.y * w + st.x) {
                *b = c;
            }
            st.x += 1;
            if st.x >= w {
                st.x = 0;
                st.y += 1;
            }
        }
        _ => {
            if let Some(b) = buf.get_mut((st.y * w + st.x) * 3 + st.plane) {
                *b = c;
            }
            st.x += 1;
            if st.x >= w {
                st.x = 0;
                st.plane += 1;
                if st.plane == 3 {
                    st.y += 1;
                }
            }
        }
    }
}

/// `_read_pcx` (readpcx.c, public domain, from zgv): reads a PCX image,
/// 1 bit per pixel in 1 to 4 planes, 8 bit indexed or 24 bit RGB; anything
/// else is "unsupported". Ported literally: the scanline length for 8 and
/// 24 bit images is the width (so a padded line is misread), and a short
/// file reads as `0xff` bytes (`fgetc` returns -1).
pub fn read_pcx(ctx: &Ctx, data: &[u8], pic: &mut Pic) -> Result<(), String> {
    let mut hdr = [0u8; 128];
    let n = data.len().min(128);
    if let (Some(d), Some(s)) = (hdr.get_mut(..n), data.get(..n)) {
        d.copy_from_slice(s);
    }
    let at = |i: usize| usize::from(hdr.get(i).copied().unwrap_or(0));
    let word = |i: usize| at(i) + 256 * at(i + 1);
    if at(0) != 10 || at(2) != 1 {
        return Err("not a PCX file".into());
    }
    let (bpp, nplanes) = (at(3), at(65));
    let (real_bpp, bytepp) = match (bpp, nplanes) {
        (1, 1..=4) => (nplanes as u32, 1),
        (8, 1) => (8, 1),
        (8, 3) => (24, 3),
        _ => return Err("unsupported PCX format".into()),
    };
    let w = word(8) as i64 - word(4) as i64 + 1;
    let h = word(10) as i64 - word(6) as i64 + 1;
    if w <= 0 || h <= 0 {
        return Err("empty image".into());
    }
    check_size(w as u64, h as u64)?;
    let (w, h) = (w as usize, h as usize);
    let byteline = word(66);
    // dispbyte() seems to reach beyond a line: C allocates a bit more
    let mut bitmap = vec![0u8; w.max(8) * (h + 2) * bytepp];
    let mut pos = 128usize;
    let mut next = || -> u8 {
        let b = data.get(pos).copied().unwrap_or(0xff);
        pos += 1;
        b
    };
    for yy in 0..h {
        let mut st = PcxState {
            x: 0,
            y: yy,
            plane: 0,
            pmask: 1,
        };
        while st.y == yy {
            let b = next();
            if b < 192 {
                pcx_dispbyte(&mut bitmap, &mut st, b, w, h, real_bpp, byteline);
            } else {
                let c = next();
                for _ in 0..(b & 63) {
                    pcx_dispbyte(&mut bitmap, &mut st, c, w, h, real_bpp, byteline);
                }
            }
        }
    }
    let gray = |c: [u8; 3]| if ctx.grayonly { [luminance(c); 3] } else { c };
    match real_bpp {
        1..=4 => {
            // the palette is in the header
            let n = 1usize << real_bpp;
            pic.cmap = (0..n)
                .map(|i| gray([at(16 + i * 3) as u8, at(17 + i * 3) as u8, at(18 + i * 3) as u8]))
                .collect();
            pic.numcols = n as i32;
        }
        8 => {
            // after a splitter byte, 256 colours follow the image data
            let _splitter = next();
            pic.cmap = (0..256).map(|_| gray([next(), next(), next()])).collect();
            pic.numcols = 256;
        }
        _ => {
            pic.cmap = Vec::new();
            pic.numcols = RGB_NUMCOLS;
        }
    }
    bitmap.truncate(w * h * bytepp);
    pic.bitmap = bitmap;
    pic.num_transp = NO_TRANSPARENCY;
    pic.subtype = P_PCX;
    set_size(pic, w as u32, h as u32);
    Ok(())
}

/// A picture as fig2dev's `readpics.c` reads it, for RustFig.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedPicture {
    /// A bitmap, as RGBA pixels.
    Raster(RasterImage),
    /// An EPS file: its bounding box (points) and PostScript, which the
    /// EPS export embeds as genps.c does.
    Eps {
        llx: i32,
        lly: i32,
        urx: i32,
        ury: i32,
        ps: Vec<u8>,
        /// The preview the file carries (a TIFF in a DOS EPS header, or
        /// EPSI's `%%BeginPreview`), cut to the bounding box. The bitmap
        /// languages draw it where fig2dev has ghostscript run the
        /// PostScript.
        preview: Option<RasterImage>,
    },
    /// A JPEG file: its header as genps reads it and the compressed data,
    /// which the PostScript output passes on as it is; `raster` is the
    /// decoded picture for the other outputs (`None` when it can't be
    /// decoded: genps never decodes it).
    Jpeg {
        raster: Option<RasterImage>,
        info: readjpg::JpegInfo,
        data: Vec<u8>,
    },
    /// A PDF file: fig2dev converts it with external programs (`pdftops`);
    /// here its first page is read with the `hayro` crates. `None` when
    /// the file can't be read that way (the picture is then left out).
    Pdf(Option<super::pdfpic::PdfPicture>),
}

/// Reads picture `name` the way fig2dev does (`readpics.c`: the file, or
/// it with `.gz`, `.Z`, `.z` added or removed, uncompressed; GIF, JPEG,
/// PNG, TIFF, PPM, PCX, XBM, XPM, EPS and PDF). Relative names are
/// taken relative to `ctx.dir`. Errors carry genps.c's messages ("No such
/// picture file: …", "…: Unknown image format").
pub fn load_picture(ctx: &mut Ctx, name: &str) -> Result<LoadedPicture, String> {
    load_picture_as(ctx, name, false)
}

/// [`load_picture`] with the pixels as genps draws them (`genps`: see
/// [`pic_to_raster_as`]).
pub fn load_picture_genps(ctx: &mut Ctx, name: &str) -> Result<LoadedPicture, String> {
    load_picture_as(ctx, name, true)
}

fn load_picture_as(ctx: &mut Ctx, name: &str, genps: bool) -> Result<LoadedPicture, String> {
    if file_on_disk(&pic_path(ctx, name)).is_none() {
        return Err(format!("No such picture file: {name}"));
    }
    let stream = open_stream(ctx, name)?;
    let Some(kind) = pic_type(&stream.data) else {
        return Err(format!("{name}: Unknown image format"));
    };
    let mut pic = Pic {
        file: name.to_string(),
        ..Pic::default()
    };
    if kind == PicType::Pdf {
        // The page box comes from the PDF reader; C scans for /MediaBox or
        // asks ghostscript, which fail on what the reader can still open.
        if let Some(p) = super::pdfpic::PdfPicture::open(stream.data.clone()) {
            return Ok(LoadedPicture::Pdf(Some(p)));
        }
    }
    let (llx, lly) = read_pic(ctx, &stream, &mut pic)?;
    match kind {
        PicType::Pdf => Ok(LoadedPicture::Pdf(None)),
        PicType::Eps | PicType::Epsi => Ok(LoadedPicture::Eps {
            llx,
            lly,
            urx: llx.saturating_add(pic.bit_size.x),
            ury: lly.saturating_add(pic.bit_size.y),
            ps: eps_section(&stream.data).to_vec(),
            preview: eps_preview(&stream.data, pic.bit_size.x, pic.bit_size.y),
        }),
        PicType::Jpeg => {
            let info = readjpg::analyze_jpeg(&stream.data)?;
            let mut decoded = Pic::default();
            let raster = read_raster(ctx, &stream.data, image::ImageFormat::Jpeg, kind, &mut decoded)
                .ok()
                .and_then(|_| pic_to_raster(&decoded).ok());
            Ok(LoadedPicture::Jpeg {
                raster,
                info,
                data: stream.data,
            })
        }
        _ => pic_to_raster_as(&pic, genps).map(LoadedPicture::Raster),
    }
}

/// The pixels of a picture `readpics` read, as RGBA: XBM bits in black
/// on transparent, the transparent colour of the others transparent.
pub fn pic_to_raster(pic: &Pic) -> Result<RasterImage, String> {
    pic_to_raster_as(pic, false)
}

/// [`pic_to_raster`]; with `genps` an indexed picture (at most 256 colours)
/// whose transparency is one colour (`TRANSP_COLOR`: a grey PNG with a
/// `tRNS` colour or an alpha channel, composited already) stays opaque,
/// as genps's `indexed_image` writes a colour map and no `/MaskColor`.
pub fn pic_to_raster_as(pic: &Pic, genps: bool) -> Result<RasterImage, String> {
    let (w, h) = (pic.bit_size.x, pic.bit_size.y);
    let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h)) else {
        return Err(format!("{}: bad picture size", pic.file));
    };
    let (wu, hu) = (w as usize, h as usize);
    let n = wu.checked_mul(hu).filter(|&n| n > 0 && n as u64 <= MAX_PIXELS);
    let Some(n) = n else {
        return Err(format!("{}: bad picture size", pic.file));
    };
    let transparent = (pic.num_transp == TRANSP_COLOR).then_some(pic.transp_col);
    let mut rgba = Vec::with_capacity(n * 4);
    let push = |rgba: &mut Vec<u8>, c: [u8; 3]| {
        let a = if Some(c) == transparent { 0 } else { 255 };
        rgba.extend_from_slice(&[c[0], c[1], c[2], a]);
    };
    if pic.subtype == P_XBM {
        let row = wu.div_ceil(8);
        for y in 0..hu {
            for x in 0..wu {
                let byte = pic.bitmap.get(y * row + x / 8).copied().unwrap_or(0);
                if byte & (0x80 >> (x % 8)) != 0 {
                    rgba.extend_from_slice(&[0, 0, 0, 255]);
                } else {
                    rgba.extend_from_slice(&[255, 255, 255, 0]);
                }
            }
        }
    } else if pic.numcols > 256 {
        for px in pic.bitmap.as_chunks::<3>().0.iter().take(n) {
            push(&mut rgba, *px);
        }
    } else {
        // genps writes the entries of `transp_cols` as the image's
        // /MaskColor: one index is masked, two are a range, other counts
        // make the PostScript fail and mask nothing here
        let indexed_mask = |i: u8| match *pic.transp_cols.as_slice() {
            [v] if pic.num_transp == 1 => i == v,
            [lo, hi] if pic.num_transp == 2 => (lo..=hi).contains(&i),
            _ => false,
        };
        for &i in pic.bitmap.iter().take(n) {
            let c = pic.cmap.get(usize::from(i)).copied().unwrap_or([0, 0, 0]);
            if indexed_mask(i) {
                rgba.extend_from_slice(&[c[0], c[1], c[2], 0]);
            } else if genps {
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            } else {
                push(&mut rgba, c);
            }
        }
    }
    if rgba.len() != n * 4 {
        return Err(format!("{}: short picture data", pic.file));
    }
    Ok(RasterImage {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        assert_eq!(pic_type(b"GIF89a"), Some(PicType::Gif));
        assert_eq!(pic_type(b"%!PS-Adobe-3.0 EPSF"), Some(PicType::Eps));
        assert_eq!(pic_type(b"%PDF-1.4"), Some(PicType::Pdf));
        assert_eq!(pic_type(b"P6\n1 1\n255\n\0\0\0"), Some(PicType::Ppm));
        assert_eq!(pic_type(b"P3 1 1 255 0 0 0"), Some(PicType::Ppm));
        // genps.c's table: no P1, P2, P4, P5, P7, and no JPEG without JFIF/Exif
        for no in [
            &b"P1\n1 1\n0\n"[..],
            b"P5\n1 1\n255\n\0",
            b"P4\n8 1\n\xff",
            b"\xff\xd8\xff\xdb",
        ] {
            assert_eq!(pic_type(no), None);
        }
        assert_eq!(pic_type(b"\xff\xd8\xff\xe1"), Some(PicType::Jpeg));
        assert_eq!(pic_type(b"hello"), None);
        assert_eq!(pic_type(b""), None);
    }

    #[test]
    fn eps_bounding_box() {
        let mut pic = Pic::default();
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: (atend)\n%%BeginDocument: x\n%%BoundingBox: 1 1 2 2\n%%EndDocument\n%%BoundingBox: 10.5 20 110 220\n";
        assert_eq!(read_eps(eps, &mut pic), Ok((10, 20)));
        assert_eq!(pic.bit_size, Point::new(99, 200));
        assert!(read_eps(b"%!\n%%BoundingBox: 1 2 x\n", &mut pic).is_err());
        assert_eq!(read_eps(b"%!\n", &mut pic), Ok((0, 0)));
        assert_eq!(pic.bit_size, Point::new(10, 10));
    }

    #[test]
    fn pdf_media_box() {
        let mut pic = Pic::default();
        assert_eq!(
            read_pdf(b"%PDF-1.4\n1 0 obj << /MediaBox [0 0 595.3 841.9] >>", &mut pic),
            Ok((0, 0))
        );
        assert_eq!(pic.bit_size, Point::new(596, 842));
        assert!(read_pdf(b"%PDF-1.4\n", &mut pic).is_err());
        assert!(read_pdf(b"%PDF /MediaBox [1 2", &mut pic).is_err());
        // C: the first piece with /MediaBox, a '[' later on its line.
        assert_eq!(
            read_pdf(
                b"%PDF-1.4\r1 0 obj << /Type /Page /MediaBox 7 [ -1.5 -2.5 10 20.2 ]>>\n",
                &mut pic
            ),
            Ok((-2, -3))
        );
        assert_eq!(pic.bit_size, Point::new(12, 24));
        assert_eq!(scan_mediabox(b"%PDF\n/MediaBox\n[0 0 1 1]\n"), None, "C asks gs here");
        assert_eq!(
            read_pdf(b"%PDF\n/MediaBox\n[0 0 3 4]\n", &mut pic),
            Ok((0, 0)),
            "RustFig has no gs and reads on"
        );
        // A second /MediaBox is not looked for after a failed first.
        assert_eq!(scan_mediabox(b"/MediaBox [1 2\n/MediaBox [0 0 5 5]\n"), None);
        // Lines are cut at 511 bytes.
        let mut long = vec![b' '; 520];
        long.splice(505..505, b"/MediaBox [0 0 5 5]".iter().copied());
        assert_eq!(scan_mediabox(&long), None);
    }

    #[test]
    fn xbm() {
        let src =
            b"#define t_width 10\n#define t_height 2\nstatic unsigned char t_bits[] = {\n 0x01, 0x02, 0xff, 0x03 };\n";
        let b = read_bitmap_file(src).unwrap();
        assert_eq!((b.width, b.height), (10, 2));
        assert_eq!(b.data, vec![0x80, 0x40, 0xff, 0xc0]);
        assert!(read_bitmap_file(b"#define t_width 10\nstatic char t_bits[] = { 0x01 };").is_none());
        assert!(
            read_bitmap_file(b"#define a_width 99999999\n#define a_height 99999999\nstatic char a_bits[] = {")
                .is_none()
        );
    }

    #[test]
    fn xpm() {
        let src = b"/* XPM */\nstatic char *x[] = {\n/* w h n c */\n\"2 2 2 1\",\n\"a c #ff0000\",\n\"b c None\",\n\"ab\",\n\"ba\"};\n";
        let mut pic = Pic::default();
        read_xpm(&Ctx::default(), src, &mut pic).unwrap();
        assert_eq!(pic.bit_size, Point::new(2, 2));
        assert_eq!(pic.numcols, 2);
        // xpmtoppm | ppmtopcx: `None` is black, there is no transparency, and
        // `_read_pcx` sets the subtype
        assert_eq!(pic.cmap, vec![[255, 0, 0], [0, 0, 0]]);
        assert_eq!(pic.bitmap, vec![0, 1, 1, 0]);
        assert_eq!(pic.num_transp, NO_TRANSPARENCY);
        assert_eq!(pic.subtype, P_PCX);
        assert!(read_xpm(&Ctx::default(), b"/* XPM */ \"2 2 1 1\" \"a c red\" \"aa\"", &mut pic).is_err());
        // a colour xpmtoppm cannot interpret
        let bad = b"/* XPM */ \"1 1 1 1\", \"a c nosuchcolour\", \"a\"";
        assert!(read_xpm(&Ctx::default(), bad, &mut pic).is_err());
        assert!(read_xpm(&Ctx::default(), b"/* XPM */ \"9999999 9999999 1 1\"", &mut pic).is_err());
    }

    #[test]
    fn rgb_beyond_256_colours() {
        let w = 20u32;
        let h = 20u32;
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 7, 255])
            .collect();
        let mut pic = Pic::default();
        fill_from_rgba(&Ctx::default(), &mut pic, w, h, &rgba).unwrap();
        assert_eq!(pic.numcols, RGB_NUMCOLS);
        assert_eq!(pic.bitmap.len(), (w * h * 3) as usize);
        let mut ctx = Ctx::default();
        ctx.grayonly = true;
        fill_from_rgba(&ctx, &mut pic, w, h, &rgba).unwrap();
        assert!(pic.numcols <= 256);
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &d in data {
            a = (a + u32::from(d)) % 65521;
            b = (b + a) % 65521;
        }
        b << 16 | a
    }

    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        let mut crc_data = kind.to_vec();
        crc_data.extend_from_slice(body);
        out.extend_from_slice(&crc_data);
        out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
        out
    }

    /// A PNG of `w` x `h` with `rows` (without filter bytes) and the given
    /// extra chunks before the data.
    fn png(w: u32, h: u32, color: u8, depth: u8, interlace: u8, rows: &[&[u8]], extra: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[depth, color, 0, 0, interlace]);
        out.extend(chunk(b"IHDR", &ihdr));
        for c in extra {
            out.extend_from_slice(c);
        }
        let raw: Vec<u8> = rows
            .iter()
            .flat_map(|r| std::iter::once(0u8).chain(r.iter().copied()))
            .collect();
        // stored deflate block
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(&raw);
        z.extend_from_slice(&adler32(&raw).to_be_bytes());
        out.extend(chunk(b"IDAT", &z));
        out.extend(chunk(b"IEND", &[]));
        out
    }

    fn read_bytes(ctx: &mut Ctx, data: Vec<u8>) -> (Result<(i32, i32), String>, Pic) {
        let stream = PicStream {
            name: "t".into(),
            path: PathBuf::new(),
            compression: Compression::None,
            data,
        };
        let mut pic = Pic {
            file: "t".into(),
            ..Pic::default()
        };
        (read_pic(ctx, &stream, &mut pic), pic)
    }

    fn rgb_of(pic: &Pic, x: usize) -> [u8; 3] {
        let b = |i: usize| pic.bitmap.get(i).copied().unwrap_or(0);
        [b(3 * x), b(3 * x + 1), b(3 * x + 2)]
    }

    #[test]
    fn png_alpha_composites_as_libpng_does() {
        // two RGBA pixels: (30,40,20) at alpha 40 and (0,40,10) at alpha 0
        let rows: [&[u8]; 1] = [&[30, 40, 20, 40, 0, 40, 10, 0]];
        let mut ctx = Ctx::default();
        // white background, the gamma of a file without gAMA (the values are
        // what fig2dev 3.2.9a with libpng 1.6 writes)
        let (r, pic) = read_bytes(&mut ctx, png(2, 1, 6, 8, 0, &rows, &[]));
        assert!(r.is_ok());
        assert_eq!(
            (pic.numcols, pic.num_transp, pic.transp_col),
            (RGB_NUMCOLS, TRANSP_COLOR, [255; 3])
        );
        assert_eq!(rgb_of(&pic, 0), [236, 236, 236]);
        assert_eq!(rgb_of(&pic, 1), [255, 255, 255]);
        // gAMA 1.0 changes the arithmetic
        let gama = chunk(b"gAMA", &100_000u32.to_be_bytes());
        let (_, pic) = read_bytes(&mut ctx, png(2, 1, 6, 8, 0, &rows, &[gama]));
        assert_eq!(rgb_of(&pic, 0), [238, 239, 237]);
        // bKGD (16-bit values, here 200, 50, 60) replaces the background and
        // becomes the transparent colour
        let bkgd = chunk(b"bKGD", &[0, 200, 0, 50, 0, 60]);
        let (_, pic) = read_bytes(&mut ctx, png(2, 1, 6, 8, 0, &rows, &[bkgd]));
        assert_eq!(pic.transp_col, [200, 50, 60]);
        assert_eq!(rgb_of(&pic, 0), [185, 50, 56]);
        assert_eq!(rgb_of(&pic, 1), [200, 50, 60]);
        // -g colour
        ctx.bgspec = true;
        ctx.background.red = 0x1200;
        ctx.background.green = 0x3400;
        ctx.background.blue = 0x5600;
        let (_, pic) = read_bytes(&mut ctx, png(2, 1, 6, 8, 0, &rows, &[]));
        assert_eq!(pic.transp_col, [0x12, 0x34, 0x56]);
        assert_eq!(rgb_of(&pic, 1), [0x12, 0x34, 0x56]);
    }

    #[test]
    fn png_gamma_corrects_pixels_without_alpha() {
        let rows: [&[u8]; 1] = [&[0, 0, 0, 30, 30, 30]];
        let gama = chunk(b"gAMA", &100_000u32.to_be_bytes());
        let (_, pic) = read_bytes(
            &mut Ctx::default(),
            png(2, 1, 2, 8, 0, &rows, std::slice::from_ref(&gama)),
        );
        assert_eq!(rgb_of(&pic, 1), [96; 3]);
        // gAMA 0.45455 is the default: untouched
        let same = chunk(b"gAMA", &45_455u32.to_be_bytes());
        let (_, pic) = read_bytes(&mut Ctx::default(), png(2, 1, 2, 8, 0, &rows, &[same]));
        assert_eq!(rgb_of(&pic, 1), [30; 3]);
        // palette entries are not corrected, as the colour map is read before
        // libpng sets up its transformations
        let plte = chunk(b"PLTE", &[30, 30, 30]);
        let (_, pic) = read_bytes(&mut Ctx::default(), png(1, 1, 3, 8, 0, &[&[0]], &[gama, plte]));
        assert_eq!(pic.cmap, vec![[30, 30, 30]]);
    }

    #[test]
    fn png_palette_transparency_is_kept_as_transp_cols() {
        let plte = chunk(b"PLTE", &[255, 0, 0, 0, 255, 0, 0, 0, 255]);
        let rows: [&[u8]; 1] = [&[2, 1, 0]];
        let mut ctx = Ctx::default();
        let (r, pic) = read_bytes(
            &mut ctx,
            png(3, 1, 3, 8, 0, &rows, &[plte.clone(), chunk(b"tRNS", &[0, 255])]),
        );
        assert!(r.is_ok());
        // the colour map is in file order; C stores the alpha values
        assert_eq!(pic.cmap, vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        assert_eq!(pic.bitmap, vec![2, 1, 0]);
        assert_eq!((pic.num_transp, pic.transp_cols.clone()), (2, vec![0, 255]));
        // more tRNS entries than palette entries: libpng warns and ignores
        let (_, pic) = read_bytes(
            &mut ctx,
            png(3, 1, 3, 8, 0, &rows, &[plte, chunk(b"tRNS", &[0, 0, 0, 0])]),
        );
        assert_eq!(pic.num_transp, NO_TRANSPARENCY);
        assert_eq!(ctx.messages, vec!["libpng warning: tRNS: invalid".to_string()]);
        // one entry masks that index in the raster
        let plte = chunk(b"PLTE", &[255, 0, 0, 0, 255, 0]);
        let (_, pic) = read_bytes(
            &mut Ctx::default(),
            png(2, 1, 3, 8, 0, &[&[0, 1]], &[plte, chunk(b"tRNS", &[0])]),
        );
        let img = pic_to_raster(&pic).unwrap();
        assert_eq!(img.rgba.chunks(4).map(|p| p[3]).collect::<Vec<_>>(), vec![0, 255]);
    }

    #[test]
    fn png_messages_are_libpngs() {
        let mut ctx = Ctx::default();
        // palette index beyond the palette, interlace handling
        let plte = chunk(b"PLTE", &[0, 0, 0]);
        let (r, _) = read_bytes(&mut ctx, png(1, 1, 3, 8, 1, &[&[9]], &[plte]));
        assert!(r.is_ok());
        assert_eq!(
            ctx.messages,
            vec![
                "libpng warning: Interlace handling should be turned on when using png_read_image".to_string(),
                "libpng warning: IDAT: Read palette index exceeding num_palette".to_string()
            ]
        );
        // a truncated file
        let mut ctx = Ctx::default();
        let mut data = png(2, 2, 2, 8, 0, &[&[0; 6], &[0; 6]], &[]);
        data.truncate(data.len() - 20);
        let (r, _) = read_bytes(&mut ctx, data);
        assert_eq!(r.unwrap_err(), "libpng error: Read Error\nt: Bad PNG format");
    }

    #[test]
    fn png_grayscale_transparent_colour() {
        let (_, pic) = read_bytes(
            &mut Ctx::default(),
            png(2, 1, 0, 8, 0, &[&[1, 2]], &[chunk(b"tRNS", &[0, 1])]),
        );
        assert_eq!((pic.num_transp, pic.transp_col), (TRANSP_COLOR, [1; 3]));
        assert_eq!(pic.numcols, 256);
        // a 2-bit grey scale: colour map of four entries, ramp 0 85 170 255
        let (_, pic) = read_bytes(&mut Ctx::default(), png(2, 1, 0, 2, 0, &[&[0b0111_0000]], &[]));
        assert_eq!(pic.cmap, vec![[0; 3], [85; 3], [170; 3], [255; 3]]);
        assert_eq!(pic.bitmap, vec![1, 3]);
    }

    /// A GIF with a 4-colour global map, 1 byte per code (LZW code size 8,
    /// each pixel a literal).
    fn gif(w: u16, h: u16, transparent: Option<u8>, pixels: &[u8]) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&h.to_le_bytes());
        g.extend_from_slice(&[0x81, 0, 0]);
        g.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        if let Some(t) = transparent {
            g.extend_from_slice(&[0x21, 0xf9, 4, 1, 0, 0, t, 0]);
        }
        g.push(0x2c);
        g.extend_from_slice(&[0, 0, 0, 0]);
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&h.to_le_bytes());
        g.push(0);
        g.push(8);
        // codes of 9 bits: clear (256), the pixels, end (257), LSB first
        let mut codes = vec![256u32];
        codes.extend(pixels.iter().map(|&p| u32::from(p)));
        codes.push(257);
        let (mut bits, mut n, mut data) = (0u32, 0u32, Vec::new());
        for c in codes {
            bits |= c << n;
            n += 9;
            while n >= 8 {
                data.push(bits as u8);
                bits >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            data.push(bits as u8);
        }
        for block in data.chunks(255) {
            g.push(block.len() as u8);
            g.extend_from_slice(block);
        }
        g.extend_from_slice(&[0, 0x3b]);
        g
    }

    #[test]
    fn gif_transparent_index_is_found_again_in_the_colour_map() {
        let mut ctx = Ctx::default();
        let (r, pic) = read_bytes(&mut ctx, gif(2, 2, Some(1), &[0, 1, 2, 1]));
        assert!(r.is_ok());
        assert_eq!(pic.subtype, P_PCX);
        assert_eq!((pic.num_transp, pic.numcols), (1, 3));
        // colours in order of first use: red, green, blue; green is transparent
        assert_eq!(pic.cmap, vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        assert_eq!(pic.transp_cols, vec![1]);
        let img = pic_to_raster(&pic).unwrap();
        assert_eq!(
            img.rgba.chunks(4).map(|p| p[3]).collect::<Vec<_>>(),
            vec![255, 0, 255, 0]
        );
        // a transparent colour no pixel uses: no transparency
        let (_, pic) = read_bytes(&mut ctx, gif(2, 1, Some(3), &[0, 1]));
        assert_eq!(pic.num_transp, NO_TRANSPARENCY);
        // no graphic control extension
        let (_, pic) = read_bytes(&mut ctx, gif(2, 1, None, &[0, 1]));
        assert_eq!((pic.num_transp, pic.transp_cols.len()), (NO_TRANSPARENCY, 0));
        // truncated
        let mut g = gif(2, 2, None, &[0, 1, 2, 1]);
        g.truncate(30);
        assert_eq!(read_bytes(&mut ctx, g).0.unwrap_err(), "t: Bad GIF format");
    }

    #[test]
    fn xbm_reads_like_readxbm_c() {
        // the end of the data reads as ones (C's NextInt returns 0xff for EOF)
        let b =
            read_bitmap_file(b"#define a_width 16\n#define a_height 2\nstatic char a_bits[] = {\n 0x01 };\n").unwrap();
        assert_eq!(b.data, vec![0x80, 0xff, 0xff, 0xff]);
        // X10: only the low byte of each value is kept; widths of 1 to 8 pad
        let b = read_bitmap_file(
            b"#define a_width 12\n#define a_height 1\nstatic short a_bits[] = {\n 0x0f0f, 0x00ff };\n",
        )
        .unwrap();
        assert_eq!(b.data, vec![0xf0, 0x00]);
        // decimal numbers are read as hex, other characters skipped
        let b = read_bitmap_file(b"#define a_width 8\n#define a_height 2\nstatic char a_bits[] = {\r\n 129, 0176};\n")
            .unwrap();
        assert_eq!(b.data, vec![0x94, 0x6e]);
        // the name must end in `bits[]`, and the size comes first
        assert!(read_bitmap_file(b"#define a_width 8\n#define a_height 1\nstatic char a_data[] = { 1 };\n").is_none());
        assert!(read_bitmap_file(b"static char a_bits[] = { 1 };\n#define a_width 8\n#define a_height 1\n").is_none());
        // a line of 254 characters or more is the end of the file
        let mut long = b"#define a_width 8\n#define a_height 1\n/*".to_vec();
        long.extend(std::iter::repeat_n(b'x', 260));
        long.extend_from_slice(b"*/\nstatic char a_bits[] = { 1 };\n");
        assert!(read_bitmap_file(&long).is_none());
    }

    #[test]
    fn ppm_reads_like_readppm_c() {
        let mut ctx = Ctx::default();
        // binary, maxval 15 scaled to 255; a comment in the header
        let mut p6 = b"P6\n# c\n2 1\n15\n".to_vec();
        p6.extend_from_slice(&[0, 15, 5, 15, 0, 1]);
        let (r, pic) = read_bytes(&mut ctx, p6);
        assert!(r.is_ok());
        assert_eq!(pic.subtype, P_PCX);
        assert_eq!(pic.cmap, vec![[0, 255, 85], [255, 0, 17]]);
        // 16 bit samples
        let mut p6 = b"P6 1 1 65535 ".to_vec();
        p6.extend_from_slice(&[0x80, 0x00, 0xff, 0xff, 0x00, 0x00]);
        let (_, pic) = read_bytes(&mut ctx, p6);
        assert_eq!(pic.cmap, vec![[128, 255, 0]]);
        // ASCII: values above maxval are clamped (the C fallback reader)
        let (_, pic) = read_bytes(&mut ctx, b"P3 1 1 7 0 7 9".to_vec());
        assert_eq!(pic.cmap, vec![[0, 255, 255]]);
        // failures are "Bad PPM format"
        for bad in [
            &b"P6 1 1 255 \0\0"[..],
            b"P6 0 1 255 ",
            b"P6 1 1 0 \0\0\0",
            b"P6 1 1 70000 \0\0\0\0\0\0",
            b"P3 2 1 255 1 2 3 4",
        ] {
            assert_eq!(read_bytes(&mut ctx, bad.to_vec()).0.unwrap_err(), "t: Bad PPM format");
        }
        // P5 is not a picture type at all
        assert_eq!(
            read_bytes(&mut ctx, b"P5 1 1 255 \0".to_vec()).0.unwrap_err(),
            "t: Unknown image format"
        );
    }

    /// A PCX file: header, then `body`.
    fn pcx(bpp: u8, planes: u8, w: u16, h: u16, bytes_per_line: u16, pal16: &[u8], body: &[u8]) -> Vec<u8> {
        let mut f = vec![10, 5, 1, bpp, 0, 0, 0, 0];
        f.extend_from_slice(&(w - 1).to_le_bytes());
        f.extend_from_slice(&(h - 1).to_le_bytes());
        f.extend_from_slice(&[0; 4]);
        let mut p = pal16.to_vec();
        p.resize(48, 0);
        f.extend(p);
        f.extend_from_slice(&[0, planes]);
        f.extend_from_slice(&bytes_per_line.to_le_bytes());
        f.resize(128, 0);
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn pcx_subtypes_as_read_pcx_c() {
        let mut ctx = Ctx::default();
        // 1 bit, 4 planes: the planes add up to the pixel value (bytes from
        // 0xc0 on need a run count)
        let (r, pic) = read_bytes(
            &mut ctx,
            pcx(
                1,
                4,
                8,
                1,
                2,
                &[1, 1, 1, 2, 2, 2],
                &[0xc1, 0xaa, 0x00, 0xc1, 0xcc, 0x00, 0xc1, 0xf0, 0x00, 0x0f, 0x00],
            ),
        );
        assert!(r.is_ok());
        assert_eq!((pic.numcols, pic.subtype), (16, P_PCX));
        assert_eq!(pic.bitmap, vec![7, 6, 5, 4, 11, 10, 9, 8]);
        assert_eq!(pic.cmap[..2], [[1, 1, 1], [2, 2, 2]]);
        // 8 bit: the palette follows the data after a splitter byte
        let mut body = vec![0, 1, 2, 3, 0x0c];
        body.extend((0..256).flat_map(|i| [i as u8, 0, 255 - i as u8]));
        let (_, pic) = read_bytes(&mut ctx, pcx(8, 1, 4, 1, 4, &[], &body));
        assert_eq!(pic.numcols, 256);
        assert_eq!(pic.cmap.get(3), Some(&[3, 0, 252]));
        // a run of 0xc2 0x07 is two 7s; without a palette C reads 0xff
        let (_, pic) = read_bytes(&mut ctx, pcx(8, 1, 2, 1, 2, &[], &[0xc2, 7]));
        assert_eq!(pic.bitmap, vec![7, 7]);
        assert_eq!(pic.cmap[0], [255; 3]);
        // 24 bit: one plane after the other in a row
        let (_, pic) = read_bytes(&mut ctx, pcx(8, 3, 2, 1, 2, &[], &[1, 2, 3, 4, 5, 6]));
        assert_eq!(pic.numcols, RGB_NUMCOLS);
        assert_eq!(pic.bitmap, vec![1, 3, 5, 2, 4, 6]);
        // 2 and 4 bits per pixel in one plane, and 3 planes of 8 bits per
        // pixel with 4 planes, are unsupported
        for (bpp, planes) in [(2, 1), (4, 1), (8, 4), (8, 2), (1, 5), (1, 0)] {
            let r = read_bytes(&mut ctx, pcx(bpp, planes, 2, 1, 1, &[], &[0, 0, 0, 0])).0;
            assert_eq!(r.unwrap_err(), "t: Bad PCX format", "{bpp} {planes}");
        }
    }

    #[test]
    fn lzw_round_trip() {
        // "abababab" compressed by `compress -c` (block mode, 16 bits).
        let z = [0x1f, 0x9d, 0x90, 0x61, 0xc4, 0x04, 0x1c, 0x28, 0x06];
        assert_eq!(uncompress_lzw(&z).unwrap(), b"abababab");
        assert!(uncompress_lzw(&[0x1f, 0x9d, 0x90, 0xff, 0xff, 0xff]).is_err());
        assert!(uncompress_lzw(&[0x1f, 0x9d, 0x99]).is_err());
    }

    #[test]
    fn junk_does_not_panic() {
        let ctx = Ctx::default();
        let mut seed = 12345u32;
        for len in 0..300usize {
            let mut junk: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    (seed >> 16) as u8
                })
                .collect();
            for prefix in [
                &b"GIF89a"[..],
                b"\x0a\x05\x01",
                b"%!",
                b"\xc5\xd0\xd3\xc6",
                b"%PDF",
                b"P6",
                b"II*\0",
                b"#define",
                b"\x89PNG\r\n\x1a\n",
                b"\xff\xd8\xff\xe0",
                b"/* XPM */",
            ] {
                let mut data = prefix.to_vec();
                data.append(&mut junk.clone());
                let stream = PicStream {
                    name: "junk".into(),
                    path: PathBuf::new(),
                    compression: Compression::None,
                    data,
                };
                let mut pic = Pic::default();
                let _ = read_pic(&mut ctx.clone(), &stream, &mut pic);
            }
            junk.insert(0, 0x1f);
            junk.insert(1, 0x9d);
            let _ = uncompress(&junk, Compression::Gzip);
            junk[1] = 0x8b;
            let _ = uncompress(&junk, Compression::Gzip);
        }
    }
}
