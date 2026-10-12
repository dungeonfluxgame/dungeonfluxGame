//! The 3D battlefield for the shared display: a staged, painted-ground or Gaussian-splat scene
//! with billboard figures, drawn by WebGPU.
//!
//! The pure core (math, camera, grid, formation, stage model, director, scene files, splats,
//! sorting) builds and tests natively and knows nothing about the graphics API. The WebGPU
//! backend and the DOM overlay are `wasm32` only and sit behind [`BattleStage`]. Everything shown
//! is presentation of server decisions; positions are staging only (see [`formation`]).

#[cfg(target_arch = "wasm32")]
mod gpu;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod hud;

mod camera;
mod combat_feed;
mod director;
mod figure_art;
mod formation;
mod grid;
mod lod;
mod math;
mod model;
mod scene_file;
mod splat;
mod stage;
mod terrain;
mod theme;

pub use camera::{CameraPose, CameraRig, PoseError, ShotName};
pub use combat_feed::{CombatFeed, CombatSnapshot, FeedMode, FeedReport, Member, Outcome};
pub use director::{Anim, FlashTone, PlanError, Step, TimedStep, Timeline, plan};
pub use figure_art::{VisibleBounds, visible_bounds};
pub use formation::{Formation, FormationError, derive as derive_formation};
pub use grid::{Cell, Grid, GridError};
pub use lod::{
    ChunkFile, DecodeJob, DecodePlan, Focus, Limits, LodError, LodIndex, MAX_RANGES, Pick, Range,
    Selection,
};
pub use math::{Mat4, Quat, Vec3};
pub use model::{
    AttackResult, Beat, CameraCommand, CameraFocus, FinishingBlow, FloatTone, FloatingText,
    Highlight, HighlightKind, HitPoints, HudState, RollCard, Side, Status, Token, TokenId,
};
pub use scene_file::{
    Attribution, BackdropLayer, BattleScene, DEFAULT_SCENE_ID, Distribution, GroundLayer,
    SceneFileError, SplatFormat, SplatGrade, SplatLayer, embedded_ids, load_embedded,
    parse as parse_scene,
};
pub use splat::{
    ProceduralGround, RECORD_WORDS, RawSplat, SogLoader, SogMeta, SogPlanes, Splat, SplatCloud,
    SplatError, SplatFileLoader, SplatLoader, SplatSorter, SplatTransform, counting_order,
    depth_sort_key, encode_splat_file, orientation_from_euler_deg, pack_record, pack_records,
};
pub use stage::{FigureView, FlashView, FrameView, Stage, StageError};
pub use terrain::{
    GroundPoint, GroundSettings, HeightField, TerrainError, WalkSettings, blocked_cells,
    blocker_density, estimate_height_field, estimate_height_field_covered,
};
pub use theme::{ColorGrade, Theme};

#[cfg(target_arch = "wasm32")]
pub use host::{
    Availability, BattleStage, FixedQuality, StageOptions, UnavailableReason,
    prefers_reduced_motion,
};
