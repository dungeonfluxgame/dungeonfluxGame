//! What the battlefield shows of a figure's health, without owning any threshold of its own.
//!
//! The renderer cannot depend on the application crate that defines the health tiers, so the
//! application installs three small things once per page: the classifier that turns hit points
//! into a [`Tone`] (the one shared tier function), the reaction a blow gets (sized by the share
//! of the bar it took), and the Critical heartbeat tremor. It also publishes, by combatant name,
//! the server-made health variant of each figure's picture. Nothing here decides an outcome:
//! everything is presentation of numbers the server's view already carries.

use std::{cell::RefCell, collections::BTreeMap};

/// How hurt a figure looks, by name only: the thresholds live with the installed classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Healthy,
    Hurt,
    Bloodied,
    Critical,
    Down,
}

impl Tone {
    /// The `data-health-tier` value the stylesheet keys on.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Hurt => "hurt",
            Self::Bloodied => "bloodied",
            Self::Critical => "critical",
            Self::Down => "down",
        }
    }
}

/// A keyframe of a reaction: how far through it (0 to 1), and how far to each side and downward
/// as a share of the amplitude (-1 to 1).
pub type Frame = (f64, f64, f64);

/// One small bodily reaction: a shake that dies away, or a tremor that repeats. Pure data, so a
/// director can start it on any figure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub frames: &'static [Frame],
    /// The largest sideways or downward shift, in metres.
    pub amplitude_m: f32,
    pub duration_s: f32,
    /// A tremor repeats for as long as it is shown; a hit reaction plays once.
    pub looping: bool,
}

impl Reaction {
    /// The shift `age_s` seconds in: `(sideways, downward)` in metres. Past the end of a hit
    /// reaction it is zero.
    pub fn offset(&self, age_s: f32) -> (f32, f32) {
        if self.duration_s <= 0.0 || self.frames.len() < 2 || age_s < 0.0 {
            return (0.0, 0.0);
        }
        let progress = if self.looping {
            (age_s / self.duration_s).fract()
        } else if age_s >= self.duration_s {
            return (0.0, 0.0);
        } else {
            age_s / self.duration_s
        };
        let progress = f64::from(progress);
        let next_frame_index = self
            .frames
            .iter()
            .position(|frame| frame.0 >= progress)
            .unwrap_or(self.frames.len() - 1)
            .max(1);
        let (previous_frame, next_frame) = (
            self.frames[next_frame_index - 1],
            self.frames[next_frame_index],
        );
        let frame_span = (next_frame.0 - previous_frame.0).max(f64::EPSILON);
        let interpolation_share = ((progress - previous_frame.0) / frame_span).clamp(0.0, 1.0);
        let amplitude = f64::from(self.amplitude_m);
        (
            ((previous_frame.1 + (next_frame.1 - previous_frame.1) * interpolation_share)
                * amplitude) as f32,
            ((previous_frame.2 + (next_frame.2 - previous_frame.2) * interpolation_share)
                * amplitude) as f32,
        )
    }

    /// Whether a hit reaction has finished.
    pub fn finished(&self, age_s: f32) -> bool {
        !self.looping && age_s >= self.duration_s
    }
}

/// The tone of a figure with these hit points; `down` is true for one the view marks down.
pub type ToneFn = fn(current: u32, maximum: u32, down: bool) -> Option<Tone>;

/// The reaction to a blow that took `lost` hit points of `maximum`, or none.
pub type ReactionFn =
    fn(lost: u32, maximum: u32, critical: bool, dropped: bool) -> Option<Reaction>;

/// What the application installs.
#[derive(Clone, Copy)]
pub struct Hooks {
    pub tone: ToneFn,
    pub reaction: ReactionFn,
    pub tremor: Reaction,
}

