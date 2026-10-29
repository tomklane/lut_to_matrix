//! Reading chart stills: the pixel values exactly as stored, as `f32` RGB.
//!
//! No colour management of any kind is applied: integer files are normalised to 0–1 by
//! their largest code (full range, DECISIONS.md D4), float files (OpenEXR, float TIFF)
//! are taken as they are. Alpha is dropped. Supported: TIFF (8/16-bit integer, 16/32-bit
//! float; uncompressed, LZW, Deflate, PackBits), OpenEXR (half/float, any compression),
//! PNG (8/16-bit), JPEG and DPX (8/10/12/16-bit, the format Resolve's gallery exports by
//! default).

use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::math::{Primaries, Xy};

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}: {1}")]
    Decode(String, String),
    #[error("{0}: unsupported DPX file ({1})")]
    Dpx(String, String),
}

/// A decoded still.
#[derive(Debug, Clone)]
pub struct ChartImage {
    pub file_name: String,
    pub width: usize,
    pub height: usize,
    /// RGB, row-major, interleaved, as stored (integers normalised to 0–1).
    pub data: Vec<f32>,
    pub info: ImageInfo,
}

/// What the file is, for the UI and for clipping checks.
#[derive(Debug, Clone, Serialize)]
pub struct ImageInfo {
    /// "TIFF", "OpenEXR", "PNG", "JPEG", "DPX".
    pub format: String,
    /// "16-bit integer", "half float", …
    pub sample_type: String,
    pub float: bool,
    /// The largest code of an integer file (65535, 1023, …); 0 for float files.
    pub max_code: u32,
    /// Chromaticities stored in an OpenEXR header, if any.
    pub chromaticities: Option<Primaries>,
    pub width: usize,
    pub height: usize,
}

