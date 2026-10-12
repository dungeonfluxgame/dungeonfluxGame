//! Presentation cadence for the battlefield: a fixed 30 fps (24 fps selectable), never higher.
//!
//! The browser calls the frame loop at the display's refresh rate. [`FramePacer`] decides, for each
//! call, whether the battlefield presents a frame or leaves the canvas alone, with a time
//! accumulator so presented frames fall at even intervals and the long-run rate is exact. Only the
//! presentation rate is capped: the caller still advances the stage by the real time since the
//! previous presented frame, so walks, camera moves, dice and projectiles keep their speed.
//!
//! When the page cannot present at the target (the display delivers frames late), the pacer drops
//! to the next even divisor of the target rate (30, 15, 10, 7.5) instead of drifting, and probes
//! the faster rate again after a while. The function is pure and takes the browser's timestamps,
//! so it is tested natively with simulated displays; browser timing is not trusted for assertions.

use std::collections::VecDeque;

/// The presentation rates the battlefield accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BattleFps {
    Fps24,
    Fps30,
}

impl BattleFps {
    /// The rate used when nothing is configured.
    pub const DEFAULT: Self = Self::Fps30;

    /// `24` or `30`; anything else is refused.
    pub fn from_hz(hz: u32) -> Option<Self> {
        match hz {
            24 => Some(Self::Fps24),
            30 => Some(Self::Fps30),
            _ => None,
        }
    }

    pub fn hz(self) -> u32 {
        match self {
            Self::Fps24 => 24,
            Self::Fps30 => 30,
        }
    }

    /// The time one frame lasts at this rate.
    pub fn period_ms(self) -> f64 {
        1000.0 / f64::from(self.hz())
    }
}

impl Default for BattleFps {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What the frame loop does with one animation-frame call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pace {
    /// Do no work: neither the stage nor the GPU is touched.
    Skip,
    /// Update and draw one frame. `elapsed_ms` is the real time since the previous presented frame
    /// (one period for the first), which is what the simulation advances by.
    Present { elapsed_ms: f64 },
}

/// A display tick shorter or longer than this is not a refresh interval (a stall, a duplicate).
const DISPLAY_INTERVAL_RANGE_MS: (f64, f64) = (2.0, 50.0);
/// A tick this many times the display estimate is a hitch and does not move the estimate.
const HITCH_FACTOR: f64 = 1.5;
/// Ticks that update the estimate quickly before it settles.
const DISPLAY_WARMUP_TICKS: u32 = 12;
const REFRESH_ESTIMATE_START_MS: f64 = 1000.0 / 60.0;
/// A frame may be presented this share of a display interval early, which absorbs timestamp
/// jitter; the carry keeps the long-run rate exact.
const EARLY_SHARE: f64 = 0.25;
/// The most carry kept between presents, as a share of a display interval: a late frame is
/// dropped, never made up with a burst.
const CARRY_SHARE: f64 = 0.6;
/// A presented interval longer than this many periods counts as late.
const LATE_FACTOR: f64 = 1.5;
/// Presented frames per window in which lateness is judged.
const WINDOW_PRESENTS: u32 = 24;
/// Late frames in a window that make the pacer step down one divisor.
const LATE_TO_STEP_DOWN: u32 = 12;
/// Late frames in a probe window that still allow keeping the faster rate.
const LATE_TO_KEEP_PROBE: u32 = 6;
/// The slowest the cadence goes: a quarter of the target rate.
const MAX_DIVISOR: u32 = 4;
/// Presents at a reduced rate before the faster rate is tried again, doubled after each failure.
const PROBE_AFTER_PRESENTS: u32 = 240;
const PROBE_AFTER_PRESENTS_MAX: u32 = 1920;
/// Presented frames remembered for the measured rate (about one second at 30 fps).
const RATE_HISTORY: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Settled at the current divisor.
    Steady,
    /// Trying the next faster divisor for one window.
    Probing,
}

#[derive(Clone, Debug)]
pub struct FramePacer {
    target: BattleFps,
    divisor: u32,
    phase: Phase,
    accumulator_ms: f64,
    last_tick_ms: Option<f64>,
    last_present_ms: Option<f64>,
    display_ms: f64,
    display_ticks: u32,
    window_presents: u32,
    window_late: u32,
    presents_since_change: u32,
    probe_after: u32,
    history: VecDeque<f64>,
}

impl FramePacer {
    pub fn new(target: BattleFps) -> Self {
        Self {
            target,
            divisor: 1,
            phase: Phase::Steady,
            // Start owing one period, so the first call presents.
            accumulator_ms: target.period_ms(),
            last_tick_ms: None,
            last_present_ms: None,
            display_ms: REFRESH_ESTIMATE_START_MS,
            display_ticks: 0,
            window_presents: 0,
            window_late: 0,
            presents_since_change: 0,
            probe_after: PROBE_AFTER_PRESENTS,
            history: VecDeque::with_capacity(RATE_HISTORY),
        }
    }

