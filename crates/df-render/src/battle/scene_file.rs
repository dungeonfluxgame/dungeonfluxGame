//! Battle scene descriptions as data: `content/battle-scenes/<location-id>.json`.
//!
//! Each file names the colour-grade theme, the ground and backdrop art stems, the grid, the six
//! named cameras and, optionally, a Gaussian splat scene. The files are embedded in the WebAssembly
//! build with `include_str!` rather than fetched: they are a few hundred bytes each, a missing or
//! malformed file then fails the native tests instead of a player's fight, and the display has
//! no extra request to lose. Splat scenes themselves are large and are always fetched from their
//! own bounded route.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{
    camera::{CameraPose, PoseError, ShotName},
    grid::{Cell, Grid, GridError},
    layout::{Cover, Layout, LayoutError, Obstacle},
    math::{Quat, Vec3},
    sound::Surface,
    splat::orientation_from_euler_deg,
    terrain::{HeightField, TerrainError},
    theme::Theme,
};

/// The current schema. Version 1 files (no layouts, no surface) still load.
pub const SCHEMA_VERSION: u32 = 2;
const OLDEST_SCHEMA_VERSION: u32 = 1;
/// The grid extent the authored `tactical` and `combat_est` cameras frame without being moved
/// back; larger grids push those two shots back in proportion.
const FRAMED_WIDTH_M: f32 = 12.0;
const FRAMED_DEPTH_M: f32 = 9.0;
/// The only place a splat scene may be fetched from.
pub const SPLAT_ROUTE_PREFIX: &str = "/assets/splat/";

