//! Measures one painting's pixels for the home screen: the mood colours and grade, the mean
//! brightness and the warm or cool balance the atmosphere follows. Every function is pure over the
//! decoded image, so one set of painting bytes always yields one set of measures.
use image::RgbImage;

/// The painting is averaged down to this size before any measure is taken.
const SAMPLE_WIDTH: u32 = 320;
const SAMPLE_HEIGHT: u32 = 180;
/// Samples this many pixels from an edge form the border the vignette compares with the centre.
const BORDER: u32 = 12;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnalysisError {
    /// The painting has no pixels to measure.
    Empty,
    /// No part of the painting is lit, so there is no light to take a mood from.
    Flat,
}

/// The painting's measures, in the units the home theme needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Measures {
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
    if mean_luminance <= 0.0 {
        return Err(AnalysisError::Flat);
    }
    Ok(Measures {
        mood: mood(&pixels),
        mean_luminance,
        warm_balance,
    })
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