impl ChartImage {
    pub fn pixel(&self, x: usize, y: usize) -> [f32; 3] {
        let i = 3 * (y * self.width + x);
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    /// From raw RGB data (tests, synthetic charts).
    pub fn from_rgb(name: &str, width: usize, height: usize, data: Vec<f32>, float: bool) -> Self {
        assert_eq!(data.len(), width * height * 3);
        ChartImage {
            file_name: name.into(),
            width,
            height,
            data,
            info: ImageInfo {
                format: "memory".into(),
                sample_type: if float { "32-bit float".into() } else { "16-bit integer".into() },
                float,
                max_code: if float { 0 } else { 65535 },
                chromaticities: None,
                width,
                height,
            },
        }
    }

    /// Quantise to `bits` integer codes in 0–1, as an integer file would store it
    /// (tests: negative values and values above 1 clip, as in a 16-bit TIFF).
    pub fn quantised(mut self, bits: u32) -> Self {
        let max = ((1u64 << bits) - 1) as f32;
        for v in &mut self.data {
            *v = (v.clamp(0.0, 1.0) * max).round() / max;
        }
        self.info.float = false;
        self.info.max_code = max as u32;
        self.info.sample_type = format!("{bits}-bit integer");
        self
    }

    pub fn load(path: &Path) -> Result<ChartImage, ImageError> {
        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext == "dpx" {
            let bytes = std::fs::read(path).map_err(|e| ImageError::Io(name.clone(), e))?;
            return read_dpx(&name, &bytes);
        }
        let reader = image::ImageReader::open(path)
            .map_err(|e| ImageError::Io(name.clone(), e))?
            .with_guessed_format()
            .map_err(|e| ImageError::Io(name.clone(), e))?;
        let format = reader.format();
        let mut reader = reader;
        reader.no_limits();
        let img = reader.decode().map_err(|e| ImageError::Decode(name.clone(), e.to_string()))?;
        use image::ColorType as C;
        let (sample_type, float, max_code) = match img.color() {
            C::L8 | C::La8 | C::Rgb8 | C::Rgba8 => ("8-bit integer", false, 255),
            C::L16 | C::La16 | C::Rgb16 | C::Rgba16 => ("16-bit integer", false, 65535),
            C::Rgb32F | C::Rgba32F => ("float", true, 0),
            _ => ("other", true, 0),
        };
        let fmt = match format {
            Some(image::ImageFormat::Tiff) => "TIFF",
            Some(image::ImageFormat::OpenExr) => "OpenEXR",
            Some(image::ImageFormat::Png) => "PNG",
            Some(image::ImageFormat::Jpeg) => "JPEG",
            _ => "image",
        };
        let chromaticities = if fmt == "OpenEXR" { exr_chromaticities(path) } else { None };
        let sample_type = if fmt == "OpenEXR" { exr_sample_type(path).unwrap_or(sample_type.into()) } else { sample_type.into() };
        let rgb = img.into_rgb32f();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        Ok(ChartImage {
            file_name: name,
            width: w,
            height: h,
            data: rgb.into_raw(),
            info: ImageInfo {
                format: fmt.into(),
                sample_type,
                float,
                max_code,
                chromaticities,
                width: w,
                height: h,
            },
        })
    }
}

/// Chromaticities attribute of an OpenEXR file (first header).
fn exr_chromaticities(path: &Path) -> Option<Primaries> {
    let meta = exr::meta::MetaData::read_from_file(path, false).ok()?;
    let c = meta.headers.first()?.shared_attributes.chromaticities?;
    let xy = |v: exr::math::Vec2<f32>| Xy::new(v.0 as f64, v.1 as f64);
    Some(Primaries { r: xy(c.red), g: xy(c.green), b: xy(c.blue), white: xy(c.white) })
}

fn exr_sample_type(path: &Path) -> Option<String> {
    let meta = exr::meta::MetaData::read_from_file(path, false).ok()?;
    let ch = meta.headers.first()?.channels.list.first()?;
    Some(match ch.sample_type {
        exr::meta::attribute::SampleType::F16 => "half float".into(),
        exr::meta::attribute::SampleType::F32 => "32-bit float".into(),
        exr::meta::attribute::SampleType::U32 => "32-bit integer".into(),
    })
}

/// DPX (SMPTE 268M): RGB or RGBA, 8/10/12/16 bits, packing 0 for 8/16 bit, 1 (method A)
/// or 2 (method B) for 10/12 bit, uncompressed, either byte order.
fn read_dpx(name: &str, b: &[u8]) -> Result<ChartImage, ImageError> {
    let bad = |why: &str| ImageError::Dpx(name.into(), why.into());
    if b.len() < 812 {
        return Err(bad("file too short"));
    }
    let big = match &b[0..4] {
        b"SDPX" => true,
        b"XPDS" => false,
        _ => return Err(bad("not a DPX file")),
    };
    let u32_at = |o: usize| {
        let a = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        if big {
            u32::from_be_bytes(a)
        } else {
            u32::from_le_bytes(a)
        }
    };
    let u16_at = |o: usize| {
        let a = [b[o], b[o + 1]];
        if big {
            u16::from_be_bytes(a)
        } else {
            u16::from_le_bytes(a)
        }
    };
    let width = u32_at(772) as usize;
    let height = u32_at(776) as usize;
    let descriptor = b[800];
    let bits = b[803] as u32;
    let packing = u16_at(804);
    let encoding = u16_at(806);
    let mut offset = u32_at(808) as usize;
    if offset == 0 || offset == u32::MAX as usize {
        offset = u32_at(4) as usize;
    }
    let comps = match descriptor {
        50 => 3,
        51 => 4,
        _ => return Err(bad(&format!("descriptor {descriptor} (only RGB and RGBA)"))),
    };
    if encoding != 0 {
        return Err(bad("run-length encoded"));
    }
    if width == 0 || height == 0 || width > 1 << 16 || height > 1 << 16 {
        return Err(bad("bad image size"));
    }
    let max = ((1u32 << bits) - 1) as f32;
    let n = width * height * comps;
    let mut vals: Vec<f32> = Vec::with_capacity(n);
    let data = b.get(offset..).ok_or_else(|| bad("image data offset past the end"))?;
    match (bits, packing) {
        (8, _) => {
            let row = (width * comps).div_ceil(4) * 4;
            for y in 0..height {
                let r = data.get(y * row..y * row + width * comps).ok_or_else(|| bad("truncated"))?;
                vals.extend(r.iter().map(|&v| v as f32 / max));
            }
        }
        (16, _) => {
            if data.len() < 2 * n {
                return Err(bad("truncated"));
            }
            for k in 0..n {
                let a = [data[2 * k], data[2 * k + 1]];
                let v = if big { u16::from_be_bytes(a) } else { u16::from_le_bytes(a) };
                vals.push(v as f32 / max);
            }
        }
        (10, 1) | (10, 2) => {
            // Three components per 32-bit word; each line starts on a new word.
            let words_per_line = (width * comps).div_ceil(3);
            for y in 0..height {
                let mut line = Vec::with_capacity(words_per_line * 3);
                for w in 0..words_per_line {
                    let o = 4 * (y * words_per_line + w);
                    if o + 4 > data.len() {
                        return Err(bad("truncated"));
                    }
                    let a = [data[o], data[o + 1], data[o + 2], data[o + 3]];
                    let word = if big { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) };
                    let word = if packing == 1 { word >> 2 } else { word };
                    for s in [20, 10, 0] {
                        line.push(((word >> s) & 0x3ff) as f32 / max);
                    }
                }
                vals.extend(&line[..width * comps]);
            }
        }
        (12, 1) | (12, 2) => {
            if data.len() < 2 * n {
                return Err(bad("truncated"));
            }
            for k in 0..n {
                let a = [data[2 * k], data[2 * k + 1]];
                let v = if big { u16::from_be_bytes(a) } else { u16::from_le_bytes(a) };
                let v = if packing == 1 { v >> 4 } else { v & 0xfff };
                vals.push(v as f32 / max);
            }
        }
        _ => return Err(bad(&format!("{bits}-bit with packing {packing}"))),
    }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for px in vals.chunks_exact(comps) {
        rgb.extend_from_slice(&px[..3]);
    }
    Ok(ChartImage {
        file_name: name.into(),
        width,
        height,
        data: rgb,
        info: ImageInfo {
            format: "DPX".into(),
            sample_type: format!("{bits}-bit integer"),
            float: false,
            max_code: max as u32,
            chromaticities: None,
            width,
            height,
        },
    })
}

