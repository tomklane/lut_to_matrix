//! Finding a 24-patch chart in a still, and sampling its patches.
//!
//! 1. A single-channel "detection image": the largest of R, G, B (a patch is bright in
//!    at least one channel, the black grid between patches is dark in all), stretched
//!    between its 1st and 99.5th percentiles, at most ~1600 px across.
//! 2. Patch candidates at four scales: pixels brighter than their local mean (box filter
//!    1, 2, 4 and 8 % of the image size), opened, split into connected blobs; a blob is a
//!    candidate when its minimum-area rectangle is square-ish (aspect ≤ 1.6) and well
//!    filled (≥ 75 %).
//! 3. The lattice: for each candidate, the nearest similar-sized neighbour and the
//!    nearest roughly perpendicular one span a grid; every candidate is given integer grid
//!    coordinates and the largest connected blob of grid cells wins. A homography fitted
//!    to the members (perspective allowed) is refined twice, adding members.
//! 4. The chart's 6 × 4 footprint is placed on the lattice in every orientation (and
//!    every offset when edge patches were not found, e.g. a black patch on a black
//!    surround); the caller picks the placement whose colours fit the reference best.
//!
//! Everything works on pixel values as stored, whatever their encoding.

use nalgebra::{DMatrix, Matrix3, Vector3};
use serde::{Deserialize, Serialize};

use super::image::ChartImage;
use crate::math::Mat3;

/// How the chart lies in the image. The names are the chart's rotation clockwise, as
/// seen on screen, with the neutral row at the bottom for `Rot0`. `Mirror*` are the same
/// seen in a mirror (a flipped image).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Rot0,
    Rot90,
    Rot180,
    Rot270,
    Mirror0,
    Mirror90,
    Mirror180,
    Mirror270,
}

impl Orientation {
    pub const ALL: [Orientation; 8] = [
        Orientation::Rot0,
        Orientation::Rot90,
        Orientation::Rot180,
        Orientation::Rot270,
        Orientation::Mirror0,
        Orientation::Mirror90,
        Orientation::Mirror180,
        Orientation::Mirror270,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Orientation::Rot0 => "Upright",
            Orientation::Rot90 => "Rotated 90° clockwise",
            Orientation::Rot180 => "Upside down",
            Orientation::Rot270 => "Rotated 90° counter-clockwise",
            Orientation::Mirror0 => "Mirrored",
            Orientation::Mirror90 => "Mirrored, rotated 90° clockwise",
            Orientation::Mirror180 => "Mirrored, upside down",
            Orientation::Mirror270 => "Mirrored, rotated 90° counter-clockwise",
        }
    }

    pub fn mirrored(self) -> bool {
        matches!(self, Orientation::Mirror0 | Orientation::Mirror90 | Orientation::Mirror180 | Orientation::Mirror270)
    }

    /// Landscape (6 wide, 4 high on screen) or portrait.
    pub fn landscape(self) -> bool {
        matches!(self, Orientation::Rot0 | Orientation::Rot180 | Orientation::Mirror0 | Orientation::Mirror180)
    }

    /// Chart (column 0–5, row 0–3) → screen-aligned footprint cell (u right, v down).
    fn to_screen(self, c: f64, r: f64) -> (f64, f64) {
        match self {
            Orientation::Rot0 => (c, r),
            Orientation::Rot180 => (5.0 - c, 3.0 - r),
            Orientation::Rot90 => (3.0 - r, c),
            Orientation::Rot270 => (r, 5.0 - c),
            Orientation::Mirror0 => (5.0 - c, r),
            Orientation::Mirror180 => (c, 3.0 - r),
            Orientation::Mirror90 => (r, c),
            Orientation::Mirror270 => (3.0 - r, 5.0 - c),
        }
    }
}

/// Where the chart is: a homography from chart coordinates (column 0–5, row 0–3, dark
/// skin at (0, 0), black at (5, 3); patch centres on integers) to image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(with = "mat_rows")]
    pub h: Mat3,
    pub orientation: Orientation,
}

impl Placement {
    pub fn project(&self, c: f64, r: f64) -> [f64; 2] {
        project(&self.h, c, r)
    }