#[derive(Clone, Debug, PartialEq)]
pub enum SceneFileError {
    Json(String),
    Schema(u32),
    Identifier(&'static str),
    Theme(String),
    Grid(GridError),
    Camera(String, PoseError),
    CameraMissing(&'static str),
    CameraUnknown(String),
    Layer(&'static str),
    Splat(&'static str),
    Terrain(TerrainError),
    Distribution(String),
    Attribution(&'static str),
    Surface(String),
    /// Layouts or a surface in a version 1 file.
    NeedsSchemaTwo(&'static str),
    Layout(LayoutError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileScene {
    schema: u32,
    location_id: String,
    title: String,
    theme: FileTheme,
    ground: FileGround,
    backdrop: FileBackdrop,
    grid: FileGrid,
    cameras: BTreeMap<String, FilePose>,
    #[serde(default)]
    splat: Option<FileSplat>,
    /// `public` (default) or `private-test-only`: a scan used for local testing under no licence.
    #[serde(default)]
    distribution: Option<String>,
    #[serde(default)]
    attribution: Option<FileAttribution>,
    #[serde(default)]
    terrain: Option<FileTerrain>,
    /// What the ground is made of, for footsteps: `wood`, `dirt` or `stone`.
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    layouts: Vec<FileLayout>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileAttribution {
    title: String,
    author: String,
    source_url: String,
    licence: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLayout {
    id: String,
    label: String,
    #[serde(default)]
    obstacles: Vec<FileObstacle>,
    #[serde(default)]
    difficult: Vec<[i32; 2]>,
    spawns: FileSpawns,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTerrain {
    origin: [f32; 2],
    step_m: f32,
    cols: u32,
    rows: u32,
    /// Ground height at each lattice point, row by row, in whole centimetres.
    heights_cm: Vec<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileObstacle {
    id: String,
    kind: String,
    cells: Vec<[i32; 2]>,
    blocks_movement: bool,
    cover: String,
    sprite: String,
    height_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileGrade {
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    exposure_ev: Option<f32>,
    #[serde(default)]
    saturation: Option<f32>,
    #[serde(default)]
    tint: Option<[f32; 3]>,
    #[serde(default)]
    sky: Option<[f32; 3]>,
    /// Brightness of the splats away from the fight, 0..1 (the fight itself is lit fully).
    #[serde(default)]
    ambient: Option<f32>,
    /// Radius of the lit pool around the grid, metres.
    #[serde(default)]
    pool_m: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpawns {
    heroes: Vec<[i32; 2]>,
    enemies: Vec<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTheme {
    name: String,
    #[serde(default)]
    strength: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileGround {
    stem: String,
    size_m: f32,
    centre: [f32; 2],
    falloff_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileBackdrop {
    stem: String,
    distance_m: f32,
    width_m: f32,
    base_m: f32,
    height_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileGrid {
    origin: [f32; 2],
    cell_m: f32,
    cols: u32,
    rows: u32,
    #[serde(default)]
    blocked: Vec<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePose {
    target: [f32; 3],
    distance: f32,
    yaw_deg: f32,
    pitch_deg: f32,
    fov_deg: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSplat {
    url: String,
    format: SplatFormat,
    #[serde(default = "one")]
    scale: f32,
    #[serde(default)]
    offset_y: f32,
    #[serde(default)]
    rot_y_deg: f32,
    #[serde(default)]
    translate: [f32; 3],
    #[serde(default)]
    max_splats: Option<u32>,
    /// Rotation about X, Y, then Z in degrees that stands the scan upright.
    #[serde(default)]
    orient_deg: [f32; 3],
    /// The finest and coarsest level present on disk (`lod` format only).
    #[serde(default)]
    levels: Option<[u32; 2]>,
    /// Radius around the grid that gets the finest level; default derived from the grid.
    #[serde(default)]
    detail_radius_m: Option<f32>,
    #[serde(default)]
    grade: Option<FileGrade>,
    /// How far behind the splat surface figures and grid tiles may sink before being hidden.
    #[serde(default)]
    depth_bias_m: Option<f32>,
    /// Cuts the scan above a height near the fight, like a ceiling cut-away, so a forest canopy
    /// does not fill the view of the arena.
    #[serde(default)]
    clip: Option<FileClip>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileClip {
    /// World height above which splats are not drawn.
    above_m: f32,
    /// Only within this horizontal distance of the grid centre (distant hills stay).
    radius_m: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplatFormat {
    /// The antimatter15 32-byte-per-splat `.splat` layout.
    Splat,
    /// PlayCanvas SOG: `meta.json` plus WebP planes, fetched beside the URL.
    Sog,
    /// The built-in procedural ground used to verify the pass without external data.
    Procedural,
    /// A streamed level-of-detail scan: `lod-meta.json` plus `<level>_<chunk>/` SOG chunks.
    Lod,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroundLayer {
    pub stem: String,
    pub size_m: f32,
    pub centre: [f32; 2],
    pub falloff_m: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackdropLayer {
    pub stem: String,
    pub distance_m: f32,
    pub width_m: f32,
    pub base_m: f32,
    pub height_m: f32,
}

/// How a scan is graded to sit with the figures: applied to every splat's colour, so a daylight
/// scan can be brought to dusk. `ambient` and `pool_m` leave a warm lit pool around the fight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplatGrade {
    pub exposure_ev: f32,
    pub saturation: f32,
    pub tint: [f32; 3],
    pub sky: [f32; 3],
    pub ambient: f32,
    pub pool_m: f32,
}

impl SplatGrade {
    pub const NEUTRAL: Self = Self {
        exposure_ev: 0.0,
        saturation: 1.0,
        tint: [1.0, 1.0, 1.0],
        sky: [0.43, 0.55, 0.72],
        ambient: 1.0,
        pool_m: 0.0,
    };
    pub const DUSK: Self = Self {
        exposure_ev: -0.3,
        saturation: 0.9,
        tint: [1.04, 0.92, 0.82],
        sky: [0.20, 0.17, 0.24],
        ambient: 0.7,
        pool_m: 12.0,
    };
    pub const NIGHT: Self = Self {
        exposure_ev: -1.3,
        saturation: 0.7,
        tint: [0.62, 0.78, 1.1],
        sky: [0.035, 0.05, 0.1],
        ambient: 0.4,
        pool_m: 9.0,
    };

    pub fn preset(name: &str) -> Option<Self> {
        match name {
            "neutral" => Some(Self::NEUTRAL),
            "dusk" => Some(Self::DUSK),
            "night" => Some(Self::NIGHT),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplatLayer {
    pub url: String,
    pub format: SplatFormat,
    pub scale: f32,
    pub offset_y: f32,
    pub rot_y_deg: f32,
    pub translate: Vec3,
    pub max_splats: Option<u32>,
    pub orient: Quat,
    /// Finest and coarsest level on disk for a `lod` scan.
    pub levels: Option<(u32, u32)>,
    pub detail_radius_m: Option<f32>,
    pub grade: SplatGrade,
    pub depth_bias_m: f32,
    /// `(height, radius)`: splats above `height` within `radius` of the grid centre are not drawn.
    pub clip: Option<(f32, f32)>,
}

/// Where a scene may be shown from: `PrivateTestOnly` scans have no licence and are used only
/// for local testing, so they never take part in the default scene rotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Distribution {
    Public,
    PrivateTestOnly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribution {
    pub title: String,
    pub author: String,
    pub source_url: String,
    pub licence: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BattleScene {
    pub location_id: String,
    pub title: String,
    pub theme: Theme,
    pub grade_strength: Option<f32>,
    pub ground: GroundLayer,
    pub backdrop: BackdropLayer,
    pub grid: Grid,
    pub cameras: BTreeMap<ShotName, CameraPose>,
    pub splat: Option<SplatLayer>,
    pub distribution: Distribution,
    pub attribution: Option<Attribution>,
    pub surface: Surface,
    /// Empty for a version 1 file, in which case figures stand on the derived formation.
    pub layouts: Vec<Layout>,
}

fn stem_ok(stem: &str) -> bool {
    !stem.is_empty()
        && stem.len() <= 64
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn positive(value: f32, maximum: f32) -> bool {
    value.is_finite() && value > 0.0 && value <= maximum
}

/// Parses and validates one scene file.
pub fn parse(json: &str) -> Result<BattleScene, SceneFileError> {
    let file: FileScene =
        serde_json::from_str(json).map_err(|error| SceneFileError::Json(error.to_string()))?;
    if !(OLDEST_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&file.schema) {
        return Err(SceneFileError::Schema(file.schema));
    }
    if file.schema < 2 && !file.layouts.is_empty() {
        return Err(SceneFileError::NeedsSchemaTwo("layouts"));
    }
    if file.schema < 2 && file.surface.is_some() {
        return Err(SceneFileError::NeedsSchemaTwo("surface"));
    }
    if !stem_ok(&file.location_id) {
        return Err(SceneFileError::Identifier("location_id"));
    }
    if file.title.trim().is_empty() || file.title.len() > 80 {
        return Err(SceneFileError::Identifier("title"));
    }
    let theme = Theme::from_key(&file.theme.name)
        .ok_or_else(|| SceneFileError::Theme(file.theme.name.clone()))?;
    if !stem_ok(&file.ground.stem) || !stem_ok(&file.backdrop.stem) {
        return Err(SceneFileError::Identifier("art stem"));
    }
    if !positive(file.ground.size_m, 200.0)
        || !(0.0..=100.0).contains(&file.ground.falloff_m)
        || file.ground.centre.iter().any(|value| !value.is_finite())
    {
        return Err(SceneFileError::Layer("ground"));
    }
    let backdrop = &file.backdrop;
    if !positive(backdrop.distance_m, 400.0)
        || !positive(backdrop.width_m, 800.0)
        || !positive(backdrop.height_m, 400.0)
        || !backdrop.base_m.is_finite()
    {
        return Err(SceneFileError::Layer("backdrop"));
    }
    let blocked: Vec<Cell> = file
        .grid
        .blocked
        .iter()
        .map(|[col, row]| Cell::new(*col, *row))
        .collect();
    let mut grid = Grid::open_except(
        file.grid.origin,
        file.grid.cell_m,
        file.grid.cols,
        file.grid.rows,
        &blocked,
    )
    .map_err(SceneFileError::Grid)?;
    if let Some(terrain) = file.terrain {
        let heights = terrain
            .heights_cm
            .iter()
            .map(|cm| *cm as f32 * 0.01)
            .collect();
        let field = HeightField::new(
            terrain.origin,
            terrain.step_m,
            terrain.cols,
            terrain.rows,
            heights,
        )
        .map_err(SceneFileError::Terrain)?;
        grid = grid.with_terrain(field);
    }
    let distribution = match file.distribution.as_deref() {
        None | Some("public") => Distribution::Public,
        Some("private-test-only") => Distribution::PrivateTestOnly,
        Some(other) => return Err(SceneFileError::Distribution(other.to_owned())),
    };
    let attribution = file.attribution.map(validate_attribution).transpose()?;
    if distribution == Distribution::PrivateTestOnly && attribution.is_none() {
        return Err(SceneFileError::Attribution(
            "private-test-only scenes name their source",
        ));
    }
    let mut cameras = BTreeMap::new();
    for (key, pose) in file.cameras {
        let shot =
            ShotName::from_key(&key).ok_or_else(|| SceneFileError::CameraUnknown(key.clone()))?;
        let pose = CameraPose {
            target: Vec3::new(pose.target[0], pose.target[1], pose.target[2]),
            distance: pose.distance,
            yaw_deg: pose.yaw_deg,
            pitch_deg: pose.pitch_deg,
            fov_deg: pose.fov_deg,
        };
        pose.validate()
            .map_err(|error| SceneFileError::Camera(key, error))?;
        cameras.insert(shot, pose);
    }
    for shot in ShotName::ALL {
        if !cameras.contains_key(&shot) {
            return Err(SceneFileError::CameraMissing(shot.key()));
        }
    }
    let splat = file.splat.map(validate_splat).transpose()?;
    let surface = match file.surface.as_deref() {
        None => Surface::default(),
        Some(key) => {
            Surface::from_key(key).ok_or_else(|| SceneFileError::Surface(key.to_owned()))?
        }
    };
    let layouts = convert_layouts(file.layouts, &grid)?;
    frame_for_grid(&mut cameras, &grid);
    Ok(BattleScene {
        location_id: file.location_id,
        title: file.title,
        theme,
        grade_strength: file.theme.strength,
        ground: GroundLayer {
            stem: file.ground.stem,
            size_m: file.ground.size_m,
            centre: file.ground.centre,
            falloff_m: file.ground.falloff_m,
        },
        backdrop: BackdropLayer {
            stem: file.backdrop.stem,
            distance_m: file.backdrop.distance_m,
            width_m: file.backdrop.width_m,
            base_m: file.backdrop.base_m,
            height_m: file.backdrop.height_m,
        },
        grid,
        cameras,
        splat,
        distribution,
        attribution,
        surface,
        layouts,
    })
}

fn validate_attribution(file: FileAttribution) -> Result<Attribution, SceneFileError> {
    let fields = [&file.title, &file.author, &file.source_url, &file.licence];
    if fields
        .iter()
        .any(|text| text.trim().is_empty() || text.len() > 160)
    {
        return Err(SceneFileError::Attribution(
            "fields are required and at most 160 bytes",
        ));
    }
    Ok(Attribution {
        title: file.title,
        author: file.author,
        source_url: file.source_url,
        licence: file.licence,
    })
}

/// Pushes the wide shots back when the grid is bigger than the authored framing covers, so a
/// 14 by 10 field fits the frame the way an 8 by 6 one does.
fn frame_for_grid(cameras: &mut BTreeMap<ShotName, CameraPose>, grid: &Grid) {
    let width = grid.cols() as f32 * grid.cell_m();
    let depth = grid.rows() as f32 * grid.cell_m();
    let scale = (width / FRAMED_WIDTH_M)
        .max(depth / FRAMED_DEPTH_M)
        .max(1.0);
    for shot in [ShotName::Tactical, ShotName::CombatEst] {
        if let Some(pose) = cameras.get_mut(&shot) {
            pose.distance *= scale;
        }
    }
}

fn convert_layouts(files: Vec<FileLayout>, grid: &Grid) -> Result<Vec<Layout>, SceneFileError> {
    let cell = |[col, row]: &[i32; 2]| Cell::new(*col, *row);
    let mut layouts: Vec<Layout> = Vec::with_capacity(files.len());
    for file in files {
        if !stem_ok(&file.id) {
            return Err(SceneFileError::Layout(LayoutError::Identifier(file.id)));
        }
        if file.label.trim().is_empty() || file.label.len() > 60 {
            return Err(SceneFileError::Layout(LayoutError::Identifier(file.label)));
        }
        if layouts.iter().any(|known| known.id == file.id) {
            return Err(SceneFileError::Layout(LayoutError::DuplicateLayout(
                file.id,
            )));
        }
        let mut obstacles = Vec::with_capacity(file.obstacles.len());
        for obstacle in file.obstacles {
            if !stem_ok(&obstacle.id) || !stem_ok(&obstacle.kind) || !stem_ok(&obstacle.sprite) {
                return Err(SceneFileError::Layout(LayoutError::Identifier(obstacle.id)));
            }
            let cover = Cover::from_key(&obstacle.cover).ok_or_else(|| {
                SceneFileError::Layout(LayoutError::Identifier(obstacle.cover.clone()))
            })?;
            obstacles.push(Obstacle {
                id: obstacle.id,
                kind: obstacle.kind,
                cells: obstacle.cells.iter().map(cell).collect(),
                blocks_movement: obstacle.blocks_movement,
                cover,
                sprite: obstacle.sprite,
                height_m: obstacle.height_m,
            });
        }
        let layout = Layout {
            id: file.id,
            label: file.label,
            obstacles,
            difficult: file.difficult.iter().map(cell).collect(),
            heroes: file.spawns.heroes.iter().map(cell).collect(),
            enemies: file.spawns.enemies.iter().map(cell).collect(),
        };
        layout.validate(grid).map_err(SceneFileError::Layout)?;
        layouts.push(layout);
    }
    Ok(layouts)
}

fn validate_splat(splat: FileSplat) -> Result<SplatLayer, SceneFileError> {
    let procedural = splat.format == SplatFormat::Procedural;
    if !procedural
        && (!splat.url.starts_with(SPLAT_ROUTE_PREFIX)
            || splat.url.len() > 128
            || splat.url.contains("..")
            || !splat.url.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.')
            }))
    {
        return Err(SceneFileError::Splat("url must be under /assets/splat/"));
    }
    if !positive(splat.scale, 100.0)
        || !splat.offset_y.is_finite()
        || !splat.rot_y_deg.is_finite()
        || splat.translate.iter().any(|value| !value.is_finite())
    {
        return Err(SceneFileError::Splat("transform"));
    }
    if splat.max_splats == Some(0) {
        return Err(SceneFileError::Splat("max_splats"));
    }
    if splat.format == SplatFormat::Lod && !splat.url.ends_with("/lod-meta.json") {
        return Err(SceneFileError::Splat("a lod scan names its lod-meta.json"));
    }
    if splat.orient_deg.iter().any(|value| !value.is_finite()) {
        return Err(SceneFileError::Splat("orient_deg"));
    }
    let levels = match splat.levels {
        Some([finest, coarsest]) if finest <= coarsest && coarsest < 32 => Some((finest, coarsest)),
        Some(_) => return Err(SceneFileError::Splat("levels")),
        None => None,
    };
    if splat
        .detail_radius_m
        .is_some_and(|radius| !positive(radius, 500.0))
        || splat
            .depth_bias_m
            .is_some_and(|bias| !(0.0..=2.0).contains(&bias))
    {
        return Err(SceneFileError::Splat("detail radius or depth bias"));
    }
    let clip = match splat.clip {
        Some(clip) if clip.above_m.is_finite() && positive(clip.radius_m, 500.0) => {
            Some((clip.above_m, clip.radius_m))
        }
        Some(_) => return Err(SceneFileError::Splat("clip")),
        None => None,
    };
    let grade = splat
        .grade
        .map(resolve_grade)
        .transpose()?
        .unwrap_or(SplatGrade::NEUTRAL);
    Ok(SplatLayer {
        url: splat.url,
        format: splat.format,
        scale: splat.scale,
        offset_y: splat.offset_y,
        rot_y_deg: splat.rot_y_deg,
        translate: Vec3::new(splat.translate[0], splat.translate[1], splat.translate[2]),
        max_splats: splat.max_splats,
        orient: orientation_from_euler_deg(splat.orient_deg),
        levels,
        detail_radius_m: splat.detail_radius_m,
        grade,
        depth_bias_m: splat.depth_bias_m.unwrap_or(0.15),
        clip,
    })
}

fn resolve_grade(file: FileGrade) -> Result<SplatGrade, SceneFileError> {
    let mut grade = match file.preset.as_deref() {
        None => SplatGrade::NEUTRAL,
        Some(name) => SplatGrade::preset(name).ok_or(SceneFileError::Splat("grade preset"))?,
    };
    grade.exposure_ev = file.exposure_ev.unwrap_or(grade.exposure_ev);
    grade.saturation = file.saturation.unwrap_or(grade.saturation);
    grade.tint = file.tint.unwrap_or(grade.tint);
    grade.sky = file.sky.unwrap_or(grade.sky);
    grade.ambient = file.ambient.unwrap_or(grade.ambient);
    grade.pool_m = file.pool_m.unwrap_or(grade.pool_m);
    let finite = [
        grade.exposure_ev,
        grade.saturation,
        grade.ambient,
        grade.pool_m,
    ]
    .into_iter()
    .chain(grade.tint)
    .chain(grade.sky)
    .all(f32::is_finite);
    if !finite
        || !(-6.0..=6.0).contains(&grade.exposure_ev)
        || !(0.0..=3.0).contains(&grade.saturation)
        || !(0.0..=1.0).contains(&grade.ambient)
        || !(0.0..=100.0).contains(&grade.pool_m)
    {
        return Err(SceneFileError::Splat("grade"));
    }
    Ok(grade)
}

macro_rules! embedded {
    ($($id:literal),+ $(,)?) => {
        const EMBEDDED: &[(&str, &str)] = &[
            $(($id, include_str!(concat!("../../../../content/battle-scenes/", $id, ".json")))),+
        ];
    };
}

embedded!(
    "salt-quay",
    "pinewood-trail",
    "windward-pass",
    "lamplit-market",
    "rookhill-gate",
    "reedmoor-boardwalk",
    "thistledown-farm",
    "orrery-library",
    "harbor-journey",
    "scan-riverbed",
    "scan-village",
    "scan-valley-bridges",
    "scan-summit",
);

/// The scene a display shows when a location has no description of its own.
pub const DEFAULT_SCENE_ID: &str = "harbor-journey";

pub fn embedded_ids() -> impl Iterator<Item = &'static str> {
    EMBEDDED.iter().map(|(id, _)| *id)
}

/// Which battle scene a fight is staged in, and why if it is not the fight's own location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneChoice {
    pub scene_id: &'static str,
    /// `None` when the location has its own scene; otherwise `fallback:<location id>` (or
    /// `fallback:none` when the view carried no location), recorded on the page for diagnosis.
    pub fallback_reason: Option<String>,
}

/// The battle scene for an adventure location id (`GameplayScene.location_id`). Everything
/// is keyed by the location, never by the painting: the story art for a fight can be any
/// `scene-place-*` or `scene-dest-*` picture and no longer names the place.
pub fn resolve_scene(location_id: &str) -> SceneChoice {
    match embedded_ids().find(|id| *id == location_id) {
        Some(scene_id) => SceneChoice {
            scene_id,
            fallback_reason: None,
        },
        None => SceneChoice {
            scene_id: DEFAULT_SCENE_ID,
            fallback_reason: Some(format!(
                "fallback:{}",
                if location_id.is_empty() {
                    "none"
                } else {
                    location_id
                }
            )),
        },
    }
}

/// Loads an embedded scene by location id.
pub fn load_embedded(location_id: &str) -> Option<Result<BattleScene, SceneFileError>> {
    EMBEDDED
        .iter()
        .find(|(id, _)| *id == location_id)
        .map(|(_, json)| parse(json))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCATIONS: [&str; 8] = [
        "salt-quay",
        "pinewood-trail",
        "windward-pass",
        "lamplit-market",
        "rookhill-gate",
        "reedmoor-boardwalk",
        "thistledown-farm",
        "orrery-library",
    ];

    #[test]
    fn every_generated_location_and_the_harbor_pack_have_a_valid_embedded_scene() {
        for id in LOCATIONS.into_iter().chain([DEFAULT_SCENE_ID]) {
            let scene = load_embedded(id)
                .unwrap_or_else(|| panic!("{id} not embedded"))
                .unwrap_or_else(|error| panic!("{id}: {error:?}"));
            assert_eq!(scene.location_id, id);
            assert_eq!(scene.cameras.len(), 6);
            assert!(scene.grid.walkable_cells().count() >= 12, "{id}");
            assert!(scene.ground.stem.starts_with("ground-"));
        }
        assert_eq!(embedded_ids().count(), 13);
    }

    #[test]
    fn every_adventure_location_resolves_to_its_own_battle_scene() {
        let table: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../content/adventure-tables/locations.json"
        ))
        .unwrap();
        let entries = table["tables"]["locations"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 8);
        for entry in entries {
            let id = entry["id"].as_str().unwrap();
            let choice = resolve_scene(id);
            assert_eq!(choice.scene_id, id);
            assert_eq!(choice.fallback_reason, None, "{id} must not fall back");
            assert_eq!(
                load_embedded(choice.scene_id).unwrap().unwrap().location_id,
                id
            );
        }
    }

    #[test]
    fn an_unknown_or_missing_location_falls_back_and_records_why() {
        let unknown = resolve_scene("moonbase-alpha");
        assert_eq!(unknown.scene_id, DEFAULT_SCENE_ID);
        assert_eq!(
            unknown.fallback_reason.as_deref(),
            Some("fallback:moonbase-alpha")
        );
        let missing = resolve_scene("");
        assert_eq!(missing.scene_id, DEFAULT_SCENE_ID);
        assert_eq!(missing.fallback_reason.as_deref(), Some("fallback:none"));
        // The fight's painting is not a location: a story art stem does not select a scene.
        let painting = resolve_scene("scene-dest-orrery-library-forecourt");
        assert!(painting.fallback_reason.is_some());
    }

    #[test]
    fn embedded_scene_ids_cover_the_location_table() {
        let table = include_str!("../../../../content/adventure-tables/locations.json");
        for id in LOCATIONS {
            assert!(table.contains(&format!("\"id\": \"{id}\"")), "{id}");
        }
    }

    fn base() -> serde_json::Value {
        serde_json::from_str(load_embedded("salt-quay").map(|_| EMBEDDED[0].1).unwrap()).unwrap()
    }

    fn rejects(edit: impl FnOnce(&mut serde_json::Value)) -> SceneFileError {
        let mut value = base();
        edit(&mut value);
        parse(&value.to_string()).expect_err("must be rejected")
    }

    #[test]
    fn unknown_schema_versions_and_fields_are_rejected() {
        assert_eq!(
            rejects(|v| v["schema"] = 3.into()),
            SceneFileError::Schema(3)
        );
        assert!(matches!(
            rejects(|v| v["surprise"] = true.into()),
            SceneFileError::Json(_)
        ));
    }

    #[test]
    fn a_missing_or_misnamed_camera_is_rejected() {
        assert_eq!(
            rejects(|v| {
                v["cameras"].as_object_mut().unwrap().remove("victory");
            }),
            SceneFileError::CameraMissing("victory")
        );
        let renamed = rejects(|v| {
            let pose = v["cameras"]["ko"].clone();
            v["cameras"]["knockout"] = pose;
        });
        assert_eq!(
            renamed,
            SceneFileError::CameraUnknown("knockout".to_owned())
        );
    }

    #[test]
    fn an_unusable_camera_or_grid_or_theme_is_rejected_with_a_typed_error() {
        assert!(matches!(
            rejects(|v| v["cameras"]["tactical"]["distance"] = 0.0.into()),
            SceneFileError::Camera(_, PoseError::DistanceOutOfRange)
        ));
        assert_eq!(
            rejects(|v| v["grid"]["cols"] = 0.into()),
            SceneFileError::Grid(GridError::EmptyGrid)
        );
        assert!(matches!(
            rejects(|v| v["theme"]["name"] = "sunny".into()),
            SceneFileError::Theme(_)
        ));
    }

    #[test]
    fn art_stems_cannot_escape_the_asset_folder() {
        assert_eq!(
            rejects(|v| v["ground"]["stem"] = "../secret".into()),
            SceneFileError::Identifier("art stem")
        );
    }

    #[test]
    fn splat_urls_are_confined_to_the_splat_route() {
        let with_splat = |url: &str| {
            let mut value = base();
            value["splat"] = serde_json::json!({"url": url, "format": "splat"});
            parse(&value.to_string())
        };
        assert!(with_splat("/assets/splat/quay.splat").is_ok());
        assert!(matches!(
            with_splat("/assets/ui/generated/quay.splat"),
            Err(SceneFileError::Splat(_))
        ));
        assert!(matches!(
            with_splat("/assets/splat/../x.splat"),
            Err(SceneFileError::Splat(_))
        ));
        assert!(matches!(
            with_splat("https://example.com/x.splat"),
            Err(SceneFileError::Splat(_))
        ));
    }

    fn scan_scene(
        edit: impl FnOnce(&mut serde_json::Value),
    ) -> Result<BattleScene, SceneFileError> {
        let mut value = base();
        value["schema"] = 2.into();
        value["splat"] = serde_json::json!({
            "url": "/assets/splat/abc/v1/lod-meta.json", "format": "lod",
            "orient_deg": [180, 0, 0], "levels": [1, 3], "max_splats": 2500000,
            "clip": {"above_m": 6.0, "radius_m": 40.0},
            "grade": {"preset": "dusk", "exposure_ev": -0.2}
        });
        edit(&mut value);
        parse(&value.to_string())
    }

    #[test]
    fn a_scan_scene_carries_its_transform_levels_grade_clip_terrain_and_attribution() {
        let scene = scan_scene(|v| {
            v["terrain"] = serde_json::json!({
                "origin": [-1.0, -1.0], "step_m": 1.0, "cols": 2, "rows": 2,
                "heights_cm": [0, 100, 200, 300]
            });
            v["distribution"] = "private-test-only".into();
            v["attribution"] = serde_json::json!({
                "title": "T", "author": "A", "source_url": "https://example.com", "licence": "none"
            });
            v["layouts"] = serde_json::json!([]);
        })
        .unwrap();
        let layer = scene.splat.as_ref().unwrap();
        assert_eq!(layer.format, SplatFormat::Lod);
        assert_eq!(layer.levels, Some((1, 3)));
        assert_eq!(layer.clip, Some((6.0, 40.0)));
        assert_eq!(
            layer.grade.exposure_ev, -0.2,
            "an explicit value overrides the preset"
        );
        assert_eq!(layer.grade.saturation, SplatGrade::DUSK.saturation);
        // A half turn about X puts the scan's -Y up.
        let up = layer.orient.rotate(Vec3::new(0.0, -1.0, 0.0));
        assert!(up.y > 0.999, "{up:?}");
        assert_eq!(scene.distribution, Distribution::PrivateTestOnly);
        assert_eq!(scene.attribution.as_ref().unwrap().author, "A");
        // The grid stands on the terrain: heights follow the stored field.
        let centre = scene.grid.cell_centre(Cell::new(0, 0));
        assert!(centre.y.is_finite());
        assert_eq!(scene.grid.ground_height(-1.0, 1.0), 2.0);
        assert_eq!(scene.grid.ground_height(-0.5, -0.5), 1.5);
    }

    #[test]
    fn scan_scene_fields_are_validated() {
        let bad = |edit: fn(&mut serde_json::Value)| scan_scene(edit).unwrap_err();
        assert!(matches!(
            bad(|v| v["splat"]["url"] = "/assets/splat/abc/v1/meta.json".into()),
            SceneFileError::Splat(_)
        ));
        assert!(matches!(
            bad(|v| v["splat"]["levels"] = serde_json::json!([3, 1])),
            SceneFileError::Splat("levels")
        ));
        assert!(matches!(
            bad(|v| v["splat"]["clip"]["radius_m"] = 0.into()),
            SceneFileError::Splat("clip")
        ));
        assert!(matches!(
            bad(|v| v["splat"]["grade"]["preset"] = "noon".into()),
            SceneFileError::Splat("grade preset")
        ));
        assert!(matches!(
            bad(|v| v["splat"]["grade"]["ambient"] = 3.into()),
            SceneFileError::Splat("grade")
        ));
        assert!(matches!(
            bad(|v| v["distribution"] = "everyone".into()),
            SceneFileError::Distribution(_)
        ));
        // A private-test-only scene must say where its scan came from.
        assert!(matches!(
            bad(|v| v["distribution"] = "private-test-only".into()),
            SceneFileError::Attribution(_)
        ));
        assert!(matches!(
            bad(|v| {
                v["terrain"] = serde_json::json!({
                    "origin": [0, 0], "step_m": 1.0, "cols": 2, "rows": 2, "heights_cm": [0, 1, 2]
                })
            }),
            SceneFileError::Terrain(_)
        ));
        // Layouts need the version 2 schema.
        assert_eq!(
            bad(|v| v["schema"] = 1.into()),
            SceneFileError::NeedsSchemaTwo("layouts")
        );
    }

    #[test]
    fn the_embedded_scan_scenes_are_private_test_only_unless_licensed_and_leave_the_rotation() {
        let mut private = 0;
        for id in embedded_ids().filter(|id| id.starts_with("scan-")) {
            let scene = load_embedded(id).unwrap().unwrap();
            let layer = scene
                .splat
                .as_ref()
                .expect("a scan scene has a splat layer");
            assert_eq!(layer.format, SplatFormat::Lod, "{id}");
            assert!(layer.url.ends_with("/v1/lod-meta.json"), "{id}");
            let attribution = scene.attribution.as_ref().expect("scans name their source");
            if scene.distribution == Distribution::PrivateTestOnly {
                private += 1;
                assert!(attribution.licence.contains("private test only"), "{id}");
            } else {
                assert_eq!(attribution.licence, "CC BY 4.0", "{id}");
            }
            assert!(
                scene.grid.terrain().is_some(),
                "{id} follows measured ground"
            );
            assert!(scene.grid.walkable_cells().count() >= 40, "{id}");
        }
        assert_eq!(private, 3, "three unlicensed scans");
        // No location of the adventure tables is a scan: the default rotation never reaches one.
        let table = include_str!("../../../../content/adventure-tables/locations.json");
        assert!(!table.contains("scan-"));
        assert!(!LOCATIONS.iter().any(|id| id.starts_with("scan-")));
    }

    #[test]
    fn every_location_has_three_distinct_valid_layouts_that_play_differently() {
        let mut total = 0;
        for id in LOCATIONS.into_iter().chain([DEFAULT_SCENE_ID]) {
            let scene = load_embedded(id).unwrap().unwrap();
            assert!(
                scene.layouts.len() >= 3,
                "{id}: {} layouts",
                scene.layouts.len()
            );
            total += scene.layouts.len();
            let signature = |layout: &Layout| {
                let mut cells: Vec<_> = layout
                    .obstacles
                    .iter()
                    .flat_map(|obstacle| obstacle.cells.clone())
                    .collect();
                cells.sort();
                cells
            };
            let mut seen = Vec::new();
            for layout in &scene.layouts {
                assert!(layout.heroes.len() >= 4 && layout.enemies.len() >= 4);
                assert_eq!(layout.validate(&scene.grid), Ok(()), "{id}/{}", layout.id);
                let shape = signature(layout);
                assert!(
                    !seen.contains(&shape),
                    "{id}/{} repeats another layout",
                    layout.id
                );
                seen.push(shape);
            }
            let ids: std::collections::BTreeSet<_> = scene.layouts.iter().map(|l| &l.id).collect();
            assert_eq!(ids.len(), scene.layouts.len(), "{id}: unique layout ids");
        }
        assert!(total >= 27);
    }

    #[test]
    fn grid_sizes_vary_from_eight_by_six_up_to_fourteen_by_ten() {
        let sizes: std::collections::BTreeSet<(u32, u32)> = LOCATIONS
            .into_iter()
            .chain([DEFAULT_SCENE_ID])
            .map(|id| {
                let scene = load_embedded(id).unwrap().unwrap();
                (scene.grid.cols(), scene.grid.rows())
            })
            .collect();
        assert!(sizes.len() >= 6, "{sizes:?}");
        assert!(sizes.contains(&(8, 6)) && sizes.contains(&(14, 10)));
        assert!(
            sizes
                .iter()
                .all(|(c, r)| (8..=14).contains(c) && (6..=10).contains(r))
        );
    }

    #[test]
    fn every_location_declares_its_ground_surface() {
        let surfaces: Vec<(&str, Surface)> = [
            ("salt-quay", Surface::Wood),
            ("reedmoor-boardwalk", Surface::Wood),
            ("orrery-library", Surface::Wood),
            ("pinewood-trail", Surface::Dirt),
            ("thistledown-farm", Surface::Dirt),
            ("lamplit-market", Surface::Stone),
            ("rookhill-gate", Surface::Stone),
            ("windward-pass", Surface::Stone),
        ]
        .into();
        for (id, surface) in surfaces {
            assert_eq!(load_embedded(id).unwrap().unwrap().surface, surface, "{id}");
        }
    }

    #[test]
    fn the_wide_shots_pull_back_for_bigger_grids_and_not_for_the_authored_size() {
        let library = load_embedded("orrery-library").unwrap().unwrap();
        let quay = load_embedded("harbor-journey").unwrap().unwrap();
        let tactical = |scene: &BattleScene| scene.cameras[&ShotName::Tactical].distance;
        assert_eq!(tactical(&quay), 15.0);
        assert!(tactical(&library) > 15.0 * 1.3);
        let focus = |scene: &BattleScene| scene.cameras[&ShotName::TurnFocus].distance;
        assert_eq!(
            focus(&library),
            focus(&quay),
            "close shots follow the figure instead"
        );
    }

    fn v1() -> serde_json::Value {
        let mut value = base();
        value["schema"] = 1.into();
        let object = value.as_object_mut().unwrap();
        object.remove("layouts");
        object.remove("surface");
        value
    }

    #[test]
    fn a_version_one_file_still_loads_with_no_layouts() {
        let scene = parse(&v1().to_string()).unwrap();
        assert!(scene.layouts.is_empty());
        assert_eq!(scene.surface, Surface::Stone);
    }

    #[test]
    fn layouts_and_a_surface_need_schema_two() {
        let mut with_layouts = v1();
        with_layouts["layouts"] = base()["layouts"].clone();
        assert_eq!(
            parse(&with_layouts.to_string()),
            Err(SceneFileError::NeedsSchemaTwo("layouts"))
        );
        let mut with_surface = v1();
        with_surface["surface"] = "wood".into();
        assert_eq!(
            parse(&with_surface.to_string()),
            Err(SceneFileError::NeedsSchemaTwo("surface"))
        );
    }

    #[test]
    fn a_bad_surface_cover_spawn_or_cut_field_is_rejected() {
        assert!(matches!(
            rejects(|v| v["surface"] = "lava".into()),
            SceneFileError::Surface(_)
        ));
        assert!(matches!(
            rejects(|v| v["layouts"][0]["obstacles"][0]["cover"] = "full".into()),
            SceneFileError::Layout(LayoutError::Identifier(_))
        ));
        assert!(matches!(
            rejects(|v| v["layouts"][0]["spawns"]["heroes"] = serde_json::json!([[1, 1]])),
            SceneFileError::Layout(LayoutError::TooFewSpawns(_))
        ));
        assert!(matches!(
            rejects(|v| v["layouts"][1] = v["layouts"][0].clone()),
            SceneFileError::Layout(LayoutError::DuplicateLayout(_))
        ));
        // A full-width wall of crates cuts every hero off from every enemy.
        let wall: Vec<[i32; 2]> = (0..10).map(|col| [col, 3]).collect();
        assert!(matches!(
            rejects(|v| v["layouts"][0]["obstacles"] = serde_json::json!([{
                "id": "wall", "kind": "crate", "cells": wall, "blocks_movement": true,
                "cover": "half", "sprite": "prop-crates", "height_m": 1.0
            }])),
            SceneFileError::Layout(LayoutError::Unreachable(..))
        ));
    }
}
