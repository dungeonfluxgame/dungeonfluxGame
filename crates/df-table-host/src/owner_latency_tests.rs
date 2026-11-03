use crate::owner_latency::commit::CommitMode;
use crate::owner_latency::producers::Arrival;
use crate::owner_latency::projection::project;
use crate::owner_latency::report::render;
use crate::owner_latency::run::{RunConfig, SnapshotMode, Stop, run};
use crate::owner_latency::sha256::sha256;
use crate::owner_latency::state::{
    Audience, Fact, SEAT_COUNT, TableState, Visibility, WorkloadProfile,
};
use crate::owner_latency::statistics::{
    BusyInterval, PERMILLE_P50, PERMILLE_P95, PERMILLE_P99, StatisticsError,
    busy_permille_per_window, percentile_of_sorted, summarise,
};

const SECOND_NS: u64 = 1_000_000_000;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn nearest_rank_percentiles_of_one_to_one_hundred() {
    let sorted: Vec<u64> = (1..=100).collect();
    assert_eq!(percentile_of_sorted(&sorted, PERMILLE_P50), Ok(50));
    assert_eq!(percentile_of_sorted(&sorted, PERMILLE_P95), Ok(95));
    assert_eq!(percentile_of_sorted(&sorted, PERMILLE_P99), Ok(99));
    assert_eq!(percentile_of_sorted(&sorted, 1_000), Ok(100));
    assert_eq!(percentile_of_sorted(&sorted, 0), Ok(1));
}

#[test]
fn percentile_rounds_the_rank_up() {
    let sorted = [10, 20, 30];
    assert_eq!(percentile_of_sorted(&sorted, 500), Ok(20));
    assert_eq!(percentile_of_sorted(&sorted, 950), Ok(30));
    assert_eq!(percentile_of_sorted(&sorted, 334), Ok(20));
    assert_eq!(percentile_of_sorted(&sorted, 333), Ok(10));
}

#[test]
fn a_single_sample_is_every_percentile() {
    let summary = summarise(&mut [42]).expect("one sample");
    assert_eq!(
        (
            summary.count,
            summary.mean_ns,
            summary.p50_ns,
            summary.p99_ns,
            summary.max_ns
        ),
        (1, 42, 42, 42, 42)
    );
}

#[test]
fn no_samples_and_bad_percentiles_are_typed_errors() {
    assert_eq!(percentile_of_sorted(&[], 500), Err(StatisticsError::Empty));
    assert_eq!(summarise(&mut []), Err(StatisticsError::Empty));
    assert_eq!(
        percentile_of_sorted(&[1], 1_001),
        Err(StatisticsError::PercentileOutOfRange { permille: 1_001 })
    );
}

#[test]
fn summarise_sorts_and_does_not_overflow_the_mean() {
    let mut values = [u64::MAX, 3, u64::MAX, 1];
    let summary = summarise(&mut values).expect("four samples");
    assert_eq!(values, [1, 3, u64::MAX, u64::MAX]);
    assert_eq!(summary.max_ns, u64::MAX);
    assert_eq!(summary.p50_ns, 3);
    assert_eq!(summary.mean_ns, u64::MAX / 2 + 1);
}

#[test]
fn busy_time_is_split_across_windows_and_partial_windows_are_left_out() {
    let intervals = [
        BusyInterval {
            start_ns: 500_000_000,
            end_ns: 1_250_000_000,
        },
        BusyInterval {
            start_ns: 2_000_000_000,
            end_ns: 2_100_000_000,
        },
    ];
    let windows = busy_permille_per_window(&intervals, SECOND_NS, 3_500_000_000).expect("windows");
    assert_eq!(windows, vec![500, 250, 100]);
}

#[test]
fn busy_time_never_exceeds_a_whole_window() {
    let overlapping = [
        BusyInterval {
            start_ns: 0,
            end_ns: SECOND_NS,
        },
        BusyInterval {
            start_ns: 0,
            end_ns: SECOND_NS,
        },
    ];
    assert_eq!(
        busy_permille_per_window(&overlapping, SECOND_NS, SECOND_NS),
        Ok(vec![1_000])
    );
}

