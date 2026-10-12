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
    math::Vec3,
    sound::Surface,
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
    /// What the ground is made of, for footsteps: `wood`, `dirt` or `stone`.
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    layouts: Vec<FileLayout>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct SplatLayer {
    pub url: String,
    pub format: SplatFormat,
    pub scale: f32,
    pub offset_y: f32,
    pub rot_y_deg: f32,
    pub translate: Vec3,
    pub max_splats: Option<u32>,
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
    let grid = Grid::open_except(
        file.grid.origin,
        file.grid.cell_m,
        file.grid.cols,
        file.grid.rows,
        &blocked,
    )
    .map_err(SceneFileError::Grid)?;
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
        Some(key) => Surface::from_key(key).ok_or_else(|| SceneFileError::Surface(key.to_owned()))?,
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
        surface,
        layouts,
    })
}

/// Pushes the wide shots back when the grid is bigger than the authored framing covers, so a
/// 14 by 10 field fits the frame the way an 8 by 6 one does.
fn frame_for_grid(cameras: &mut BTreeMap<ShotName, CameraPose>, grid: &Grid) {
    let width = grid.cols() as f32 * grid.cell_m();
    let depth = grid.rows() as f32 * grid.cell_m();
    let scale = (width / FRAMED_WIDTH_M).max(depth / FRAMED_DEPTH_M).max(1.0);
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
            return Err(SceneFileError::Layout(LayoutError::DuplicateLayout(file.id)));
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
    Ok(SplatLayer {
        url: splat.url,
        format: splat.format,
        scale: splat.scale,
        offset_y: splat.offset_y,
        rot_y_deg: splat.rot_y_deg,
        translate: Vec3::new(splat.translate[0], splat.translate[1], splat.translate[2]),
        max_splats: splat.max_splats,
    })
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
);

/// The scene a display shows when a location has no description of its own.
pub const DEFAULT_SCENE_ID: &str = "harbor-journey";

pub fn embedded_ids() -> impl Iterator<Item = &'static str> {
    EMBEDDED.iter().map(|(id, _)| *id)
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
        assert_eq!(embedded_ids().count(), 9);
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
}
