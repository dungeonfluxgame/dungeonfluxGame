//! The home screen's theme (PLAY-V05), decided by the server from the painting it is about to show.
//! The first time a location painting is used, its bytes are decoded and measured, and the
//! measures are cached by the painting's content hash. The chosen arrangement is the one whose
//! blocks leave the painting's focal region clear and sit on the calmest, darkest ground; the mood
//! comes from the painting's own colours and light; the atmosphere follows the location's kind and
//! those same measures. Clients never analyse art or choose a layout.
//!
//! A painting that cannot be read, decoded, or laid out returns a typed error to the caller, and
//! [`lobby_view`] then sends the documented fallback: today's layout with a neutral mood, flagged
//! `fallback` so the browser knows the view has no theme.
pub(crate) mod analysis;

use super::adventure::setting::{SceneSetting, SettingKind};
use super::rpc;
use analysis::{AnalysisError, Measures, MoodMeasures};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// The asset URL prefix every painting path starts with; the rest is relative to the asset root.
pub(crate) const PAINTING_PREFIX: &str = "assets/ui/";
/// The served painting formats, in the order the scene resolution looks for them.
const PAINTING_FORMATS: [&str; 3] = ["webp", "png", "jpg"];
/// Bounds on the painting input: the file, its decoded size and its decoder allocation.
const MAX_PAINTING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PAINTING_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ATMOSPHERES: usize = 2;
/// The mood a painting without a theme shows: the existing gold on navy, with the grade unchanged.
const NEUTRAL_ACCENT: &str = "#e8cb94";
const NEUTRAL_TINT: &str = "#0b1623";
const NEUTRAL_VIGNETTE: u32 = 550;
const NEUTRAL_GRADE: u32 = 1000;
/// The arrangement the stylesheet draws without a theme, which the fallback view names.
const FALLBACK_ARRANGEMENT: &str = "title-top-left-panels-bottom";

/// The closed set of atmosphere effects. The browser draws each as a looping overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Atmosphere {
    Mist,
    LanternGlow,
    Fireflies,
    Embers,
    FallingLeaves,
    Rain,
    DustMotes,
    WaterGlints,
    CloudShadow,
}

impl Atmosphere {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Mist => "mist",
            Self::LanternGlow => "lantern-glow",
            Self::Fireflies => "fireflies",
            Self::Embers => "embers",
            Self::FallingLeaves => "falling-leaves",
            Self::Rain => "rain",
            Self::DustMotes => "dust-motes",
            Self::WaterGlints => "water-glints",
            Self::CloudShadow => "cloud-shadow",
        }
    }
}

/// A block's box in permille of the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Boxed {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

impl Boxed {
    const fn new(left: u32, top: u32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    fn as_array(self) -> [u32; 4] {
        [self.left, self.top, self.width, self.height]
    }

    fn to_wire(self) -> rpc::HomeBox {
        rpc::HomeBox {
            left_permille: self.left,
            top_permille: self.top,
            width_permille: self.width,
            height_permille: self.height,
        }
    }
}

/// The three blocks of one named arrangement. Each party box is sized for six heroes, so a
/// smaller party only fills part of the same space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arrangement {
    pub(crate) name: &'static str,
    title: Boxed,
    code: Boxed,
    party: Boxed,
}

impl Arrangement {
    pub(crate) fn boxes(&self) -> [[u32; 4]; 3] {
        [self.title, self.code, self.party].map(Boxed::as_array)
    }
}

