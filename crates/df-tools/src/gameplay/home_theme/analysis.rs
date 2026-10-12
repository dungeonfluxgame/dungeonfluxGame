//! Measures one painting's pixels for the home screen: a coarse grid of brightness and busyness,
//! the focal region the blocks must leave clear, the mood colours and grade, and how well a set of
//! block boxes fits the painting at the reference viewports. Every function is pure over the
//! decoded image, so one set of painting bytes always yields one set of measures.
use image::RgbImage;

/// The coarse grid a painting is measured on.
pub(crate) const COLUMNS: usize = 16;
pub(crate) const ROWS: usize = 9;
/// The painting is averaged down to this size before any measure is taken.
const SAMPLE_WIDTH: u32 = 320;
const SAMPLE_HEIGHT: u32 = 180;
/// Samples this many pixels from an edge form the border the vignette compares with the centre.
const BORDER: u32 = 12;
/// Cells at least this share of the peak score belong to the focal cluster.
const CLUSTER_SHARE: f64 = 0.85;
/// The stylesheet's vertical background position (`center 40%`).
const POSITION_Y: f64 = 0.4;
/// Sample points across and down each block when judging the ground beneath it.
const PROBE_COLUMNS: usize = 12;
const PROBE_ROWS: usize = 8;
/// The reference viewports the home screen is laid out for: 1080p, 720p and the desktop pane.
pub(crate) const VIEWPORTS: [(u32, u32); 3] = [(1920, 1080), (1280, 720), (800, 577)];
/// How much covering each block's ground costs, in title, room code and party order. The room
/// code must stay readable from across a room, so its ground is worth the most.
pub(crate) const BLOCK_WEIGHTS: [f64; 3] = [2.0, 3.0, 1.5];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnalysisError {
    /// The painting has no pixels to measure.
    Empty,
    /// No part of the painting is lit, so there is no focal region to protect.
    Flat,
}

/// The painting's measures, in the units the home theme needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Measures {
    /// Painting width and height in pixels, which the cover mapping needs.
    pub(crate) size: [u32; 2],
    /// Left, top, right and bottom of the focal region, as fractions of the painting.
    pub(crate) focal: [f64; 4],
    pub(crate) brightness: Grid,
    pub(crate) busyness: Grid,
    pub(crate) mood: MoodMeasures,
    pub(crate) mean_luminance: f64,
    /// Mean of red minus blue, in 0 to 1 units: above zero is a warm painting.
    pub(crate) warm_balance: f64,
}

/// The mood as permille (1000 is unchanged) and colours as `#rrggbb`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MoodMeasures {
    pub(crate) accent: String,
    pub(crate) tint: String,
    pub(crate) vignette_permille: u32,
    pub(crate) brightness_permille: u32,
    pub(crate) contrast_permille: u32,
    pub(crate) saturation_permille: u32,
}

/// A value per grid cell, row-major, from 0 at the darkest or calmest to 1 at the brightest or
/// busiest after normalisation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Grid([[f64; COLUMNS]; ROWS]);

impl Grid {
    /// The mean of `value` over each cell of a `width` by `height` sample image.
    fn from_pixels(pixels: &Pixels, value: impl Fn(&Pixels, u32, u32) -> f64) -> Self {
        let mut cells = [[0.0; COLUMNS]; ROWS];
        for (row, cells_row) in cells.iter_mut().enumerate() {
            for (column, cell) in cells_row.iter_mut().enumerate() {
                let (x0, x1) = cell_span(column, COLUMNS, pixels.width);
                let (y0, y1) = cell_span(row, ROWS, pixels.height);
                let mut total = 0.0;
                let mut count = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        total += value(pixels, x, y);
                        count += 1;
                    }
                }
                *cell = if count == 0 {
                    0.0
                } else {
                    total / f64::from(count)
                };
            }
        }
        Self(cells)
    }

    fn at(&self, row: usize, column: usize) -> f64 {
        self.0[row.min(ROWS - 1)][column.min(COLUMNS - 1)]
    }

    fn cells(&self) -> impl Iterator<Item = f64> + '_ {
        self.0.iter().flatten().copied()
    }

    fn peak(&self) -> f64 {
        self.cells().fold(0.0, f64::max)
    }

    /// The value at the given percentile of the cells, nearest rank.
    fn percentile(&self, share: f64) -> f64 {
        percentile(&self.cells().collect::<Vec<f64>>(), share)
    }
}

