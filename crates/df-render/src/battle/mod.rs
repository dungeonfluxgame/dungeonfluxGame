//! The 3D battlefield for the shared display: a staged, painted-ground or Gaussian-splat scene
//! with billboard figures, drawn by WebGPU.
//!
//! The pure core (math, camera, grid, formation, stage model, director, scene files, splats,
//! sorting) builds and tests natively and knows nothing about the graphics API. The WebGPU
//! backend and the DOM overlay are `wasm32` only and sit behind [`BattleStage`]. Everything shown
//! is presentation of server decisions; positions are staging only (see [`formation`]).

mod camera;
mod director;
mod formation;
mod grid;
mod math;
mod model;
mod scene_file;
mod splat;
mod stage;
mod theme;

pub use camera::{CameraPose, CameraRig, PoseError, ShotName};
pub use director::{Anim, FlashTone, PlanError, Step, TimedStep, Timeline, plan};
pub use formation::{Formation, FormationError, derive as derive_formation};
pub use grid::{Cell, Grid, GridError};
pub use math::{Mat4, Quat, Vec3};
pub use model::{
    AttackResult, Beat, CameraCommand, CameraFocus, FinishingBlow, FloatTone, FloatingText,
    Highlight, HighlightKind, HitPoints, HudState, Side, Status, Token, TokenId,
};
pub use scene_file::{
    BackdropLayer, BattleScene, DEFAULT_SCENE_ID, GroundLayer, SceneFileError, SplatFormat,
    SplatLayer, embedded_ids, load_embedded, parse as parse_scene,
};
pub use splat::{
    ProceduralGround, RawSplat, SogLoader, SogMeta, SogPlanes, Splat, SplatCloud, SplatError,
    SplatFileLoader, SplatLoader, SplatSorter, SplatTransform, encode_splat_file,
};
pub use stage::{FigureView, FlashView, FrameView, Stage, StageError};
pub use theme::{ColorGrade, Theme};
