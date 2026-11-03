//! A plate job: one snapshot of the table, taken at a tick, that asks for one picture (D16, D.2).
//!
//! It owns `PlateJob` (what the plate is of: the figures the public may see, the set, and the
//! stamp of the scene generation and revision it was taken under), `PlateJobId`, `PlateFigure`
//! and the digest of what the picture depends on. A job keeps its tick time, so how late its
//! plate is can always be measured and never hidden. The figures are the public cast only: a hero
//! is always drawn (K.13), another creature only when the party can see it, so a hidden thing
//! never reaches a prompt (RES6).
//!
//! It never decides when to tick or what to ask a provider: `plate_scheduler` and the host do.

use std::fmt;
use std::num::NonZeroU128;

use crate::element::ElementId;
use crate::identity::{Correlation, OperationId, SceneId, TurnSeq};
use crate::plate_digest::PlateDigest;
use crate::scene_plan::SceneStage;
use crate::scene_plan_brief::{SceneBrief, SetPiece};
use crate::table_state::{
    CastEntry, CastKind, DisplayName, Placement, ResultStamp, TableSnapshot, Visibility,
};

/// Which plate job of a scene this is, in the order they were taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlateJobId(u64);

impl PlateJobId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PlateJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Why a job was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobCause {
    /// The clock ticked and something visible had changed.
    Tick,
    /// The scene was just entered: the first plate, which is built with the scene (K.2).
    SceneEntered,
}

impl JobCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::SceneEntered => "scene_entered",
        }
    }
}

/// Why a job could not be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobError {
    /// The scene is not open in the snapshot.
    SceneNotOpen { scene: SceneId },
    /// The brief is for another scene than the one asked for.
    BriefForOtherScene { asked: SceneId, brief: SceneId },
}

impl fmt::Display for JobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SceneNotOpen { scene } => write!(formatter, "scene {scene} is not open"),
            Self::BriefForOtherScene { asked, brief } => write!(
                formatter,
                "the brief is for scene {brief}, not scene {asked}"
            ),
        }
    }
}

impl std::error::Error for JobError {}

/// One creature the plate is to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlateFigure {
    pub id: ElementId,
    pub name: DisplayName,
    pub kind: CastKind,
    pub visibility: Visibility,
    /// Where the rules owner's layout says it stands, mirrored on the canvas.
    pub stated: Option<Placement>,
}

impl PlateFigure {
    fn of(entry: &CastEntry) -> Self {
        Self {
            id: entry.id().clone(),
            name: entry.name().clone(),
            kind: entry.kind(),
            visibility: entry.visibility(),
            stated: entry.placement(),
        }
    }

    fn digest_into(&self, digest: &mut PlateDigest) {
        digest.text(self.id.as_str());
        digest.text(self.kind.key());
        digest.text(self.visibility.key());
        digest.option(self.stated, |digest, placement| {
            digest.number(u64::from(placement.x_units().to_bits()));
            digest.number(u64::from(placement.y_units().to_bits()));
        });
    }
}

/// The figures a plate of `scene` shows: every hero, and any other creature the party can see.
fn public_figures(snapshot: &TableSnapshot, scene: SceneId) -> Vec<PlateFigure> {
    snapshot
        .cast()
        .filter(|entry| entry.scene() == scene)
        .filter(|entry| entry.kind() == CastKind::Party || entry.visibility() == Visibility::Seen)
        .map(PlateFigure::of)
        .collect()
}

fn digest_set_piece(digest: &mut PlateDigest, piece: &SetPiece) {
    digest.text(piece.id().as_str());
    digest.text(piece.name().as_str());
    digest.option(piece.position(), |digest, placement| {
        digest.number(u64::from(placement.x_units().to_bits()));
        digest.number(u64::from(placement.y_units().to_bits()));
    });
}

/// A snapshot of the table, taken to make one plate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlateJob {
    id: PlateJobId,
    stamp: ResultStamp,
    tick_ms: u64,
    cause: JobCause,
    figures: Vec<PlateFigure>,
    set: Vec<SetPiece>,
    digest: u64,
}

impl PlateJob {
    /// Takes the job: the scene's public cast from `snapshot`, the set from `brief`, the stamp of
    /// the generation and revision they were read under.
    pub fn capture(
        id: PlateJobId,
        cause: JobCause,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
    ) -> Result<Self, JobError> {
        let scene = brief.scene();
        let record = snapshot
            .scene(scene)
            .ok_or(JobError::SceneNotOpen { scene })?;
        let figures = public_figures(snapshot, scene);
        let mut digest = PlateDigest::new();
        digest.number(scene.get());
        digest.number(record.generation().get());
        digest.number(u64::try_from(figures.len()).unwrap_or(u64::MAX));
        for figure in &figures {
            figure.digest_into(&mut digest);
        }
        digest.number(u64::try_from(brief.set().len()).unwrap_or(u64::MAX));
        for piece in brief.set() {
            digest_set_piece(&mut digest, piece);
        }
        Ok(Self {
            id,
            stamp: ResultStamp {
                scene,
                generation: record.generation(),
                revision: snapshot.revision(),
            },
            tick_ms: now_ms,
            cause,
            figures,
            set: brief.set().to_vec(),
            digest: digest.finish(),
        })
    }

    pub fn id(&self) -> PlateJobId {
        self.id
    }

    /// The scene, generation and revision the job was taken under (D.2, RES42).
    pub fn stamp(&self) -> ResultStamp {
        self.stamp
    }

    pub fn scene(&self) -> SceneId {
        self.stamp.scene
    }

    /// When the clock ticked: the time lateness is measured from.
    pub fn tick_ms(&self) -> u64 {
        self.tick_ms
    }

    pub fn cause(&self) -> JobCause {
        self.cause
    }

    pub fn figures(&self) -> &[PlateFigure] {
        &self.figures
    }

    pub fn set(&self) -> &[SetPiece] {
        &self.set
    }

    /// The digest of everything a picture depends on. Two jobs with one digest would draw the same
    /// picture (RES9, K.3), so the second is never paid for.
    pub fn staging_digest(&self) -> u64 {
        self.digest
    }

    /// The correlation of a request this job makes. The job is the engine's own work, not a
    /// player's input, so its operation id is made from the job's identity and the request it is,
    /// the same every run: `discriminant` tells the requests of one stage apart.
    pub fn correlation(&self, stage: SceneStage, discriminant: u32) -> Correlation {
        let mut digest = PlateDigest::new();
        digest.number(self.stamp.scene.get());
        digest.number(self.stamp.generation.get());
        digest.number(self.id.get());
        digest.text(stage.key());
        digest.number(u64::from(discriminant));
        let operation = NonZeroU128::MIN.saturating_add(u128::from(digest.finish()));
        Correlation {
            operation_id: OperationId::from_nonzero_u128(operation),
            turn_seq: TurnSeq::new(self.stamp.revision.sequence()),
            revision: self.stamp.revision,
        }
    }
}