/// The named arrangements a home screen may use: a handful of composed layouts, never free-form.
pub(crate) const ARRANGEMENTS: [Arrangement; 7] = [
    Arrangement {
        name: FALLBACK_ARRANGEMENT,
        title: Boxed::new(45, 50, 520, 420),
        code: Boxed::new(40, 600, 430, 360),
        party: Boxed::new(490, 590, 480, 370),
    },
    Arrangement {
        name: "title-top-right-panels-stacked-left",
        title: Boxed::new(490, 50, 470, 420),
        code: Boxed::new(40, 300, 430, 340),
        party: Boxed::new(40, 650, 500, 320),
    },
    Arrangement {
        name: "title-centre-panels-bottom-corners",
        title: Boxed::new(230, 190, 540, 420),
        code: Boxed::new(30, 625, 420, 340),
        party: Boxed::new(500, 625, 480, 340),
    },
    Arrangement {
        name: "title-top-right-panels-bottom-band",
        title: Boxed::new(490, 50, 470, 420),
        code: Boxed::new(40, 560, 430, 360),
        party: Boxed::new(490, 560, 480, 400),
    },
    Arrangement {
        name: "title-top-left-narrow-panels-bottom",
        title: Boxed::new(45, 50, 410, 500),
        code: Boxed::new(40, 580, 430, 380),
        party: Boxed::new(490, 580, 480, 380),
    },
    Arrangement {
        name: "title-top-right-narrow-panels-bottom",
        title: Boxed::new(550, 50, 410, 500),
        code: Boxed::new(40, 580, 430, 380),
        party: Boxed::new(490, 580, 480, 380),
    },
    Arrangement {
        name: "single-bottom-band",
        title: Boxed::new(45, 50, 520, 420),
        code: Boxed::new(30, 600, 400, 360),
        party: Boxed::new(460, 600, 500, 360),
    },
];

/// One painting's theme: the arrangement, the mood and the atmosphere.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Theme {
    pub(crate) arrangement: &'static Arrangement,
    pub(crate) mood: MoodMeasures,
    pub(crate) atmosphere: Vec<Atmosphere>,
}

#[derive(Debug)]
pub(crate) enum HomeThemeError {
    NotInstalled,
    AlreadyInstalled,
    AssetRootMissing {
        path: PathBuf,
    },
    CacheUnavailable,
    Painting {
        painting: String,
        reason: &'static str,
    },
    Read {
        painting: String,
        source: io::Error,
    },
    Decode {
        painting: String,
        message: String,
    },
    Analysis {
        painting: String,
        reason: AnalysisError,
    },
    NoUncoveredArrangement {
        painting: String,
    },
}

impl fmt::Display for HomeThemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => formatter.write_str("home themes are not installed"),
            Self::AlreadyInstalled => formatter.write_str("home themes are already installed"),
            Self::AssetRootMissing { path } => {
                write!(
                    formatter,
                    "home theme asset root {} is not a directory",
                    path.display()
                )
            }
            Self::CacheUnavailable => formatter.write_str("home theme cache is unavailable"),
            Self::Painting { painting, reason } => {
                write!(formatter, "home theme {painting}: {reason}")
            }
            Self::Read { painting, source } => {
                write!(formatter, "home theme {painting} unreadable: {source}")
            }
            Self::Decode { painting, message } => {
                write!(
                    formatter,
                    "home theme {painting} does not decode: {message}"
                )
            }
            Self::Analysis { painting, reason } => {
                write!(
                    formatter,
                    "home theme {painting} cannot be measured: {reason:?}"
                )
            }
            Self::NoUncoveredArrangement { painting } => write!(
                formatter,
                "home theme {painting}: every arrangement covers its focal region"
            ),
        }
    }
}

impl std::error::Error for HomeThemeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Measures are cached per content hash. `hashes` remembers which hash a painting path had, so a
/// painting's bytes are read and hashed once per process.
#[derive(Default)]
struct Cache {
    hashes: BTreeMap<String, [u8; 32]>,
    measures: BTreeMap<[u8; 32], Measures>,
}

static ASSET_ROOT: OnceLock<PathBuf> = OnceLock::new();
static CACHE: Mutex<Cache> = Mutex::new(Cache {
    hashes: BTreeMap::new(),
    measures: BTreeMap::new(),
});

/// Records the asset root the server was started with; paintings are read beneath it.
pub(crate) fn install(asset_root: &Path) -> Result<(), HomeThemeError> {
    if !asset_root.is_dir() {
        return Err(HomeThemeError::AssetRootMissing {
            path: asset_root.to_path_buf(),
        });
    }
    ASSET_ROOT
        .set(asset_root.to_path_buf())
        .map_err(|_| HomeThemeError::AlreadyInstalled)
}

/// The home view of one lobby: the theme for `painting` and the location's setting, or the
/// documented fallback when no theme can be made.
pub(crate) fn lobby_view(painting: &str, setting: Option<&SceneSetting>) -> rpc::HomeTheme {
    let location = setting.map_or_else(String::new, |setting| setting.place_id.clone());
    let kind = setting.map(|setting| setting.kind);
    let theme = match ASSET_ROOT.get() {
        Some(root) => theme_for(root, painting, kind),
        None => Err(HomeThemeError::NotInstalled),
    };
    match theme {
        Ok(theme) => theme_view(&theme, location),
        Err(_) => fallback_view(location),
    }
}