    pub fn target(&self) -> BattleFps {
        self.target
    }

    /// The divisor of the target rate in force: 1 is the target, 2 half of it, and so on.
    pub fn divisor(&self) -> u32 {
        self.divisor
    }

    /// The rate the pacer is aiming for now, after any step down.
    pub fn effective_fps(&self) -> f32 {
        self.target.hz() as f32 / self.divisor as f32
    }

    /// The display refresh interval the pacer has measured, in milliseconds.
    pub fn display_interval_ms(&self) -> f64 {
        self.display_ms
    }

    /// The rate actually presented over the last second or so; 0 until two frames are out.
    pub fn presented_fps(&self) -> f32 {
        let (Some(first), Some(last)) = (self.history.front(), self.history.back()) else {
            return 0.0;
        };
        let span = last - first;
        if self.history.len() < 2 || span <= 0.0 {
            return 0.0;
        }
        ((self.history.len() - 1) as f64 * 1000.0 / span) as f32
    }

    fn period_ms(&self) -> f64 {
        self.target.period_ms() * f64::from(self.divisor)
    }

    /// One animation-frame call, with the browser's timestamp in milliseconds.
    pub fn tick(&mut self, timestamp_ms: f64) -> Pace {
        let delta = self
            .last_tick_ms
            .map_or(0.0, |last| (timestamp_ms - last).max(0.0));
        self.last_tick_ms = Some(timestamp_ms);
        self.observe_display(delta);
        self.accumulator_ms += delta;
        let period = self.period_ms();
        if self.accumulator_ms + self.display_ms * EARLY_SHARE < period {
            return Pace::Skip;
        }
        let carry_limit = self.display_ms * CARRY_SHARE;
        self.accumulator_ms = (self.accumulator_ms - period).clamp(-carry_limit, carry_limit);
        let elapsed_ms = self
            .last_present_ms
            .map_or(period, |last| timestamp_ms - last);
        let had_previous = self.last_present_ms.is_some();
        self.last_present_ms = Some(timestamp_ms);
        if self.history.len() == RATE_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(timestamp_ms);
        if had_previous {
            self.judge(elapsed_ms > period * LATE_FACTOR);
        }
        Pace::Present { elapsed_ms }
    }

    fn observe_display(&mut self, delta: f64) {
        let (shortest, longest) = DISPLAY_INTERVAL_RANGE_MS;
        if !(shortest..=longest).contains(&delta) {
            return;
        }
        let settled = self.display_ticks >= DISPLAY_WARMUP_TICKS;
        if settled && delta > self.display_ms * HITCH_FACTOR {
            return;
        }
        let rate = if settled { 0.04 } else { 0.25 };
        self.display_ms += (delta - self.display_ms) * rate;
        self.display_ticks += 1;
    }

    /// Counts a presented frame as on time or late and moves the divisor at window ends.
    fn judge(&mut self, late: bool) {
        self.window_presents += 1;
        self.presents_since_change += 1;
        if late {
            self.window_late += 1;
        }
        if self.window_presents < WINDOW_PRESENTS {
            return;
        }
        let late_frames = self.window_late;
        self.window_presents = 0;
        self.window_late = 0;
        match self.phase {
            Phase::Steady => {
                if late_frames >= LATE_TO_STEP_DOWN && self.divisor < MAX_DIVISOR {
                    self.change_divisor(self.divisor + 1);
                } else if self.divisor > 1 && self.presents_since_change >= self.probe_after {
                    self.phase = Phase::Probing;
                    self.change_divisor(self.divisor - 1);
                }
            }
            Phase::Probing => {
                self.phase = Phase::Steady;
                if late_frames >= LATE_TO_KEEP_PROBE {
                    self.change_divisor(self.divisor + 1);
                    self.probe_after = (self.probe_after * 2).min(PROBE_AFTER_PRESENTS_MAX);
                } else if self.divisor == 1 {
                    self.probe_after = PROBE_AFTER_PRESENTS;
                }
            }
        }
    }

