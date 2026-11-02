use crate::event_log::{EventLog, MAXIMUM_LOGGED_LINES};

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
