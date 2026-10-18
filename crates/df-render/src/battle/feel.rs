//! Battle feel: how hard a blow lands and how the camera lives between blows.
//!
//! Everything here is pure presentation and time-based: it is a function of seconds, never of
//! frames, so a 24 fps and a 30 fps battlefield feel the same. The stage owns one [`FeelState`],
//! starts effects from the events it already has (a blow landing, a miss, a camera cut, a
//! finishing blow) and reads a [`FeelView`] each presented frame. Nothing here changes an outcome
//! or moves a figure across squares: it shakes and blurs the picture, freezes the stage for a few
//! frames, slows time, pulses the light and keeps the camera breathing.
//!
//! Every intensity is a named constant in this one module.

use std::f32::consts::TAU;

// ---- how big an event is --------------------------------------------------------------------

/// Share of the target's maximum hit points below which a hit is only a graze.
pub const GRAZE_SHARE: f32 = 0.10;
/// Share at or above which a hit is heavy.
pub const HEAVY_SHARE: f32 = 0.25;

/// How a blow reads, from the server's outcome and how much of the target's health it took.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ImpactClass {
    Graze,
    Solid,
    Heavy,
    Critical,
    Knockout,
}

/// The class of a blow that landed. A knockout outranks a critical, which outranks the size of the
/// hit; the size is the share of the target's maximum hit points the damage took.
pub fn classify(damage: u32, maximum_hit_points: u32, critical: bool, downed: bool) -> ImpactClass {
    if downed {
        return ImpactClass::Knockout;
    }
    if critical {
        return ImpactClass::Critical;
    }
    let share = damage_share(damage, maximum_hit_points);
    if share < GRAZE_SHARE {
        ImpactClass::Graze
    } else if share < HEAVY_SHARE {
        ImpactClass::Solid
    } else {
        ImpactClass::Heavy
    }
}

/// Damage as a share of the target's maximum hit points, 0 to 1 (0 when the maximum is unknown).
pub fn damage_share(damage: u32, maximum_hit_points: u32) -> f32 {
    if maximum_hit_points == 0 {
        return 0.0;
    }
    (damage as f32 / maximum_hit_points as f32).clamp(0.0, 1.0)
}

/// One number from 0 to 1 for how heavy a blow is: the class sets the band, the share of health
/// lost moves it within the band. This is the intensity the HUD work subscribes to.
pub fn weight(class: ImpactClass, share: f32) -> f32 {
    let share = share.clamp(0.0, 1.0);
    let (low, high) = match class {
        ImpactClass::Graze => (0.08, 0.2),
        ImpactClass::Solid => (0.25, 0.45),
        ImpactClass::Heavy => (0.55, 0.8),
        ImpactClass::Critical => (0.85, 0.95),
        ImpactClass::Knockout => (1.0, 1.0),
    };
    low + (high - low) * share
}

// ---- camera shake ---------------------------------------------------------------------------

/// Largest shake offset anywhere, in pixels at 1080p, however the numbers are set.
pub const SHAKE_MAX_PX: f32 = 40.0;
/// Shake amplitude (px at 1080p) at the lightest and the heaviest ordinary hit.
pub const SHAKE_GRAZE_PX: f32 = 2.5;
pub const SHAKE_HEAVY_PX: f32 = 18.0;
/// A critical is a sharper, faster kick; a knockout is a low heavy thud.
pub const SHAKE_CRITICAL_PX: f32 = 24.0;
pub const SHAKE_KNOCKOUT_PX: f32 = 30.0;
/// Roll of the picture (degrees) per pixel of shake amplitude.
pub const SHAKE_ROLL_DEG_PER_PX: f32 = 0.045;
/// Shake duration in seconds: light hits are over at once, a knockout rolls on.
pub const SHAKE_GRAZE_S: f32 = 0.14;
pub const SHAKE_HEAVY_S: f32 = 0.42;
pub const SHAKE_CRITICAL_S: f32 = 0.5;
pub const SHAKE_KNOCKOUT_S: f32 = 0.85;
/// Shake frequency in hertz: ordinary hits rattle, a critical kicks sharply, a knockout thuds.
pub const SHAKE_HZ: f32 = 11.0;
pub const SHAKE_CRITICAL_HZ: f32 = 17.0;
pub const SHAKE_KNOCKOUT_HZ: f32 = 5.0;
/// A miss: the camera sways past by this many pixels, slowly.
pub const WHOOSH_PX: f32 = 5.0;
pub const WHOOSH_S: f32 = 0.45;