/// File extensions the chart view opens.
pub const IMAGE_EXTENSIONS: [&str; 7] = ["tif", "tiff", "exr", "png", "jpg", "jpeg", "dpx"];

pub fn is_image_path(p: &str) -> bool {
    let ext = Path::new(p).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×2 10-bit DPX (method A, big-endian) written by hand reads back exactly.
    #[test]
    fn dpx_10bit_method_a() {
        let (w, h) = (3usize, 2usize);
        let mut b = vec![0u8; 2048];
        b[0..4].copy_from_slice(b"SDPX");
        b[4..8].copy_from_slice(&2048u32.to_be_bytes());
        b[772..776].copy_from_slice(&(w as u32).to_be_bytes());
        b[776..780].copy_from_slice(&(h as u32).to_be_bytes());
        b[800] = 50;
        b[803] = 10;
        b[804..806].copy_from_slice(&1u16.to_be_bytes());
        b[808..812].copy_from_slice(&2048u32.to_be_bytes());
        let codes: Vec<u32> = (0..(w * h * 3) as u32).map(|i| (i * 97) % 1024).collect();
        for y in 0..h {
            for x in 0..w {
                let c = &codes[3 * (y * w + x)..3 * (y * w + x) + 3];
                let word = (c[0] << 22) | (c[1] << 12) | (c[2] << 2);
                b.extend_from_slice(&word.to_be_bytes());
            }
        }
        let img = read_dpx("t.dpx", &b).unwrap();
        assert_eq!((img.width, img.height, img.info.max_code), (3, 2, 1023));
        for (v, c) in img.data.iter().zip(&codes) {
            assert!((v * 1023.0 - *c as f32).abs() < 1e-3);
        }
    }

    #[test]
    fn tiff_16bit_round_trip() {
        let dir = std::env::temp_dir().join(format!("lutmatrix-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.tif");
        let mut buf = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(4, 3);
        for (x, y, px) in buf.enumerate_pixels_mut() {
            *px = image::Rgba([(x * 1000) as u16, (y * 20000) as u16, 65535, 65535]);
        }
        buf.save(&p).unwrap();
        let img = ChartImage::load(&p).unwrap();
        assert_eq!((img.width, img.height, img.info.format.as_str(), img.info.max_code), (4, 3, "TIFF", 65535));
        let px = img.pixel(3, 2);
        assert!((px[0] - 3000.0 / 65535.0).abs() < 1e-7 && (px[1] - 40000.0 / 65535.0).abs() < 1e-7 && px[2] == 1.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