thread_local! {
    static HOOKS: RefCell<Option<Hooks>> = const { RefCell::new(None) };
    static VARIANTS: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Installs the classifier and reactions; later calls replace earlier ones.
pub fn install(hooks: Hooks) {
    HOOKS.with(|slot| *slot.borrow_mut() = Some(hooks));
}

/// The tone of a figure; `None` when nothing is installed or it has no hit points to show.
pub fn tone_of(current: u32, maximum: u32, down: bool) -> Option<Tone> {
    HOOKS
        .with(|slot| *slot.borrow())
        .and_then(|hooks| (hooks.tone)(current, maximum, down))
}

/// The reaction of a figure that just lost hit points.
pub fn reaction_for(lost: u32, maximum: u32, critical: bool, dropped: bool) -> Option<Reaction> {
    HOOKS
        .with(|slot| *slot.borrow())
        .and_then(|hooks| (hooks.reaction)(lost, maximum, critical, dropped))
}

/// The Critical heartbeat tremor, when installed.
pub fn tremor() -> Option<Reaction> {
    HOOKS.with(|slot| slot.borrow().map(|hooks| hooks.tremor))
}

/// A published picture is only ever a server-made image: `/gameplay/imagery/` and a 64-digit
/// lowercase hexadecimal key.
pub fn valid_variant(path: &str) -> bool {
    crate::art_contract::player_hero_image(path).is_some()
}

/// Publishes the health variants the server has made, by combatant name (replacing the previous
/// set). Paths that are not server-made images are ignored.
pub fn set_variants(variants: impl IntoIterator<Item = (String, String)>) {
    VARIANTS.with(|slot| {
        *slot.borrow_mut() = variants
            .into_iter()
            .filter(|(_, path)| valid_variant(path))
            .collect();
    });
}

/// The published variant of the combatant called `name`.
pub fn variant_for(name: &str) -> Option<String> {
    VARIANTS.with(|slot| slot.borrow().get(name).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAKE: &[Frame] = &[(0.0, 0.0, 0.0), (0.5, 1.0, 0.5), (1.0, 0.0, 0.0)];

    fn hit() -> Reaction {
        Reaction {
            frames: SHAKE,
            amplitude_m: 0.2,
            duration_s: 0.4,
            looping: false,
        }
    }

    #[test]
    fn a_hit_reaction_plays_once_and_comes_to_rest() {
        let reaction = hit();
        assert_eq!(reaction.offset(0.0), (0.0, 0.0));
        let (side, down) = reaction.offset(0.2);
        assert!((side - 0.2).abs() < 1e-6 && (down - 0.1).abs() < 1e-6);
        assert_eq!(reaction.offset(0.4), (0.0, 0.0));
        assert_eq!(reaction.offset(5.0), (0.0, 0.0));
        assert!(reaction.finished(0.4) && !reaction.finished(0.39));
        // Halfway up to the peak is half the peak.
        assert!((reaction.offset(0.1).0 - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_tremor_repeats_and_never_finishes() {
        let tremor = Reaction {
            looping: true,
            ..hit()
        };
        let first = tremor.offset(0.2);
        let later = tremor.offset(0.2 + 0.4 * 7.0);
        assert!((first.0 - later.0).abs() < 1e-4);
        assert!(!tremor.finished(1000.0));
    }

    #[test]
    fn only_server_made_images_are_published_as_variants() {
        let good = format!("/gameplay/imagery/{}", "a".repeat(64));
        set_variants([
            ("Brakka".to_owned(), good.clone()),
            ("Thrain".to_owned(), "/assets/ui/generated/x".to_owned()),
            ("Marek".to_owned(), "/gameplay/imagery/short".to_owned()),
        ]);
        assert_eq!(variant_for("Brakka"), Some(good));
        assert_eq!(variant_for("Thrain"), None);
        assert_eq!(variant_for("Marek"), None);
        set_variants([]);
        assert_eq!(variant_for("Brakka"), None);
    }

    #[test]
    fn nothing_is_classified_or_shaken_until_the_application_installs_its_hooks() {
        assert_eq!(tone_of(5, 13, false), None);
        assert_eq!(reaction_for(5, 13, false, false), None);
        assert_eq!(tremor(), None);
        install(Hooks {
            tone: |current, _, down| (down || current == 0).then_some(Tone::Down),
            reaction: |_, _, _, _| Some(hit()),
            tremor: hit(),
        });
        assert_eq!(tone_of(0, 13, false), Some(Tone::Down));
        assert_eq!(tone_of(5, 13, false), None);
        assert!(reaction_for(5, 13, false, false).is_some());
        assert!(tremor().is_some());
        HOOKS.with(|slot| *slot.borrow_mut() = None);
    }
}
