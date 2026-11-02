//! Travel: the component that moves an entity toward a point at a speed, and the system that
//! advances it one tick.
//!
//! It is pure arithmetic on positions. The clock that decides how long a tick is belongs to the
//! host; this module only refuses a tick that would be absurd (a stalled host waking up must not
//! teleport everything across the canvas in one step) and never reads time itself.

use std::fmt;

use crate::ecs::{Entity, Store};
use crate::element::Position;

/// Slowest travel accepted, in canvas units per second.
pub const MINIMUM_SPEED: f32 = 0.1;
/// Fastest travel accepted, in canvas units per second: the whole canvas in a quarter second.
pub const MAXIMUM_SPEED: f32 = 400.0;
/// Longest tick the system will advance, in seconds. A longer gap is cut to this.
pub const MAXIMUM_TICK_SECONDS: f32 = 0.25;

/// A speed that has been checked: finite and within the limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed(f32);

/// Why a speed was refused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedError(pub f32);

impl fmt::Display for SpeedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a speed is from {MINIMUM_SPEED} to {MAXIMUM_SPEED} canvas units per second, not {}",
            self.0
        )
    }
}

impl Speed {
    pub fn new(units_per_second: f32) -> Result<Self, SpeedError> {
        if units_per_second.is_finite()
            && (MINIMUM_SPEED..=MAXIMUM_SPEED).contains(&units_per_second)
        {
            Ok(Self(units_per_second))
        } else {
            Err(SpeedError(units_per_second))
        }
    }

    pub fn units_per_second(self) -> f32 {
        self.0
    }
}

/// An entity on its way to `target`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub target: Position,
    pub speed: Speed,
}

/// What one tick of the motion system did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MotionReport {
    /// Entities whose position changed.
    pub moved: usize,
    /// Entities that reached their target; their motion is gone.
    pub arrived: Vec<Entity>,
}

/// Moves every entity that has a motion toward its target by `seconds` of travel. An entity that
/// would pass its target stops on it. A tick that is not a finite non-negative number, or is longer
/// than `MAXIMUM_TICK_SECONDS`, is cut to the nearest allowed value.
pub fn advance(
    positions: &mut Store<Position>,
    motions: &mut Store<Motion>,
    seconds: f32,
) -> MotionReport {
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, MAXIMUM_TICK_SECONDS)
    } else {
        0.0
    };
    let mut report = MotionReport::default();
    if seconds == 0.0 {
        return report;
    }
    for (entity, motion) in motions.iter() {
        let Some(position) = positions.get_mut(entity) else {
            continue;
        };
        let (dx, dy) = (motion.target.x - position.x, motion.target.y - position.y);
        let distance = dx.hypot(dy);
        let step = motion.speed.units_per_second() * seconds;
        if distance <= step {
            if *position != motion.target {
                *position = motion.target;
                report.moved += 1;
            }
            report.arrived.push(entity);
        } else {
            position.x += dx / distance * step;
            position.y += dy / distance * step;
            report.moved += 1;
        }
    }
    for entity in &report.arrived {
        motions.remove(*entity);
    }
    report
}
