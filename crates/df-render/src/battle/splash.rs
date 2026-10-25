//! When a fight's loading splash holds and when it lets go. The splash covers the battlefield
//! from the moment it mounts until the scan at the chosen level is fully resolved, its art has
//! arrived and the figures stand on their cells; then one transition reveals the finished scene.
//! Nothing of the fight (its banner, dice, turns) is played while the splash holds: the stage
//! clock waits behind it. This module is the pure rule (testable natively); the host draws it.

/// How long the splash fades out once the scene is resolved, in milliseconds.
pub const FADE_MS: f64 = 700.0;
/// Frames drawn with everything resolved before the reveal starts, so the first revealed frame
/// is the finished one (the last plan sorted and drawn).
pub const SETTLE_FRAMES: u64 = 3;
/// The splash never holds longer than this, in milliseconds: a scan that cannot finish (a lost
/// connection, a missing chunk) must not trap the table behind it. The scene shows what it has.
pub const MAX_HOLD_MS: f64 = 45_000.0;

/// What the splash is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Covering the battlefield; the fight waits.
    Holding,
    /// The scene is resolved: the cover fades out over the finished scene.
    Revealing,
    /// The cover is gone.
    Gone,
}

/// What the scene has finished, as the host reads it each frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Readiness {
    /// The scan is fully resolved at the chosen level (or there is none, or it failed and the
    /// painted ground stands in).
    pub scan: bool,
    /// The art the frame needs (figures, props) has arrived.
    pub art: bool,
    /// The figures are on their cells.
    pub figures: bool,
}

impl Readiness {
    pub fn all(self) -> bool {
        self.scan && self.art && self.figures
    }
}

/// The splash's clock and state.
#[derive(Clone, Debug)]
pub struct Gate {
    started_ms: f64,
    settled_frames: u64,
    revealed_at_ms: Option<f64>,
}

impl Gate {
    pub fn new(now_ms: f64) -> Self {
        Self {
            started_ms: now_ms,
            settled_frames: 0,
            revealed_at_ms: None,
        }
    }

    /// Called once per drawn frame. A scene that has stopped being ready (a new chunk wanted)
    /// starts its settling over until the reveal has begun; once revealing it never goes back.
    pub fn observe(&mut self, now_ms: f64, ready: Readiness) -> Phase {
        if self.revealed_at_ms.is_none() {
            if ready.all() {
                self.settled_frames += 1;
            } else {
                self.settled_frames = 0;
            }
            if self.settled_frames >= SETTLE_FRAMES || now_ms - self.started_ms >= MAX_HOLD_MS {
                self.revealed_at_ms = Some(now_ms);
            }
        }
        self.phase(now_ms)
    }

    pub fn phase(&self, now_ms: f64) -> Phase {
        match self.revealed_at_ms {
            None => Phase::Holding,
            Some(revealed_at_ms) if now_ms - revealed_at_ms < FADE_MS => Phase::Revealing,
            Some(_) => Phase::Gone,
        }
    }

    /// Whether the scene is resolved: the signal the display's battle machine waits on.
    pub fn resolved(&self) -> bool {
        self.revealed_at_ms.is_some()
    }
}

/// How far the scan has got, from 0 to 1, from the real chunk count. Complete is exactly 1; until
/// then it never reads full, so a bar at the end means done.
pub fn progress(loaded: u32, wanted: u32, complete: bool) -> f32 {
    if complete {
        return 1.0;
    }
    if wanted == 0 {
        return 0.0;
    }
    (loaded as f32 / wanted as f32).min(0.99)
}

/// The words under the bar.
pub fn label(loaded: u32, wanted: u32, complete: bool) -> String {
    if wanted == 0 && !complete {
        return "Preparing the field".to_owned();
    }
    if complete {
        return "The field is ready".to_owned();
    }
    format!(
        "Raising the field  {:.0}%",
        progress(loaded, wanted, false) * 100.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: Readiness = Readiness {
        scan: true,
        art: true,
        figures: true,
    };

    #[test]
    fn the_splash_holds_until_the_scan_the_art_and_the_figures_are_all_ready() {
        let mut gate = Gate::new(0.0);
        for ready in [
            Readiness {
                scan: false,
                ..READY
            },
            Readiness {
                art: false,
                ..READY
            },
            Readiness {
                figures: false,
                ..READY
            },
        ] {
            for frame in 0..100 {
                assert_eq!(gate.observe(f64::from(frame) * 33.0, ready), Phase::Holding);
            }
            assert!(!gate.resolved());
        }
    }

    #[test]
    fn it_reveals_after_a_few_settled_frames_then_fades_and_is_gone() {
        let mut gate = Gate::new(0.0);
        let mut phases = Vec::new();
        for frame in 0..SETTLE_FRAMES {
            phases.push(gate.observe(1000.0 + frame as f64 * 33.0, READY));
        }
        assert_eq!(&phases[..2], &[Phase::Holding, Phase::Holding]);
        assert_eq!(phases[2], Phase::Revealing);
        assert!(gate.resolved());
        assert_eq!(gate.phase(1066.0 + FADE_MS - 1.0), Phase::Revealing);
        assert_eq!(gate.phase(1066.0 + FADE_MS), Phase::Gone);
    }

    #[test]
    fn a_scene_that_stops_being_ready_restarts_its_settling_but_a_reveal_is_never_undone() {
        let mut gate = Gate::new(0.0);
        gate.observe(10.0, READY);
        gate.observe(20.0, READY);
        gate.observe(
            30.0,
            Readiness {
                scan: false,
                ..READY
            },
        );
        assert!(!gate.resolved());
        for frame in 0..SETTLE_FRAMES {
            gate.observe(40.0 + frame as f64, READY);
        }
        assert!(gate.resolved());
        gate.observe(900.0, Readiness::default());
        assert!(gate.resolved(), "once revealing it stays revealed");
    }

    #[test]
    fn a_scan_that_never_finishes_cannot_hold_the_table_forever() {
        let mut gate = Gate::new(0.0);
        assert_eq!(
            gate.observe(MAX_HOLD_MS - 1.0, Readiness::default()),
            Phase::Holding
        );
        assert_eq!(
            gate.observe(MAX_HOLD_MS, Readiness::default()),
            Phase::Revealing
        );
        assert!(gate.resolved());
    }

    #[test]
    fn progress_follows_the_chunk_count_and_only_complete_reads_full() {
        assert_eq!(progress(0, 0, false), 0.0);
        assert_eq!(progress(45, 90, false), 0.5);
        assert!(progress(90, 90, false) < 1.0);
        assert_eq!(progress(90, 90, true), 1.0);
        assert_eq!(label(45, 90, false), "Raising the field  50%");
        assert_eq!(label(0, 0, false), "Preparing the field");
        assert_eq!(label(90, 90, true), "The field is ready");
    }
}
