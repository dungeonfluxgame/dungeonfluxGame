//! Per-theme colour grade. The numbers and the tint maths are ported from the earlier build's
//! `theme_grades.mjs`; the same grade runs in the post pass and, for tests, on the CPU.

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Theme {
    Neutral,
    Crypt,
    FloodedHall,
    Forest,
    Harbor,
    Lava,
    Library,
    Mountain,
    Swamp,
    Tavern,
    Throne,
}

struct Base {
    exposure: f32,
    contrast: f32,
    saturation: f32,
    shadow: [f32; 3],
    highlight: [f32; 3],
    strength: f32,
}

impl Theme {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "neutral" => Self::Neutral,
            "crypt" => Self::Crypt,
            "flooded_hall" => Self::FloodedHall,
            "forest" => Self::Forest,
            "harbor" => Self::Harbor,
            "lava" => Self::Lava,
            "library" => Self::Library,
            "mountain" => Self::Mountain,
            "swamp" => Self::Swamp,
            "tavern" => Self::Tavern,
            "throne" => Self::Throne,
            _ => return None,
        })
    }

    fn base(self) -> Base {
        let (exposure, contrast, saturation, shadow, highlight, strength) = match self {
            Self::Neutral => (0.0, 1.0, 1.0, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0.0),
            Self::Crypt => (-0.28, 1.1, 0.76, [0.06, 0.13, 0.1], [0.18, 0.1, 0.03], 0.6),
            Self::FloodedHall => (-0.25, 1.08, 0.8, [0.02, 0.1, 0.14], [0.08, 0.12, 0.16], 0.7),
            Self::Forest => (-0.22, 1.08, 0.8, [0.05, 0.1, 0.06], [0.15, 0.11, 0.04], 0.7),
            Self::Harbor => (-0.3, 1.1, 0.76, [0.03, 0.07, 0.15], [0.16, 0.12, 0.04], 0.7),
            Self::Lava => (
                -0.26,
                1.12,
                0.78,
                [0.12, 0.04, 0.03],
                [0.18, 0.07, 0.025],
                0.7,
            ),
            Self::Library => (
                -0.24,
                1.08,
                0.78,
                [0.08, 0.05, 0.14],
                [0.14, 0.1, 0.18],
                0.7,
            ),
            Self::Mountain => (-0.2, 1.06, 0.7, [0.04, 0.1, 0.14], [0.12, 0.15, 0.16], 0.65),
            Self::Swamp => (
                -0.35,
                1.1,
                0.72,
                [0.08, 0.12, 0.03],
                [0.16, 0.12, 0.03],
                0.7,
            ),
            Self::Tavern => (
                -0.2,
                1.08,
                0.82,
                [0.03, 0.09, 0.08],
                [0.2, 0.13, 0.035],
                0.65,
            ),
            Self::Throne => (
                -0.24,
                1.1,
                0.76,
                [0.12, 0.04, 0.08],
                [0.18, 0.14, 0.06],
                0.7,
            ),
        };
        Base {
            exposure,
            contrast,
            saturation,
            shadow,
            highlight,
            strength,
        }
    }

    /// The grade at the theme's own strength, or at `strength` clamped to 0..=1.
    pub fn grade(self, strength: Option<f32>) -> ColorGrade {
        let base = self.base();
        let amount = strength
            .filter(|value| value.is_finite())
            .unwrap_or(base.strength)
            .clamp(0.0, 1.0);
        ColorGrade {
            enabled: self != Self::Neutral && amount > 0.0,
            exposure: base.exposure * amount,
            contrast: 1.0 + (base.contrast - 1.0) * amount,
            saturation: 1.0 + (base.saturation - 1.0) * amount,
            shadow: tint_factors(base.shadow.map(|value| value * amount)),
            highlight: tint_factors(base.highlight.map(|value| value * amount)),
        }
    }
}

fn tint_factors(values: [f32; 3]) -> [f32; 3] {
    let mean = (values[0] + values[1] + values[2]) / 3.0;
    values.map(|value| 1.0 + (value - mean) * 1.5)
}

/// Uniform values for the grade, identical to the earlier `gradeParameters`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorGrade {
    pub enabled: bool,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub shadow: [f32; 3],
    pub highlight: [f32; 3],
}

