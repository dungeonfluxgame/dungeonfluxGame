//! The dev table the scene builder's workbench runs on: a manual clock, a table with a party, and
//! the dev scene's set.
//!
//! The workbench drives the real pure pieces (the table state, the scheduler, the buffer) with
//! fake providers, so the table here is a real `TableState` changed by real effects, and its clock
//! is a counter that only the workbench advances: a `scene build` that takes a minute of provider
//! time takes none of ours, and a run is the same every time. The dev scene has two heroes, one
//! person who lives there, and three things worth pointing at: a lantern, a crate and an urn, with
//! the noun phrases Locate tries for each (the urn has a picture of its own).
//!
//! It holds no secrets and reads no real clock.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use df_table::effects::{EffectError, TableEffect, TableState, TableStateError};
use df_table::element::{ElementId, NameError};
use df_table::identity::{
    Correlation, OperationId, RecoveryEpoch, SceneId, SeatId, SessionRevision, TableId, TableScope,
    TurnSeq,
};
use df_table::ports::Clock;
use df_table::scene_plan::brief::{LocatePhrase, PhraseError, SceneBrief, SetPiece, SetPieceError};
use df_table::scene_plan::locate::{Instance, Region, RegionError};

use super::fakes::StageScript;
use df_table::table_state::{
    CastChange, CastEntry, CastKind, Controller, DisplayName, DisplayNameError, MediaName,
    MediaNameError, Placement, PlacementError, SceneGeneration, TableSnapshot, Vitals, VitalsError,
};

/// The id of the one table the workbench runs.
const DEV_TABLE: u64 = 1;
/// Seats the dev table has: the two that play the heroes.
const DEV_SEATS: [u64; 2] = [1, 2];
/// The dev heroes: id, name, the seat that plays them and where they stand along the row.
pub const HEROES: [(&str, &str, u64, u32); 2] = [("pip", "Pip", 1, 30), ("mara", "Mara", 2, 45)];
/// The row the heroes stand on, in canvas units.
pub const HERO_ROW_Y: u32 = 20;
/// The person who lives in every dev scene: id, name and where they stand. The dev script never
/// finds them, to show an unlocated person.
pub const LOCAL: (&str, &str, u32, u32) = ("local", "A local", 70, 30);
/// How far each dev person's box reaches around their stated place, in canvas units.
const PERSON_WIDTH_UNITS: u32 = 6;
const PERSON_HEIGHT_UNITS: u32 = 20;
const ITEM_SIZE_UNITS: u32 = 4;
/// How sure the dev segmentation model is of a person and of an item, in thousandths.
const PERSON_SCORE_PERMILLE: u16 = 920;
const ITEM_SCORE_PERMILLE: u16 = 810;

/// A clock that only moves when it is told to, in milliseconds.
#[derive(Debug, Default)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    pub fn advance_ms(&self, ms: u64) {
        // Relaxed: the counter orders no other data.
        self.now_ms.fetch_add(ms, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

/// Why the dev table could not be set up or changed.
#[derive(Debug)]
pub enum WorldError {
    Name(NameError),
    DisplayName(DisplayNameError),
    MediaName(MediaNameError),
    Phrase(PhraseError),
    SetPiece(SetPieceError),
    Placement(PlacementError),
    Vitals(VitalsError),
    Table(TableStateError),
    Effect(EffectError),
    Region(RegionError),
    /// The table accepted the scene and does not list it.
    SceneNotOpened {
        scene: SceneId,
    },
}

impl fmt::Display for WorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(error) => write!(formatter, "{error}"),
            Self::DisplayName(error) => write!(formatter, "{error}"),
            Self::MediaName(error) => write!(formatter, "{error}"),
            Self::Phrase(error) => write!(formatter, "{error}"),
            Self::SetPiece(error) => write!(formatter, "{error}"),
            Self::Placement(error) => write!(formatter, "{error}"),
            Self::Vitals(error) => write!(formatter, "{error}"),
            Self::Table(error) => write!(formatter, "the dev table cannot be built: {error}"),
            Self::Effect(error) => write!(formatter, "the dev table refused a change: {error}"),
            Self::Region(error) => write!(formatter, "{error}"),
            Self::SceneNotOpened { scene } => {
                write!(formatter, "the table did not open scene {scene}")
            }
        }
    }
}

