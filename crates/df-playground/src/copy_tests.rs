use crate::copy::{
    ReadingPace, caption_dwell_ms, result_card_dwell_ms, status_blocks_input, status_copy,
    word_count,
};
use crate::fixture::Status;
use crate::fixtures::assert_keys_are_serde_names;
use crate::limits::{DWELL_MS_PER_WORD, MINIMUM_CAPTION_DWELL_MS};

fn every_status() -> Vec<Status> {
    vec![
        Status::Ok,
        Status::Reading,
        Status::Rolling,
        Status::Telling,
        Status::ArtDelayed,
        Status::Degraded,
        Status::ReadOnly,
        Status::Reconnecting,
        Status::WaitingFor {
            who: "Pip".into(),
            what: "to answer a reaction".into(),
        },
    ]
}

#[test]
fn a_short_caption_stays_up_for_the_minimum_and_a_long_one_for_each_word() {
    assert_eq!(
        caption_dwell_ms(3, ReadingPace::Normal),
        MINIMUM_CAPTION_DWELL_MS
    );
    assert_eq!(caption_dwell_ms(10, ReadingPace::Normal), 3_300);
}

#[test]
fn the_reading_pace_scales_every_dwell() {
    assert_eq!(caption_dwell_ms(10, ReadingPace::Slow), 4_950);
    assert_eq!(caption_dwell_ms(10, ReadingPace::Fast), 2_475);
    assert_eq!(result_card_dwell_ms(ReadingPace::Normal), 3_000);
    assert_eq!(result_card_dwell_ms(ReadingPace::Slow), 4_500);
}

#[test]
fn a_huge_word_count_keeps_its_whole_dwell_and_never_wraps_short() {
    let normal_ms = u64::from(u32::MAX) * DWELL_MS_PER_WORD;
    assert_eq!(
        caption_dwell_ms(u32::MAX, ReadingPace::Slow),
        normal_ms * 3 / 2
    );
    assert_eq!(
        caption_dwell_ms(u32::MAX, ReadingPace::Fast),
        normal_ms * 3 / 4
    );
}

#[test]
fn words_are_counted_by_whitespace() {
    assert_eq!(word_count("  The potter looks up.  "), 4);
    assert_eq!(word_count(""), 0);
}

#[test]
fn only_a_status_of_ok_says_nothing_and_every_other_says_a_plain_sentence() {
    for status in every_status() {
        match (&status, status_copy(&status)) {
            (Status::Ok, copy) => assert_eq!(copy, None),
            (_, Some(copy)) => {
                assert!(!copy.is_empty());
                assert!(!copy.contains('_'), "'{copy}' shows a code word");
            }
            (status, None) => panic!("{status:?} must say something"),
        }
    }
}

#[test]
fn the_wait_names_the_person_being_waited_for() {
    let status = Status::WaitingFor {
        who: "Pip".into(),
        what: "to answer a reaction".into(),
    };
    assert_eq!(
        status_copy(&status).as_deref(),
        Some("Waiting for Pip to answer a reaction")
    );
}

#[test]
fn the_fixed_phrases_are_the_ones_the_plan_names() {
    assert_eq!(
        status_copy(&Status::Reading).as_deref(),
        Some("Reading your move")
    );
    assert_eq!(status_copy(&Status::Rolling).as_deref(), Some("Rolling"));
    assert_eq!(status_copy(&Status::Telling).as_deref(), Some("Telling it"));
    assert_eq!(
        status_copy(&Status::ArtDelayed).as_deref(),
        Some("Picture catching up")
    );
}

#[test]
fn a_status_that_stops_input_is_marked_and_progress_statuses_are_not() {
    assert!(status_blocks_input(&Status::ReadOnly));
    assert!(status_blocks_input(&Status::Reconnecting));
    assert!(!status_blocks_input(&Status::Reading));
    assert!(!status_blocks_input(&Status::ArtDelayed));
}

#[test]
fn every_status_key_in_the_menu_gives_a_status() {
    for key in crate::copy::STATUS_KEYS {
        assert!(crate::copy::status_from_key(key).is_some(), "{key}");
    }
    assert!(crate::copy::status_from_key("sleepy").is_none());
}

#[test]
fn the_keys_of_a_reading_pace_are_its_serde_names() {
    assert_keys_are_serde_names(&ReadingPace::ALL, ReadingPace::key, ReadingPace::from_key);
}
