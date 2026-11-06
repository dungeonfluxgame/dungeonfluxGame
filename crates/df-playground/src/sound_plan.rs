//! What the page should do about sound, decided from numbers and data alone: which cues are new
//! and what a change of music means.
//!
//! The audio module starts and stops the browser's elements; this module only says which ones. It
//! reads no clock and touches no element, so native tests cover it.

use crate::controls::Device;
use crate::scene::{Cue, Track};

/// The cues a page should play now, and what to remember afterwards.
#[derive(Debug, PartialEq, Eq)]
pub struct FreshCues<'a> {
    /// Cues not played yet that are meant for this device, oldest first.
    pub play: Vec<&'a Cue>,
    /// The newest cue count the page has taken in. It never goes down within one run of the host
    /// (a slow answer that was overtaken must not make a played cue fresh again); a new run of the
    /// host starts the page's count again by its boot id.
    pub seen: u64,
    /// Cues that were made while the page was away and are no longer kept, so they cannot be
    /// played.
    pub missed: u64,
}

/// The cues to play given the newest count `seen` before (`None` before the first answer: cues from
/// before the page opened are not replayed) and the `count` and `cues` the host just said.
pub fn fresh_cues(seen: Option<u64>, count: u64, cues: &[Cue], device: Device) -> FreshCues<'_> {
    let Some(seen) = seen else {
        return FreshCues {
            play: Vec::new(),
            seen: count,
            missed: 0,
        };
    };
    if count <= seen {
        return FreshCues {
            play: Vec::new(),
            seen,
            missed: 0,
        };
    }
    let kept = cues.iter().filter(|cue| cue.id > seen && cue.id <= count);
    let newer = kept.clone().count() as u64;
    FreshCues {
        play: kept.filter(|cue| cue.target.plays_on(device)).collect(),
        seen: count,
        missed: (count - seen).saturating_sub(newer),
    }
}

/// What a change of the wanted music means for the music that is playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicChange {
    /// The same music is wanted as is playing.
    Keep,
    /// The same song with another volume or loop setting: change it where it plays, without
    /// starting the song again.
    Update { volume_percent: u8, looping: bool },
    /// Another song: stop what plays and start the new one (or start one when none plays).
    Replace,
    /// No music is wanted: stop what plays.
    Stop,
}

/// A song is told by its file and the screens it is for; its volume and loop setting change in
/// place.
pub fn music_change(playing: Option<&Track>, wanted: Option<&Track>) -> MusicChange {
    match (playing, wanted) {
        (None, None) => MusicChange::Keep,
        (Some(_), None) => MusicChange::Stop,
        (None, Some(_)) => MusicChange::Replace,
        (Some(playing), Some(wanted)) => {
            if playing.media != wanted.media || playing.target != wanted.target {
                MusicChange::Replace
            } else if playing.volume_percent == wanted.volume_percent
                && playing.looping == wanted.looping
            {
                MusicChange::Keep
            } else {
                MusicChange::Update {
                    volume_percent: wanted.volume_percent,
                    looping: wanted.looping,
                }
            }
        }
    }
}

/// The browser's volume, from 0 to 1, for a percent. A percent over 100 is held at full (the
/// scene checks it before it gets here).
pub fn volume_of(percent: u8) -> f64 {
    f64::from(percent.min(crate::limits::MAXIMUM_VOLUME_PERCENT)) / 100.0
}