fn cell_span(index: usize, count: usize, length: u32) -> (u32, u32) {
    let count = count as u32;
    let index = index as u32;
    (index * length / count, (index + 1) * length / count)
}

/// The painting averaged down to the sample size, as linear 0 to 1 colour.
struct Pixels {
    width: u32,
    height: u32,
    rgb: Vec<[f64; 3]>,
}

impl Pixels {
    fn sample(image: &RgbImage) -> Self {
        let small = image::imageops::thumbnail(image, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        let rgb = small
            .pixels()
            .map(|pixel| [pixel[0], pixel[1], pixel[2]].map(|channel| f64::from(channel) / 255.0))
            .collect();
        Self {
            width: small.width(),
            height: small.height(),
            rgb,
        }
    }

    fn at(&self, x: u32, y: u32) -> [f64; 3] {
        let index = (y * self.width + x) as usize;
        self.rgb.get(index).copied().unwrap_or([0.0; 3])
    }

    fn luminance(&self, x: u32, y: u32) -> f64 {
        luminance(self.at(x, y))
    }

    /// The brightness gradient magnitude at a sample, from neighbours clamped at the edges.
    fn busyness(&self, x: u32, y: u32) -> f64 {
        let left = self.luminance(x.saturating_sub(1), y);
        let right = self.luminance((x + 1).min(self.width - 1), y);
        let up = self.luminance(x, y.saturating_sub(1));
        let down = self.luminance(x, (y + 1).min(self.height - 1));
        ((right - left).powi(2) + (down - up).powi(2)).sqrt()
    }

    fn is_border(&self, x: u32, y: u32) -> bool {
        x < BORDER || y < BORDER || x + BORDER >= self.width || y + BORDER >= self.height
    }

    fn coordinates(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.height).flat_map(move |y| (0..self.width).map(move |x| (x, y)))
    }
}

fn luminance([red, green, blue]: [f64; 3]) -> f64 {
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// Measures a decoded painting. Fails only for an empty or an unlit painting.
pub(crate) fn measure(image: &RgbImage) -> Result<Measures, AnalysisError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(AnalysisError::Empty);
    }
    let pixels = Pixels::sample(image);
    if pixels.rgb.is_empty() {
        return Err(AnalysisError::Empty);
    }
    let brightness = Grid::from_pixels(&pixels, |pixels, x, y| pixels.luminance(x, y));
    let busyness = Grid::from_pixels(&pixels, |pixels, x, y| pixels.busyness(x, y));
    let peak = brightness.peak();
    if peak <= 0.0 {
        return Err(AnalysisError::Flat);
    }
    let busy_scale = busyness.percentile(0.995).max(f64::EPSILON);
    let mut focal_score = [[0.0; COLUMNS]; ROWS];
    for (row, scores) in focal_score.iter_mut().enumerate() {
        for (column, score) in scores.iter_mut().enumerate() {
            let light = brightness.at(row, column) / peak;
            let busy = (busyness.at(row, column) / busy_scale).min(1.0);
            *score = light * (0.5 + busy);
        }
    }
    let focal = focal_cluster(&focal_score);
    let mean_luminance = pixels
        .coordinates()
        .map(|(x, y)| pixels.luminance(x, y))
        .sum::<f64>()
        / pixels.rgb.len() as f64;
    let warm_balance = pixels
        .rgb
        .iter()
        .map(|[red, _, blue]| red - blue)
        .sum::<f64>()
        / pixels.rgb.len() as f64;
    Ok(Measures {
        size: [image.width(), image.height()],
        focal,
        brightness,
        busyness,
        mood: mood(&pixels),
        mean_luminance,
        warm_balance,
    })
}

