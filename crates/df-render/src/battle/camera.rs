//! Orbit camera poses, named shots and the rig that eases between them. Ported from the earlier
//! build's `camera_motion.mjs`: eased pan of at most five seconds, impact shake of at most 200 ms
//! and 32 px. The rig owns no clock; callers pass the elapsed time to [`CameraRig::update`], so
//! tests drive it deterministically.

use std::time::Duration;

use super::math::{Mat4, Vec3, ease_in_out, lerp};

pub const MAX_PAN: Duration = Duration::from_secs(5);
pub const MAX_SHAKE: Duration = Duration::from_millis(200);
pub const MAX_SHAKE_PX: f32 = 32.0;

/// The named shots of the earlier protocol. Their poses come from the scene file.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ShotName {
    Tactical,
    CombatEst,
    TurnFocus,
    Impact,
    Ko,
    Victory,
}

impl ShotName {
    pub const ALL: [Self; 6] = [
        Self::Tactical,
        Self::CombatEst,
        Self::TurnFocus,
        Self::Impact,
        Self::Ko,
        Self::Victory,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Tactical => "tactical",
            Self::CombatEst => "combat_est",
            Self::TurnFocus => "turn_focus",
            Self::Impact => "impact",
            Self::Ko => "ko",
            Self::Victory => "victory",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shot| shot.key() == key)
    }
}

/// A camera orbiting `target`. `yaw_deg` 0 puts the camera on the +Z side looking toward -Z;
/// positive pitch looks down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub target: Vec3,
    pub distance: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub fov_deg: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoseError {
    NotFinite,
    DistanceOutOfRange,
    PitchOutOfRange,
    FovOutOfRange,
}

impl CameraPose {
    pub fn validate(&self) -> Result<(), PoseError> {
        let values = [
            self.target.x,
            self.target.y,
            self.target.z,
            self.distance,
            self.yaw_deg,
            self.pitch_deg,
            self.fov_deg,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(PoseError::NotFinite);
        }
        if !(0.5..=200.0).contains(&self.distance) {
            return Err(PoseError::DistanceOutOfRange);
        }
        if !(2.0..=89.0).contains(&self.pitch_deg) {
            return Err(PoseError::PitchOutOfRange);
        }
        if !(10.0..=100.0).contains(&self.fov_deg) {
            return Err(PoseError::FovOutOfRange);
        }
        Ok(())
    }

