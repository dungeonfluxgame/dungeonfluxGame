//! Plan section 17, TIME-01 to TIME-03: state-driven active-time billing (plan 4.1).
//!
//! The lifecycle holds, the presence window, the blocking-wait board, the meter segments and F's
//! pricing of closed segments do not exist yet (O1 and F own them), so every scenario here is
//! blocked. They are written against `planning/hosted-runtime-research.md` 5.2 to 5.4 and
//! `planning/finance-ledger-research.md` 13, on a controlled clock with the database clock as the
//! only time source, at the default `$2.00` per active table-hour of the proposed pilot price.

use df_acceptance::blocked_scenario;

blocked_scenario! {
    fn time_01_pausing_or_everyone_disconnecting_follows_the_documented_meter_policy,
    blocked_on: "blocked on O1 and F API: df-session::lifecycle holds (NoPlayersConnected, PlayersAway), the presence watch, meter segments and credits (O1), and the meter pricing of closed segments (F)",
    setup: [
        "session S Active with six connected players and a paired display, controlled clock, price version PV1 at $2.00 per active hour",
        "presence.window_ms (proposed 30,000) and meter.backdate_window_seconds read from configuration, not hard-coded in the test",
    ],
    steps: [
        "run 10 minutes Active; the host pauses for 10 minutes; resume and run 10 minutes",
        "assert: F prices 20 active minutes; the paused interval is never priced (paused_and_all_away_intervals_are_never_priced)",
        "disconnect every interactive player at once; the display does not count as presence",
        "assert: at once the table is NoPlayersConnected: new paid work stops and the meter still runs",
        "advance the clock past presence.window_ms with nobody back",
        "assert: the table is PlayersAway, the segment is closed at the last-seen time with a credit for any part already rolled, and a save point of kind all_away exists",
        "a player returns before the window ends in a second case: assert both holds clear and the meter never stopped",
        "assert: saving and viewing stay available throughout, and the session summary shows the same figures as F's statement",
    ],
    outcome: "Meter follows documented useful-service policy",
}

blocked_scenario! {
    fn time_02_a_long_blocking_supplier_wait_bills_platform_time_only_up_to_its_grace,
    blocked_on: "blocked on O1 and F API: the BlockingWaitBoard and BlockingWaitGuard with blocking_wait.gated_grace_ms and outage_grace_ms, and the credit function for the blocked interval (hosted runtime design 5.4)",
    setup: [
        "session S Active; a required narration call that blocks the whole table (BlockingReason::RequiredNarration) for 4 minutes while no other play is possible",
        "a second case: a provider outage (BlockingReason::ProviderOutage) with outage_grace_ms = 0; a third: an ordinary NPC reply wait, which is not blocking",
    ],
    steps: [
        "run the gated wait with a controlled clock to just under blocking_wait.gated_grace_ms (proposed 10 s) and then complete it",
        "assert: a wait just under the grace is billed in full, because the table could meaningfully be waiting for a moment",
        "run the 4 minute wait past its grace",
        "assert: the meter is free from the wait's start (blocked_interval_free), with a credit for any rolled part, and F does not charge it (a_credited_blocking_interval_is_not_charged)",
        "assert: the outage case is free at once (grace 0), the ordinary NPC wait is billed, and the lifecycle state BlockingWait{reason} is projected into the views",
        "assert: a begin with all 32 wait slots full is refused with one audit line and the wait is not credited",
    ],
    outcome: "Platform billing paused for blocked interval",
}

blocked_scenario! {
    fn time_03_replay_restart_and_database_clock_back_dating_never_double_charge_or_go_negative,
    blocked_on: "blocked on O1 and F API: transition_seq replay on meter transitions with the database clock only (hosted runtime design 5.2), and F pricing closed segments once (finance design 13.2)",
    setup: [
        "meter segments recorded with clock_timestamp() / stored last_progress_at only; CHECK (closed_at >= opened_at); the test advances a fake database clock, there is no wall clock and no client time",
    ],
    steps: [
        "replay the same transition (same transition_seq) three times, restart the runner mid-segment and recover",
        "assert: each replay returns the recorded row, each real second is charged once, and a back-dated close is clamped to GREATEST(opened_at, now - lag) so no duration is negative",
        "price a closed segment only once its closed_at is older than meter.backdate_window_seconds; price it twice",
        "assert: pricing a segment twice charges once (pricing_a_segment_twice_charges_once), and time after the last proven progress is never charged (time_after_the_last_proven_progress_is_never_charged)",
        "run 3,600 one-second intervals split at random points across segments",
        "assert: the cumulative charge is exactly 2,000,000 micro-dollars with no per-interval rounding up (the_cumulative_meter_never_charges_more_than_the_exact_amount_for_any_split_of_time), and a negative interval is refused with an alert (a_negative_interval_is_refused_with_an_alert)",
    ],
    outcome: "No double charge or negative duration",
}
