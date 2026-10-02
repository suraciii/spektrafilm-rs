//! Pinned Python LUT format parity. Documents use blue-fast `(r * N + g) * N + b`
//! indexing; files use red-fast `(b * N + g) * N + r` indexing.
//!
//! Generic CUBE uses ten significant decimal digits (maximum relative rounding
//! error 5e-10 for normal values); Lumix uses six fractional digits. Quantized
//! formats clip to [0, 1] and use NumPy-compatible ties-to-even rounding.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct LutDocument {
    pub resolution: usize,
    pub table: Vec<[f64; 3]>,
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
    pub title: String,
}

#[derive(Clone, Copy, Debug)]
pub struct LutDocumentRef<'a> {
    pub resolution: usize,
    pub table: &'a [[f64; 3]],
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
    pub title: &'a str,
}

impl LutDocument {
    pub fn as_ref(&self) -> LutDocumentRef<'_> {
        LutDocumentRef { resolution: self.resolution, table: &self.table, domain_min: self.domain_min, domain_max: self.domain_max, title: &self.title }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutFormat {
    Cube,
    Lumix,
    ThreeDl,
    HaldPng,
}

impl LutFormat {
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "cube" => Ok(Self::Cube),
            "lumix" => Ok(Self::Lumix),
            "3dl" => Ok(Self::ThreeDl),
            "hald_png" => Ok(Self::HaldPng),
            _ => bail!("unknown LUT format {name:?}; expected cube, lumix, 3dl, or hald_png"),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Cube => "cube",
            Self::Lumix => "lumix",
            Self::ThreeDl => "3dl",
            Self::HaldPng => "hald_png",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Cube | Self::Lumix => ".cube",
            Self::ThreeDl => ".3dl",
            Self::HaldPng => ".png",
        }
    }
}

pub fn write_lut(
    format: LutFormat,
    document: &LutDocument,
    path: impl AsRef<Path>,
    header_lines: &[String],
    photo_style_tag: Option<&str>,
) -> Result<()> {
    write_lut_ref(format, &document.as_ref(), path, header_lines, photo_style_tag)
}

pub fn write_lut_ref(
    format: LutFormat,
    document: &LutDocumentRef<'_>,
    path: impl AsRef<Path>,
    header_lines: &[String],
    photo_style_tag: Option<&str>,
) -> Result<()> {
    validate_document(document)?;
    let path = path.as_ref();
    match format {
        LutFormat::HaldPng => write_hald(document, path),
        _ => {
            let mut out = BufWriter::new(File::create(path).with_context(|| format!("creating {}", path.display()))?);
            match format {
                LutFormat::Cube => {
                    write_comments(&mut out, header_lines)?;
                    write_title(&mut out, &document.title)?;
                    write_domain(&mut out, document, false)?;
                    writeln!(out, "LUT_3D_SIZE {}", document.resolution)?;
                }
                LutFormat::Lumix => {
                    write_title(&mut out, &document.title)?;
                    if let Some(tag) = photo_style_tag.filter(|tag| !tag.is_empty()) {
                        ensure!(!tag.contains(['\r', '\n']), "photo-style tag must occupy one line");
                        writeln!(out, "#LUMIXPHOTOSTYLE {tag}")?;
                    }
                    writeln!(out, "LUT_3D_SIZE {}", document.resolution)?;
                    write_domain(&mut out, document, true)?;
                    writeln!(out)?;
                }
                LutFormat::ThreeDl => {
                    write_comments(&mut out, header_lines)?;
                    let n = document.resolution;
                    // NumPy linspace multiplies by a precomputed step and fixes
                    // its endpoint, rather than dividing each individual index.
                    let step = if n > 1 { 1023.0 / (n - 1) as f64 } else { 0.0 };
                    for i in 0..n {
                        if i > 0 { write!(out, " ")?; }
                        let value = if n > 1 && i == n - 1 { 1023 } else { (i as f64 * step).round_ties_even() as u16 };
                        write!(out, "{value}")?;
                    }
                    writeln!(out)?;
                }
                LutFormat::HaldPng => unreachable!(),
            }
            for file_index in 0..document.table.len() {
                let rgb = document.table[internal_index(file_index, document.resolution)];
                match format {
                    LutFormat::Cube => writeln!(out, "{} {} {}", general10(rgb[0]), general10(rgb[1]), general10(rgb[2]))?,
                    LutFormat::Lumix => writeln!(out, "{:.6} {:.6} {:.6}", rgb[0], rgb[1], rgb[2])?,
                    LutFormat::ThreeDl => writeln!(out, "{} {} {}", quantize(rgb[0], 1023), quantize(rgb[1], 1023), quantize(rgb[2], 1023))?,
                    LutFormat::HaldPng => unreachable!(),
                }
            }
            out.flush().with_context(|| format!("writing {}", path.display()))
        }
    }
}