/// The shape of one shake: where it starts, how long it lasts, how fast it shakes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeSpec {
    pub amplitude_px: f32,
    pub roll_deg: f32,
    pub duration_s: f32,
    pub frequency_hz: f32,
}

/// The shake for a blow of `class` and `weight` (see [`weight`]). Amplitude and duration grow
/// with the weight, never past [`SHAKE_MAX_PX`].
pub fn shake_spec(class: ImpactClass, weight: f32) -> ShakeSpec {
    let w = weight.clamp(0.0, 1.0);
    let (amplitude_px, duration_s, frequency_hz) = match class {
        ImpactClass::Graze | ImpactClass::Solid | ImpactClass::Heavy => {
            // 0.08 (lightest graze) up to 0.8 (heaviest ordinary hit) spans the ordinary range.
            let t = ((w - 0.08) / 0.72).clamp(0.0, 1.0);
            (
                SHAKE_GRAZE_PX + (SHAKE_HEAVY_PX - SHAKE_GRAZE_PX) * t,
                SHAKE_GRAZE_S + (SHAKE_HEAVY_S - SHAKE_GRAZE_S) * t,
                SHAKE_HZ,
            )
        }
        ImpactClass::Critical => (SHAKE_CRITICAL_PX, SHAKE_CRITICAL_S, SHAKE_CRITICAL_HZ),
        ImpactClass::Knockout => (SHAKE_KNOCKOUT_PX, SHAKE_KNOCKOUT_S, SHAKE_KNOCKOUT_HZ),
    };
    let amplitude_px = amplitude_px.min(SHAKE_MAX_PX);
    ShakeSpec {
        amplitude_px,
        roll_deg: amplitude_px * SHAKE_ROLL_DEG_PER_PX,
        duration_s,
        frequency_hz,
    }
}

/// The shake at `age_s` seconds: `(x px, y px, roll degrees)`, decaying to zero at the duration.
/// Bounded by the amplitude at every instant.
pub fn shake_sample(spec: &ShakeSpec, age_s: f32) -> (f32, f32, f32) {
    if age_s < 0.0 || age_s >= spec.duration_s || spec.duration_s <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let remaining = 1.0 - age_s / spec.duration_s;
    // Quadratic decay: a hard first instant, a fast settle.
    let envelope = remaining * remaining;
    let phase = TAU * spec.frequency_hz * age_s;
    (
        spec.amplitude_px * envelope * phase.sin(),
        spec.amplitude_px * 0.72 * envelope * (phase * 1.37 + 0.7).cos(),
        spec.roll_deg * envelope * (phase * 0.71 + 1.1).sin(),
    )
}

/// The sway of the camera past a dodged blow at `age_s`: slow, one-sided, smooth.
pub fn whoosh_sample(age_s: f32) -> (f32, f32, f32) {
    if !(0.0..WHOOSH_S).contains(&age_s) {
        return (0.0, 0.0, 0.0);
    }
    let t = age_s / WHOOSH_S;
    let bump = (std::f32::consts::PI * t).sin();
    (WHOOSH_PX * bump, 0.0, WHOOSH_PX * 0.02 * bump)
}

// ---- hit-stop and slow motion ---------------------------------------------------------------

/// How long the stage freezes at the moment of contact, in seconds ("a few frames").
pub fn hit_stop_s(class: ImpactClass) -> f32 {
    match class {
        ImpactClass::Graze => 0.0,
        ImpactClass::Solid => 0.05,
        ImpactClass::Heavy => 0.075,
        ImpactClass::Critical => 0.12,
        ImpactClass::Knockout => 0.15,
    }
}

/// The finishing blow's slow-motion ramp: time slows to [`SLOWMO_SCALE`] over [`SLOWMO_RAMP_IN_S`]
/// before contact, holds, then comes back over [`SLOWMO_RAMP_OUT_S`].
pub const SLOWMO_SCALE: f32 = 0.3;
pub const SLOWMO_RAMP_IN_S: f32 = 0.35;
pub const SLOWMO_HOLD_S: f32 = 0.55;
pub const SLOWMO_RAMP_OUT_S: f32 = 0.4;

/// Total real seconds a slow-motion ramp lasts.
pub fn slowmo_duration_s() -> f32 {
    SLOWMO_RAMP_IN_S + SLOWMO_HOLD_S + SLOWMO_RAMP_OUT_S
}