    fn change_divisor(&mut self, divisor: u32) {
        self.divisor = divisor;
        self.accumulator_ms = 0.0;
        self.window_presents = 0;
        self.window_late = 0;
        self.presents_since_change = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a simulated display of `hz` for `seconds` and returns the timestamps (ms) at which
    /// frames were presented, with `jitter_ms` of deterministic timestamp noise.
    fn run(
        target: BattleFps,
        hz: f64,
        seconds: f64,
        jitter_ms: f64,
        stall: Option<(f64, f64)>,
    ) -> (FramePacer, Vec<f64>) {
        let mut pacer = FramePacer::new(target);
        let tick_ms = 1000.0 / hz;
        let ticks = (seconds * hz) as u32;
        let mut shift = 0.0;
        let mut presents = Vec::new();
        for index in 0..ticks {
            let nominal = f64::from(index) * tick_ms;
            if let Some((at, length)) = stall
                && (nominal - at).abs() < tick_ms / 2.0
            {
                shift += length;
            }
            let noise = jitter_ms * (((index * 7919) % 13) as f64 / 6.0 - 1.0);
            let timestamp = 1000.0 + nominal + shift + noise;
            if matches!(pacer.tick(timestamp), Pace::Present { .. }) {
                presents.push(timestamp);
            }
        }
        (pacer, presents)
    }

    fn intervals(presents: &[f64]) -> Vec<f64> {
        presents.windows(2).map(|pair| pair[1] - pair[0]).collect()
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: {actual} not within {tolerance} of {expected}"
        );
    }

    #[test]
    fn only_24_and_30_are_accepted() {
        assert_eq!(BattleFps::from_hz(30), Some(BattleFps::Fps30));
        assert_eq!(BattleFps::from_hz(24), Some(BattleFps::Fps24));
        for refused in [0, 1, 25, 29, 31, 48, 60, 120, u32::MAX] {
            assert_eq!(BattleFps::from_hz(refused), None, "{refused}");
        }
        assert_eq!(BattleFps::DEFAULT, BattleFps::Fps30);
    }

    #[test]
    fn the_first_call_presents_with_one_period_elapsed() {
        let mut pacer = FramePacer::new(BattleFps::Fps30);
        let Pace::Present { elapsed_ms } = pacer.tick(5000.0) else {
            panic!("first call must present");
        };
        assert_close(elapsed_ms, 1000.0 / 30.0, 1e-9, "first elapsed");
    }

    #[test]
    fn thirty_on_sixty_hz_is_every_second_tick_at_even_intervals() {
        let (pacer, presents) = run(BattleFps::Fps30, 60.0, 10.0, 0.0, None);
        assert_close(presents.len() as f64, 300.0, 1.0, "frames in 10 s");
        for interval in intervals(&presents) {
            assert_close(interval, 1000.0 / 30.0, 0.01, "interval");
        }
        assert_close(f64::from(pacer.presented_fps()), 30.0, 0.1, "measured fps");
        assert_eq!(pacer.divisor(), 1);
    }

