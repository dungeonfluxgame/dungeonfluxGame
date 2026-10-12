//! Where a cut-out figure actually is inside its picture. The pictures share a 2:3 canvas, but a
//! wolf fills far less of it than a hero, so figures are scaled and planted by their visible
//! bounding box (alpha above a threshold), not by the canvas.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleBounds {
    /// Fractions of the picture: 0 is the top or left edge, 1 the bottom or right edge.
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl VisibleBounds {
    /// What a typical full-length hero cut-out measures, used until the real bounds are read.
    pub const TYPICAL: Self = Self {
        top: 0.04,
        bottom: 0.955,
        left: 0.15,
        right: 0.85,
    };

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// Horizontal centre of the visible part; the feet are planted under it.
    pub fn centre(&self) -> f32 {
        (self.left + self.right) * 0.5
    }
}

/// Bounds of the pixels whose alpha exceeds `alpha_floor` in straight or premultiplied RGBA.
/// `None` for an empty or malformed picture. A few stray pixels do not count: a row or column
/// needs at least `min_pixels` of them.
pub fn visible_bounds(
    rgba: &[u8],
    width: u32,
    height: u32,
    alpha_floor: u8,
    min_pixels: u32,
) -> Option<VisibleBounds> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let mut rows = vec![0u32; h];
    let mut columns = vec![0u32; w];
    for (y, row) in rgba.chunks_exact(w * 4).take(h).enumerate() {
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            if pixel[3] > alpha_floor {
                rows[y] += 1;
                columns[x] += 1;
            }
        }
    }
    let first = |counts: &[u32]| counts.iter().position(|count| *count >= min_pixels);
    let last = |counts: &[u32]| counts.iter().rposition(|count| *count >= min_pixels);
    let (top, bottom) = (first(&rows)?, last(&rows)?);
    let (left, right) = (first(&columns)?, last(&columns)?);
    Some(VisibleBounds {
        top: top as f32 / h as f32,
        bottom: (bottom + 1) as f32 / h as f32,
        left: left as f32 / w as f32,
        right: (right + 1) as f32 / w as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32, filled: impl Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[10, 10, 10, if filled(x, y) { 255 } else { 0 }]);
            }
        }
        data
    }

    #[test]
    fn bounds_follow_the_opaque_pixels_not_the_canvas() {
        let data = picture(20, 30, |x, y| (5..15).contains(&x) && (10..28).contains(&y));
        let bounds = visible_bounds(&data, 20, 30, 16, 1).unwrap();
        assert_eq!((bounds.left, bounds.right), (0.25, 0.75));
        assert!((bounds.top - 10.0 / 30.0).abs() < 1e-6);
        assert!((bounds.bottom - 28.0 / 30.0).abs() < 1e-6);
        assert!((bounds.centre() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_wide_low_creature_has_a_small_visible_height() {
        let wolf = picture(30, 45, |x, y| (2..28).contains(&x) && (25..43).contains(&y));
        let hero = picture(30, 45, |x, y| (10..20).contains(&x) && (2..43).contains(&y));
        let wolf = visible_bounds(&wolf, 30, 45, 16, 1).unwrap();
        let hero = visible_bounds(&hero, 30, 45, 16, 1).unwrap();
        assert!(wolf.height() < hero.height() * 0.6);
    }

    #[test]
    fn stray_pixels_are_ignored_and_empty_pictures_have_no_bounds() {
        let mut data = picture(20, 20, |x, y| (5..15).contains(&x) && (5..15).contains(&y));
        data[3] = 255; // a stray opaque pixel in the top-left corner
        let bounds = visible_bounds(&data, 20, 20, 16, 2).unwrap();
        assert_eq!(bounds.top, 0.25);
        assert!(visible_bounds(&picture(4, 4, |_, _| false), 4, 4, 16, 1).is_none());
        assert!(visible_bounds(&[0u8; 3], 4, 4, 16, 1).is_none());
    }
}
