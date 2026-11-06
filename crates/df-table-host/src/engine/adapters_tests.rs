//! The dev clock: frozen, advanced and resumed as the `clock` verb drives it.

use std::time::Duration;

use df_table::ports::Clock;

use super::adapters::DevClock;

/// Long enough that a resumed clock that jumped to real time is far from where it stood.
const PAUSE: Duration = Duration::from_millis(300);
/// Far below `PAUSE`, and far above the time between the resume and the read.
const RESUME_TOLERANCE_MS: u64 = 150;

#[test]
fn a_frozen_clock_resumes_from_where_it_stood_and_not_from_real_time() {
    let clock = DevClock::starting_now();
    clock.freeze();
    let frozen_ms = clock.now_ms();

    // Real time passes while the clock is frozen (the subject of the test, not a wait for work).
    std::thread::sleep(PAUSE);
    assert_eq!(clock.now_ms(), frozen_ms, "a frozen clock does not move");
    clock.resume();
    let resumed_ms = clock.now_ms();

    // Before, a clock that was never advanced jumped to the time since the host started.
    assert!(
        resumed_ms < frozen_ms + RESUME_TOLERANCE_MS,
        "frozen at {frozen_ms} ms, read {resumed_ms} ms after resuming"
    );
}

#[test]
fn an_advance_moves_a_frozen_and_a_running_clock_alike() {
    let clock = DevClock::starting_now();
    clock.freeze();
    let frozen_ms = clock.now_ms();

    clock.advance_ms(10_000);

    assert_eq!(clock.now_ms(), frozen_ms + 10_000);
    clock.resume();
    let running_ms = clock.now_ms();
    assert!(running_ms >= frozen_ms + 10_000);
    clock.advance_ms(5_000);
    assert!(clock.now_ms() >= running_ms + 5_000);
}
