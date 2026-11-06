use crate::event_log::EventLog;
use crate::limits::MAXIMUM_LOGGED_LINES;

fn log_with(count: usize) -> EventLog {
    let log = EventLog::default();
    for index in 1..=count {
        log.push(format!("line {index}"));
    }
    log
}

#[test]
fn an_empty_log_has_sequence_zero_and_no_lines() {
    let log = EventLog::default();
    assert_eq!(log.latest_sequence(), 0);
    assert!(log.newest(10).is_empty());
    assert!(log.after_sequence(0, 10).is_empty());
}

#[test]
fn sequences_start_at_one_and_rise_by_one() {
    let log = log_with(3);
    let sequences: Vec<u64> = log.newest(10).iter().map(|line| line.sequence).collect();
    assert_eq!(sequences, vec![1, 2, 3]);
    assert_eq!(log.latest_sequence(), 3);
}

#[test]
fn after_a_sequence_returns_only_newer_lines_oldest_first() {
    let log = log_with(5);
    let texts: Vec<String> = log
        .after_sequence(2, 10)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert_eq!(texts, vec!["line 3", "line 4", "line 5"]);
    assert!(log.after_sequence(5, 10).is_empty());
}

#[test]
fn a_limit_on_after_keeps_the_oldest_of_the_newer_lines() {
    let log = log_with(5);
    let texts: Vec<String> = log
        .after_sequence(0, 2)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert_eq!(texts, vec!["line 1", "line 2"]);
}

#[test]
fn newest_keeps_the_latest_lines_and_still_reads_oldest_first() {
    let log = log_with(5);
    let texts: Vec<String> = log.newest(2).into_iter().map(|line| line.text).collect();
    assert_eq!(texts, vec!["line 4", "line 5"]);
    assert_eq!(log.newest(0).len(), 0);
}

#[test]
fn the_oldest_lines_are_forgotten_first_and_numbering_goes_on() {
    let log = log_with(MAXIMUM_LOGGED_LINES + 1);
    let kept = log.newest(MAXIMUM_LOGGED_LINES + 10);
    assert_eq!(kept.len(), MAXIMUM_LOGGED_LINES);
    assert_eq!(kept[0].sequence, 2);
    assert_eq!(
        log.latest_sequence(),
        u64::try_from(MAXIMUM_LOGGED_LINES + 1).expect("fits")
    );
}

#[test]
fn after_a_sequence_that_was_forgotten_starts_at_the_oldest_kept_line() {
    let log = log_with(MAXIMUM_LOGGED_LINES + 3);
    let after_zero = log.after_sequence(0, 2);
    assert_eq!(
        after_zero
            .iter()
            .map(|line| line.sequence)
            .collect::<Vec<_>>(),
        vec![4, 5],
        "lines 1 to 3 were forgotten"
    );
    let latest = log.latest_sequence();
    assert!(log.after_sequence(latest, 10).is_empty());
    assert!(log.after_sequence(latest + 50, 10).is_empty());
    let last = log.after_sequence(latest - 1, 10);
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].sequence, latest);
}

#[test]
fn a_log_installed_as_the_audit_sink_takes_the_audit_lines_of_the_libraries() {
    let log = std::sync::Arc::new(EventLog::default());
    assert!(crate::event_log::install_as_audit_sink(&log));
    df_observe::record_audit_line("TEST", "TEST probe key=1".to_owned());
    let lines = log.newest(10);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].text.starts_with("TEST probe key=1"), "{lines:?}");
    let later = std::sync::Arc::new(EventLog::default());
    assert!(
        !crate::event_log::install_as_audit_sink(&later),
        "the first log keeps the lines"
    );
}

#[test]
fn each_line_is_stamped_with_the_time_of_the_injected_clock() {
    let clock = std::sync::Arc::new(df_table::testkit::ManualClock::starting_at(5_000));
    let log = EventLog::default().with_clock(clock.clone());
    log.push("HOST first".to_owned());
    clock.advance_ms(250);
    log.push("HOST second".to_owned());
    let stamps: Vec<u64> = log.newest(10).iter().map(|line| line.at_ms).collect();
    assert_eq!(stamps, vec![5_000, 5_250]);
}

#[test]
fn lines_pushed_out_by_the_line_or_byte_bound_are_counted_as_forgotten() {
    let by_lines = EventLog::with_bounds(2, usize::MAX);
    for index in 1..=5 {
        by_lines.push(format!("line {index}"));
    }
    assert_eq!(by_lines.forgotten(), 3);
    let by_bytes = EventLog::with_bounds(100, 10);
    for _ in 0..3 {
        by_bytes.push("12345678".to_owned());
    }
    let counts = by_bytes.counts();
    assert_eq!(counts.forgotten, 2, "{counts:?}");
    assert_eq!((counts.latest, counts.kept, counts.bytes), (3, 1, 8));
}

#[test]
fn an_area_filter_keeps_only_lines_whose_first_word_is_the_area() {
    let log = EventLog::default();
    for text in [
        "HOST a",
        "CANVAS b",
        "HOST c",
        "HOSTILE d",
        "HOST e",
        "CANVAS f",
    ] {
        log.push(text.to_owned());
    }
    let texts = |lines: Vec<crate::event_log::LoggedLine>| -> Vec<String> {
        lines.into_iter().map(|line| line.text).collect()
    };
    assert_eq!(
        texts(log.after_sequence_in(1, 10, "HOST")),
        vec!["HOST c", "HOST e"]
    );
    assert_eq!(texts(log.after_sequence_in(0, 1, "HOST")), vec!["HOST a"]);
    assert_eq!(
        texts(log.newest_in(2, "CANVAS")),
        vec!["CANVAS b", "CANVAS f"]
    );
    assert_eq!(texts(log.newest_in(1, "HOST")), vec!["HOST e"]);
    assert!(log.newest_in(10, "ENGINE").is_empty());
}
