//! Bounded, asset-independent practical magic. The GPU draws soft flame volumes and a thin
//! refracting ward from these samples; a missing media asset never hides a committed result.
use std::time::Duration;

use super::{grid::Cell, math::Vec3, model::TokenId};

pub(crate) const MAX_EFFECTS: usize = 24;
pub(crate) const MAX_AREA_TARGETS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectKind {
    Bolt,
    Dart,
    Bead,
    Flare,
    Burst,
    Ward,
    Surge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectAnchor {
    Token(TokenId),
    Cell(Cell),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectCue {
    pub kind: EffectKind,
    pub from: EffectAnchor,
    pub to: EffectAnchor,
    pub duration: Duration,
    pub radius_m: f32,
    /// Signed lateral arc in metres: the three missile paths stay distinct even on one target.
    pub fan_m: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireLight {
    /// xyz and falloff radius in metres. A zero radius disables the spill.
    pub position_radius: [f32; 4],
    /// Warm RGB radiant tint and intensity, shared by figures and both splat backends.
    pub colour_strength: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectView {
    pub kind: EffectKind,
    pub position: Vec3,
    pub radius_m: f32,
    pub age_s: f32,
    pub opacity: f32,
}

pub(crate) struct ActiveEffect {
    pub cue: EffectCue,
    pub from: Vec3,
    pub to: Vec3,
    pub age_s: f32,
}

impl ActiveEffect {
    pub fn view(&self) -> EffectView {
        let t = (self.age_s / self.cue.duration.as_secs_f32().max(0.001)).clamp(0.0, 1.0);
        let moving = matches!(
            self.cue.kind,
            EffectKind::Bolt | EffectKind::Dart | EffectKind::Bead
        );
        let direction = (self.to - self.from).normalized();
        let lateral = Vec3::new(-direction.z, 0.0, direction.x);
        let position = if moving {
            self.from
                + (self.to - self.from) * t
                + lateral * (self.cue.fan_m * (std::f32::consts::PI * t).sin())
                + Vec3::UP * (self.cue.fan_m.abs() * 0.25 * (std::f32::consts::PI * t).sin())
        } else {
            self.to
        };
        let radius_m = if self.cue.kind == EffectKind::Burst {
            // The fire front grows over 360 ms, followed by rolling flame and smoke.
            self.cue.radius_m * (self.age_s / 0.36).clamp(0.03, 1.0)
        } else {
            self.cue.radius_m
        };
        EffectView {
            kind: self.cue.kind,
            position,
            radius_m,
            age_s: self.age_s,
            opacity: if moving {
                1.0
            } else {
                (t * 12.0).min(1.0) * (1.0 - t).powf(0.65)
            },
        }
    }
}

/// One strongest warm source keeps relighting constant-cost, with no per-splat light loop.
pub(crate) fn fire_light(effects: &[EffectView]) -> FireLight {
    effects
        .iter()
        .filter_map(|effect| {
            let (radius, strength) = match effect.kind {
                EffectKind::Bolt => (2.8, 0.65),
                EffectKind::Bead => (1.8, 0.45),
                EffectKind::Flare => (4.0, 1.1),
                EffectKind::Burst => (effect.radius_m * 1.8, 1.65),
                _ => return None,
            };
            Some(FireLight {
                position_radius: [
                    effect.position.x,
                    effect.position.y,
                    effect.position.z,
                    radius,
                ],
                colour_strength: [1.0, 0.37, 0.075, strength * effect.opacity],
            })
        })
        .max_by(|a, b| a.colour_strength[3].total_cmp(&b.colour_strength[3]))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(kind: EffectKind, fan_m: f32) -> ActiveEffect {
        ActiveEffect {
            cue: EffectCue {
                kind,
                from: EffectAnchor::Cell(Cell::new(0, 0)),
                to: EffectAnchor::Cell(Cell::new(1, 0)),
                duration: Duration::from_secs(1),
                radius_m: 2.0,
                fan_m,
            },
            from: Vec3::ZERO,
            to: Vec3::new(4.0, 0.0, 0.0),
            age_s: 0.5,
        }
    }
    #[test]
    fn technique_missile_paths_fan_out_then_meet_their_destinations() {
        let mut darts = [-0.85, 0.0, 0.85].map(|fan| effect(EffectKind::Dart, fan));
        assert!(darts[0].view().position.z < darts[1].view().position.z);
        assert!(darts[1].view().position.z < darts[2].view().position.z);
        for dart in &mut darts {
            dart.age_s = 1.0;
            assert!((dart.view().position - dart.to).length() < 0.001);
        }
    }
    #[test]
    fn technique_light_has_a_finite_radius_and_expires_with_the_effect() {
        let burst = effect(EffectKind::Burst, 0.0).view();
        assert!(fire_light(&[burst]).colour_strength[3] > 0.0);
        assert!(fire_light(&[burst]).position_radius[3] < 10.0);
        assert_eq!(fire_light(&[]), FireLight::default());
    }
}