/// The theme of one painting under `root`. Every failure is a typed error naming the painting.
pub(crate) fn theme_for(
    root: &Path,
    painting: &str,
    kind: Option<SettingKind>,
) -> Result<Theme, HomeThemeError> {
    let measures = measures_for(root, painting)?;
    let arrangement =
        choose_arrangement(&measures).ok_or_else(|| HomeThemeError::NoUncoveredArrangement {
            painting: painting.to_owned(),
        })?;
    Ok(Theme {
        arrangement,
        mood: measures.mood.clone(),
        atmosphere: atmosphere_for(kind, &measures),
    })
}

/// The arrangement with the least penalty among those that leave the focal region clear at every
/// reference viewport; `None` when every arrangement covers it somewhere.
pub(crate) fn choose_arrangement(measures: &Measures) -> Option<&'static Arrangement> {
    ARRANGEMENTS
        .iter()
        .map(|arrangement| (arrangement, analysis::fit(arrangement.boxes(), measures)))
        .filter(|(_, fit)| fit.is_clear())
        .min_by(|left, right| left.1.penalty.total_cmp(&right.1.penalty))
        .map(|(arrangement, _)| arrangement)
}

fn measures_for(root: &Path, painting: &str) -> Result<Measures, HomeThemeError> {
    let known = with_cache(|cache| {
        cache
            .hashes
            .get(painting)
            .copied()
            .and_then(|hash| cache.measures.get(&hash).cloned())
    })?;
    if let Some(measures) = known {
        return Ok(measures);
    }
    let path = painting_file(root, painting)?;
    let bytes = read_painting(painting, &path)?;
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    let cached = with_cache(|cache| cache.measures.get(&hash).cloned())?;
    let measures = match cached {
        Some(measures) => measures,
        None => {
            let measures = decode_and_measure(painting, &bytes)?;
            with_cache(|cache| {
                cache.measures.insert(hash, measures.clone());
            })?;
            measures
        }
    };
    with_cache(|cache| {
        cache.hashes.insert(painting.to_owned(), hash);
    })?;
    Ok(measures)
}

fn with_cache<R>(use_cache: impl FnOnce(&mut Cache) -> R) -> Result<R, HomeThemeError> {
    let mut guard = CACHE.lock().map_err(|_| HomeThemeError::CacheUnavailable)?;
    Ok(use_cache(&mut guard))
}

/// The file a painting path names. A scene's path is a stem without an extension, so the file is
/// the first of the served formats that exists, as the scene resolution itself looks for it.
pub(crate) fn painting_file(root: &Path, painting: &str) -> Result<PathBuf, HomeThemeError> {
    let relative = painting
        .strip_prefix(PAINTING_PREFIX)
        .filter(|relative| {
            relative
                .split('/')
                .all(|part| !part.is_empty() && part != "..")
        })
        .ok_or_else(|| HomeThemeError::Painting {
            painting: painting.to_owned(),
            reason: "painting path is outside the asset root",
        })?;
    let named = root.join(relative);
    if named.extension().is_some() {
        return Ok(named);
    }
    PAINTING_FORMATS
        .iter()
        .map(|format| named.with_extension(format))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| HomeThemeError::Painting {
            painting: painting.to_owned(),
            reason: "no painting file with a served format exists",
        })
}