    #[test]
    fn twenty_four_on_sixty_hz_alternates_two_and_three_ticks() {
        let (_, presents) = run(BattleFps::Fps24, 60.0, 10.0, 0.0, None);
        assert_close(presents.len() as f64, 240.0, 1.0, "frames in 10 s");
        let tick = 1000.0 / 60.0;
        let steps: Vec<f64> = intervals(&presents)
            .into_iter()
            .map(|interval| (interval / tick).round())
            .collect();
        assert!(steps.iter().all(|step| *step == 2.0 || *step == 3.0));
        // The 3:2 pattern is the best a 60 Hz display can do for 24 fps: never two long in a row.
        assert!(steps.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn a_120_hz_display_gives_exactly_even_30_and_24() {
        let (_, thirty) = run(BattleFps::Fps30, 120.0, 10.0, 0.0, None);
        assert_close(thirty.len() as f64, 300.0, 1.0, "30 fps frames");
        for interval in intervals(&thirty) {
            assert_close(interval, 1000.0 / 30.0, 0.01, "30 fps interval");
        }
        let (_, twenty_four) = run(BattleFps::Fps24, 120.0, 10.0, 0.0, None);
        assert_close(twenty_four.len() as f64, 240.0, 1.0, "24 fps frames");
        for interval in intervals(&twenty_four) {
            assert_close(interval, 1000.0 / 24.0, 0.01, "24 fps interval");
        }
    }

    #[test]
    fn a_144_hz_display_is_exact_for_24_and_mixes_four_and_five_ticks_for_30() {
        let (_, twenty_four) = run(BattleFps::Fps24, 144.0, 10.0, 0.0, None);
        assert_close(twenty_four.len() as f64, 240.0, 1.0, "24 fps frames");
        for interval in intervals(&twenty_four) {
            assert_close(interval, 1000.0 / 24.0, 0.01, "24 fps interval");
        }
        let (_, thirty) = run(BattleFps::Fps30, 144.0, 10.0, 0.0, None);
        assert_close(thirty.len() as f64, 300.0, 1.0, "30 fps frames");
        let tick = 1000.0 / 144.0;
        for interval in intervals(&thirty) {
            let ticks = (interval / tick).round();
            assert!(ticks == 4.0 || ticks == 5.0, "{ticks} ticks");
        }
    }

    #[test]
    fn timestamp_jitter_does_not_make_intervals_uneven_or_move_the_rate() {
        for hz in [60.0, 120.0] {
            let (pacer, presents) = run(BattleFps::Fps30, hz, 20.0, 1.0, None);
            assert_close(presents.len() as f64, 600.0, 2.0, "frames in 20 s");
            for interval in intervals(&presents) {
                assert_close(
                    interval,
                    1000.0 / 30.0,
                    1000.0 / hz + 2.5,
                    "jittered interval",
                );
            }
            assert_eq!(pacer.divisor(), 1, "{hz} Hz");
        }
    }

    #[test]
    fn the_display_refresh_is_measured_from_the_ticks() {
        for hz in [60.0, 120.0, 144.0] {
            let (pacer, _) = run(BattleFps::Fps30, hz, 5.0, 0.0, None);
            assert_close(
                pacer.display_interval_ms(),
                1000.0 / hz,
                0.3,
                "display interval",
            );
        }
    }

    #[test]
    fn a_long_frame_costs_one_frame_and_does_not_burst_to_catch_up() {
        for hz in [60.0, 120.0] {
            let (pacer, presents) = run(BattleFps::Fps30, hz, 10.0, 0.0, Some((4000.0, 500.0)));
            let gaps = intervals(&presents);
            let longest = gaps.iter().copied().fold(0.0, f64::max);
            assert!(longest >= 500.0, "{hz} Hz: the stall shows as one gap");
            // No catch-up burst: nothing is presented sooner than most of a period after another.
            let shortest = gaps.iter().copied().fold(f64::MAX, f64::min);
            assert!(
                shortest >= 1000.0 / 30.0 * 0.7,
                "{hz} Hz: shortest gap {shortest}"
            );
            // After the stall the cadence is even again.
            for interval in gaps.iter().rev().take(60) {
                assert_close(
                    *interval,
                    1000.0 / 30.0,
                    1000.0 / hz + 0.5,
                    "recovered interval",
                );
            }
            assert_eq!(pacer.divisor(), 1, "one stall is not a slow page");
        }
    }

    #[test]
    fn a_page_that_cannot_keep_up_drops_to_the_next_even_divisor() {
        // A 15 Hz delivery cannot present 30 fps; 15 fps is the next even divisor.
        let (pacer, presents) = run(BattleFps::Fps30, 15.0, 20.0, 0.0, None);
        assert_eq!(pacer.divisor(), 2);
        assert_close(
            f64::from(pacer.effective_fps()),
            15.0,
            1e-6,
            "effective fps",
        );
        let tail = intervals(&presents);
        for interval in tail.iter().rev().take(20) {
            assert_close(
                *interval,
                1000.0 / 15.0,
                0.01,
                "steady interval at the lower rate",
            );
        }
    }

    #[test]
    fn the_faster_rate_is_tried_again_when_the_display_recovers() {
        let mut pacer = FramePacer::new(BattleFps::Fps30);
        let mut now = 1000.0;
        // Slow delivery first, then a healthy 60 Hz.
        for _ in 0..120 {
            now += 1000.0 / 15.0;
            pacer.tick(now);
        }
        assert_eq!(pacer.divisor(), 2);
        for _ in 0..4000 {
            now += 1000.0 / 60.0;
            pacer.tick(now);
        }
        assert_eq!(pacer.divisor(), 1, "back at the target");
        assert_close(f64::from(pacer.presented_fps()), 30.0, 0.5, "measured fps");
    }

    #[test]
    fn a_failed_probe_returns_to_the_lower_rate() {
        let mut pacer = FramePacer::new(BattleFps::Fps30);
        let mut now = 1000.0;
        // The page never keeps up: every tick arrives 66 ms apart.
        for _ in 0..3000 {
            now += 1000.0 / 15.0;
            pacer.tick(now);
        }
        assert_eq!(pacer.divisor(), 2);
    }

    #[test]
    fn simulation_time_is_real_time_between_presented_frames() {
        let mut pacer = FramePacer::new(BattleFps::Fps30);
        let mut total = 0.0;
        let mut count = 0;
        let tick = 1000.0 / 60.0;
        for index in 0..600 {
            let timestamp = 2000.0 + f64::from(index) * tick;
            if let Pace::Present { elapsed_ms } = pacer.tick(timestamp) {
                if count > 0 {
                    total += elapsed_ms;
                }
                count += 1;
            }
        }
        // The advance adds up to the wall time between the first and the last presented frame.
        assert_close(
            total,
            f64::from(count - 1) * 1000.0 / 30.0,
            0.1,
            "sum of steps",
        );
    }
}
