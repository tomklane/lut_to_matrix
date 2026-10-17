//! `.cube` reader/writer.
//!
//! Follows the Adobe "Cube LUT Specification" v1.0 (2013): TITLE, LUT_1D_SIZE,
//! LUT_3D_SIZE, DOMAIN_MIN, DOMAIN_MAX, `#` comments, red-fastest data order.
//! Also accepts the Resolve extensions: a 1D shaper and a 3D table in one file
//! (1D data first), LUT_1D_INPUT_RANGE / LUT_3D_INPUT_RANGE, and the
//! LUT_IN/OUT_VIDEO_RANGE flags (recorded as warnings).

use std::fmt::Write as _;

use super::{Lut, Lut1D, Lut3D, LutError, LutFormat};

pub fn parse_cube(text: &str) -> Result<Lut, LutError> {
    let mut title = None;
    let mut comments = Vec::new();
    let mut warnings = Vec::new();
    let mut size_1d: Option<usize> = None;
    let mut size_3d: Option<usize> = None;
    let mut domain_min: Option<[f64; 3]> = None;
    let mut domain_max: Option<[f64; 3]> = None;
    let mut range_1d: Option<(f64, f64)> = None;
    let mut range_3d: Option<(f64, f64)> = None;
    let mut data: Vec<[f64; 3]> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(c) = line.strip_prefix('#') {
            let c = c.trim();
            if !c.is_empty() {
                comments.push(c.to_string());
            }
            continue;
        }
        // Strip a trailing comment.
        let line = match line.find('#') {
            Some(p) if !line.starts_with("TITLE") => line[..p].trim(),
            _ => line,
        };
        let first = line.as_bytes()[0];
        if first.is_ascii_alphabetic() {
            let (kw, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            let err = |msg: &str| LutError::Parse { line: line_no, msg: format!("{kw}: {msg}") };
            match kw.to_ascii_uppercase().as_str() {
                "TITLE" => title = Some(rest.trim_matches('"').to_string()),
                "LUT_1D_SIZE" => size_1d = Some(rest.parse().map_err(|_| err("bad size"))?),
                "LUT_3D_SIZE" => size_3d = Some(rest.parse().map_err(|_| err("bad size"))?),
                "DOMAIN_MIN" => domain_min = Some(parse3(rest).ok_or_else(|| err("need 3 numbers"))?),
                "DOMAIN_MAX" => domain_max = Some(parse3(rest).ok_or_else(|| err("need 3 numbers"))?),
                "LUT_1D_INPUT_RANGE" => range_1d = Some(parse2(rest).ok_or_else(|| err("need 2 numbers"))?),
                "LUT_3D_INPUT_RANGE" => range_3d = Some(parse2(rest).ok_or_else(|| err("need 2 numbers"))?),
                "LUT_IN_VIDEO_RANGE" => warnings.push("LUT declares video-range input (LUT_IN_VIDEO_RANGE)".into()),
                "LUT_OUT_VIDEO_RANGE" => warnings.push("LUT declares video-range output (LUT_OUT_VIDEO_RANGE)".into()),
                other => warnings.push(format!("line {line_no}: ignored keyword {other}")),
            }
            continue;
        }
        let v = parse3(line).ok_or_else(|| LutError::Parse {
            line: line_no,
            msg: format!("expected three numbers, got '{line}'"),
        })?;
        data.push(v);
    }

    if size_1d.is_none() && size_3d.is_none() {
        return Err(LutError::Invalid("no LUT_1D_SIZE or LUT_3D_SIZE found".into()));
    }
    let n1 = size_1d.unwrap_or(0);
    let n3 = size_3d.map(|n| n * n * n).unwrap_or(0);
    if data.len() != n1 + n3 {
        return Err(LutError::Invalid(format!(
            "expected {} data lines ({} 1D + {} 3D), found {}",
            n1 + n3,
            n1,
            n3,
            data.len()
        )));
    }
    if let Some(n) = size_1d {
        if n < 2 {
            return Err(LutError::Invalid("LUT_1D_SIZE must be ≥ 2".into()));
        }
    }
    if let Some(n) = size_3d {
        if n < 2 {
            return Err(LutError::Invalid("LUT_3D_SIZE must be ≥ 2".into()));
        }
    }

    let dmin = domain_min.unwrap_or([0.0; 3]);
    let dmax = domain_max.unwrap_or([1.0; 3]);
    for c in 0..3 {
        if dmax[c] <= dmin[c] {
            return Err(LutError::Invalid("DOMAIN_MAX must exceed DOMAIN_MIN".into()));
        }
    }
    let r3 = |r: Option<(f64, f64)>| r.map(|(a, b)| ([a; 3], [b; 3]));

    let both = size_1d.is_some() && size_3d.is_some();
    let shaper = size_1d.map(|n| {
        let (lo, hi) = r3(range_1d).unwrap_or((dmin, dmax));
        Lut1D { data: data[..n].to_vec(), domain_min: lo, domain_max: hi }
    });
    let cube = size_3d.map(|n| {
        // With a shaper, DOMAIN_* belongs to the shaper; the 3D table then takes its
        // own LUT_3D_INPUT_RANGE or defaults to [0, 1].
        let default = if both { ([0.0; 3], [1.0; 3]) } else { (dmin, dmax) };
        let (lo, hi) = r3(range_3d).unwrap_or(default);
        Lut3D { size: n, data: data[n1..].to_vec(), domain_min: lo, domain_max: hi }
    });

    Ok(Lut {
        format: LutFormat::Cube,
        filename: None,
        title,
        comments,
        shaper,
        cube,
        warnings,
    })
}