/// The stage's time scale `age_s` seconds into a slow-motion ramp.
pub fn slowmo_scale(age_s: f32) -> f32 {
    if !(0.0..slowmo_duration_s()).contains(&age_s) {
        return 1.0;
    }
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    if age_s < SLOWMO_RAMP_IN_S {
        1.0 + (SLOWMO_SCALE - 1.0) * smooth(age_s / SLOWMO_RAMP_IN_S)
    } else if age_s < SLOWMO_RAMP_IN_S + SLOWMO_HOLD_S {
        SLOWMO_SCALE
    } else {
        let t = (age_s - SLOWMO_RAMP_IN_S - SLOWMO_HOLD_S) / SLOWMO_RAMP_OUT_S;
        SLOWMO_SCALE + (1.0 - SLOWMO_SCALE) * smooth(t)
    }
}

// ---- blur, split, flash, pulse --------------------------------------------------------------

/// Directional blur: pixels of blur per degree per second of camera yaw/pitch speed, and its cap.
/// Kept light: at 26 px every fighter smeared for half a second of each attack on a TV (B-13).
pub const MOTION_BLUR_PX_PER_DEG_S: f32 = 0.1;
pub const MOTION_BLUR_MAX_PX: f32 = 10.0;
/// Camera speed (degrees per second) below which there is no motion blur at all.
pub const MOTION_BLUR_FLOOR_DEG_S: f32 = 12.0;
/// The critical's zoom-blur punch-in: strength (share of the frame radius) and duration.
pub const RADIAL_PUNCH: f32 = 0.085;
pub const RADIAL_PUNCH_S: f32 = 0.32;
/// Chromatic split on heavy impacts, in pixels at the screen edge at 1080p, and its duration.
pub const CHROMA_HEAVY_PX: f32 = 3.0;
pub const CHROMA_CRITICAL_PX: f32 = 6.5;
pub const CHROMA_S: f32 = 0.3;
/// The focus pull when the camera changes subject: the tilt-shift blur starts at this many times
/// its resting size and eases back over the duration.
pub const FOCUS_PULL_BLUR_FACTOR: f32 = 2.8;
pub const FOCUS_PULL_S: f32 = 0.55;
/// The vignette pulse on the acting side when a hero is hit: strength and duration.
pub const VIGNETTE_PULSE: f32 = 0.34;
pub const VIGNETTE_PULSE_S: f32 = 0.55;
/// Flash strength by class (white to warm), before the stage's own tone choice.
pub fn flash_strength(class: ImpactClass) -> f32 {
    match class {
        ImpactClass::Graze => 0.0,
        ImpactClass::Solid => 0.1,
        ImpactClass::Heavy => 0.26,
        ImpactClass::Critical => 0.5,
        ImpactClass::Knockout => 0.4,
    }
}

/// Blur in pixels for a camera turning at `speed_deg_s` (yaw and pitch speed combined).
pub fn motion_blur_px(speed_deg_s: f32) -> f32 {
    ((speed_deg_s - MOTION_BLUR_FLOOR_DEG_S).max(0.0) * MOTION_BLUR_PX_PER_DEG_S)
        .min(MOTION_BLUR_MAX_PX)
}

// ---- the living camera ----------------------------------------------------------------------

/// Handheld sway at calm and at peak tempo: speed (hertz) and amplitude (degrees). Calm is slow
/// and small; peak is quicker and tighter.
pub const SWAY_CALM_HZ: f32 = 0.09;
pub const SWAY_PEAK_HZ: f32 = 0.42;
pub const SWAY_CALM_DEG: f32 = 0.3;
pub const SWAY_PEAK_DEG: f32 = 0.42;
/// The slow orbit drift on a wide shot: how far it wanders (degrees of yaw) and how slowly.
pub const ORBIT_DRIFT_DEG: f32 = 2.2;
pub const ORBIT_DRIFT_HZ: f32 = 0.018;
/// A subtle push-in while a roll is pending: the distance share removed at its end.
pub const PUSH_IN_SHARE: f32 = 0.06;

/// Camera offsets from the living camera: degrees of yaw and pitch and a roll in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sway {
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
}

