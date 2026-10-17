//! `.3dl` reader (Autodesk Lustre / Flame style).
//!
//! Layout: optional `3DMESH` / `Mesh <inBits> <outBits>` header (Lustre), a line of
//! N input shaper values (integers in input bit depth), then N³ integer triples with
//! **blue varying fastest**. Output bit depth comes from the `Mesh` line, or is
//! inferred from the largest value (≤1 → float, else 10/12/14/16-bit).

use super::{Lut, Lut3D, LutError, LutFormat};

pub fn parse_3dl(text: &str) -> Result<Lut, LutError> {
    let mut comments = Vec::new();
    let mut warnings = Vec::new();
    let mut out_bits: Option<u32> = None;
    let mut shaper: Option<Vec<f64>> = None;
    let mut triples: Vec<[f64; 3]> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
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
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("3dmesh") || lower.starts_with("lut8") || lower.starts_with("gamma") {
            continue;
        }
        if lower.starts_with("mesh") {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                out_bits = parts[2].parse().ok();
            }
            continue;
        }
        let nums: Result<Vec<f64>, _> = line.split_whitespace().map(str::parse::<f64>).collect();
        let nums = nums.map_err(|_| LutError::Parse { line: idx + 1, msg: format!("unexpected '{line}'") })?;
        if shaper.is_none() && nums.len() > 3 {
            shaper = Some(nums);
            continue;
        }
        if nums.len() != 3 {
            return Err(LutError::Parse { line: idx + 1, msg: "expected three values".into() });
        }
        triples.push([nums[0], nums[1], nums[2]]);
    }

    let n = match &shaper {
        Some(s) => s.len(),
        None => (triples.len() as f64).cbrt().round() as usize,
    };
    if n < 2 || n * n * n != triples.len() {
        return Err(LutError::Invalid(format!(
            ".3dl: {} triples is not a cube of the {}-point input mesh",
            triples.len(),
            n
        )));
    }
    if let Some(s) = &shaper {
        let step = s[1] - s[0];
        let uniform = s.windows(2).all(|w| ((w[1] - w[0]) - step).abs() <= 1.0);
        if !uniform {
            warnings.push(".3dl input mesh is not uniform; treated as uniform".into());
        }
    }

    let max = triples.iter().flatten().cloned().fold(0.0, f64::max);
    let scale = match out_bits {
        Some(b) => ((1u64 << b) - 1) as f64,
        None if max <= 1.0 => 1.0,
        None => [1023.0, 4095.0, 16383.0, 65535.0]
            .into_iter()
            .find(|&m| max <= m)
            .unwrap_or(max),
    };

    // Reorder blue-fastest → red-fastest.
    let mut data = vec![[0.0; 3]; n * n * n];
    for (i, t) in triples.iter().enumerate() {
        let b = i % n;
        let g = (i / n) % n;
        let r = i / (n * n);
        data[r + g * n + b * n * n] = [t[0] / scale, t[1] / scale, t[2] / scale];
    }

    Ok(Lut {
        format: LutFormat::ThreeDl,
        filename: None,
        title: None,
        comments,
        shaper: None,
        cube: Some(Lut3D { size: n, data, domain_min: [0.0; 3], domain_max: [1.0; 3] }),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::Interpolation;
    use crate::math::Vec3;

    #[test]
    fn parses_identity_3dl() {
        let n = 5;
        let mut s = String::from("# test\n0 256 512 767 1023\n");
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    let v = |i: usize| i * 4095 / (n - 1);
                    s.push_str(&format!("{} {} {}\n", v(r), v(g), v(b)));
                }
            }
        }
        let lut = parse_3dl(&s).unwrap();
        let o = lut.apply(&Vec3::new(1.0, 0.0, 0.5), Interpolation::Tetrahedral);
        assert!((o - Vec3::new(1.0, 0.0, 0.5)).abs().max() < 1e-3);
        assert_eq!(lut.comments, vec!["test"]);
    }
}