    /// Patch centres in chart order.
    pub fn centres(&self) -> [[f64; 2]; 24] {
        std::array::from_fn(|i| self.project((i % 6) as f64, (i / 6) as f64))
    }

    /// Distance between neighbouring patch centres, pixels (geometric mean of the two
    /// directions at the chart centre).
    pub fn pitch(&self) -> f64 {
        let c = self.project(2.5, 1.5);
        let a = self.project(3.5, 1.5);
        let b = self.project(2.5, 2.5);
        (dist(c, a) * dist(c, b)).sqrt()
    }

    /// From the centres of the four corner patches the user clicked: dark skin (1),
    /// bluish green (6), black (24) and white (19), in that order. The orientation is
    /// implied by which corner is which.
    pub fn from_corners(p: [[f64; 2]; 4]) -> Option<Placement> {
        let src = [[0.0, 0.0], [5.0, 0.0], [5.0, 3.0], [0.0, 3.0]];
        let h = homography(&src, &p)?;
        // Orientation from where the chart's axes point on screen.
        let o = project(&h, 0.0, 0.0);
        let cx = project(&h, 1.0, 0.0);
        let ry = project(&h, 0.0, 1.0);
        let (ux, uy) = (cx[0] - o[0], cx[1] - o[1]);
        let (vx, vy) = (ry[0] - o[0], ry[1] - o[1]);
        let mirrored = ux * vy - uy * vx < 0.0;
        let orientation = match (ux.abs() >= uy.abs(), ux > 0.0, uy > 0.0, mirrored) {
            (true, true, _, false) => Orientation::Rot0,
            (true, false, _, false) => Orientation::Rot180,
            (false, _, true, false) => Orientation::Rot90,
            (false, _, false, false) => Orientation::Rot270,
            (true, false, _, true) => Orientation::Mirror0,
            (true, true, _, true) => Orientation::Mirror180,
            (false, _, true, true) => Orientation::Mirror90,
            (false, _, false, true) => Orientation::Mirror270,
        };
        Some(Placement { h, orientation })
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn project(h: &Mat3, x: f64, y: f64) -> [f64; 2] {
    let p = h * Vector3::new(x, y, 1.0);
    [p.x / p.z, p.y / p.z]
}

/// Least-squares homography (normalised DLT) mapping `src[i]` → `dst[i]`; ≥ 4 points.
pub fn homography(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Mat3> {
    let n = src.len();
    if n < 4 || dst.len() != n {
        return None;
    }
    let norm = |p: &[[f64; 2]]| -> Mat3 {
        let (mx, my) = (p.iter().map(|q| q[0]).sum::<f64>() / n as f64, p.iter().map(|q| q[1]).sum::<f64>() / n as f64);
        let d = p.iter().map(|q| (q[0] - mx).hypot(q[1] - my)).sum::<f64>() / n as f64;
        let s = if d > 1e-12 { std::f64::consts::SQRT_2 / d } else { 1.0 };
        Matrix3::new(s, 0.0, -s * mx, 0.0, s, -s * my, 0.0, 0.0, 1.0)
    };
    let (ts, td) = (norm(src), norm(dst));
    let mut a = DMatrix::<f64>::zeros(2 * n, 9);
    for i in 0..n {
        let s = ts * Vector3::new(src[i][0], src[i][1], 1.0);
        let d = td * Vector3::new(dst[i][0], dst[i][1], 1.0);
        let (x, y, u, v) = (s.x / s.z, s.y / s.z, d.x / d.z, d.y / d.z);
        let r0 = [-x, -y, -1.0, 0.0, 0.0, 0.0, u * x, u * y, u];
        let r1 = [0.0, 0.0, 0.0, -x, -y, -1.0, v * x, v * y, v];
        for k in 0..9 {
            a[(2 * i, k)] = r0[k];
            a[(2 * i + 1, k)] = r1[k];
        }
    }
    // Null vector of A: eigenvector of AᵀA with the smallest eigenvalue.
    let ata = a.transpose() * &a;
    let eig = nalgebra::SymmetricEigen::new(ata);
    let k = (0..9).min_by(|&i, &j| eig.eigenvalues[i].total_cmp(&eig.eigenvalues[j]))?;
    let h = eig.eigenvectors.column(k);
    let hn = Matrix3::new(h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]);
    let m = td.try_inverse()? * hn * ts;
    if !m.iter().all(|v| v.is_finite()) || m[(2, 2)].abs() < 1e-300 {
        return None;
    }
    Some(m / m[(2, 2)])
}

/// Single-channel image used only to find the chart.
struct Gray {
    w: usize,
    h: usize,
    v: Vec<f32>,
    /// Full-resolution pixels per detection pixel.
    scale: usize,
}

fn detection_image(img: &ChartImage, max_side: usize) -> Gray {
    let scale = img.width.max(img.height).div_ceil(max_side).max(1);
    let (w, h) = (img.width / scale, img.height / scale);
    let mut v = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0f32;
            for dy in 0..scale {
                for dx in 0..scale {
                    let p = img.pixel(x * scale + dx, y * scale + dy);
                    let m = p[0].max(p[1]).max(p[2]);
                    acc += if m.is_finite() { m.max(0.0) } else { 0.0 };
                }
            }
            v[y * w + x] = acc / (scale * scale) as f32;
        }
    }
    let mut s: Vec<f32> = v.iter().step_by(((w * h) / 200_000).max(1)).copied().collect();
    s.sort_by(f32::total_cmp);
    let q = |f: f64| s[((s.len() - 1) as f64 * f) as usize];
    let (lo, hi) = (q(0.01), q(0.995));
    let span = (hi - lo).max(1e-6);
    for p in &mut v {
        *p = ((*p - lo) / span).clamp(0.0, 1.0);
    }
    Gray { w, h, v, scale }
}

