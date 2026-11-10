//! The director's choice of cue for scripted impulse sequences on a controlled clock: hysteresis,
//! minimum dwell, priority, and the not-ready skip. No host, no composer, no clock but the numbers.

use super::cue::{Choice, Decision, Playing, decide};
use super::request::DecisionReason;
use super::run::TempoRun;
use super::table::MusicTable;

const SHIPPED: &str = include_str!("../../../../development/market-threads/free-roam.music.json");

fn index_of(table: &MusicTable, id: &str) -> usize {
    table
        .profiles
        .iter()
        .position(|profile| profile.id == id)
        .expect("the profile is in the table")
}

/// A journey on a controlled clock: the run, what plays, and which tracks are ready.
struct Journey {
    table: MusicTable,
    run: TempoRun,
    playing: Option<Playing>,
    ready: Vec<bool>,
}

impl Journey {
    fn new() -> Self {
        Self::with_table(|_| {})
    }

    /// A journey over the shipped table, edited as JSON first.
    fn with_table(edit: impl FnOnce(&mut serde_json::Value)) -> Self {
        let mut value: serde_json::Value = serde_json::from_str(SHIPPED).expect("is JSON");
        edit(&mut value);
        let table = MusicTable::from_json(value.to_string().as_bytes()).expect("a valid table");
        let run = TempoRun::new(&table, 0).expect("a run starts");
        let ready = vec![true; table.profiles.len()];
        Self {
            table,
            run,
            playing: None,
            ready,
        }
    }

    /// One boundary at `now_ms`, with an impulse of `kind` when given. A switch starts the cue.
    fn boundary(&mut self, now_ms: u64, kind: Option<&str>) -> Decision {
        match kind {
            Some(kind) => {
                let rule = self
                    .table
                    .impulse(kind)
                    .expect("the kind is in the table")
                    .clone();
                self.run.impulse(&rule, 1_000, None, now_ms).expect("steps");
            }
            None => self.run.advance(now_ms).expect("steps"),
        }
        let decision = decide(
            &self.table,
            &self.run.moment(now_ms),
            self.playing,
            &self.ready,
        );
        if let Choice::Switch(_) = decision.choice {
            self.playing = Some(Playing {
                index: decision.wanted,
                since_ms: now_ms,
            });
        }
        decision
    }

    fn playing_id(&self) -> Option<&str> {
        self.playing
            .map(|playing| self.table.profiles[playing.index].id.as_str())
    }
}

#[test]
fn the_journey_starts_on_the_base_profile() {
    let mut journey = Journey::new();
    let decision = journey.boundary(0, None);
    assert_eq!(decision.choice, Choice::Switch(DecisionReason::Start));
    assert_eq!(journey.playing_id(), Some("journey"));
}

#[test]
fn a_change_wanted_inside_the_dwell_is_held_until_the_dwell_is_over_and_then_made() {
    let mut journey = Journey::new();
    journey.boundary(0, None);
    let held = journey.boundary(10_000, Some("arrive_road"));
    assert_eq!(held.wanted, index_of(&journey.table, "road"));
    assert_eq!(held.choice, Choice::Hold { until_ms: 75_000 });
    assert_eq!(journey.playing_id(), Some("journey"));

    let made = journey.boundary(75_000, None);
    assert_eq!(made.choice, Choice::Switch(DecisionReason::Changed));
    assert_eq!(journey.playing_id(), Some("road"));
}

#[test]
fn a_flurry_of_impulses_inside_one_dwell_makes_at_most_one_change() {
    let mut journey = Journey::new();
    journey.boundary(0, None);
    let mut switches = 0;
    let kinds = ["arrive_town", "look", "declare", "trade", "look", "painted"];
    for (index, kind) in kinds.iter().cycle().take(14).enumerate() {
        let now_ms = 76_000 + 4_000 * u64::try_from(index).expect("small");
        if matches!(
            journey.boundary(now_ms, Some(kind)).choice,
            Choice::Switch(_)
        ) {
            switches += 1;
        }
    }
    assert_eq!(switches, 1, "one change, then every later one is held");
}

#[test]
fn the_playing_profile_stays_while_it_holds_within_the_margin() {
    // The market asks for an intensity of 230 and the table's margin is 80.
    let mut journey = Journey::with_table(|table| {
        table["profiles"][1]["when"]["intensity_min"] = 230.into();
        table["profiles"][1]["when"]["recent"]["within_ms"] = 900_000.into();
    });
    journey.boundary(0, None);
    journey.boundary(80_000, Some("trade"));
    journey.boundary(120_000, None);
    assert_eq!(journey.playing_id(), Some("market"));

    // The trade settles below the market's own bound but within the margin of it: the market
    // stays, although the base is what the tempo now wants.
    let kept = journey.boundary(200_000, None);
    let intensity = journey.run.moment(200_000).intensity;
    assert!((150..230).contains(&intensity), "{intensity}");
    assert_eq!(kept.wanted, journey.table.base_index());
    assert_eq!(kept.choice, Choice::Keep(DecisionReason::Margin));
    assert_eq!(journey.playing_id(), Some("market"));

    // Past the margin it is let go.
    let changed = journey.boundary(320_000, None);
    assert_eq!(changed.choice, Choice::Switch(DecisionReason::Changed));
    assert_eq!(journey.playing_id(), Some("journey"));
}