    pub fn eye(&self) -> Vec3 {
        let yaw = self.yaw_deg.to_radians();
        let pitch = self.pitch_deg.to_radians();
        self.target
            + Vec3::new(
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            ) * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.eye(), self.target, Vec3::UP)
    }

    pub fn projection(&self, aspect: f32, near: f32, far: f32) -> Mat4 {
        Mat4::perspective(self.fov_deg.to_radians(), aspect, near, far)
    }

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            target: self.target.lerp(other.target, t),
            distance: lerp(self.distance, other.distance, t),
            yaw_deg: lerp(self.yaw_deg, other.yaw_deg, t),
            pitch_deg: lerp(self.pitch_deg, other.pitch_deg, t),
            fov_deg: lerp(self.fov_deg, other.fov_deg, t),
        }
    }

    /// The same angle, distance and lens aimed at `focus` (the earlier `focus_token_id`).
    pub fn aimed_at(&self, focus: Vec3) -> Self {
        Self {
            target: focus,
            ..*self
        }
    }

    /// The same shot pushed in by `factor` (below one is closer).
    pub fn dollied(&self, factor: f32) -> Self {
        Self {
            distance: self.distance * factor,
            ..*self
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pan {
    from: CameraPose,
    to: CameraPose,
    elapsed: f32,
    duration: f32,
    /// A snappy move: overshoots a little and ends with a zoom punch.
    whip: bool,
}

/// How far a whip overshoots its target before settling (as a share of the travel).
const WHIP_OVERSHOOT: f32 = 1.45;
/// How long the zoom punch after a whip lasts, and how much the lens tightens at its peak.
const PUNCH: f32 = 0.2;
const PUNCH_FOV_SHARE: f32 = 0.09;

/// Ease out with a slight overshoot (the standard "back" curve): reaches 1 at `t = 1`.
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    1.0 + (WHIP_OVERSHOOT + 1.0) * t * t * t + WHIP_OVERSHOOT * t * t
}

#[derive(Clone, Copy, Debug)]
struct Shake {
    elapsed: f32,
    duration: f32,
    amplitude_px: f32,
}

#[derive(Clone, Debug)]
pub struct CameraRig {
    pose: CameraPose,
    punch: f32,
    pan: Option<Pan>,
    shake: Option<Shake>,
}

impl CameraRig {
    pub fn new(pose: CameraPose) -> Self {
        Self {
            pose,
            punch: 0.0,
            pan: None,
            shake: None,
        }
    }

    /// The current pose, with the lens tightened while a whip's zoom punch plays.
    pub fn pose(&self) -> CameraPose {
        if self.punch <= 0.0 {
            return self.pose;
        }
        let progress = 1.0 - self.punch / PUNCH;
        CameraPose {
            fov_deg: self.pose.fov_deg
                * (1.0 - PUNCH_FOV_SHARE * (std::f32::consts::PI * progress).sin()),
            ..self.pose
        }
    }

    pub fn is_panning(&self) -> bool {
        self.pan.is_some()
    }

    /// Jumps straight to `pose`, cancelling any pan and shake.
    pub fn cut_to(&mut self, pose: CameraPose) {
        self.pose = pose;
        self.pan = None;
        self.shake = None;
        self.punch = 0.0;
    }

    /// A fast, snappy move to `pose`: `duration` of travel with a slight overshoot and settle,
    /// then a short zoom punch on arrival. A zero duration is a cut.
    pub fn whip_to(&mut self, pose: CameraPose, duration: Duration) {
        let duration = duration.min(MAX_PAN);
        if duration.is_zero() {
            self.cut_to(pose);
            return;
        }
        self.shake = None;
        self.punch = 0.0;
        self.pan = Some(Pan {
            from: self.pose,
            to: pose,
            elapsed: 0.0,
            duration: duration.as_secs_f32(),
            whip: true,
        });
    }

    /// Eases to `pose` over `duration` (clamped to [`MAX_PAN`]). A zero duration is a cut.
    pub fn pan_to(&mut self, pose: CameraPose, duration: Duration) {
        let duration = duration.min(MAX_PAN);
        if duration.is_zero() {
            self.cut_to(pose);
            return;
        }
        self.shake = None;
        self.pan = Some(Pan {
            from: self.pose,
            to: pose,
            elapsed: 0.0,
            duration: duration.as_secs_f32(),
            whip: false,
        });
    }

    /// Starts an impact shake. Amplitude and duration are clamped.
    pub fn shake(&mut self, amplitude_px: f32, duration: Duration) {
        let amplitude = if amplitude_px.is_finite() {
            amplitude_px.clamp(0.0, MAX_SHAKE_PX)
        } else {
            0.0
        };
        let duration = duration.min(MAX_SHAKE);
        if amplitude == 0.0 || duration.is_zero() {
            return;
        }
        self.shake = Some(Shake {
            elapsed: 0.0,
            duration: duration.as_secs_f32(),
            amplitude_px: amplitude,
        });
    }

    /// Screen-space shake offset in pixels for this instant.
    pub fn shake_offset_px(&self) -> (f32, f32) {
        let Some(shake) = self.shake else {
            return (0.0, 0.0);
        };
        let phase = shake.elapsed * 77.0;
        let fade = (1.0 - shake.elapsed / shake.duration).max(0.0);
        (
            phase.sin() * shake.amplitude_px * fade,
            (phase * 1.37 + 0.7).cos() * shake.amplitude_px * 0.72 * fade,
        )
    }

    pub fn update(&mut self, elapsed: Duration) {
        let dt = elapsed.as_secs_f32().min(1.0);
        if let Some(mut pan) = self.pan.take() {
            pan.elapsed = (pan.elapsed + dt).min(pan.duration);
            let progress = pan.elapsed / pan.duration;
            let eased = if pan.whip {
                ease_out_back(progress)
            } else {
                ease_in_out(progress)
            };
            self.pose = pan.from.lerp(&pan.to, eased);
            if pan.elapsed < pan.duration {
                self.pan = Some(pan);
            } else if pan.whip {
                self.punch = PUNCH + dt;
            }
        }
        self.punch = (self.punch - dt).max(0.0);
        if let Some(mut shake) = self.shake.take() {
            shake.elapsed += dt;
            if shake.elapsed < shake.duration {
                self.shake = Some(shake);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(distance: f32, yaw: f32) -> CameraPose {
        CameraPose {
            target: Vec3::ZERO,
            distance,
            yaw_deg: yaw,
            pitch_deg: 30.0,
            fov_deg: 40.0,
        }
    }

    #[test]
    fn eye_sits_on_the_positive_z_side_at_zero_yaw() {
        let eye = pose(10.0, 0.0).eye();
        assert!(eye.z > 0.0 && eye.x.abs() < 1e-4);
        assert!((eye.y - 5.0).abs() < 1e-3);
        assert!(((eye - Vec3::ZERO).length() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn pose_validation_rejects_unusable_values() {
        assert_eq!(pose(10.0, 0.0).validate(), Ok(()));
        assert_eq!(
            pose(0.1, 0.0).validate(),
            Err(PoseError::DistanceOutOfRange)
        );
        assert_eq!(pose(f32::NAN, 0.0).validate(), Err(PoseError::NotFinite));
        let mut flat = pose(10.0, 0.0);
        flat.pitch_deg = 0.0;
        assert_eq!(flat.validate(), Err(PoseError::PitchOutOfRange));
        let mut wide = pose(10.0, 0.0);
        wide.fov_deg = 170.0;
        assert_eq!(wide.validate(), Err(PoseError::FovOutOfRange));
    }

    #[test]
    fn pan_eases_with_flat_ends_and_lands_exactly_on_the_target_pose() {
        let mut rig = CameraRig::new(pose(10.0, 0.0));
        rig.pan_to(pose(20.0, 40.0), Duration::from_secs(2));
        rig.update(Duration::from_millis(100));
        let early = rig.pose().distance;
        assert!(early > 10.0 && early < 10.2, "slow start, got {early}");
        rig.update(Duration::from_millis(900));
        assert!((rig.pose().distance - 15.0).abs() < 0.01);
        rig.update(Duration::from_secs(5));
        assert_eq!(rig.pose(), pose(20.0, 40.0));
        assert!(!rig.is_panning());
    }

    #[test]
    fn pan_duration_is_capped_at_five_seconds() {
        let mut rig = CameraRig::new(pose(10.0, 0.0));
        rig.pan_to(pose(20.0, 0.0), Duration::from_secs(60));
        rig.update(Duration::from_secs(1));
        rig.update(Duration::from_secs(1));
        rig.update(Duration::from_secs(1));
        rig.update(Duration::from_secs(1));
        assert!(rig.is_panning());
        rig.update(Duration::from_secs(1));
        assert!(!rig.is_panning());
    }

    #[test]
    fn shake_is_bounded_in_amplitude_and_duration_and_decays() {
        let mut rig = CameraRig::new(pose(10.0, 0.0));
        rig.shake(500.0, Duration::from_secs(10));
        let mut peak = 0.0f32;
        let mut ticks = 0;
        while rig.shake_offset_px() != (0.0, 0.0) && ticks < 100 {
            let (x, y) = rig.shake_offset_px();
            peak = peak.max(x.abs()).max(y.abs());
            rig.update(Duration::from_millis(10));
            ticks += 1;
        }
        assert!(peak <= MAX_SHAKE_PX, "peak {peak}");
        assert!(ticks <= 21, "lasted {ticks} ticks of 10 ms");
        assert_eq!(rig.shake_offset_px(), (0.0, 0.0));
    }

    #[test]
    fn aimed_and_dollied_poses_keep_the_other_parameters() {
        let base = pose(10.0, 20.0);
        let focus = Vec3::new(3.0, 1.0, -2.0);
        let aimed = base.aimed_at(focus);
        assert_eq!(aimed.target, focus);
        assert_eq!(aimed.yaw_deg, 20.0);
        assert_eq!(base.dollied(0.5).distance, 5.0);
    }

    #[test]
    fn shot_keys_round_trip() {
        for shot in ShotName::ALL {
            assert_eq!(ShotName::from_key(shot.key()), Some(shot));
        }
        assert_eq!(ShotName::from_key("nonsense"), None);
    }

    #[test]
    fn a_whip_overshoots_a_little_settles_exactly_and_punches_the_lens() {
        let mut rig = CameraRig::new(pose(10.0, 0.0));
        rig.whip_to(pose(20.0, 0.0), Duration::from_millis(300));
        let mut peak = 0.0f32;
        let mut distances = Vec::new();
        for _ in 0..31 {
            rig.update(Duration::from_millis(10));
            peak = peak.max(rig.pose().distance);
            distances.push(rig.pose().distance);
        }
        assert!(
            peak > 20.2 && peak < 21.5,
            "overshoot of a few percent, got {peak}"
        );
        assert!(!rig.is_panning());
        // Arrived: the base pose is exact once the punch is over.
        let mut narrowest = f32::MAX;
        for _ in 0..40 {
            rig.update(Duration::from_millis(10));
            narrowest = narrowest.min(rig.pose().fov_deg);
        }
        assert!(
            narrowest < 40.0 * 0.95,
            "the lens tightens on arrival: {narrowest}"
        );
        assert_eq!(rig.pose(), pose(20.0, 0.0));
        // It travels fast: more than half the way in the first third of the time.
        assert!(distances[9] > 15.0, "snappy start: {}", distances[9]);
    }
}
