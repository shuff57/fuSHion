//! A tiny z-buffered triangle / line rasterizer.

/// Screen-space vertex: pixel x, pixel y (down), depth (smaller = closer).
pub type SVert = [f32; 3];

pub struct Target {
    pub w: usize,
    pub h: usize,
    pub color: Vec<[u8; 3]>,
    pub depth: Vec<f32>,
}

impl Target {
    pub fn new(w: usize, h: usize, bg: [u8; 3]) -> Self {
        Self { w, h, color: vec![bg; w * h], depth: vec![f32::INFINITY; w * h] }
    }

    /// Fill a triangle with a flat colour, depth-tested. Both windings are
    /// drawn (double-sided). Pixel centres are sampled at `+0.5`.
    #[inline]
    pub fn tri(&mut self, a: SVert, b: SVert, c: SVert, rgb: [u8; 3]) {
        let (w, h) = (self.w as f32, self.h as f32);
        let minx = a[0].min(b[0]).min(c[0]);
        let maxx = a[0].max(b[0]).max(c[0]);
        let miny = a[1].min(b[1]).min(c[1]);
        let maxy = a[1].max(b[1]).max(c[1]);
        // Pixel centres x + 0.5 inside [minx, maxx].
        let x0 = (minx - 0.5).ceil().max(0.0);
        let x1 = (maxx - 0.5).floor().min(w - 1.0);
        let y0 = (miny - 0.5).ceil().max(0.0);
        let y1 = (maxy - 0.5).floor().min(h - 1.0);
        if x0 > x1 || y0 > y1 || !(x0.is_finite() && y0.is_finite()) {
            return;
        }
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-12 {
            return;
        }
        let inv = 1.0 / area;
        // Edge functions e_i(p) = A_i·px + B_i·py + C_i, normalised by area
        // so they are barycentric weights (positive inside for both windings).
        let (ea, eb, ec) = (edge(b, c, inv), edge(c, a, inv), edge(a, b, inv));
        let (x0i, x1i, y0i, y1i) = (x0 as usize, x1 as usize, y0 as usize, y1 as usize);
        let px0 = x0 + 0.5;
        // Depth as a plane: z = za + zx·px + zy·py.
        let zx = ea.0 * a[2] + eb.0 * b[2] + ec.0 * c[2];
        let zy = ea.1 * a[2] + eb.1 * b[2] + ec.1 * c[2];
        let zc = ea.2 * a[2] + eb.2 * b[2] + ec.2 * c[2];
        const E: f32 = -1e-5;
        for y in y0i..=y1i {
            let py = y as f32 + 0.5;
            let mut w0 = ea.0 * px0 + ea.1 * py + ea.2;
            let mut w1 = eb.0 * px0 + eb.1 * py + eb.2;
            let mut w2 = ec.0 * px0 + ec.1 * py + ec.2;
            let mut z = zx * px0 + zy * py + zc;
            let row = y * self.w;
            let depth = &mut self.depth[row + x0i..=row + x1i];
            let color = &mut self.color[row + x0i..=row + x1i];
            for (d, col) in depth.iter_mut().zip(color.iter_mut()) {
                if w0 >= E && w1 >= E && w2 >= E && z < *d {
                    *d = z;
                    *col = rgb;
                }
                w0 += ea.0;
                w1 += eb.0;
                w2 += ec.0;
                z += zx;
            }
        }
    }

    /// Draw a 1-px line blended over the image with `alpha`, visible where
    /// it is no more than `bias` behind the stored depth.
    #[inline]
    pub fn line(&mut self, a: SVert, b: SVert, rgb: [u8; 3], alpha: f32, bias: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let steps = dx.abs().max(dy.abs()).ceil().max(1.0);
        if !steps.is_finite() || steps > 1e5 {
            return;
        }
        let n = steps as usize;
        let inv = 1.0 / steps;
        let (sx, sy, sz) = (dx * inv, dy * inv, (b[2] - a[2]) * inv);
        let (mut x, mut y, mut z) = (a[0], a[1], a[2]);
        let ia = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
        for _ in 0..=n {
            if x >= 0.0 && y >= 0.0 {
                let (xi, yi) = (x as usize, y as usize);
                if xi < self.w && yi < self.h {
                    let i = yi * self.w + xi;
                    if z <= self.depth[i] + bias {
                        let c = &mut self.color[i];
                        for k in 0..3 {
                            c[k] = ((c[k] as u32 * (256 - ia) + rgb[k] as u32 * ia) >> 8) as u8;
                        }
                    }
                }
            }
            x += sx;
            y += sy;
            z += sz;
        }
    }

    /// Fill an axis-aligned rectangle (no depth).
    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, rgb: [u8; 3]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.color[yy * self.w + xx] = rgb;
            }
        }
    }
}

/// Edge function through `p → q` as `(A, B, C)` scaled by `inv` (1/area).
#[inline]
fn edge(p: SVert, q: SVert, inv: f32) -> (f32, f32, f32) {
    // e(x, y) = (q.x - p.x)(y - p.y) - (q.y - p.y)(x - p.x)
    let a = -(q[1] - p[1]);
    let b = q[0] - p[0];
    let c = (q[1] - p[1]) * p[0] - (q[0] - p[0]) * p[1];
    (a * inv, b * inv, c * inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_fills_inside_only() {
        let mut t = Target::new(10, 10, [0; 3]);
        t.tri([1.0, 1.0, 5.0], [9.0, 1.0, 5.0], [1.0, 9.0, 5.0], [255, 0, 0]);
        assert_eq!(t.color[2 * 10 + 2], [255, 0, 0]);
        assert_eq!(t.color[8 * 10 + 8], [0, 0, 0]);
        assert_eq!(t.depth[2 * 10 + 2], 5.0);
        // Opposite winding draws too.
        let mut u = Target::new(10, 10, [0; 3]);
        u.tri([1.0, 1.0, 5.0], [1.0, 9.0, 5.0], [9.0, 1.0, 5.0], [255, 0, 0]);
        assert_eq!(t.color, u.color);
    }

    #[test]
    fn depth_test_keeps_nearest() {
        let mut t = Target::new(4, 4, [0; 3]);
        let big = |z| ([-1.0, -1.0, z], [9.0, -1.0, z], [-1.0, 9.0, z]);
        let (a, b, c) = big(1.0);
        t.tri(a, b, c, [1, 1, 1]);
        let (a, b, c) = big(2.0);
        t.tri(a, b, c, [2, 2, 2]);
        assert_eq!(t.color[0], [1, 1, 1]);
    }
}