fn parse3(s: &str) -> Option<[f64; 3]> {
    let mut it = s.split_whitespace().map(|t| t.parse::<f64>());
    let a = it.next()?.ok()?;
    let b = it.next()?.ok()?;
    let c = it.next()?.ok()?;
    if it.next().is_some() || !(a.is_finite() && b.is_finite() && c.is_finite()) {
        return None;
    }
    Some([a, b, c])
}

fn parse2(s: &str) -> Option<(f64, f64)> {
    let mut it = s.split_whitespace().map(|t| t.parse::<f64>());
    let a = it.next()?.ok()?;
    let b = it.next()?.ok()?;
    Some((a, b))
}

/// Serialise a 3D LUT as `.cube` (used for synthetic test LUTs and exports).
pub fn write_cube(lut: &Lut3D, title: &str, comments: &[&str]) -> String {
    let mut s = String::with_capacity(lut.data.len() * 30 + 256);
    for c in comments {
        let _ = writeln!(s, "# {c}");
    }
    let _ = writeln!(s, "TITLE \"{title}\"");
    let _ = writeln!(s, "LUT_3D_SIZE {}", lut.size);
    if lut.domain_min != [0.0; 3] || lut.domain_max != [1.0; 3] {
        let [a, b, c] = lut.domain_min;
        let _ = writeln!(s, "DOMAIN_MIN {a} {b} {c}");
        let [a, b, c] = lut.domain_max;
        let _ = writeln!(s, "DOMAIN_MAX {a} {b} {c}");
    }
    for v in &lut.data {
        let _ = writeln!(s, "{:.10} {:.10} {:.10}", v[0], v[1], v[2]);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::Interpolation;
    use crate::math::Vec3;

    #[test]
    fn parses_minimal_3d() {
        let txt = "# hello\nTITLE \"Test LUT\"\nLUT_3D_SIZE 2\n\
            0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = parse_cube(txt).unwrap();
        assert_eq!(lut.title.as_deref(), Some("Test LUT"));
        assert_eq!(lut.comments, vec!["hello"]);
        let c = lut.cube.as_ref().unwrap();
        assert_eq!(c.size, 2);
        let v = Vec3::new(0.25, 0.5, 0.75);
        assert!((lut.apply(&v, Interpolation::Tetrahedral) - v).abs().max() < 1e-12);
    }

    #[test]
    fn domain_and_shaper() {
        // 1D shaper maps [0, 2] → [0, 1] linearly, then identity 3D.
        let mut txt = String::from("LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0 2\nLUT_3D_SIZE 2\n0 0 0\n1 1 1\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    txt.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        let lut = parse_cube(&txt).unwrap();
        assert!(lut.shaper.is_some() && lut.cube.is_some());
        let o = lut.apply(&Vec3::new(1.0, 0.5, 2.0), Interpolation::Tetrahedral);
        assert!((o - Vec3::new(0.5, 0.25, 1.0)).abs().max() < 1e-12);
        assert!(lut.in_domain(&Vec3::new(1.9, 0.0, 0.0)));
        assert!(!lut.in_domain(&Vec3::new(2.1, 0.0, 0.0)));
    }

    #[test]
    fn domain_min_max_3d() {
        let txt = "LUT_3D_SIZE 2\nDOMAIN_MIN -1 -1 -1\nDOMAIN_MAX 1 1 1\n\
            0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = parse_cube(txt).unwrap();
        let o = lut.apply(&Vec3::new(0.0, 0.0, 0.0), Interpolation::Trilinear);
        assert!((o - Vec3::new(0.5, 0.5, 0.5)).abs().max() < 1e-12);
    }

    #[test]
    fn round_trip_writer() {
        let l = Lut3D::from_fn(5, |v| Vec3::new(v.x.powf(0.5), v.y * 0.5, 1.0 - v.z));
        let txt = write_cube(&l, "rt", &["made by test"]);
        let back = parse_cube(&txt).unwrap();
        let c = back.cube.unwrap();
        assert_eq!(c.size, 5);
        for (a, b) in c.data.iter().zip(&l.data) {
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn rejects_wrong_count_and_garbage() {
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 zero\n").is_err());
        assert!(parse_cube("0 0 0\n").is_err());
    }
}