/// The handheld sway at `time_s` for a tempo `intensity_permille` (0 calm to 1000 peak, the value
/// the tempo cue already carries). `wide` adds the slow orbit drift of a wide tactical shot.
pub fn sway(time_s: f32, intensity_permille: u16, wide: bool) -> Sway {
    let tempo = f32::from(intensity_permille.min(1000)) / 1000.0;
    let hz = SWAY_CALM_HZ + (SWAY_PEAK_HZ - SWAY_CALM_HZ) * tempo;
    let amplitude = SWAY_CALM_DEG + (SWAY_PEAK_DEG - SWAY_CALM_DEG) * tempo;
    // Two incommensurate slow waves per axis so it never reads as a loop. The phase is kept
    // continuous by integrating a changing rate would be overkill: a tempo change only changes
    // the rate of the current wave, which is smooth enough at these slow speeds.
    let phase = TAU * hz * time_s;
    let drift = if wide {
        ORBIT_DRIFT_DEG * (TAU * ORBIT_DRIFT_HZ * time_s).sin()
    } else {
        0.0
    };
    Sway {
        yaw_deg: amplitude * (phase.sin() * 0.7 + (phase * 0.43 + 1.3).sin() * 0.3) + drift,
        pitch_deg: amplitude
            * 0.6
            * ((phase * 0.81 + 0.4).sin() * 0.7 + (phase * 0.37).cos() * 0.3),
        roll_deg: amplitude * 0.35 * (phase * 0.59 + 2.1).sin(),
    }
}

// ---- the state ------------------------------------------------------------------------------

/// One blow landing, as the stage tells it to the feel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactEffect {
    pub class: ImpactClass,
    pub weight: f32,
    /// A hero was hit: pulse the vignette on the acting side.
    pub on_hero: bool,
}

#[derive(Clone, Copy, Debug)]
struct Timed<T> {
    age_s: f32,
    value: T,
}

/// What the picture needs this frame, all in screen units at 1080p.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeelView {
    pub shake_px: (f32, f32),
    pub shake_roll_deg: f32,
    pub motion_blur_px: f32,
    /// Screen direction of the motion blur, unit length (or zero).
    pub motion_blur_dir: (f32, f32),
    pub radial_blur: f32,
    pub chromatic_px: f32,
    pub focus_blur_factor: f32,
    pub vignette_pulse: f32,
}

#[derive(Clone, Debug, Default)]
pub struct FeelState {
    shakes: Vec<Timed<ShakeSpec>>,
    whooshes: Vec<f32>,
    hit_stop_left_s: f32,
    slowmo_age_s: Option<f32>,
    radial: Option<f32>,
    chroma: Option<Timed<f32>>,
    pull: Option<f32>,
    pulse: Option<f32>,
    /// Smoothed camera speed in degrees per second and its dominant axis (yaw, pitch).
    camera_speed: (f32, f32),
}

/// The shakes alive at once never exceed this many.
const MAX_SHAKES: usize = 4;

impl FeelState {
    pub fn new() -> Self {
        Self::default()
    }

    /// A blow landed: shake, freeze, kick, split and pulse by its size.
    pub fn impact(&mut self, effect: ImpactEffect) {
        let spec = shake_spec(effect.class, effect.weight);
        if self.shakes.len() == MAX_SHAKES {
            self.shakes.remove(0);
        }
        self.shakes.push(Timed {
            age_s: 0.0,
            value: spec,
        });
        self.hit_stop_left_s = self.hit_stop_left_s.max(hit_stop_s(effect.class));
        if effect.class >= ImpactClass::Critical {
            self.radial = Some(0.0);
        }
        let chroma_px = match effect.class {
            ImpactClass::Heavy => CHROMA_HEAVY_PX,
            ImpactClass::Critical | ImpactClass::Knockout => CHROMA_CRITICAL_PX,
            _ => 0.0,
        };
        if chroma_px > 0.0 {
            self.chroma = Some(Timed {
                age_s: 0.0,
                value: chroma_px,
            });
        }
        if effect.on_hero && effect.class >= ImpactClass::Solid {
            self.pulse = Some(0.0);
        }
    }

    /// A blow was dodged: a small slow sway past instead of a shake.
    pub fn miss(&mut self) {
        self.whooshes.push(0.0);
        self.whooshes.truncate(MAX_SHAKES);
    }

    /// The finishing blow's slow-motion ramp starts now (see [`slowmo_scale`]).
    pub fn slow_motion(&mut self) {
        self.slowmo_age_s = Some(0.0);
    }

    /// The camera changed subject: pull focus.
    pub fn focus_pull(&mut self) {
        self.pull = Some(0.0);
    }