#[test]
fn an_earlier_profile_outranks_the_one_that_plays() {
    let mut journey = Journey::new();
    journey.boundary(0, None);
    journey.boundary(80_000, Some("arrive_town"));
    journey.boundary(100_000, None);
    assert_eq!(journey.playing_id(), Some("town"));

    let show = journey.boundary(110_000, Some("show"));
    assert_eq!(show.wanted, index_of(&journey.table, "show"));
    assert_eq!(show.choice, Choice::Hold { until_ms: 155_000 });
    let made = journey.boundary(155_000, None);
    assert_eq!(made.choice, Choice::Switch(DecisionReason::Changed));
    assert_eq!(journey.playing_id(), Some("show"));
}

#[test]
fn the_tempo_settles_and_the_music_returns_to_the_base() {
    let mut journey = Journey::new();
    journey.boundary(0, None);
    journey.boundary(80_000, Some("arrive_road"));
    journey.boundary(160_000, None);
    assert_eq!(journey.playing_id(), Some("road"));
    // Five minutes later nothing recent is left and the intensity is back at rest.
    let decision = journey.boundary(460_000, None);
    assert_eq!(decision.wanted, journey.table.base_index());
    assert_eq!(journey.playing_id(), Some("journey"));
}

#[test]
fn a_profile_whose_track_is_not_ready_is_skipped_and_the_playing_cue_stays() {
    let mut journey = Journey::new();
    journey.boundary(0, None);
    let road = index_of(&journey.table, "road");
    journey.ready[road] = false;
    let skipped = journey.boundary(80_000, Some("arrive_road"));
    assert_eq!(skipped.choice, Choice::Skip);
    assert_eq!(journey.playing_id(), Some("journey"));

    journey.ready[road] = true;
    let made = journey.boundary(90_000, None);
    assert_eq!(made.choice, Choice::Switch(DecisionReason::Changed));
    assert_eq!(journey.playing_id(), Some("road"));
}

#[test]
fn nothing_plays_when_the_base_track_is_not_ready_at_the_start() {
    let mut journey = Journey::new();
    journey.ready = vec![false; journey.table.profiles.len()];
    let decision = journey.boundary(0, None);
    assert_eq!(decision.choice, Choice::Skip);
    assert_eq!(journey.playing, None);
}

/// The runner's events for one move by `way`, as (seconds after the move began, kind): the
/// arrival, the close look, then a trade at a market or the declared painting and the painting
/// finished.
fn day_events(way: &str) -> Vec<(u64, String)> {
    let arrival = format!("arrive_{way}");
    let mut events = vec![(120, arrival), (180, "look".to_owned())];
    if way == "market" {
        events.push((270, "trade".to_owned()));
    } else {
        events.push((210, "declare".to_owned()));
        events.push((330, "painted".to_owned()));
    }
    events
}

#[test]
fn a_roam_day_follows_the_journey_and_never_changes_inside_a_dwell() {
    let mut journey = Journey::new();
    let ways = [
        "road", "town", "market", "road", "town", "town", "market", "road", "town", "market",
    ];
    let mut events: Vec<(u64, String)> = Vec::new();
    for (move_index, way) in ways.iter().enumerate() {
        let began_ms = u64::try_from(move_index).expect("small") * 420_000;
        for (seconds, kind) in day_events(way) {
            events.push((began_ms + seconds * 1_000, kind));
        }
    }
    let show_ms = u64::try_from(ways.len()).expect("small") * 420_000 + 60_000;
    events.push((show_ms, "show".to_owned()));

    // The director's own schedule: a decision at the tick, or at the end of a held dwell.
    let mut changes: Vec<(u64, String)> = vec![(0, "journey".to_owned())];
    let mut wake = Some(journey_start(&mut journey));
    for (at_ms, kind) in events {
        while let Some(due) = wake.filter(|due| *due <= at_ms) {
            let decision = journey.boundary(due, None);
            wake = Some(record(&mut changes, &journey, due, decision));
        }
        let decision = journey.boundary(at_ms, Some(&kind));
        wake = Some(record(&mut changes, &journey, at_ms, decision));
    }
    let end_ms = show_ms + 300_000;
    while let Some(due) = wake.filter(|due| *due <= end_ms) {
        let decision = journey.boundary(due, None);
        wake = Some(record(&mut changes, &journey, due, decision));
    }
    eprintln!("{changes:?}");
    let played: std::collections::BTreeSet<&str> =
        changes.iter().map(|(_, id)| id.as_str()).collect();
    assert_eq!(
        played.into_iter().collect::<Vec<_>>(),
        ["journey", "market", "road", "show", "study", "town"]
    );
    for pair in changes.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= 75_000, "{pair:?}");
    }
    assert!(
        changes.len() <= 3 * ways.len() + 3,
        "at most a few changes a move: {}",
        changes.len()
    );
    assert_eq!(changes.last().map(|(_, id)| id.as_str()), Some("show"));
}

/// Starts the day at 0 and returns when the director first decides again.
fn journey_start(journey: &mut Journey) -> u64 {
    journey.boundary(0, None);
    TICK_MS
}

/// The table's tick, which is also the longest the director waits between decisions.
const TICK_MS: u64 = 30_000;

/// Writes a change down and returns when the director decides next: the end of a held dwell, or
/// the tick.
fn record(
    changes: &mut Vec<(u64, String)>,
    journey: &Journey,
    at_ms: u64,
    decision: Decision,
) -> u64 {
    match decision.choice {
        Choice::Switch(_) => {
            changes.push((at_ms, journey.table.profiles[decision.wanted].id.clone()));
            at_ms + TICK_MS
        }
        Choice::Hold { until_ms } => at_ms + until_ms.saturating_sub(at_ms).clamp(1, TICK_MS),
        Choice::Keep(_) | Choice::Skip => at_ms + TICK_MS,
    }
}