#[test]
fn busy_time_rejects_a_zero_window_and_a_reversed_interval() {
    assert_eq!(
        busy_permille_per_window(&[], 0, SECOND_NS),
        Err(StatisticsError::ZeroWindow)
    );
    let reversed = [BusyInterval {
        start_ns: 9,
        end_ns: 3,
    }];
    assert_eq!(
        busy_permille_per_window(&reversed, SECOND_NS, SECOND_NS),
        Err(StatisticsError::IntervalReversed {
            start_ns: 9,
            end_ns: 3
        })
    );
    assert_eq!(busy_permille_per_window(&[], SECOND_NS, 10), Ok(Vec::new()));
}

#[test]
fn sha256_matches_the_published_vectors() {
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let million = vec![b'a'; 1_000_000];
    assert_eq!(
        hex(&sha256(&million)),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn generated_state_has_the_stated_size_and_stays_bounded_under_steps() {
    let profile = WorkloadProfile::representative();
    let mut state = TableState::generate(&profile);
    assert_eq!(state.elements.len(), profile.elements);
    assert_eq!(state.facts.len(), profile.facts);
    for turn in 1..=3_000 {
        state.step(&profile, turn);
    }
    assert_eq!(state.facts.len(), profile.facts);
    assert!(state.narration.len() <= 200);
    assert_eq!(state.revision, 3_001);
}

#[test]
fn step_is_deterministic() {
    let profile = WorkloadProfile::representative();
    let mut first = TableState::generate(&profile);
    let mut second = TableState::generate(&profile);
    for turn in 1..=50 {
        first.step(&profile, turn);
        second.step(&profile, turn);
    }
    let encode = |state: &TableState| serde_json::to_string(state).expect("state encodes");
    assert_eq!(encode(&first), encode(&second));
}

#[test]
fn a_private_fact_reaches_its_own_phone_and_nothing_else() {
    let profile = WorkloadProfile::representative();
    let mut state = TableState::generate(&profile);
    let marked = |visibility, text: &str| Fact {
        id: 9_000_000,
        visibility,
        text: text.to_owned(),
    };
    state
        .facts
        .push_back(marked(Visibility::Seat(2), "MARK-PRIVATE-TWO"));
    state
        .facts
        .push_back(marked(Visibility::Hidden, "MARK-HIDDEN"));
    state
        .facts
        .push_back(marked(Visibility::Public, "MARK-PUBLIC"));
    for index in 0..=SEAT_COUNT {
        let audience = Audience::at(index);
        let view = String::from_utf8(project(&state, &profile, audience).expect("projects"))
            .expect("views are text");
        assert!(
            view.contains("MARK-PUBLIC"),
            "{audience:?} sees public facts"
        );
        assert!(
            !view.contains("MARK-HIDDEN"),
            "{audience:?} never sees hidden facts"
        );
        assert_eq!(
            view.contains("MARK-PRIVATE-TWO"),
            audience == Audience::Seat(2),
            "{audience:?} and a private fact"
        );
    }
}

#[test]
fn the_combat_profile_makes_larger_views_than_the_representative_one() {
    let size = |profile: WorkloadProfile| {
        let state = TableState::generate(&profile);
        project(&state, &profile, Audience::Seat(0))
            .expect("projects")
            .len()
    };
    let representative = size(WorkloadProfile::representative());
    let combat = size(WorkloadProfile::combat());
    assert!(
        combat > 2 * representative,
        "{combat} against {representative}"
    );
}

fn small_run(
    arrival: Arrival,
    commit: CommitMode,
    measured: usize,
) -> crate::owner_latency::run::RunResult {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let config = RunConfig {
        profile: WorkloadProfile::representative(),
        commit,
        checkpoint_bytes: 200_000,
        snapshot: SnapshotMode::CopyOnWrite,
        arrival,
        stop: Stop::AfterInputs(measured),
        warmup_inputs: 10,
        directory: directory.path().to_path_buf(),
    };
    run(&config).expect("the run finishes")
}

fn assert_ordered_and_complete(result: &crate::owner_latency::run::RunResult) {
    assert_eq!(result.refused, 0);
    assert_in_total_order(result);
}

fn assert_in_total_order(result: &crate::owner_latency::run::RunResult) {
    let mut previous = result
        .records
        .first()
        .map_or(0, |record| record.turn_seq - 1);
    for record in &result.records {
        assert_eq!(record.turn_seq, previous + 1, "one total order, no gaps");
        previous = record.turn_seq;
        assert!(record.serve_started_ns >= record.submitted_ns);
        assert!(record.finished_ns >= record.serve_started_ns);
        assert!(record.projection_ns >= record.projection_per_audience_ns[0]);
    }
    assert!(result.phone_view_bytes > 0 && result.tv_view_bytes > 0);
}

#[test]
fn a_saturated_run_serves_every_input_in_order() {
    let result = small_run(Arrival::Saturated { window: 3 }, CommitMode::Memory, 120);
    assert!(result.records.len() >= 120);
    assert_ordered_and_complete(&result);
    assert!(result.checkpoint_bytes >= 200_000);
}

#[test]
fn a_steady_run_serves_in_order_and_accounts_for_every_input() {
    let result = small_run(Arrival::Steady { inputs_per_s: 50 }, CommitMode::Memory, 20);
    let served = result.records.len() as u64;
    assert!((1..=20).contains(&served), "{served} served");
    assert!(served + result.refused <= 30, "nothing is counted twice");
    assert_in_total_order(&result);
}

#[test]
fn six_seats_at_once_are_served_in_one_total_order() {
    let result = small_run(
        Arrival::Burst {
            inputs: 6,
            period_ms: 200,
        },
        CommitMode::Memory,
        12,
    );
    assert!(!result.records.is_empty());
    assert!(result.records.len() as u64 + result.refused <= 22);
    assert_in_total_order(&result);
}

#[test]
fn an_fsync_commit_leaves_the_checkpoint_on_disk() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let config = RunConfig {
        profile: WorkloadProfile::representative(),
        commit: CommitMode::Fsync,
        checkpoint_bytes: 200_000,
        snapshot: SnapshotMode::InPlace,
        arrival: Arrival::Saturated { window: 1 },
        stop: Stop::AfterInputs(12),
        warmup_inputs: 3,
        directory: directory.path().to_path_buf(),
    };
    let result = run(&config).expect("the run finishes");
    let written = std::fs::metadata(directory.path().join("checkpoint.json")).expect("a file");
    assert_eq!(written.len(), result.checkpoint_bytes as u64);
    assert!(result.records.iter().all(|record| record.store_ns > 0));
}

#[test]
fn the_report_names_every_phase_and_the_budgets() {
    let result = small_run(Arrival::Saturated { window: 3 }, CommitMode::Memory, 60);
    let text = render(&result).expect("renders");
    for phase in [
        "queue_wait",
        "serve_next",
        "step",
        "projection_phone",
        "projection_tv",
        "projection_total",
        "commit_serialise",
        "commit_hash",
        "commit_store",
        "reduce",
        "end_to_end",
        "H.5 projection, one phone",
        "H.5 rules-only path",
        "owner busy per 1 s window",
    ] {
        assert!(text.contains(phase), "the report lacks {phase}:\n{text}");
    }
}

#[tokio::test]
async fn the_real_engine_scenarios_run_and_report_what_they_measured() {
    use crate::owner_latency::real::{lobby_burst, projection_cost, rules_path};

    let directory = tempfile::tempdir().expect("a directory");
    let burst = lobby_burst("six at once", CommitMode::Memory, 6, 4, 1, directory.path())
        .await
        .expect("the burst runs");
    assert_eq!(burst.inputs, 24);
    assert!(
        burst.commits < 6 + 24,
        "group commit: the six seats of a round share commits ({} commits)",
        burst.commits
    );
    assert!(burst.render().contains("six at once: 24 inputs"));
    let rules = rules_path("rules", CommitMode::Memory, 3, directory.path())
        .await
        .expect("the rules path runs");
    assert_eq!(rules.inputs, 3);
    let cost = projection_cost(20, directory.path())
        .await
        .expect("the projection cost is read");
    assert!(cost.phone_wire_bytes > cost.tv_wire_bytes);
    assert!(cost.render().starts_with("projection"));
}