    /// Reports how fast the camera is turning (degrees per second of yaw and of pitch), smoothed.
    pub fn observe_camera_speed(&mut self, yaw_deg_s: f32, pitch_deg_s: f32, dt_s: f32) {
        let blend = (dt_s * 12.0).clamp(0.0, 1.0);
        self.camera_speed.0 += (yaw_deg_s - self.camera_speed.0) * blend;
        self.camera_speed.1 += (pitch_deg_s - self.camera_speed.1) * blend;
    }

    /// Advances every effect by `dt_s` real seconds.
    pub fn advance(&mut self, dt_s: f32) {
        let dt = dt_s.clamp(0.0, 0.25);
        for shake in &mut self.shakes {
            shake.age_s += dt;
        }
        self.shakes
            .retain(|shake| shake.age_s < shake.value.duration_s);
        for age in &mut self.whooshes {
            *age += dt;
        }
        self.whooshes.retain(|age| *age < WHOOSH_S);
        self.hit_stop_left_s = (self.hit_stop_left_s - dt).max(0.0);
        if let Some(age) = &mut self.slowmo_age_s {
            *age += dt;
            if *age >= slowmo_duration_s() {
                self.slowmo_age_s = None;
            }
        }
        for (envelope, duration) in [
            (&mut self.radial, RADIAL_PUNCH_S),
            (&mut self.pull, FOCUS_PULL_S),
            (&mut self.pulse, VIGNETTE_PULSE_S),
        ] {
            if let Some(age) = envelope {
                *age += dt;
                if *age >= duration {
                    *envelope = None;
                }
            }
        }
        if let Some(chroma) = &mut self.chroma {
            chroma.age_s += dt;
            if chroma.age_s >= CHROMA_S {
                self.chroma = None;
            }
        }
    }

    /// How fast the stage's own clock runs: 0 during hit-stop, below 1 during slow motion.
    pub fn time_scale(&self) -> f32 {
        if self.hit_stop_left_s > 0.0 {
            return 0.0;
        }
        self.slowmo_age_s.map_or(1.0, slowmo_scale)
    }

    /// Whether hit-stop is holding the stage still right now.
    pub fn is_frozen(&self) -> bool {
        self.hit_stop_left_s > 0.0
    }

    /// Whether a hit-stop or a slow-motion ramp is playing: the stage is still "busy" until both
    /// are over, so whatever waits on the stage (the exit, the next beat) waits for them too.
    pub fn is_active(&self) -> bool {
        self.hit_stop_left_s > 0.0 || self.slowmo_age_s.is_some()
    }

