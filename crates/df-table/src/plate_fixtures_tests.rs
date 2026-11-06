//! Shared fixtures for the tests of the scene plan, the plate scheduler and the playout buffer:
//! a table with a scene and a cast that tests change one fact at a time, a brief, regions and
//! ready plates. Time is always an explicit number of milliseconds.

use crate::effects::{TableEffect, TableState};
use crate::element::ElementId;
use crate::identity::{SceneId, SeatId, TableId, TableScope};
use crate::plate::job::{JobReason, PlateJob, PlateJobId};
use crate::plate::timing::PlateTiming;
use crate::scene_plan::brief::{LocatePhrase, SceneBrief, SetPiece};
use crate::scene_plan::locate::{Instance, LocateOutcome, Region};
use crate::scene_plan::{ReadyPlate, StageOutputs, StagePlan, StoredPicture};
use crate::table_state::{
    CastChange, CastEntry, CastKind, Controller, DisplayName, MediaName, Placement, Presence,
    SceneGeneration, SplitPolicy, TableSnapshot, Visibility, Vitals,
};
use crate::testkit::correlation;

pub(crate) const SCENE: u64 = 10;

pub(crate) fn scene(number: u64) -> SceneId {
    SceneId::new(number)
}

pub(crate) fn element(name: &str) -> ElementId {
    ElementId::new(name).expect("a valid element id")
}

pub(crate) fn display_name(text: &str) -> DisplayName {
    DisplayName::new(text).expect("a valid display name")
}

pub(crate) fn place(x_units: u32, y_units: u32) -> Placement {
    Placement::from_milli_units(x_units * 1000, y_units * 1000).expect("on the canvas")
}

pub(crate) fn phrase(text: &str) -> LocatePhrase {
    LocatePhrase::new(text).expect("a valid phrase")
}

pub(crate) fn picture(handle: u64) -> StoredPicture {
    StoredPicture {
        handle,
        width: 1_024,
        height: 576,
    }
}

/// A region `width` by `height` canvas units whose top left corner is (`x`, `y`).
pub(crate) fn region(x: u32, y: u32, width: u32, height: u32) -> Region {
    Region::new(x * 1000, y * 1000, (x + width) * 1000, (y + height) * 1000)
        .expect("a valid region")
}

pub(crate) fn instance(x: u32, y: u32, width: u32, height: u32, score_permille: u16) -> Instance {
    Instance::new(region(x, y, width, height), score_permille).expect("a valid instance")
}

/// A table with seats 1 and 2 in scene 10, which tests change through its methods.
pub(crate) struct World {
    table: TableState,
    turn: u64,
}

impl World {
    pub(crate) fn new() -> Self {
        let mut table =
            TableState::seated(4, &[SeatId::new(1), SeatId::new(2)]).expect("a valid table");
        table
            .apply(
                &TableScope::of_table(TableId::new(1)),
                correlation(1),
                &[TableEffect::SceneEntered {
                    scene: scene(SCENE),
                }],
            )
            .expect("the party enters");
        Self { table, turn: 1 }
    }

    pub(crate) fn apply(&mut self, effects: &[TableEffect]) {
        self.turn += 1;
        self.table
            .apply(
                &TableScope::of_table(TableId::new(1)),
                correlation(self.turn),
                effects,
            )
            .expect("the commit applies");
    }

    pub(crate) fn snapshot(&self) -> TableSnapshot {
        self.table.snapshot()
    }

    pub(crate) fn generation(&self) -> SceneGeneration {
        self.snapshot()
            .scene(scene(SCENE))
            .expect("the scene is open")
            .generation()
    }

    pub(crate) fn add_hero(&mut self, id: &str, name: &str, seat: u64, x: u32) {
        let entry = CastEntry::new(
            element(id),
            CastKind::Party,
            Controller::Seat {
                seat: SeatId::new(seat),
            },
            scene(SCENE),
            display_name(name),
            Vitals::new(7, 9).expect("valid vitals"),
        )
        .at(place(x, 20))
        .with_portrait(MediaName::new(&format!("{id}.png")).expect("a media name"));
        self.apply(&[TableEffect::CastEntered { entry }]);
    }

    pub(crate) fn add_creature(&mut self, id: &str, name: &str, x: u32) {
        let entry = CastEntry::new(
            element(id),
            CastKind::Npc,
            Controller::Engine,
            scene(SCENE),
            display_name(name),
            Vitals::new(12, 19).expect("valid vitals"),
        )
        .at(place(x, 30));
        self.apply(&[TableEffect::CastEntered { entry }]);
    }