/// A square-ish blob: centre (detection pixels), size (√ of rectangle area).
#[derive(Debug, Clone, Copy)]
struct Cand {
    x: f64,
    y: f64,
    size: f64,
}

fn candidates(g: &Gray, win: usize, out: &mut Vec<Cand>) {
    let (w, h) = (g.w, g.h);
    // Integral image for the local mean.
    let mut ii = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f64;
        for x in 0..w {
            row += g.v[y * w + x] as f64;
            ii[(y + 1) * (w + 1) + x + 1] = ii[y * (w + 1) + x + 1] + row;
        }
    }
    let r = win / 2;
    let mut bin = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = ii[y1 * (w + 1) + x1] - ii[y0 * (w + 1) + x1] - ii[y1 * (w + 1) + x0] + ii[y0 * (w + 1) + x0];
            let mean = sum / ((y1 - y0) * (x1 - x0)) as f64;
            bin[y * w + x] = g.v[y * w + x] as f64 > mean + 0.02;
        }
    }
    // Opening with a 3×3 square.
    let morph = |src: &[bool], all: bool| -> Vec<bool> {
        let mut dst = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = all;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                        let v = xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h && src[yy as usize * w + xx as usize];
                        if all {
                            acc &= v;
                        } else {
                            acc |= v;
                        }
                    }
                }
                dst[y * w + x] = acc;
            }
        }
        dst
    };
    let bin = morph(&morph(&bin, true), false);
    // Connected blobs (4-connectivity).
    let max_area = w * h / 50;
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut pix: Vec<(i64, i64)> = Vec::new();
    for start in 0..w * h {
        if !bin[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        pix.clear();
        let mut too_big = false;
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            if pix.len() <= max_area {
                pix.push((x as i64, y as i64));
            } else {
                too_big = true;
            }
            let mut push = |q: usize| {
                if bin[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if x > 0 {
                push(p - 1);
            }
            if x + 1 < w {
                push(p + 1);
            }
            if y > 0 {
                push(p - w);
            }
            if y + 1 < h {
                push(p + w);
            }
        }
        if too_big || pix.len() < 30 {
            continue;
        }
        if let Some(c) = square_blob(&pix) {
            out.push(c);
        }
    }
}

/// Minimum-area rectangle of a blob (rotating calipers over its convex hull); a
/// candidate if square-ish and well filled.
fn square_blob(pix: &[(i64, i64)]) -> Option<Cand> {
    let area = pix.len() as f64;
    let mut pts: Vec<(i64, i64)> = pix.to_vec();
    pts.sort_unstable();
    pts.dedup();
    let hull = convex_hull(&pts);
    if hull.len() < 3 {
        return None;
    }
    let mut best: Option<(f64, f64, f64, f64, f64)> = None; // area, w, h, cx, cy
    for k in 0..hull.len() {
        let (a, b) = (hull[k], hull[(k + 1) % hull.len()]);
        let (dx, dy) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
        let len = dx.hypot(dy);
        if len < 1e-9 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &hull {
            let (x, y) = (p.0 as f64, p.1 as f64);
            let u = x * ux + y * uy;
            let v = -x * uy + y * ux;
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        // Pixel centres: the covered extent is one pixel more.
        let (rw, rh) = (u1 - u0 + 1.0, v1 - v0 + 1.0);
        if best.is_none_or(|b| rw * rh < b.0) {
            let (cu, cv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            best = Some((rw * rh, rw, rh, cu * ux - cv * uy, cu * uy + cv * ux));
        }
    }
    let (ra, rw, rh, cx, cy) = best?;
    if rw.min(rh) < 5.0 || rw.max(rh) / rw.min(rh) > 1.6 || area / ra < 0.75 {
        return None;
    }
    Some(Cand { x: cx, y: cy, size: ra.sqrt() })
}

/// Andrew's monotone chain; `pts` sorted and unique.
fn convex_hull(pts: &[(i64, i64)]) -> Vec<(i64, i64)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let cross = |o: (i64, i64), a: (i64, i64), b: (i64, i64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(i64, i64)> = Vec::new();
    for &p in pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(i64, i64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Detected patch lattice.
#[derive(Debug, Clone)]
pub struct Lattice {
    /// Lattice (i, j) → full-resolution image pixels.
    pub h: Mat3,
    /// Members: integer lattice coordinates and their blob centres (full-res pixels).
    pub members: Vec<((i32, i32), [f64; 2])>,
    /// Median blob size, full-res pixels.
    pub patch_px: f64,
    /// Candidates found in total (diagnostics).
    pub candidates: usize,
}

type Keys = std::collections::HashMap<(i32, i32), usize>;

fn connected(keys: &Keys, seed: (i32, i32)) -> Vec<usize> {
    if !keys.contains_key(&seed) {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::from([seed]);
    let mut stack = vec![seed];
    while let Some((a, b)) = stack.pop() {
        for da in -1..=1 {
            for db in -1..=1 {
                let t = (a + da, b + db);
                if keys.contains_key(&t) && seen.insert(t) {
                    stack.push(t);
                }
            }
        }
    }
    seen.iter().map(|t| keys[t]).collect()
}

/// Find the patch lattice, or None if no grid of at least 6 similar squares exists.
pub fn find_lattice(img: &ChartImage) -> Option<Lattice> {
    let g = detection_image(img, 1600);
    let side = g.w.max(g.h) as f64;
    let mut all = Vec::new();
    for f in [0.01, 0.02, 0.04, 0.08] {
        candidates(&g, ((side * f) as usize).max(9), &mut all);
    }
    // Merge duplicates across scales, largest first.
    all.sort_by(|a, b| b.size.total_cmp(&a.size));
    let mut c: Vec<Cand> = Vec::new();
    for a in all {
        if c.iter().all(|k| (a.x - k.x).hypot(a.y - k.y) > 0.5 * a.size.min(k.size)) {
            c.push(a);
        }
    }
    let ncand = c.len();
    if c.len() < 6 {
        return None;
    }
    let pts: Vec<[f64; 2]> = c.iter().map(|k| [k.x, k.y]).collect();
    let size: Vec<f64> = c.iter().map(|k| k.size).collect();

    // Seed lattices.
    let mut best: Option<Vec<(usize, (i32, i32))>> = None;
    for s0 in 0..c.len() {
        let sim: Vec<bool> = size.iter().map(|s| (s / size[s0]).ln().abs() < 0.35).collect();
        let d: Vec<f64> = pts.iter().map(|p| dist(*p, pts[s0])).collect();
        let mut nb: Vec<usize> =
            (0..c.len()).filter(|&k| sim[k] && d[k] > 0.8 * size[s0] && d[k] < 2.2 * size[s0]).collect();
        if nb.len() < 2 {
            continue;
        }
        nb.sort_by(|&a, &b| d[a].total_cmp(&d[b]));
        let u = [pts[nb[0]][0] - pts[s0][0], pts[nb[0]][1] - pts[s0][1]];
        let v = nb[1..].iter().map(|&k| [pts[k][0] - pts[s0][0], pts[k][1] - pts[s0][1]]).find(|w| {
            (u[0] * w[0] + u[1] * w[1]).abs() / (u[0].hypot(u[1]) * w[0].hypot(w[1])) < 0.35
        });
        let Some(v) = v else { continue };
        let det = u[0] * v[1] - u[1] * v[0];
        if det.abs() < 1e-9 {
            continue;
        }
        let mut keys: Keys = Keys::new();
        let mut errs: Vec<f64> = vec![f64::MAX; c.len()];
        for k in 0..c.len() {
            if !sim[k] {
                continue;
            }
            let (dx, dy) = (pts[k][0] - pts[s0][0], pts[k][1] - pts[s0][1]);
            let gi = (dx * v[1] - dy * v[0]) / det;
            let gj = (u[0] * dy - u[1] * dx) / det;
            let (ri, rj) = (gi.round(), gj.round());
            let (ex, ey) = ((gi - ri) * u[0] + (gj - rj) * v[0], (gi - ri) * u[1] + (gj - rj) * v[1]);
            let e = ex.hypot(ey);
            if e < 0.25 * size[s0] && ri.abs() <= 7.0 && rj.abs() <= 7.0 {
                let t = (ri as i32, rj as i32);
                errs[k] = e;
                match keys.get(&t) {
                    Some(&o) if errs[o] <= e => {}
                    _ => {
                        keys.insert(t, k);
                    }
                }
            }
        }
        let idx = connected(&keys, (0, 0));
        if best.as_ref().is_none_or(|b| idx.len() > b.len()) {
            let inv: std::collections::HashMap<usize, (i32, i32)> = keys.iter().map(|(t, &k)| (k, *t)).collect();
            best = Some(idx.iter().map(|&k| (k, inv[&k])).collect());
        }
    }
    let mut members = best?;
    if members.len() < 6 {
        return None;
    }
    // Refine with a homography, adding members.
    let mut h = None;
    for _ in 0..3 {
        let src: Vec<[f64; 2]> = members.iter().map(|(_, t)| [t.0 as f64, t.1 as f64]).collect();
        let dst: Vec<[f64; 2]> = members.iter().map(|(k, _)| pts[*k]).collect();
        let Some(hh) = homography(&src, &dst) else { break };
        h = Some(hh);
        let Some(hinv) = hh.try_inverse() else { break };
        let mut sizes: Vec<f64> = members.iter().map(|(k, _)| size[*k]).collect();
        sizes.sort_by(f64::total_cmp);
        let med = sizes[sizes.len() / 2];
        let mut keys: Keys = Keys::new();
        let mut errs = vec![f64::MAX; c.len()];
        for k in 0..c.len() {
            let gp = project(&hinv, pts[k][0], pts[k][1]);
            let (ri, rj) = (gp[0].round(), gp[1].round());
            if ri.abs() > 12.0 || rj.abs() > 12.0 {
                continue;
            }
            let back = project(&hh, ri, rj);
            let e = dist(back, pts[k]);
            if e < 0.3 * med && (size[k] / med).ln().abs() < 0.4 {
                let t = (ri as i32, rj as i32);
                errs[k] = e;
                match keys.get(&t) {
                    Some(&o) if errs[o] <= e => {}
                    _ => {
                        keys.insert(t, k);
                    }
                }
            }
        }
        let cur: std::collections::HashSet<(i32, i32)> = members.iter().map(|m| m.1).collect();
        let inv: std::collections::HashMap<usize, (i32, i32)> = keys.iter().map(|(t, &k)| (k, *t)).collect();
        let blob = keys.keys().filter(|t| cur.contains(t)).map(|t| connected(&keys, *t)).max_by_key(|b| b.len());
        let Some(blob) = blob else { break };
        members = blob.iter().map(|&k| (k, inv[&k])).collect();
    }
    let src: Vec<[f64; 2]> = members.iter().map(|(_, t)| [t.0 as f64, t.1 as f64]).collect();
    let dst: Vec<[f64; 2]> = members.iter().map(|(k, _)| pts[*k]).collect();
    let hd = homography(&src, &dst).or(h)?;
    // Detection pixels → full resolution: x_full = s·x + (s − 1)/2.
    let s = g.scale as f64;
    let up = Matrix3::new(s, 0.0, (s - 1.0) / 2.0, 0.0, s, (s - 1.0) / 2.0, 0.0, 0.0, 1.0);
    let mut sizes: Vec<f64> = members.iter().map(|(k, _)| size[*k]).collect();
    sizes.sort_by(f64::total_cmp);
    Some(Lattice {
        h: up * hd,
        members: members.iter().map(|(k, t)| (*t, [pts[*k][0] * s + (s - 1.0) / 2.0, pts[*k][1] * s + (s - 1.0) / 2.0])).collect(),
        patch_px: sizes[sizes.len() / 2] * s,
        candidates: ncand,
    })
}

/// Every way the 6 × 4 chart can sit on the lattice: each orientation (mirrored ones
/// only if `mirrors`) at each offset that covers all detected members.
pub fn placements(l: &Lattice, mirrors: bool) -> Vec<Placement> {
    // Screen alignment of the lattice axes at the lattice centre.
    let n = l.members.len() as f64;
    let (mi, mj) = (
        l.members.iter().map(|m| m.0 .0 as f64).sum::<f64>() / n,
        l.members.iter().map(|m| m.0 .1 as f64).sum::<f64>() / n,
    );
    let p0 = project(&l.h, mi, mj);
    let pi = project(&l.h, mi + 1.0, mj);
    let pj = project(&l.h, mi, mj + 1.0);
    let (di, dj) = ([pi[0] - p0[0], pi[1] - p0[1]], [pj[0] - p0[0], pj[1] - p0[1]]);
    let swap = dj[0].abs() > di[0].abs();
    let (ax, ay) = if swap { (dj, di) } else { (di, dj) };
    let sx = if ax[0] > 0.0 { 1 } else { -1 };
    let sy = if ay[1] > 0.0 { 1 } else { -1 };
    // Lattice (i, j) → screen (u, v) and back.
    let to_uv = |t: (i32, i32)| -> (i32, i32) {
        let (a, b) = if swap { (t.1, t.0) } else { (t.0, t.1) };
        (sx * a, sy * b)
    };
    let uv: Vec<(i32, i32)> = l.members.iter().map(|m| to_uv(m.0)).collect();
    let (u0, u1) = (uv.iter().map(|p| p.0).min().unwrap_or(0), uv.iter().map(|p| p.0).max().unwrap_or(0));
    let (v0, v1) = (uv.iter().map(|p| p.1).min().unwrap_or(0), uv.iter().map(|p| p.1).max().unwrap_or(0));
    let mut out = Vec::new();
    for o in Orientation::ALL {
        if o.mirrored() && !mirrors {
            continue;
        }
        let (w, hh) = if o.landscape() { (6, 4) } else { (4, 6) };
        if u1 - u0 + 1 > w || v1 - v0 + 1 > hh {
            continue;
        }
        for ou in (u1 - w + 1)..=u0 {
            for ov in (v1 - hh + 1)..=v0 {
                // chart (c, r) → screen (u, v) = to_screen + offset → lattice (i, j).
                let mut src = Vec::new();
                let mut dst = Vec::new();
                for (c, r) in [(0.0, 0.0), (5.0, 0.0), (5.0, 3.0), (0.0, 3.0)] {
                    let (su, sv) = o.to_screen(c, r);
                    let (u, v) = (su + ou as f64, sv + ov as f64);
                    let (a, b) = (sx as f64 * u, sy as f64 * v);
                    let (i, j) = if swap { (b, a) } else { (a, b) };
                    src.push([c, r]);
                    dst.push(project(&l.h, i, j));
                }
                if let Some(h) = homography(&src, &dst) {
                    out.push(Placement { h, orientation: o });
                }
            }
        }
    }
    out
}

/// One patch as sampled.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PatchSample {
    /// Trimmed mean of the sample area, as stored in the file.
    pub mean: [f64; 3],
    /// Standard deviation over the kept pixels (noise / non-uniformity).
    pub std: [f64; 3],
    /// Fraction of sampled pixels at the bottom / top clip level, per channel.
    pub low_clip: [f64; 3],
    pub high_clip: [f64; 3],
    pub n: usize,
}

/// Clip levels of an image, per channel: values at or below `low` / at or above `high`
/// are clipped. Integer files clip at their first and last code; any file also clips
/// where a channel piles up at its largest value (a flat top, e.g. sensor clip).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ClipLevels {
    pub low: [f64; 3],
    pub high: [f64; 3],
}

pub fn clip_levels(img: &ChartImage) -> ClipLevels {
    let half_code = if img.info.max_code > 0 { 0.5 / img.info.max_code as f64 } else { 0.0 };
    let mut low = [f64::NEG_INFINITY; 3];
    let mut high = [f64::INFINITY; 3];
    if !img.info.float {
        low = [half_code; 3];
        high = [1.0 - half_code; 3];
    }
    let n = img.width * img.height;
    for ch in 0..3 {
        let mut mx = f32::NEG_INFINITY;
        for p in img.data.as_chunks::<3>().0 {
            if p[ch].is_finite() {
                mx = mx.max(p[ch]);
            }
        }
        if mx <= 0.0 || !mx.is_finite() {
            continue;
        }
        let tol = 1e-4 * mx as f64 + half_code;
        let at = img.data.as_chunks::<3>().0.iter().filter(|p| p[ch] as f64 >= mx as f64 - tol).count();
        if at as f64 > 5e-4 * n as f64 {
            high[ch] = high[ch].min(mx as f64 - tol);
        }
    }
    ClipLevels { low, high }
}

/// Sample patch (c, r) of `p`: a square `frac` of the patch pitch wide around the
/// centre, every pixel (up to 48 × 48 points), trimmed to the 70 % closest to the median.
pub fn sample_patch(img: &ChartImage, p: &Placement, c: usize, r: usize, frac: f64, clip: &ClipLevels) -> PatchSample {
    let pitch = p.pitch();
    let n = ((pitch * frac).round() as usize).clamp(3, 48);
    let mut vals: Vec<[f64; 3]> = Vec::with_capacity(n * n);
    for a in 0..n {
        for b in 0..n {
            let du = (a as f64 + 0.5) / n as f64 - 0.5;
            let dv = (b as f64 + 0.5) / n as f64 - 0.5;
            let q = p.project(c as f64 + du * frac, r as f64 + dv * frac);
            let (x, y) = (q[0].round(), q[1].round());
            if x < 0.0 || y < 0.0 || x >= img.width as f64 || y >= img.height as f64 {
                continue;
            }
            let px = img.pixel(x as usize, y as usize);
            if px.iter().all(|v| v.is_finite()) {
                vals.push([px[0] as f64, px[1] as f64, px[2] as f64]);
            }
        }
    }
    let total = vals.len();
    if total == 0 {
        return PatchSample { mean: [f64::NAN; 3], std: [f64::NAN; 3], low_clip: [1.0; 3], high_clip: [0.0; 3], n: 0 };
    }
    let mut low_clip = [0.0; 3];
    let mut high_clip = [0.0; 3];
    for v in &vals {
        for k in 0..3 {
            if v[k] <= clip.low[k] {
                low_clip[k] += 1.0;
            }
            if v[k] >= clip.high[k] {
                high_clip[k] += 1.0;
            }
        }
    }
    for k in 0..3 {
        low_clip[k] /= total as f64;
        high_clip[k] /= total as f64;
    }
    let median = |k: usize| {
        let mut s: Vec<f64> = vals.iter().map(|v| v[k]).collect();
        s.sort_by(f64::total_cmp);
        s[s.len() / 2]
    };
    let med = [median(0), median(1), median(2)];
    let mut dev: Vec<(f64, usize)> =
        vals.iter().enumerate().map(|(i, v)| ((v[0] - med[0]).abs() + (v[1] - med[1]).abs() + (v[2] - med[2]).abs(), i)).collect();
    dev.sort_by(|a, b| a.0.total_cmp(&b.0));
    let keep = ((total as f64 * 0.7).ceil() as usize).max(1);
    let kept: Vec<[f64; 3]> = dev[..keep].iter().map(|(_, i)| vals[*i]).collect();
    let mean: [f64; 3] = std::array::from_fn(|k| kept.iter().map(|v| v[k]).sum::<f64>() / keep as f64);
    let std: [f64; 3] = std::array::from_fn(|k| (kept.iter().map(|v| (v[k] - mean[k]).powi(2)).sum::<f64>() / keep as f64).sqrt());
    PatchSample { mean, std, low_clip, high_clip, n: total }
}

/// Serde for a Mat3 as row-major rows.
mod mat_rows {
    use crate::math::Mat3;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(m: &Mat3, s: S) -> Result<S::Ok, S::Error> {
        crate::math::ser::mat(m, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Mat3, D::Error> {
        let r: [[f64; 3]; 3] = Deserialize::deserialize(d)?;
        Ok(Mat3::from_fn(|i, k| r[i][k]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_recovers_a_known_one() {
        let h = Matrix3::new(120.0, 8.0, 300.0, -6.0, 110.0, 200.0, 0.01, -0.02, 1.0);
        let src: Vec<[f64; 2]> = (0..24).map(|i| [(i % 6) as f64, (i / 6) as f64]).collect();
        let dst: Vec<[f64; 2]> = src.iter().map(|p| project(&h, p[0], p[1])).collect();
        let got = homography(&src, &dst).unwrap();
        assert!((got - h).abs().max() < 1e-6, "{got}");
    }

    #[test]
    fn corners_give_the_orientation() {
        // Upside down: dark skin bottom right, bluish green bottom left.
        let p = Placement::from_corners([[600.0, 400.0], [100.0, 400.0], [100.0, 100.0], [600.0, 100.0]]).unwrap();
        assert_eq!(p.orientation, Orientation::Rot180);
        let c = p.centres();
        assert!((c[23][0] - 100.0).abs() < 1e-6 && (c[23][1] - 100.0).abs() < 1e-6);
        let up = Placement::from_corners([[100.0, 100.0], [600.0, 100.0], [600.0, 400.0], [100.0, 400.0]]).unwrap();
        assert_eq!(up.orientation, Orientation::Rot0);
        let cw = Placement::from_corners([[400.0, 100.0], [400.0, 600.0], [100.0, 600.0], [100.0, 100.0]]).unwrap();
        assert_eq!(cw.orientation, Orientation::Rot90);
        let mir = Placement::from_corners([[600.0, 100.0], [100.0, 100.0], [100.0, 400.0], [600.0, 400.0]]).unwrap();
        assert_eq!(mir.orientation, Orientation::Mirror0);
    }

    /// Every orientation's chart→screen mapping is a symmetry of the footprint.
    #[test]
    fn orientations_cover_the_footprint() {
        for o in Orientation::ALL {
            let (w, h) = if o.landscape() { (6.0, 4.0) } else { (4.0, 6.0) };
            let mut cells = std::collections::HashSet::new();
            for i in 0..24 {
                let (u, v) = o.to_screen((i % 6) as f64, (i / 6) as f64);
                assert!(u >= 0.0 && v >= 0.0 && u < w && v < h, "{o:?}");
                cells.insert((u as i32, v as i32));
            }
            assert_eq!(cells.len(), 24);
        }
    }
}