    pub fn view(&self) -> FeelView {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut roll = 0.0;
        for shake in &self.shakes {
            let (sx, sy, sr) = shake_sample(&shake.value, shake.age_s);
            x += sx;
            y += sy;
            roll += sr;
        }
        for age in &self.whooshes {
            let (sx, sy, sr) = whoosh_sample(*age);
            x += sx;
            y += sy;
            roll += sr;
        }
        let limit = SHAKE_MAX_PX;
        let (yaw_speed, pitch_speed) = self.camera_speed;
        let speed = yaw_speed.hypot(pitch_speed);
        let blur = motion_blur_px(speed);
        let direction = if blur > 0.0 {
            // Yaw turns the picture sideways, pitch up and down.
            let (dx, dy) = (yaw_speed, pitch_speed);
            let length = dx.hypot(dy).max(1e-4);
            (dx / length, dy / length)
        } else {
            (0.0, 0.0)
        };
        let fall = |age: Option<f32>, duration: f32| {
            age.map_or(0.0, |age| {
                let t = (1.0 - age / duration).clamp(0.0, 1.0);
                t * t
            })
        };
        FeelView {
            shake_px: (x.clamp(-limit, limit), y.clamp(-limit, limit)),
            shake_roll_deg: roll.clamp(-2.0, 2.0),
            motion_blur_px: blur,
            motion_blur_dir: direction,
            radial_blur: RADIAL_PUNCH * fall(self.radial, RADIAL_PUNCH_S),
            chromatic_px: self.chroma.map_or(0.0, |chroma| {
                chroma.value * fall(Some(chroma.age_s), CHROMA_S)
            }),
            focus_blur_factor: 1.0 + (FOCUS_PULL_BLUR_FACTOR - 1.0) * fall(self.pull, FOCUS_PULL_S),
            vignette_pulse: VIGNETTE_PULSE * fall(self.pulse, VIGNETTE_PULSE_S),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advances in short steps, as frames do (one call is clamped to a quarter second).
    fn run(state: &mut FeelState, seconds: f32) {
        let steps = (seconds / 0.01).ceil() as u32;
        for _ in 0..steps {
            state.advance(0.01);
        }
    }

    fn peak(spec: &ShakeSpec) -> f32 {
        (0..=1000)
            .map(|step| {
                let (x, y, _) = shake_sample(spec, spec.duration_s * step as f32 / 1000.0);
                x.hypot(y)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn blows_are_classified_by_outcome_then_size() {
        assert_eq!(classify(1, 12, false, false), ImpactClass::Graze);
        assert_eq!(classify(2, 12, false, false), ImpactClass::Solid);
        assert_eq!(classify(6, 12, false, false), ImpactClass::Heavy);
        assert_eq!(classify(1, 12, true, false), ImpactClass::Critical);
        assert_eq!(classify(1, 12, true, true), ImpactClass::Knockout);
        assert_eq!(classify(0, 0, false, false), ImpactClass::Graze);
    }

    #[test]
    fn weight_rises_with_class_and_with_share_inside_a_class() {
        let classes = [
            ImpactClass::Graze,
            ImpactClass::Solid,
            ImpactClass::Heavy,
            ImpactClass::Critical,
            ImpactClass::Knockout,
        ];
        for pair in classes.windows(2) {
            assert!(
                weight(pair[0], 1.0) <= weight(pair[1], 0.0) + 1e-6,
                "{pair:?}"
            );
        }
        assert!(weight(ImpactClass::Heavy, 0.9) > weight(ImpactClass::Heavy, 0.3));
        for class in classes {
            for share in [0.0, 0.5, 1.0, 7.0, -3.0] {
                assert!((0.0..=1.0).contains(&weight(class, share)));
            }
        }
    }

    #[test]
    fn a_heavy_hit_shakes_much_harder_than_a_graze_and_a_critical_kicks_sharper() {
        let graze = shake_spec(ImpactClass::Graze, weight(ImpactClass::Graze, 0.0));
        let heavy = shake_spec(ImpactClass::Heavy, weight(ImpactClass::Heavy, 1.0));
        let critical = shake_spec(ImpactClass::Critical, weight(ImpactClass::Critical, 0.5));
        let knockout = shake_spec(ImpactClass::Knockout, weight(ImpactClass::Knockout, 0.5));
        assert!(
            heavy.amplitude_px > graze.amplitude_px * 4.0,
            "{graze:?} {heavy:?}"
        );
        assert!(heavy.duration_s > graze.duration_s * 2.0);
        assert!(critical.frequency_hz > heavy.frequency_hz, "sharper");
        assert!(knockout.frequency_hz < heavy.frequency_hz, "a low thud");
        assert!(knockout.duration_s > heavy.duration_s, "rolls on");
        assert!(critical.amplitude_px > heavy.amplitude_px);
    }

    #[test]
    fn every_shake_is_bounded_and_decays_to_exactly_nothing() {
        for class in [
            ImpactClass::Graze,
            ImpactClass::Solid,
            ImpactClass::Heavy,
            ImpactClass::Critical,
            ImpactClass::Knockout,
        ] {
            for weight in [0.0, 0.3, 1.0, 50.0] {
                let spec = shake_spec(class, weight);
                assert!(spec.amplitude_px <= SHAKE_MAX_PX);
                assert!(peak(&spec) <= spec.amplitude_px * 1.3 + 1e-3);
                assert_eq!(shake_sample(&spec, spec.duration_s), (0.0, 0.0, 0.0));
                assert_eq!(shake_sample(&spec, -1.0), (0.0, 0.0, 0.0));
                // Later is quieter: the last tenth never exceeds the first tenth.
                let early = (0..100)
                    .map(|i| {
                        shake_sample(&spec, spec.duration_s * 0.1 * i as f32 / 100.0)
                            .0
                            .abs()
                    })
                    .fold(0.0, f32::max);
                let late = (0..100)
                    .map(|i| {
                        shake_sample(&spec, spec.duration_s * (0.9 + 0.1 * i as f32 / 100.0))
                            .0
                            .abs()
                    })
                    .fold(0.0, f32::max);
                assert!(late <= early + 1e-4, "{class:?}: {late} vs {early}");
            }
        }
    }

    #[test]
    fn a_miss_sways_instead_of_shaking_and_the_sway_returns_to_rest() {
        let mut state = FeelState::new();
        state.miss();
        state.advance(WHOOSH_S * 0.5);
        let mid = state.view();
        assert!(mid.shake_px.0 > 1.0 && mid.shake_px.0 <= WHOOSH_PX + 1e-3);
        assert!(mid.shake_px.1 == 0.0, "no vertical rattle on a miss");
        run(&mut state, WHOOSH_S);
        assert_eq!(state.view().shake_px, (0.0, 0.0));
    }

    #[test]
    fn hit_stop_freezes_for_its_exact_time_then_releases() {
        let mut state = FeelState::new();
        state.impact(ImpactEffect {
            class: ImpactClass::Heavy,
            weight: 0.7,
            on_hero: false,
        });
        let stop = hit_stop_s(ImpactClass::Heavy);
        assert_eq!(state.time_scale(), 0.0);
        let mut frozen = 0.0;
        // Advance in 1/30 s frames and count the seconds the stage was frozen.
        for _ in 0..30 {
            if state.time_scale() == 0.0 {
                frozen += 1.0 / 30.0;
            }
            state.advance(1.0 / 30.0);
        }
        assert!(
            (frozen - stop).abs() <= 1.0 / 30.0 + 1e-4,
            "{frozen} vs {stop}"
        );
        assert_eq!(state.time_scale(), 1.0);
        assert!(hit_stop_s(ImpactClass::Critical) > hit_stop_s(ImpactClass::Heavy));
        assert_eq!(hit_stop_s(ImpactClass::Graze), 0.0);
    }

    #[test]
    fn slow_motion_ramps_down_holds_and_returns_to_full_speed() {
        assert_eq!(slowmo_scale(-0.1), 1.0);
        assert_eq!(slowmo_scale(0.0), 1.0);
        assert!(slowmo_scale(SLOWMO_RAMP_IN_S * 0.5) < 1.0);
        assert_eq!(slowmo_scale(SLOWMO_RAMP_IN_S + 0.1), SLOWMO_SCALE);
        assert!(slowmo_scale(slowmo_duration_s() - 0.05) > SLOWMO_SCALE);
        assert_eq!(slowmo_scale(slowmo_duration_s() + 0.01), 1.0);
        // Never faster than normal, never below the floor.
        for step in 0..200 {
            let scale = slowmo_scale(step as f32 * 0.01);
            assert!((SLOWMO_SCALE..=1.0).contains(&scale), "{scale}");
        }
        let mut state = FeelState::new();
        state.slow_motion();
        run(&mut state, SLOWMO_RAMP_IN_S + 0.1);
        assert!((state.time_scale() - SLOWMO_SCALE).abs() < 1e-5);
    }

    #[test]
    fn the_camera_sways_slow_and_small_when_calm_and_quicker_at_peak() {
        let swing = |intensity: u16| {
            let mut widest = 0.0f32;
            let mut turns = 0;
            let mut last = sway(0.0, intensity, false).yaw_deg;
            let mut last_rising = true;
            for step in 1..=6000 {
                let value = sway(step as f32 * 0.01, intensity, false).yaw_deg;
                widest = widest.max(value.abs());
                let rising = value >= last;
                if rising != last_rising {
                    turns += 1;
                }
                last = value;
                last_rising = rising;
            }
            (widest, turns)
        };
        let (calm_width, calm_turns) = swing(0);
        let (peak_width, peak_turns) = swing(1000);
        assert!(calm_turns < peak_turns, "{calm_turns} vs {peak_turns}");
        assert!(peak_width >= calm_width * 0.9);
        assert!(calm_width > 0.1 && peak_width < 0.8, "small either way");
        // Tempo beyond 1000 behaves as 1000.
        assert_eq!(sway(3.0, 5000, false), sway(3.0, 1000, false));
    }

    #[test]
    fn the_wide_shot_drifts_in_orbit_and_other_shots_do_not() {
        let drift = (0..4000)
            .map(|step| {
                sway(step as f32 * 0.5, 0, true).yaw_deg - sway(step as f32 * 0.5, 0, false).yaw_deg
            })
            .fold(0.0f32, |widest, value| widest.max(value.abs()));
        assert!(
            drift > ORBIT_DRIFT_DEG * 0.9 && drift <= ORBIT_DRIFT_DEG + 1e-3,
            "{drift}"
        );
    }

    #[test]
    fn motion_blur_follows_camera_speed_with_a_floor_and_a_cap() {
        assert_eq!(motion_blur_px(0.0), 0.0);
        assert_eq!(motion_blur_px(MOTION_BLUR_FLOOR_DEG_S), 0.0);
        assert!(motion_blur_px(60.0) > motion_blur_px(30.0));
        assert_eq!(motion_blur_px(100_000.0), MOTION_BLUR_MAX_PX);
        let mut state = FeelState::new();
        for _ in 0..60 {
            state.observe_camera_speed(120.0, 0.0, 1.0 / 30.0);
        }
        let view = state.view();
        // A fast pan blurs strongly, up to the cap.
        assert!(view.motion_blur_px > MOTION_BLUR_MAX_PX * 0.5);
        assert!(view.motion_blur_dir.0 > 0.99 && view.motion_blur_dir.1.abs() < 0.01);
        for _ in 0..90 {
            state.observe_camera_speed(0.0, 0.0, 1.0 / 30.0);
        }
        assert_eq!(state.view().motion_blur_px, 0.0);
    }

    #[test]
    fn critical_and_heavy_blows_punch_split_and_a_hero_hit_pulses_the_vignette() {
        let mut state = FeelState::new();
        state.impact(ImpactEffect {
            class: ImpactClass::Critical,
            weight: 0.9,
            on_hero: true,
        });
        let view = state.view();
        assert!(view.radial_blur > 0.0 && view.chromatic_px >= CHROMA_CRITICAL_PX - 1e-3);
        assert!(view.vignette_pulse > 0.0);
        run(&mut state, 1.0);
        let view = state.view();
        assert_eq!(
            (view.radial_blur, view.chromatic_px, view.vignette_pulse),
            (0.0, 0.0, 0.0)
        );
        let mut graze = FeelState::new();
        graze.impact(ImpactEffect {
            class: ImpactClass::Graze,
            weight: 0.1,
            on_hero: true,
        });
        let view = graze.view();
        assert_eq!(
            (view.radial_blur, view.chromatic_px, view.vignette_pulse),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn a_focus_pull_blurs_then_settles() {
        let mut state = FeelState::new();
        assert_eq!(state.view().focus_blur_factor, 1.0);
        state.focus_pull();
        assert!((state.view().focus_blur_factor - FOCUS_PULL_BLUR_FACTOR).abs() < 1e-4);
        run(&mut state, FOCUS_PULL_S + 0.02);
        assert_eq!(state.view().focus_blur_factor, 1.0);
    }

    #[test]
    fn effects_are_functions_of_seconds_not_frames() {
        // The same blow, advanced in 24 fps and 30 fps frames, agrees at every shared instant.
        let effect = ImpactEffect {
            class: ImpactClass::Heavy,
            weight: 0.75,
            on_hero: true,
        };
        let sample = |frames_per_second: u32, seconds_in: f32| {
            let mut state = FeelState::new();
            state.impact(effect);
            let frames = (seconds_in * frames_per_second as f32).round() as u32;
            for _ in 0..frames {
                state.advance(1.0 / frames_per_second as f32);
            }
            state.view()
        };
        // 24 and 30 fps frames meet every sixth of a second (4 frames against 5): there the two
        // clocks are identical, so the effects must be too.
        for sixths in 1..=4 {
            let seconds = sixths as f32 / 6.0;
            let at_24 = sample(24, seconds);
            let at_30 = sample(30, seconds);
            assert!(
                (at_24.shake_px.0 - at_30.shake_px.0).abs() < 1e-3,
                "{seconds}"
            );
            assert!(
                (at_24.shake_px.1 - at_30.shake_px.1).abs() < 1e-3,
                "{seconds}"
            );
            assert!(
                (at_24.shake_roll_deg - at_30.shake_roll_deg).abs() < 1e-4,
                "{seconds}"
            );
            assert!(
                (at_24.chromatic_px - at_30.chromatic_px).abs() < 1e-3,
                "{seconds}"
            );
            assert!(
                (at_24.vignette_pulse - at_30.vignette_pulse).abs() < 1e-4,
                "{seconds}"
            );
        }
        // The sway is a pure function of the clock.
        assert_eq!(sway(2.5, 400, false), sway(2.5, 400, false));
    }

    #[test]
    fn overlapping_shakes_stay_bounded() {
        let mut state = FeelState::new();
        for _ in 0..40 {
            state.impact(ImpactEffect {
                class: ImpactClass::Knockout,
                weight: 1.0,
                on_hero: false,
            });
            state.advance(0.01);
            let view = state.view();
            assert!(view.shake_px.0.abs() <= SHAKE_MAX_PX && view.shake_px.1.abs() <= SHAKE_MAX_PX);
            assert!(view.shake_roll_deg.abs() <= 2.0);
        }
    }
}
