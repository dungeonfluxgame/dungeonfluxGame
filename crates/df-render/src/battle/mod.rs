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
mod layout;
mod math;
mod model;
mod scene_file;
mod sound;
mod splat;
mod stage;
mod status;
mod theme;

pub use camera::{CameraPose, CameraRig, PoseError, ShotName};
pub use combat_feed::{
    CombatFeed, CombatSnapshot, FeedMode, FeedReport, InitiativeEntry, Member, Outcome,
};
pub use director::{
    Anim, FlashTone, PlanError, ReadyStyle, Step, TimedStep, Timeline, plan, plan_with,
};
pub use figure_art::{VisibleBounds, visible_bounds};
pub use formation::{Formation, FormationError, derive as derive_formation, from_layout};
pub use grid::{Cell, Grid, GridError};
pub use layout::{Cover, Layout, LayoutError, Obstacle, choose as choose_layout};
pub use math::{Mat4, Quat, Vec3};
pub use model::{
    AttackResult, AttackStyle, BadgeView, Beat, CameraCommand, CameraFocus, DieView, FinishingBlow,
    FloatTone, FloatingText, Highlight, HighlightKind, HitPoints, HudState, InitiativeRoll,
    InitiativeShow, RollCard, Side, Token, TokenId, initiative_line, ordinal,
};
pub use scene_file::{
    BackdropLayer, BattleScene, DEFAULT_SCENE_ID, GroundLayer, SceneChoice, SceneFileError,
    SplatFormat, SplatLayer, embedded_ids, load_embedded, parse as parse_scene, resolve_scene,
};
pub use sound::{
    ALL_CLIPS, ContentRating, SoundEvent, SoundKind, Surface, VariationPicker, VoiceSet, voice_for,
};
pub use splat::{
    ProceduralGround, RawSplat, SogLoader, SogMeta, SogPlanes, Splat, SplatCloud, SplatError,
    SplatFileLoader, SplatLoader, SplatSorter, SplatTransform, encode_splat_file,
};
pub use stage::{CoverPip, FigureView, FlashView, FrameView, ProjectileView, Stage, StageError};
pub use status::{
    Resolved, Status, StatusPose, StatusSet, derived_from_view, icon_statuses, pose_for, resolve,
};
pub use theme::{ColorGrade, Theme};

#[cfg(target_arch = "wasm32")]
pub use host::{Availability, BattleStage, StageOptions, UnavailableReason};