/// The connected cells near the peak, as a box with half a cell of margin on every side.
fn focal_cluster(score: &[[f64; COLUMNS]; ROWS]) -> [f64; 4] {
    let mut peak = (0, 0);
    let mut peak_value = f64::MIN;
    for (row, scores) in score.iter().enumerate() {
        for (column, value) in scores.iter().enumerate() {
            if *value > peak_value {
                peak_value = *value;
                peak = (row, column);
            }
        }
    }
    let threshold = CLUSTER_SHARE * peak_value;
    let mut seen = [[false; COLUMNS]; ROWS];
    let mut frontier = vec![peak];
    seen[peak.0][peak.1] = true;
    while let Some((row, column)) = frontier.pop() {
        let neighbours = [
            (row.wrapping_sub(1), column),
            (row + 1, column),
            (row, column.wrapping_sub(1)),
            (row, column + 1),
        ];
        for (next_row, next_column) in neighbours {
            if next_row >= ROWS || next_column >= COLUMNS || seen[next_row][next_column] {
                continue;
            }
            if score[next_row][next_column] >= threshold {
                seen[next_row][next_column] = true;
                frontier.push((next_row, next_column));
            }
        }
    }
    let cells: Vec<(usize, usize)> = (0..ROWS)
        .flat_map(|row| (0..COLUMNS).map(move |column| (row, column)))
        .filter(|(row, column)| seen[*row][*column])
        .collect();
    let (top, bottom) = span(cells.iter().map(|cell| cell.0), ROWS);
    let (left, right) = span(cells.iter().map(|cell| cell.1), COLUMNS);
    [
        (left as f64 - 0.5).max(0.0) / COLUMNS as f64,
        (top as f64 - 0.5).max(0.0) / ROWS as f64,
        (right as f64 + 1.5).min(COLUMNS as f64) / COLUMNS as f64,
        (bottom as f64 + 1.5).min(ROWS as f64) / ROWS as f64,
    ]
}

/// The first and last index covered, as a half-open span of `(first, last + 1)`.
fn span(indices: impl Iterator<Item = usize>, limit: usize) -> (usize, usize) {
    let mut first = limit;
    let mut last = 0;
    for index in indices {
        first = first.min(index);
        last = last.max(index);
    }
    (first.min(limit), (last + 1).min(limit))
}

fn mood(pixels: &Pixels) -> MoodMeasures {
    let hsv: Vec<(f64, f64, f64)> = pixels.rgb.iter().map(|rgb| rgb_to_hsv(*rgb)).collect();
    let luminances: Vec<f64> = pixels
        .coordinates()
        .map(|(x, y)| pixels.luminance(x, y))
        .collect();
    let p50 = percentile(&luminances, 0.5);
    let p80 = percentile(&luminances, 0.8);
    let p95 = percentile(&luminances, 0.95);
    let mut lit_pixels = lit_indices(&luminances, &hsv, p80, true);
    if lit_pixels.len() < 20 {
        lit_pixels = lit_indices(&luminances, &hsv, p95, false);
    }
    // A histogram of hue rather than a mean, so a blue moon and amber lanterns do not average
    // into a colour that neither of them is.
    let mut histogram = [0.0f64; 24];
    for index in &lit_pixels {
        let (hue, saturation, _) = hsv.get(*index).copied().unwrap_or((0.0, 0.0, 0.0));
        let luminance = luminances.get(*index).copied().unwrap_or(0.0);
        let bin = ((hue * 24.0).floor() as usize).min(23);
        histogram[bin] += luminance * saturation * saturation;
    }
    let peak_bin = histogram
        .iter()
        .enumerate()
        .fold((0, f64::MIN), |best, (bin, weight)| {
            if *weight > best.1 {
                (bin, *weight)
            } else {
                best
            }
        })
        .0;
    let mut peak_saturations: Vec<f64> = lit_pixels
        .iter()
        .filter_map(|index| hsv.get(*index).copied())
        .filter(|(hue, _, _)| ((hue * 24.0).floor() as usize).min(23) == peak_bin)
        .map(|(_, saturation, _)| saturation)
        .collect();
    peak_saturations.sort_by(f64::total_cmp);
    let saturation = peak_saturations
        .get(peak_saturations.len() / 2)
        .copied()
        .unwrap_or(0.5);
    let hue = (peak_bin as f64 + 0.5) / 24.0;
    let accent = hsv_to_rgb(hue, saturation.clamp(0.35, 0.8), 0.92);

    let mut dark_total = [0.0; 3];
    let mut dark_count = 0.0f64;
    for (index, rgb) in pixels.rgb.iter().enumerate() {
        if luminances.get(index).is_some_and(|value| *value <= p50) {
            for (channel, value) in dark_total.iter_mut().zip(rgb) {
                *channel += value;
            }
            dark_count += 1.0;
        }
    }
    let dark_mean = dark_total.map(|channel| channel / dark_count.max(1.0));
    let (tint_hue, tint_saturation, _) = rgb_to_hsv(dark_mean);
    let tint = hsv_to_rgb(tint_hue, (tint_saturation * 1.2).clamp(0.15, 0.4), 0.12);

    let mut border_total = 0.0;
    let mut border_count = 0.0f64;
    let mut centre_total = 0.0;
    let mut centre_count = 0.0f64;
    for (x, y) in pixels.coordinates() {
        if pixels.is_border(x, y) {
            border_total += pixels.luminance(x, y);
            border_count += 1.0;
        } else {
            centre_total += pixels.luminance(x, y);
            centre_count += 1.0;
        }
    }
    let edge_ratio =
        (border_total / border_count.max(1.0)) / (centre_total / centre_count.max(1.0)).max(1e-3);
    let vignette = (0.4 + 0.3 * (edge_ratio - 0.5)).clamp(0.35, 0.75);

    let mean = luminances.iter().sum::<f64>() / luminances.len().max(1) as f64;
    let spread = (luminances
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / luminances.len().max(1) as f64)
        .sqrt();
    MoodMeasures {
        accent: hex(accent),
        tint: hex(tint),
        vignette_permille: permille(vignette),
        brightness_permille: permille((0.95 - 0.35 * mean).clamp(0.78, 0.92)),
        contrast_permille: permille((1.12 - 0.2 * spread).clamp(1.04, 1.14)),
        saturation_permille: 950,
    }
}

