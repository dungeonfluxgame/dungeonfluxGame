//! The `world_stamp` of a look-ahead build: the facts it used, so the transition can tell whether
//! they still hold (SB-LA, idea 1 "Stale results").
//!
//! A stamp keeps three digests and no text: the place (its name, its authored description and its
//! set), the cast present (the party, which travels with the table) and the part of the day (the
//! table's world time in six-hour parts). Comparing a stamp taken when a build started with the
//! stamp of the world at the transition says one of three things: nothing changed (a hit), only the
//! cast changed (the picture of the empty place is still good, so only the dress step runs again),
//! or the place or the time of day changed (the picture is stale, and the table plays on the
//! establishing card while a rebuild runs). Weather is not a table fact yet, so it is not here.
//!
//! It reads no clock: the world time comes from the snapshot.

use crate::plate::digest::PlateDigest;
use crate::scene_plan::brief::SceneBrief;
use crate::table_state::TableSnapshot;

/// How many seconds of world time make one part of the day: a quarter of a day, so dawn, day,
/// dusk and night each change the picture of an outdoor place.
pub(crate) const DAY_PART_SECONDS: u64 = 21_600;

/// What a stamp comparison found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampChange {
    /// Nothing the build used has changed.
    Unchanged,
    /// Only the cast present changed: the picture is kept and the dress step runs again.
    CastOnly,
    /// The place or the part of the day changed: the picture is stale.
    PlaceOrTime,
}

impl StampChange {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::CastOnly => "cast_only",
            Self::PlaceOrTime => "place_or_time",
        }
    }
}

/// The facts a look-ahead build used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldStamp {
    place: u64,
    cast: u64,
    day_part: u64,
}

impl WorldStamp {
    /// The stamp of `brief`'s scene as the table is now.
    pub fn of(snapshot: &TableSnapshot, brief: &SceneBrief) -> Self {
        let mut place = PlateDigest::new();
        place.number(brief.scene().get());
        place.text(brief.place().as_str());
        place.option(brief.description(), |digest, text| {
            digest.text(text.expose());
        });
        place.number(u64::try_from(brief.set().len()).unwrap_or(u64::MAX));
        for piece in brief.set() {
            place.text(piece.id().as_str());
            place.text(piece.name().as_str());
        }
        let mut cast = PlateDigest::new();
        for hero in snapshot.heroes() {
            cast.text(hero.id().as_str());
            cast.text(hero.name().as_str());
            cast.text(hero.kind().key());
        }
        Self {
            place: place.finish(),
            cast: cast.finish(),
            day_part: snapshot.world_time_s() / DAY_PART_SECONDS,
        }
    }

    /// How the world at the transition (`now`) differs from the world this build used.
    pub fn change_to(&self, now: &Self) -> StampChange {
        if self.place != now.place || self.day_part != now.day_part {
            StampChange::PlaceOrTime
        } else if self.cast != now.cast {
            StampChange::CastOnly
        } else {
            StampChange::Unchanged
        }
    }
}