impl std::error::Error for WorldError {}

macro_rules! world_error_from {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(
            impl From<$source> for WorldError {
                fn from(source: $source) -> Self {
                    Self::$variant(source)
                }
            }
        )+
    };
}

world_error_from!(
    NameError => Name,
    DisplayNameError => DisplayName,
    MediaNameError => MediaName,
    PhraseError => Phrase,
    SetPieceError => SetPiece,
    PlacementError => Placement,
    VitalsError => Vitals,
    TableStateError => Table,
    EffectError => Effect,
    RegionError => Region,
);

/// A table of two seats that the workbench changes one fact at a time.
pub struct DevWorld {
    table: TableState,
    turn: u64,
}

fn correlation(turn: u64) -> Correlation {
    let operation = std::num::NonZeroU128::MIN.saturating_add(u128::from(turn));
    Correlation {
        operation_id: OperationId::from_nonzero_u128(operation),
        turn_seq: TurnSeq::new(turn),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, turn),
    }
}

/// A point of the canvas, in whole canvas units.
pub fn place(x_units: u32, y_units: u32) -> Result<Placement, PlacementError> {
    Placement::from_milli_units(x_units.saturating_mul(1_000), y_units.saturating_mul(1_000))
}

impl DevWorld {
    pub fn new() -> Result<Self, WorldError> {
        let seats: Vec<SeatId> = DEV_SEATS.iter().copied().map(SeatId::new).collect();
        Ok(Self {
            table: TableState::seated(seats.len().saturating_mul(2), &seats)?,
            turn: 0,
        })
    }

    pub fn apply(&mut self, effects: &[TableEffect]) -> Result<(), WorldError> {
        self.turn += 1;
        let scope = TableScope::of_table(TableId::new(DEV_TABLE));
        self.table.apply(&scope, correlation(self.turn), effects)?;
        Ok(())
    }

    pub fn snapshot(&self) -> TableSnapshot {
        self.table.snapshot()
    }

    /// The party enters `scene`, which gets a new generation.
    pub fn enter_scene(&mut self, scene: SceneId) -> Result<SceneGeneration, WorldError> {
        self.apply(&[TableEffect::SceneEntered { scene }])?;
        self.snapshot()
            .scene(scene)
            .map(|record| record.generation())
            .ok_or(WorldError::SceneNotOpened { scene })
    }

    fn cast_entry(
        id: &str,
        name: &str,
        kind: CastKind,
        controller: Controller,
        scene: SceneId,
        at: Placement,
    ) -> Result<CastEntry, WorldError> {
        Ok(CastEntry::new(
            ElementId::new(id)?,
            kind,
            controller,
            scene,
            DisplayName::new(name)?,
            Vitals::new(7, 9)?,
        )
        .at(at))
    }

    /// Puts the party and the person who lives in `scene` in it, if they are not there yet.
    pub fn people_of(&mut self, scene: SceneId) -> Result<(), WorldError> {
        let snapshot = self.snapshot();
        let mut effects = Vec::new();
        for (id, name, seat, x) in HEROES {
            if snapshot.cast_entry(&ElementId::new(id)?).is_none() {
                effects.push(TableEffect::CastEntered {
                    entry: Self::cast_entry(
                        id,
                        name,
                        CastKind::Party,
                        Controller::Seat {
                            seat: SeatId::new(seat),
                        },
                        scene,
                        place(x, HERO_ROW_Y)?,
                    )?
                    .with_portrait(MediaName::new(&format!("{id}.png"))?),
                });
            }
        }
        if snapshot.cast_entry(&ElementId::new(LOCAL.0)?).is_none() {
            effects.push(TableEffect::CastEntered {
                entry: Self::cast_entry(
                    LOCAL.0,
                    LOCAL.1,
                    CastKind::Npc,
                    Controller::Engine,
                    scene,
                    place(LOCAL.2, LOCAL.3)?,
                )?,
            });
        }
        if effects.is_empty() {
            return Ok(());
        }
        self.apply(&effects)
    }