impl ColorGrade {
    /// The grade for the flat (painted photograph, no splat world) battlefield: the same
    /// theme grade lifted so the room photograph reads as the photographed place on a TV
    /// instead of a near-black void. Exposure is raised, contrast and saturation are eased
    /// a third of the way back toward neutral, and a disabled grade is switched on at the
    /// lifted exposure so even a neutral theme is not left under the backdrop's own dimming.
    pub fn lifted_for_flat(self) -> Self {
        Self {
            enabled: true,
            exposure: self.exposure + 0.55,
            contrast: 1.0 + (self.contrast - 1.0) * 0.65,
            saturation: 1.0 + (self.saturation - 1.0) * 0.65,
            shadow: self.shadow,
            highlight: self.highlight,
        }
    }

    /// The CPU twin of the shader grade, for tests and for tinting overlay colours.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        if !self.enabled {
            return rgb.map(|value| value.clamp(0.0, 4.0));
        }
        let scale = self.exposure.exp2();
        let exposed = rgb.map(|value| value.max(0.0) * scale);
        let luma = exposed[0] * 0.2126 + exposed[1] * 0.7152 + exposed[2] * 0.0722;
        let mix = luma.clamp(0.0, 1.0);
        let mut out = [0.0; 3];
        for channel in 0..3 {
            let saturated = (luma + (exposed[channel] - luma) * self.saturation).max(0.0);
            let tint = self.shadow[channel] * (1.0 - mix) + self.highlight[channel] * mix;
            out[channel] = (saturated.powf(self.contrast) * tint).clamp(0.0, 4.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harbor_grade_matches_the_earlier_numbers_at_theme_strength() {
        let grade = Theme::Harbor.grade(None);
        assert!(grade.enabled);
        assert!((grade.exposure - (-0.3 * 0.7)).abs() < 1e-6);
        assert!((grade.contrast - (1.0 + 0.1 * 0.7)).abs() < 1e-6);
        assert!((grade.saturation - (1.0 - 0.24 * 0.7)).abs() < 1e-6);
        // Shadow tint leans blue: the blue factor is the largest.
        assert!(grade.shadow[2] > grade.shadow[0]);
        assert!(grade.highlight[0] > grade.highlight[2]);
    }

    #[test]
    fn neutral_and_zero_strength_leave_colours_unchanged() {
        let neutral = Theme::Neutral.grade(None);
        assert!(!neutral.enabled);
        assert_eq!(neutral.apply([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]);
        assert!(!Theme::Forest.grade(Some(0.0)).enabled);
    }

    #[test]
    fn strength_is_clamped_and_non_finite_falls_back_to_the_theme_default() {
        let strong = Theme::Crypt.grade(Some(9.0));
        assert!((strong.exposure - (-0.28)).abs() < 1e-6);
        assert_eq!(Theme::Crypt.grade(Some(f32::NAN)), Theme::Crypt.grade(None));
    }

    #[test]
    fn grading_darkens_and_desaturates_a_bright_colour() {
        let graded = Theme::Swamp.grade(None).apply([0.9, 0.6, 0.3]);
        let luma = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        assert!(luma(graded) < luma([0.9, 0.6, 0.3]));
    }

    #[test]
    fn flat_lift_brightens_a_dark_theme_grade_for_the_tv() {
        let dark = Theme::Swamp.grade(None);
        let flat = dark.lifted_for_flat();
        assert!(flat.enabled);
        assert!((flat.exposure - (dark.exposure + 0.55)).abs() < 1e-6);
        assert!(flat.contrast < dark.contrast && flat.contrast > 1.0);
        assert!(flat.saturation > dark.saturation && flat.saturation < 1.0);
        let luma = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        assert!(luma(flat.apply([0.5, 0.4, 0.3])) > luma(dark.apply([0.5, 0.4, 0.3])));
        // A neutral (disabled) grade is switched on by the lift, not left dark.
        let neutral = Theme::Neutral.grade(None).lifted_for_flat();
        assert!(neutral.enabled);
        assert!(luma(neutral.apply([0.4, 0.4, 0.4])) > 0.4);
    }

    #[test]
    fn theme_keys_cover_every_theme() {
        for key in [
            "neutral",
            "crypt",
            "flooded_hall",
            "forest",
            "harbor",
            "lava",
            "library",
            "mountain",
            "swamp",
            "tavern",
            "throne",
        ] {
            assert!(Theme::from_key(key).is_some(), "{key}");
        }
        assert!(Theme::from_key("sunny").is_none());
    }
}
