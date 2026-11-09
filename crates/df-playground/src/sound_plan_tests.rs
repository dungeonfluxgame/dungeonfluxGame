//! Which cues are new, and what a change of music means.

use crate::controls::Device;
use crate::scene::{Cue, Target, Track};
use crate::sound_plan::{AudioPrefs, FreshCues, MusicChange, fresh_cues, music_change, volume_of};

fn cue(id: u64, target: Target) -> Cue {
    Cue {
        id,
        target,
        media: format!("sound-{id}.wav"),
        volume_percent: 80,
    }
}

fn track(media: &str, target: Target, volume_percent: u8, looping: bool) -> Track {
    Track {
        media: media.to_owned(),
        target,
        volume_percent,
        looping,
    }
}

#[test]
fn the_first_frame_only_learns_the_count_and_plays_nothing_from_before_the_page_opened() {
    let cues = [cue(5, Target::Both), cue(6, Target::Both)];
    assert_eq!(
        fresh_cues(None, 6, &cues, Device::Both),
        FreshCues {
            play: Vec::new(),
            seen: 6,
            missed: 0
        }
    );
}

#[test]
fn a_cue_is_played_once_when_a_late_reply_with_a_lower_count_arrives_between() {
    let six = [cue(5, Target::Both), cue(6, Target::Both)];
    let five = [cue(5, Target::Both)];
    let first = fresh_cues(Some(5), 6, &six, Device::Both);
    assert_eq!(first.play.iter().map(|cue| cue.id).collect::<Vec<_>>(), [6]);
    assert_eq!(first.seen, 6);
    let late = fresh_cues(Some(first.seen), 5, &five, Device::Both);
    assert!(late.play.is_empty());
    assert_eq!(
        late.seen, 6,
        "the count never goes down, so cue 6 does not become fresh again"
    );
    let again = fresh_cues(Some(late.seen), 6, &six, Device::Both);
    assert!(again.play.is_empty(), "counts 6, 5, 6 play cue 6 once");
}

#[test]
fn only_the_cues_meant_for_the_device_are_played_but_all_are_taken_in() {
    let cues = [
        cue(1, Target::Tv),
        cue(2, Target::Phone),
        cue(3, Target::Both),
    ];
    let on_phone = fresh_cues(Some(0), 3, &cues, Device::Phone);
    assert_eq!(
        on_phone.play.iter().map(|cue| cue.id).collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!((on_phone.seen, on_phone.missed), (3, 0));
}

#[test]
fn cues_that_are_no_longer_kept_are_counted_as_missed_and_not_hidden() {
    let kept = [cue(9, Target::Both), cue(10, Target::Both)];
    let result = fresh_cues(Some(2), 10, &kept, Device::Both);
    assert_eq!(result.play.len(), 2);
    assert_eq!(result.missed, 6, "cues 3 to 8 were dropped by the host");
}

#[test]
fn music_is_started_stopped_kept_and_replaced() {
    let theme = track("theme.ogg", Target::Both, 80, true);
    assert_eq!(music_change(None, None), MusicChange::Keep);
    assert_eq!(music_change(None, Some(&theme)), MusicChange::Replace);
    assert_eq!(music_change(Some(&theme), None), MusicChange::Stop);
    assert_eq!(music_change(Some(&theme), Some(&theme)), MusicChange::Keep);
    assert_eq!(
        music_change(
            Some(&theme),
            Some(&track("other.ogg", Target::Both, 80, true))
        ),
        MusicChange::Replace
    );
    assert_eq!(
        music_change(
            Some(&theme),
            Some(&track("theme.ogg", Target::Tv, 80, true))
        ),
        MusicChange::Replace,
        "another set of screens is another playing"
    );
}

#[test]
fn a_change_of_volume_or_loop_is_made_in_place_and_does_not_restart_the_song() {
    let theme = track("theme.ogg", Target::Both, 80, true);
    assert_eq!(
        music_change(
            Some(&theme),
            Some(&track("theme.ogg", Target::Both, 40, true))
        ),
        MusicChange::Update {
            volume_percent: 40,
            looping: true
        }
    );
    assert_eq!(
        music_change(
            Some(&theme),
            Some(&track("theme.ogg", Target::Both, 80, false))
        ),
        MusicChange::Update {
            volume_percent: 80,
            looping: false
        }
    );
}

#[test]
fn a_volume_is_a_share_of_full_and_is_held_at_full() {
    assert_eq!(volume_of(0), 0.0);
    assert_eq!(volume_of(50), 0.5);
    assert_eq!(volume_of(100), 1.0);
    assert_eq!(volume_of(255), 1.0);
}

#[test]
fn the_master_volume_scales_each_sound_s_own_and_the_toggles_mute_without_losing_it() {
    let prefs = AudioPrefs::default();
    assert_eq!(
        prefs,
        AudioPrefs {
            sound_on: true,
            music_on: true,
            volume_percent: 80
        }
    );
    assert!((prefs.volume_for(50) - 0.4).abs() < 1e-9);
    assert!((prefs.volume_for(100) - 0.8).abs() < 1e-9);
    let silent = AudioPrefs {
        volume_percent: 0,
        ..prefs
    };
    assert_eq!(silent.volume_for(100), 0.0);
    let full = AudioPrefs {
        volume_percent: 100,
        ..prefs
    };
    assert!((full.volume_for(30) - 0.3).abs() < 1e-9);
    assert!(prefs.music_plays());
    assert!(
        !AudioPrefs {
            music_on: false,
            ..prefs
        }
        .music_plays()
    );
    let muted = AudioPrefs {
        sound_on: false,
        ..prefs
    };
    assert!(!muted.music_plays(), "muted, the music is off too");
    assert!(muted.music_on, "and comes back with the sound");
    assert!(
        (muted.volume_for(50) - 0.4).abs() < 1e-9,
        "the volume is kept"
    );
}

#[test]
fn stored_prefs_read_back_and_anything_else_is_the_defaults() {
    let prefs = AudioPrefs {
        sound_on: false,
        music_on: true,
        volume_percent: 35,
    };
    assert_eq!(AudioPrefs::from_stored(Some(&prefs.stored())), prefs);
    assert_eq!(AudioPrefs::from_stored(None), AudioPrefs::default());
    assert_eq!(
        AudioPrefs::from_stored(Some("not json")),
        AudioPrefs::default()
    );
    assert_eq!(
        AudioPrefs::from_stored(Some(
            r#"{"sound_on":true,"music_on":true,"volume_percent":250}"#
        ))
        .volume_percent,
        100
    );
}
