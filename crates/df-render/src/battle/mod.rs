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
#[cfg(target_arch = "wasm32")]
mod speech_bubble;

mod camera;
mod combat_feed;
mod director;
pub mod feel;
mod figure_art;
pub mod figure_clear;
pub mod float_text;
mod formation;
pub mod framing;
mod grid;
pub mod grid_lines;
pub mod health;
mod layout;
mod lod;
mod math;
mod model;
pub mod occupancy;
mod pacing;
pub mod particles;
pub mod rest_shot;
mod scene_file;
mod sound;
pub mod speech;
pub mod splash;
mod splat;
mod stage;
mod status;
mod terrain;
mod theme;

pub use camera::{CameraPose, CameraRig, PoseError, ShotName};
pub use combat_feed::{
    CombatFeed, CombatSnapshot, FeedMode, FeedReport, InitiativeEntry, Member, Outcome,
};
pub use director::{Anim, FlashTone, PlanError, ReadyStyle, Step, TimedStep, Timeline, plan};
pub use figure_art::{VisibleBounds, visible_bounds};
pub use formation::{Formation, FormationError, derive as derive_formation, from_layout};
pub use grid::{Cell, Grid, GridError};
pub use layout::{Cover, Layout, LayoutError, Obstacle, choose as choose_layout};
pub use lod::{
    ChunkFile, DecodeJob, DecodePlan, Focus, Limits, LodError, LodIndex, MAX_RANGES, Pick, Range,
    Selection,
};
pub use math::{Mat4, Quat, Vec3};
pub use model::{
    AttackResult, AttackStyle, BadgeView, Beat, CameraCommand, CameraFocus, DieView, FinishingBlow,
    FloatTone, FloatingText, Highlight, HighlightKind, HitPoints, HudState, InitiativeRoll,
    InitiativeShow, RollCard, Side, Token, TokenId, initiative_line, ordinal,
};
pub use occupancy::{Occupancy, OccupancyError};
pub use pacing::{BattleFps, FramePacer, Pace};
pub use scene_file::{
    Attribution, BackdropLayer, BattleScene, DEFAULT_SCENE_ID, Distribution, GroundLayer,
    SceneChoice, SceneFileError, SplatFormat, SplatGrade, SplatLayer, embedded_ids, load_embedded,
    parse as parse_scene, resolve_scene,
};
pub use sound::{
    ALL_CLIPS, ContentRating, SoundEvent, SoundKind, Surface, VariationPicker, VoiceSet, voice_for,
};
pub use splat::{
    ProceduralGround, RECORD_WORDS, RawSplat, SogLoader, SogMeta, SogPlanes, Splat, SplatCloud,
    SplatError, SplatFileLoader, SplatLoader, SplatSorter, SplatTransform, counting_order,
    depth_sort_key, encode_splat_file, orientation_from_euler_deg, pack_record, pack_records,
};
pub use stage::{
    CoverPip, FigureView, FlashView, FrameView, ImpactEvent, ProjectileView, Stage, StageError,
};
pub use status::{
    Resolved, Status, StatusPose, StatusSet, derived_from_view, icon_statuses, pose_for, resolve,
};
pub use terrain::{
    GroundPoint, GroundSettings, HeightField, TerrainError, WalkSettings, blocked_cells,
    blocker_density, estimate_height_field, estimate_height_field_covered,
};
pub use theme::{ColorGrade, Theme};

#[cfg(target_arch = "wasm32")]
pub use host::{Availability, BattleStage, FixedQuality, StageOptions, UnavailableReason};