    pub(crate) fn move_to(&mut self, id: &str, x: u32, y: u32) {
        self.apply(&[TableEffect::CastChanged {
            cast: element(id),
            change: CastChange::Placement {
                placement: Some(place(x, y)),
            },
        }]);
    }

    pub(crate) fn set_visibility(&mut self, id: &str, visibility: Visibility) {
        self.apply(&[TableEffect::CastChanged {
            cast: element(id),
            change: CastChange::Visibility { visibility },
        }]);
    }

    pub(crate) fn hurt(&mut self, id: &str, hit_points: u32) {
        self.apply(&[TableEffect::CastChanged {
            cast: element(id),
            change: CastChange::HitPoints { hit_points },
        }]);
    }

    pub(crate) fn enter_scene(&mut self, number: u64) {
        self.apply(&[TableEffect::SceneEntered {
            scene: scene(number),
        }]);
    }

    /// Lets the party split, sends seat 2 to scene 11 alone, and drops its connection, so scene
    /// 11 is open with nobody looking at it while the shared screen shows scene 10.
    pub(crate) fn lose_the_viewer_of_scene_eleven(&mut self) {
        self.table = self.table.clone().with_split_policy(SplitPolicy::Allowed);
        self.apply(&[TableEffect::SeatMoved {
            seat: SeatId::new(2),
            scene: scene(11),
        }]);
        self.apply(&[TableEffect::SeatPresenceChanged {
            seat: SeatId::new(2),
            presence: Presence::Disconnected,
        }]);
    }

    pub(crate) fn pause(&mut self) {
        self.apply(&[TableEffect::TablePaused]);
    }

    pub(crate) fn resume(&mut self) {
        self.apply(&[TableEffect::TableResumed]);
    }
}

/// A world with two heroes and a goblin.
pub(crate) fn market() -> World {
    let mut world = World::new();
    world.add_hero("pip", "Pip", 1, 30);
    world.add_hero("mara", "Mara", 2, 45);
    world.add_creature("goblin", "Goblin", 70);
    world
}

pub(crate) fn lantern() -> SetPiece {
    SetPiece::new(element("lantern"), display_name("Paper lantern"))
        .at(place(80, 20))
        .locating_by(vec![phrase("paper lantern"), phrase("lantern")])
        .expect("two phrases")
        .with_picture(MediaName::new("lantern.png").expect("a media name"))
}

pub(crate) fn crate_of_fish() -> SetPiece {
    SetPiece::new(element("crate"), display_name("Crate of fish"))
        .at(place(10, 35))
        .locating_by(vec![
            phrase("crate of fish"),
            phrase("wooden crate"),
            phrase("crate"),
        ])
        .expect("three phrases")
}

pub(crate) fn brief() -> SceneBrief {
    SceneBrief::new(scene(SCENE), display_name("Harbor market"))
        .with_music(MediaName::new("harbor.ogg").expect("a media name"))
        .with_set(vec![lantern(), crate_of_fish()])
}

pub(crate) fn timing() -> PlateTiming {
    PlateTiming::default()
}

pub(crate) fn job(world: &World, id: u64, now_ms: u64) -> PlateJob {
    PlateJob::capture(
        PlateJobId::new(id),
        JobReason::Tick,
        now_ms,
        &world.snapshot(),
        &brief(),
    )
    .expect("a job")
}

pub(crate) fn plan(world: &World, id: u64, now_ms: u64) -> StagePlan {
    StagePlan::for_job(job(world, id, now_ms), &StageOutputs::default())
}

/// A finished plate for `job`, ready at `ready_ms`, with nobody found.
pub(crate) fn ready_plate(job: PlateJob, ready_ms: u64) -> ReadyPlate {
    let figures: Vec<_> = job
        .figures()
        .iter()
        .map(|figure| crate::scene_plan::locate::FigureTarget {
            id: figure.id.clone(),
            stated: figure.stated,
        })
        .collect();
    let locate = LocateOutcome::unlocated(
        &figures,
        &[],
        crate::scene_plan::locate::UnlocatedReason::NoInstance,
    )
    .expect("an outcome");
    ReadyPlate {
        plate: picture(job.id().get()),
        locate,
        job,
        ready_ms,
    }
}