pub fn read_lut(format: LutFormat, path: impl AsRef<Path>) -> Result<LutDocument> {
    let path = path.as_ref();
    match format {
        LutFormat::Cube | LutFormat::Lumix => read_cube(path),
        LutFormat::ThreeDl => read_3dl(path),
        LutFormat::HaldPng => read_hald(path),
    }.with_context(|| format!("reading {} LUT {}", format.name(), path.display()))
}

fn entry_count(n: usize) -> Result<usize> {
    ensure!(n > 0, "LUT resolution must be positive");
    n.checked_pow(3).context("LUT resolution overflows entry count")
}

fn validate_document(document: &LutDocumentRef<'_>) -> Result<()> {
    ensure!(document.table.len() == entry_count(document.resolution)?, "LUT table must contain resolution cubed entries");
    ensure!(document.table.iter().flatten().chain(document.domain_min.iter()).chain(document.domain_max.iter()).all(|v| v.is_finite()), "LUT values and domains must be finite");
    ensure!(!document.title.contains(['\r', '\n']), "LUT title must occupy one line");
    Ok(())
}

fn internal_index(file_index: usize, n: usize) -> usize {
    let r = file_index % n;
    let g = (file_index / n) % n;
    let b = file_index / (n * n);
    (r * n + g) * n + b
}

fn document_from_file_order(n: usize, values: Vec<[f64; 3]>) -> Result<LutDocument> {
    ensure!(values.len() == entry_count(n)?, "body has {} entries, expected {}", values.len(), entry_count(n)?);
    let mut table = vec![[0.0; 3]; values.len()];
    for (i, rgb) in values.into_iter().enumerate() {
        table[internal_index(i, n)] = rgb;
    }
    Ok(LutDocument { resolution: n, table, domain_min: [0.0; 3], domain_max: [1.0; 3], title: String::new() })
}

fn write_comments(out: &mut impl Write, lines: &[String]) -> Result<()> {
    for line in lines {
        if line.is_empty() { writeln!(out, "#")?; } else { writeln!(out, "# {line}")?; }
    }
    Ok(())
}

fn write_title(out: &mut impl Write, title: &str) -> Result<()> {
    if !title.is_empty() { writeln!(out, "TITLE \"{title}\"")?; }
    Ok(())
}

fn write_domain(out: &mut impl Write, document: &LutDocumentRef<'_>, fixed: bool) -> Result<()> {
    for (name, rgb) in [("DOMAIN_MIN", document.domain_min), ("DOMAIN_MAX", document.domain_max)] {
        if fixed {
            writeln!(out, "{name} {:.6} {:.6} {:.6}", rgb[0], rgb[1], rgb[2])?;
        } else {
            writeln!(out, "{name} {} {} {}", general10(rgb[0]), general10(rgb[1]), general10(rgb[2]))?;
        }
    }
    Ok(())
}

fn trim_fraction(text: &mut String) {
    if text.contains('.') {
        while text.ends_with('0') { text.pop(); }
        if text.ends_with('.') { text.pop(); }
    }
}

fn general10(value: f64) -> String {
    // Determine the exponent after rounding, including decade carries.
    let scientific = format!("{value:.9e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific format contains exponent");
    let exponent: i32 = exponent.parse().expect("scientific exponent is integer");
    if !(-4..10).contains(&exponent) {
        let mut mantissa = mantissa.to_owned();
        trim_fraction(&mut mantissa);
        format!("{mantissa}e{exponent:+03}")
    } else {
        let mut text = format!("{:.*}", (9 - exponent) as usize, value);
        trim_fraction(&mut text);
        text
    }
}

fn quantize(value: f64, max: u16) -> u16 {
    (value * f64::from(max)).round_ties_even().clamp(0.0, f64::from(max)) as u16
}

fn parse_triplet(text: &str) -> Result<[f64; 3]> {
    let parts: Vec<_> = text.split_whitespace().collect();
    ensure!(parts.len() == 3, "expected 3 floats in {text:?}");
    let rgb = [parts[0].parse::<f64>()?, parts[1].parse::<f64>()?, parts[2].parse::<f64>()?];
    ensure!(rgb.iter().all(|v| v.is_finite()), "non-finite triplet in {text:?}");
    Ok(rgb)
}