fn read_painting(painting: &str, path: &Path) -> Result<Vec<u8>, HomeThemeError> {
    let metadata = fs::metadata(path).map_err(|source| HomeThemeError::Read {
        painting: painting.to_owned(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() > MAX_PAINTING_BYTES {
        return Err(HomeThemeError::Painting {
            painting: painting.to_owned(),
            reason: "painting is not a file within the size bound",
        });
    }
    fs::read(path).map_err(|source| HomeThemeError::Read {
        painting: painting.to_owned(),
        source,
    })
}

fn decode_and_measure(painting: &str, bytes: &[u8]) -> Result<Measures, HomeThemeError> {
    let decode_error = |message: String| HomeThemeError::Decode {
        painting: painting.to_owned(),
        message,
    };
    let mut reader = image::ImageReader::new(io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|source| decode_error(source.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PAINTING_SIDE);
    limits.max_image_height = Some(MAX_PAINTING_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|source| decode_error(source.to_string()))?;
    analysis::measure(&decoded.into_rgb8()).map_err(|reason| HomeThemeError::Analysis {
        painting: painting.to_owned(),
        reason,
    })
}

/// Which effects suit this place. The kind of place names a short list; the painting's own
/// brightness and warm or cool balance decide which of those the scene can carry.
pub(crate) fn atmosphere_for(kind: Option<SettingKind>, measures: &Measures) -> Vec<Atmosphere> {
    use Atmosphere::*;
    let candidates: &[Atmosphere] = match kind {
        Some(SettingKind::Quay | SettingKind::MarshBoardwalk) => &[WaterGlints, Mist],
        Some(SettingKind::ForestTrail) => &[Fireflies, Mist],
        Some(SettingKind::MountainPass) => &[CloudShadow, LanternGlow],
        Some(SettingKind::MarketStreet) => &[LanternGlow, Embers],
        Some(SettingKind::GraveyardGate) => &[Mist, LanternGlow],
        Some(SettingKind::Farmyard) => &[FallingLeaves, Fireflies],
        Some(SettingKind::LibraryHall | SettingKind::LibraryForecourt) => &[DustMotes, LanternGlow],
        _ if measures.mean_luminance < DIM => &[Rain, Mist],
        _ if measures.warm_balance > 0.0 => &[LanternGlow, Embers],
        _ => &[Mist, CloudShadow],
    };
    candidates
        .iter()
        .copied()
        .filter(|effect| suits(*effect, measures))
        .take(MAX_ATMOSPHERES)
        .collect()
}

/// Mean luminance below this is a dim painting; above `BRIGHT` it is a bright one.
const DIM: f64 = 0.35;
const BRIGHT: f64 = 0.25;

fn suits(effect: Atmosphere, measures: &Measures) -> bool {
    let dim = measures.mean_luminance < DIM;
    let bright = measures.mean_luminance > BRIGHT;
    let warm = measures.warm_balance > 0.0;
    match effect {
        Atmosphere::Mist => !warm,
        Atmosphere::LanternGlow => dim || warm,
        Atmosphere::Fireflies | Atmosphere::Rain => dim,
        Atmosphere::Embers => warm && dim,
        Atmosphere::FallingLeaves => warm && bright,
        Atmosphere::DustMotes | Atmosphere::CloudShadow => bright,
        Atmosphere::WaterGlints => !dim,
    }
}

fn theme_view(theme: &Theme, location: String) -> rpc::HomeTheme {
    rpc::HomeTheme {
        location_id: location,
        arrangement: theme.arrangement.name.to_owned(),
        title: Some(theme.arrangement.title.to_wire()),
        code: Some(theme.arrangement.code.to_wire()),
        party: Some(theme.arrangement.party.to_wire()),
        mood: Some(mood_view(&theme.mood)),
        atmosphere: theme
            .atmosphere
            .iter()
            .map(|effect| effect.name().to_owned())
            .collect(),
        fallback: false,
    }
}

fn mood_view(mood: &MoodMeasures) -> rpc::HomeMood {
    rpc::HomeMood {
        accent: mood.accent.clone(),
        tint: mood.tint.clone(),
        vignette_permille: mood.vignette_permille,
        brightness_permille: mood.brightness_permille,
        contrast_permille: mood.contrast_permille,
        saturation_permille: mood.saturation_permille,
    }
}

/// Today's layout with the neutral mood. The blocks are unset, so the stylesheet places them
/// exactly as it did before home themes existed.
fn fallback_view(location: String) -> rpc::HomeTheme {
    rpc::HomeTheme {
        location_id: location,
        arrangement: FALLBACK_ARRANGEMENT.to_owned(),
        title: None,
        code: None,
        party: None,
        mood: Some(mood_view(&MoodMeasures {
            accent: NEUTRAL_ACCENT.to_owned(),
            tint: NEUTRAL_TINT.to_owned(),
            vignette_permille: NEUTRAL_VIGNETTE,
            brightness_permille: NEUTRAL_GRADE,
            contrast_permille: NEUTRAL_GRADE,
            saturation_permille: NEUTRAL_GRADE,
        })),
        atmosphere: Vec::new(),
        fallback: true,
    }
}