/// The samples at or above `threshold` in luminance; when `saturated`, only those with colour.
fn lit_indices(
    luminances: &[f64],
    hsv: &[(f64, f64, f64)],
    threshold: f64,
    saturated: bool,
) -> Vec<usize> {
    luminances
        .iter()
        .enumerate()
        .filter(|(index, value)| {
            let saturation = hsv.get(*index).map_or(0.0, |colour| colour.1);
            **value >= threshold && (!saturated || saturation > 0.2)
        })
        .map(|(index, _)| index)
        .collect()
}

fn percentile(values: &[f64], share: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len().saturating_sub(1)) as f64 * share).round() as usize;
    sorted.get(index).copied().unwrap_or(0.0)
}

fn permille(value: f64) -> u32 {
    (value * 1000.0).round().clamp(0.0, 1000.0) as u32
}

fn hex([red, green, blue]: [f64; 3]) -> String {
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(red),
        channel(green),
        channel(blue)
    )
}

/// Hue in 0 to 1, saturation and value in 0 to 1.
fn rgb_to_hsv([red, green, blue]: [f64; 3]) -> (f64, f64, f64) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let saturation = if max <= 0.0 { 0.0 } else { delta / max };
    let hue = if delta <= 0.0 {
        0.0
    } else if max == red {
        ((green - blue) / delta).rem_euclid(6.0) / 6.0
    } else if max == green {
        ((blue - red) / delta + 2.0) / 6.0
    } else {
        ((red - green) / delta + 4.0) / 6.0
    };
    (hue, saturation, max)
}

fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> [f64; 3] {
    let scaled = hue.rem_euclid(1.0) * 6.0;
    let sector = scaled.floor();
    let fraction = scaled - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    match sector as u32 % 6 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}

/// A painting-fraction box as viewport pixels, under `background-size: cover` at 40% vertically.
#[derive(Clone, Copy, Debug)]
struct Cover {
    offset_x: f64,
    offset_y: f64,
    shown_w: f64,
    shown_h: f64,
}

impl Cover {
    fn new(viewport: (u32, u32), size: [u32; 2]) -> Self {
        let (view_w, view_h) = (f64::from(viewport.0), f64::from(viewport.1));
        let (image_w, image_h) = (f64::from(size[0]), f64::from(size[1]));
        let scale = (view_w / image_w).max(view_h / image_h);
        let shown_w = image_w * scale;
        let shown_h = image_h * scale;
        Self {
            offset_x: (view_w - shown_w) * 0.5,
            offset_y: (view_h - shown_h) * POSITION_Y,
            shown_w,
            shown_h,
        }
    }