fn read_cube(path: &Path) -> Result<LutDocument> {
    let mut n = None;
    let mut title = String::new();
    let mut domain_min = [0.0; 3];
    let mut domain_max = [1.0; 3];
    let mut values = Vec::new();
    for raw in BufReader::new(File::open(path)?).lines() {
        let raw = raw?;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (head, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match head.to_ascii_uppercase().as_str() {
            "TITLE" => title = rest.trim().trim_matches('"').to_owned(),
            "DOMAIN_MIN" => domain_min = parse_triplet(rest)?,
            "DOMAIN_MAX" => domain_max = parse_triplet(rest)?,
            "LUT_3D_SIZE" => {
                ensure!(n.is_none(), "duplicate LUT_3D_SIZE header");
                let size = rest.trim().parse::<usize>()?;
                entry_count(size)?;
                n = Some(size);
            }
            "LUT_1D_SIZE" => bail!("1D CUBE LUTs are unsupported"),
            _ => values.push(parse_triplet(line)?),
        }
    }
    let mut document = document_from_file_order(n.context("missing LUT_3D_SIZE header")?, values)?;
    document.title = title;
    document.domain_min = domain_min;
    document.domain_max = domain_max;
    Ok(document)
}

fn read_3dl(path: &Path) -> Result<LutDocument> {
    let mut grid: Option<Vec<i64>> = None;
    let mut values = Vec::new();
    for raw in BufReader::new(File::open(path)?).lines() {
        let raw = raw?;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let parts: Vec<i64> = line.split_whitespace().map(str::parse).collect::<std::result::Result<_, _>>()?;
        if grid.is_none() {
            ensure!(!parts.is_empty() && parts[0] == 0 && parts.windows(2).all(|p| p[0] < p[1]), "shape grid must start at zero and be strictly ascending");
            entry_count(parts.len())?;
            grid = Some(parts);
        } else {
            ensure!(parts.len() == 3, "expected 3 integers per data line");
            values.push([parts[0] as f64, parts[1] as f64, parts[2] as f64]);
        }
    }
    let grid = grid.context("missing shape line")?;
    let max_code = (*grid.last().context("empty shape grid")?).max(1) as f64;
    for rgb in &mut values { for channel in rgb { *channel /= max_code; } }
    document_from_file_order(grid.len(), values)
}

fn hald_level(n: usize) -> Result<usize> {
    let level = n.isqrt();
    ensure!(n > 0 && level.checked_mul(level) == Some(n), "Hald requires a perfect-square LUT resolution; got {n}");
    Ok(level)
}

fn write_hald(document: &LutDocumentRef<'_>, path: &Path) -> Result<()> {
    let level = hald_level(document.resolution)?;
    let side = document.resolution.checked_mul(level).context("Hald image dimensions overflow")?;
    let side = u32::try_from(side).context("Hald image dimensions exceed PNG limits")?;
    let mut pixels = Vec::with_capacity(document.table.len().checked_mul(3).context("Hald image byte count overflow")?);
    for i in 0..document.table.len() {
        for channel in document.table[internal_index(i, document.resolution)] {
            pixels.push(quantize(channel, 255) as u8);
        }
    }
    image::save_buffer_with_format(path, &pixels, side, side, image::ColorType::Rgb8, image::ImageFormat::Png)?;
    Ok(())
}

fn read_hald(path: &Path) -> Result<LutDocument> {
    let image = image::open(path)?;
    let side = image.width();
    ensure!(side > 0 && side == image.height(), "Hald requires a square image");
    let mut level = (f64::from(side).cbrt().round()) as usize;
    // The exact integer check is authoritative, not floating-point cbrt.
    if level == 0 { level = 1; }
    ensure!(level.checked_pow(3) == Some(side as usize), "image side {side} is not an integer Hald level cubed");
    let n = level.checked_mul(level).context("Hald resolution overflow")?;
    let values = match image {
        // PIL treats 16-bit grayscale as integer luminance and clips when
        // converting to RGB; image::to_rgb8 would scale instead.
        image::DynamicImage::ImageLuma16(buffer) => buffer.pixels().map(|p| [f64::from(p.0[0].min(255)) / 255.0; 3]).collect(),
        image::DynamicImage::ImageLumaA16(buffer) => buffer.pixels().map(|p| [f64::from(p.0[0] >> 8) / 255.0; 3]).collect(),
        // PIL decodes 16-bit RGB PNG channels by dropping their low bytes.
        image::DynamicImage::ImageRgb16(buffer) => buffer.pixels().map(|p| p.0.map(|v| f64::from(v >> 8) / 255.0)).collect(),
        image::DynamicImage::ImageRgba16(buffer) => buffer.pixels().map(|p| [f64::from(p.0[0] >> 8) / 255.0, f64::from(p.0[1] >> 8) / 255.0, f64::from(p.0[2] >> 8) / 255.0]).collect(),
        other => other.to_rgb8().pixels().map(|p| p.0.map(|v| f64::from(v) / 255.0)).collect(),
    };
    document_from_file_order(n, values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempPath(std::path::PathBuf);
    impl TempPath {
        fn new(extension: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            Self(std::env::temp_dir().join(format!("spektrafilm-format-{}-{}{extension}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))))
        }
    }
    impl Drop for TempPath { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

    fn identity(n: usize) -> LutDocument {
        let mut table = Vec::new();
        for r in 0..n { for g in 0..n { for b in 0..n {
            table.push([r as f64 / (n - 1) as f64, g as f64 / (n - 1) as f64, b as f64 / (n - 1) as f64]);
        } } }
        LutDocument { resolution: n, table, domain_min: [-1.0, -2.0, -3.0], domain_max: [2.0, 3.0, 4.0], title: "Boundary LUT".into() }
    }

    #[test]
    fn cube_preserves_domains_and_red_fast_file_order() {
        let path = TempPath::new(".cube");
        let document = identity(2);
        write_lut(LutFormat::Cube, &document, &path.0, &["provenance".into(), String::new()], None).unwrap();
        let text = std::fs::read_to_string(&path.0).unwrap();
        assert!(text.ends_with("0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n"));
        assert_eq!(read_lut(LutFormat::Cube, &path.0).unwrap(), document);
    }

    #[test]
    fn lumix_has_strict_header_and_fixed_decimal_data() {
        let path = TempPath::new(".cube");
        let document = identity(2);
        write_lut(LutFormat::Lumix, &document, &path.0, &["must be ignored".into()], Some("VLOG")).unwrap();
        let text = std::fs::read_to_string(&path.0).unwrap();
        assert!(text.starts_with("TITLE \"Boundary LUT\"\n#LUMIXPHOTOSTYLE VLOG\nLUT_3D_SIZE 2\nDOMAIN_MIN -1.000000 -2.000000 -3.000000\nDOMAIN_MAX 2.000000 3.000000 4.000000\n\n0.000000 0.000000 0.000000\n"));
        assert!(!text.contains("must be ignored"));
        assert_eq!(read_lut(LutFormat::Lumix, &path.0).unwrap(), document);
    }

    #[test]
    fn quantization_matches_numpy_ties_and_clipping() {
        for max in [255, 1023] {
            for (code, expected) in [(0.5, 0), (1.5, 2), (2.5, 2)] {
                assert_eq!(quantize(code / f64::from(max), max), expected);
            }
            assert_eq!(quantize(-0.5, max), 0);
            assert_eq!(quantize(2.0, max), max);
        }
    }

    #[test]
    fn three_dl_rounds_grid_and_data_and_reads_declared_scale() {
        let path = TempPath::new(".3dl");
        let mut document = identity(3);
        document.table[0] = [0.5 / 1023.0, 1.5 / 1023.0, 2.5 / 1023.0];
        write_lut(LutFormat::ThreeDl, &document, &path.0, &[], None).unwrap();
        let text = std::fs::read_to_string(&path.0).unwrap();
        assert!(text.starts_with("0 512 1023\n0 2 2\n512 0 0\n1023 0 0\n"));
        let decoded = read_lut(LutFormat::ThreeDl, &path.0).unwrap();
        assert_eq!(decoded.table[0], [0.0, 2.0 / 1023.0, 2.0 / 1023.0]);
        std::fs::write(&path.0, "0\n2 -1 0\n").unwrap();
        assert_eq!(read_lut(LutFormat::ThreeDl, &path.0).unwrap().table, vec![[2.0, -1.0, 0.0]]);
    }

    #[test]
    fn readers_reject_missing_malformed_and_incomplete_grids() {
        let path = TempPath::new(".txt");
        for text in ["0 0 0\n", "LUT_3D_SIZE 0\n", "LUT_3D_SIZE 2\n0 0 0\n", "LUT_3D_SIZE 1\n0 0\n", "LUT_1D_SIZE 1\n0 0 0\n", "LUT_3D_SIZE 1\nnan 0 0\n"] {
            std::fs::write(&path.0, text).unwrap();
            assert!(read_lut(LutFormat::Cube, &path.0).is_err(), "{text}");
        }
        for text in ["# empty\n", "0 0 1023\n", "0 512 400\n", "1 1023\n", "0 1023\n0 0 0\n", "0\n0 0.5 0\n"] {
            std::fs::write(&path.0, text).unwrap();
            assert!(read_lut(LutFormat::ThreeDl, &path.0).is_err(), "{text}");
        }
    }

    #[test]
    fn hald_dimensions_pixel_order_and_quantization_budget() {
        let path = TempPath::new(".png");
        let document = identity(4);
        write_lut(LutFormat::HaldPng, &document, &path.0, &[], None).unwrap();
        let image = image::open(&path.0).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (8, 8));
        assert_eq!(image.get_pixel(1, 0).0, [85, 0, 0]);
        assert_eq!(image.get_pixel(0, 2).0, [0, 0, 85]);
        let decoded = read_lut(LutFormat::HaldPng, &path.0).unwrap();
        for (before, after) in document.table.iter().zip(&decoded.table) {
            for channel in 0..3 { assert!((before[channel] - after[channel]).abs() <= 0.5 / 255.0); }
        }
        assert!(write_lut(LutFormat::HaldPng, &identity(3), &path.0, &[], None).is_err());
        for (width, height) in [(8, 7), (7, 7)] {
            image::RgbImage::new(width, height).save_with_format(&path.0, image::ImageFormat::Png).unwrap();
            assert!(read_lut(LutFormat::HaldPng, &path.0).is_err());
        }
    }

    #[test]
    fn quantized_files_clip_and_preserve_ties_even_at_pixel_boundary() {
        let path = TempPath::new(".png");
        let mut document = identity(4);
        document.table[0] = [-0.1, 1.5 / 255.0, 1.1];
        write_lut_ref(LutFormat::HaldPng, &document.as_ref(), &path.0, &[], None).unwrap();
        assert_eq!(image::open(&path.0).unwrap().to_rgb8().get_pixel(0, 0).0, [0, 2, 255]);
        let path = TempPath::new(".3dl");
        document.table[0] = [-0.1, 2.5 / 1023.0, 1.1];
        write_lut_ref(LutFormat::ThreeDl, &document.as_ref(), &path.0, &[], None).unwrap();
        assert_eq!(read_lut(LutFormat::ThreeDl, &path.0).unwrap().table[0], [0.0, 2.0 / 1023.0, 1.0]);
    }

    #[test]
    fn writers_reject_incomplete_or_nonfinite_documents() {
        let path = TempPath::new(".cube");
        let mut document = identity(2);
        document.table.pop();
        assert!(write_lut(LutFormat::Cube, &document, &path.0, &[], None).is_err());
        let mut document = identity(2);
        document.table[0][0] = f64::NAN;
        assert!(write_lut(LutFormat::Cube, &document, &path.0, &[], None).is_err());
    }

    #[test]
    fn hald_rgb_conversion_drops_alpha_and_clips_integer_gray() {
        let path = TempPath::new(".png");
        let rgba = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 0]));
        rgba.save_with_format(&path.0, image::ImageFormat::Png).unwrap();
        assert_eq!(read_lut(LutFormat::HaldPng, &path.0).unwrap().table[0], [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0]);
        let gray_alpha = image::ImageBuffer::<image::LumaA<u16>, Vec<u16>>::from_pixel(1, 1, image::LumaA([256, 0]));
        gray_alpha.save_with_format(&path.0, image::ImageFormat::Png).unwrap();
        assert_eq!(read_lut(LutFormat::HaldPng, &path.0).unwrap().table[0], [1.0 / 255.0; 3]);
        for (sample, expected) in [(1_u16, 1.0 / 255.0), (256, 1.0), (65535, 1.0)] {
            let gray = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_pixel(1, 1, image::Luma([sample]));
            gray.save_with_format(&path.0, image::ImageFormat::Png).unwrap();
            assert_eq!(read_lut(LutFormat::HaldPng, &path.0).unwrap().table[0], [expected; 3]);
        }
    }

    #[test]
    fn ten_digit_formatting_handles_decade_carries_and_error_budget() {
        for (value, expected) in [(0.0, "0"), (-0.0, "-0"), (0.00001, "1e-05"), (1e10, "1e+10"), (9.9999999996, "10"), (0.000099999999996, "0.0001")] {
            assert_eq!(general10(value), expected);
        }
        for value in [-1.23456789123, 0.123456789123, 1.00000000049e-90, 9.876543219876e90] {
            let restored: f64 = general10(value).parse().unwrap();
            assert!((restored - value).abs() / value.abs() <= 5e-10);
        }
    }
}