    /// Moves a creature to a new place: a visible change, which a plate must show.
    pub fn move_cast(&mut self, id: &str, x_units: u32, y_units: u32) -> Result<(), WorldError> {
        self.apply(&[TableEffect::CastChanged {
            cast: ElementId::new(id)?,
            change: CastChange::Placement {
                placement: Some(place(x_units, y_units)?),
            },
        }])
    }

    #[cfg(test)]
    pub fn pause(&mut self) -> Result<(), WorldError> {
        self.apply(&[TableEffect::TablePaused])
    }

    #[cfg(test)]
    pub fn resume(&mut self) -> Result<(), WorldError> {
        self.apply(&[TableEffect::TableResumed])
    }
}

fn phrases(texts: &[&str]) -> Result<Vec<LocatePhrase>, PhraseError> {
    texts.iter().map(|text| LocatePhrase::new(text)).collect()
}

/// The dev scene of `place_name`: its music and three things to point at.
pub fn dev_brief(scene: SceneId, place_name: &str) -> Result<SceneBrief, WorldError> {
    let lantern = SetPiece::new(
        ElementId::new("lantern")?,
        DisplayName::new("Paper lantern")?,
    )
    .at(place(80, 20)?)
    .locating_by(phrases(&["paper lantern", "lantern"])?)?
    .with_picture(MediaName::new("lantern.png")?);
    let crate_of_fish = SetPiece::new(ElementId::new("crate")?, DisplayName::new("Crate of fish")?)
        .at(place(10, 35)?)
        .locating_by(phrases(&["crate of fish", "wooden crate", "crate"])?)?;
    let urn = SetPiece::new(ElementId::new("urn")?, DisplayName::new("Clay urn")?)
        .at(place(60, 45)?)
        .locating_by(phrases(&["clay urn", "urn", "pot"])?)?
        .with_picture(MediaName::new("urn.png")?);
    Ok(SceneBrief::new(scene, DisplayName::new(place_name)?)
        .with_music(MediaName::new("harbor.ogg")?)
        .with_set(vec![lantern, crate_of_fish, urn]))
}

/// A box of `width` by `height` canvas units centred on (`x`, `y`).
fn instance_around(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    score_permille: u16,
) -> Result<Instance, WorldError> {
    let left = x
        .saturating_mul(1_000)
        .saturating_sub(width.saturating_mul(500));
    let top = y
        .saturating_mul(1_000)
        .saturating_sub(height.saturating_mul(500));
    let region = Region::new(
        left,
        top,
        left.saturating_add(width.saturating_mul(1_000)),
        top.saturating_add(height.saturating_mul(1_000)),
    )?;
    Ok(Instance::new(region, score_permille)?)
}

/// The dev segmentation script: both heroes are found where they stand, the person who lives
/// here is not, the lantern is found by its first phrase and the crate by its second, and nothing
/// finds the urn (so it falls back to its own picture).
pub fn dev_script() -> Result<StageScript, WorldError> {
    let mut people = Vec::new();
    for (_, _, _, x) in HEROES {
        people.push(instance_around(
            x,
            HERO_ROW_Y,
            PERSON_WIDTH_UNITS,
            PERSON_HEIGHT_UNITS,
            PERSON_SCORE_PERMILLE,
        )?);
    }
    let found = |x, y| instance_around(x, y, ITEM_SIZE_UNITS, ITEM_SIZE_UNITS, ITEM_SCORE_PERMILLE);
    Ok(StageScript::default()
        .with_people(people)
        .with_phrase("paper lantern", vec![found(80, 20)?])
        .with_phrase("wooden crate", vec![found(10, 35)?]))
}