    fn to_viewport(self, fraction: [f64; 4]) -> [f64; 4] {
        [
            self.offset_x + fraction[0] * self.shown_w,
            self.offset_y + fraction[1] * self.shown_h,
            self.offset_x + fraction[2] * self.shown_w,
            self.offset_y + fraction[3] * self.shown_h,
        ]
    }

    /// The painting fraction under a viewport point, clamped to the painting.
    fn to_painting(self, x: f64, y: f64) -> (f64, f64) {
        (
            ((x - self.offset_x) / self.shown_w).clamp(0.0, 0.999),
            ((y - self.offset_y) / self.shown_h).clamp(0.0, 0.999),
        )
    }
}

/// How one arrangement's blocks sit on one painting at the reference viewports.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fit {
    /// Share of the focal region each viewport's blocks cover, in permille; zero is clear.
    pub(crate) focal_covered_permille: [u32; 3],
    /// The weighted darkness and calm of the ground under the blocks; lower is better.
    pub(crate) penalty: f64,
}

impl Fit {
    pub(crate) fn is_clear(&self) -> bool {
        self.focal_covered_permille
            .iter()
            .all(|covered| *covered == 0)
    }
}

/// Scores three block boxes (title, room code, party), each `[left, top, width, height]` in
/// permille of the viewport, against a painting's measures.
pub(crate) fn fit(boxes: [[u32; 4]; 3], measures: &Measures) -> Fit {
    let mut covered = [0; 3];
    let mut penalty = 0.0;
    for (viewport_index, viewport) in VIEWPORTS.into_iter().enumerate() {
        let cover = Cover::new(viewport, measures.size);
        let focal = cover.to_viewport(measures.focal);
        let focal_area = ((focal[2] - focal[0]) * (focal[3] - focal[1])).max(1.0);
        let view_area = f64::from(viewport.0) * f64::from(viewport.1);
        let mut covered_area = 0.0;
        for (block, weight) in boxes.iter().zip(BLOCK_WEIGHTS) {
            let rect = block_pixels(*block, viewport);
            let overlap_w = (rect[2].min(focal[2]) - rect[0].max(focal[0])).max(0.0);
            let overlap_h = (rect[3].min(focal[3]) - rect[1].max(focal[1])).max(0.0);
            covered_area += overlap_w * overlap_h;
            let (light, texture) = probe(rect, cover, measures);
            let area = (rect[2] - rect[0]) * (rect[3] - rect[1]) / view_area;
            penalty += weight * area * (light + texture);
        }
        // Any overlap counts, so a sliver that rounds to zero still reads as covering.
        covered[viewport_index] = if covered_area > 0.0 {
            permille(covered_area / focal_area).max(1)
        } else {
            0
        };
    }
    Fit {
        focal_covered_permille: covered,
        penalty: (penalty * 1000.0).round() / 1000.0,
    }
}

fn block_pixels(block: [u32; 4], viewport: (u32, u32)) -> [f64; 4] {
    let (view_w, view_h) = (f64::from(viewport.0), f64::from(viewport.1));
    let [left, top, width, height] = block.map(f64::from);
    [
        view_w * left / 1000.0,
        view_h * top / 1000.0,
        view_w * (left + width) / 1000.0,
        view_h * (top + height) / 1000.0,
    ]
}

/// The mean brightness and busyness of the painting under a block, sampled on a grid of points.
fn probe(rect: [f64; 4], cover: Cover, measures: &Measures) -> (f64, f64) {
    let mut light = 0.0;
    let mut texture = 0.0;
    let mut count = 0.0;
    for row in 0..PROBE_ROWS {
        let y = rect[1] + (rect[3] - rect[1]) * row as f64 / (PROBE_ROWS - 1) as f64;
        for column in 0..PROBE_COLUMNS {
            let x = rect[0] + (rect[2] - rect[0]) * column as f64 / (PROBE_COLUMNS - 1) as f64;
            let (fx, fy) = cover.to_painting(x, y);
            let grid_column = (fx * COLUMNS as f64) as usize;
            let grid_row = (fy * ROWS as f64) as usize;
            light += measures.brightness.at(grid_row, grid_column);
            texture += measures.busyness.at(grid_row, grid_column);
            count += 1.0;
        }
    }
    (light / count, texture / count)
}
