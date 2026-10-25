//! Server-authored fighter clips. The shared hub owns admission, failure and spend; this
//! service owns only revision-pinned source requests and the files a ready handle names.
//! No presentation field is changed: an unavailable clip leaves the existing cutout intact.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::Cursor,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak},
    time::{Duration, Instant},
};

use df_model::checkpoint::{EntityId, GameState};
use df_providers::{GeneratedImage, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::resources::{
    Chrome, Generated, Hub, Limits, Maker, ResourceFailure, ResourceKey, ResourceKind,
    ResourceSpec, Unit,
};

/// The live research camera. Change this when the authoring home camera changes.
pub(super) const HOME_ANGLE_VERSION: &str = "battle-home-15deg-38vfov-v1";
const CAMERA: &str = "Camera: locked tripod at the battle camera home angle, looking down 15 degrees at the subject, 38 degree vertical field of view, full body in frame with both feet visible, three-quarter view with the body turned toward screen-right. The camera never moves, pans, tilts, zooms, rotates, reframes or changes focus.";
const LOOK: &str = "A real photograph from a live-action fantasy television drama: practical costume and worn real props, natural skin and fabric texture, restrained colour, soft key light from the upper left, fine film grain. Not an illustration, not a game render.";
const FRAMING: &str = "Portrait 2:3 frame. Keep the complete body and entire weapon inside the canvas throughout, including the strike or fall. Leave broad clear margins. Keep the same identity, costume, proportions, weapon, scale and camera as the reference.";
const DEADLINE: Duration = Duration::from_secs(900);
/// The length of every clip, in seconds.
const CLIP_SECONDS: u32 = 5;
/// An image edit's whole request; the shared native HTTP client refuses anything over 180 s.
const IMAGE_EDIT_TIMEOUT: Duration = Duration::from_secs(180);
const MAXIMUM_CLIP_BYTES: usize = 16 * 1024 * 1024;
const SAMPLE_PIXELS: usize = 64 * 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ClipKind {
    Idle,
    Attack,
    LastHit,
}

impl ClipKind {
    pub(super) const ALL: [Self; 3] = [Self::Idle, Self::Attack, Self::LastHit];

    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "idle" => Some(Self::Idle),
            "attack" => Some(Self::Attack),
            "last-hit" => Some(Self::LastHit),
            _ => None,
        }
    }

    /// The clip's name in cost events.
    const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Attack => "attack",
            Self::LastHit => "last_hit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AttackStyle {
    Melee,
    Thrown,
    Ranged,
    Spell,
}

impl AttackStyle {
    fn weapon(weapon: df_rules::tactical::Weapon) -> Self {
        if weapon.ranged_weapon() {
            Self::Ranged
        } else if weapon.range().is_some() {
            Self::Thrown
        } else {
            Self::Melee
        }
    }
}

/// Identity comes from the server roster, never from words in a player's utterance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Fighter {
    pub(super) identity: String,
    pub(super) identity_revision: String,
    pub(super) depiction_subject: String,
    /// An exact reviewed NPC full-body photograph, never a stock hero-seat substitute.
    #[serde(default)]
    pub(super) reviewed_figure: Option<String>,
    pub(super) name: String,
    pub(super) weapon: String,
    pub(super) attack_style: AttackStyle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct CharacterClipSpec {
    fighter: Fighter,
    depiction_revision: String,
    kind: ClipKind,
    home_angle_version: String,
}

impl CharacterClipSpec {
    fn new(fighter: Fighter, depiction_revision: String, kind: ClipKind) -> Self {
        Self {
            fighter,
            depiction_revision,
            kind,
            home_angle_version: HOME_ANGLE_VERSION.to_owned(),
        }
    }
}

impl ResourceSpec for CharacterClipSpec {
    const KIND: ResourceKind = ResourceKind::CharacterClip;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        // Display names are diagnostic, not identity. Every pixel-affecting dependency is keyed.
        json!({
            "identity": self.fighter.identity,
            "identity_revision": self.fighter.identity_revision,
            "depiction_revision": self.depiction_revision,
            "kind": self.kind,
            "weapon": self.fighter.weapon,
            "attack_style": self.fighter.attack_style,
            "home_angle_version": self.home_angle_version,
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("character-clips:{}", self.fighter.identity)
    }

    fn chrome(&self) -> Chrome {
        Chrome::Nothing
    }
}

/// The intermediate is a hub resource too; all three clips share precisely this guard key.
struct FrameSpec {
    clip: CharacterClipSpec,
    fallen: bool,
}

impl ResourceSpec for FrameSpec {
    const KIND: ResourceKind = ResourceKind::CharacterClip;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        let mut guard = self.clip.clone();
        guard.kind = ClipKind::Idle;
        json!({"frame": if self.fallen { "fallen" } else { "guard" }, "source": guard.canonical()})
            .to_string()
    }

    fn subject(&self) -> String {
        self.clip.subject()
    }

    fn chrome(&self) -> Chrome {
        Chrome::Nothing
    }
}

pub(super) struct Depiction {
    pub(super) bytes: Vec<u8>,
    pub(super) format: ImageFormat,
    /// The selected canonical depiction key, in addition to the hash of its actual bytes.
    pub(super) revision: String,
}

pub(super) type SourceFuture<'a> = Pin<Box<dyn Future<Output = Option<Depiction>> + Send + 'a>>;
pub(super) type Source = Arc<dyn for<'a> Fn(&'a Fighter) -> SourceFuture<'a> + Send + Sync>;
type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ResourceFailure>> + Send + 'a>>;

pub(super) trait FrameProvider: Send + Sync {
    fn frame<'a>(
        &'a self,
        source: &'a Depiction,
        prompt: &'a str,
    ) -> ProviderFuture<'a, GeneratedImage>;
}

pub(super) struct Video {
    bytes: Vec<u8>,
}

pub(super) trait VideoProvider: Send + Sync {
    fn video<'a>(
        &'a self,
        key: &'a ResourceKey,
        start: &'a [u8],
        end: &'a [u8],
        prompt: &'a str,
    ) -> ProviderFuture<'a, Video>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Cost {
    image_calls: u64,
    image_micro_dollars: Option<u64>,
    video_calls: u64,
    video_micro_dollars: Option<u64>,
    #[serde(default)]
    video_cost_basis: Option<String>,
}

/// Times are measured from decoded frames, relative to the clip's first frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ClipMetadata {
    pub(super) frame_count: u64,
    pub(super) fps: f64,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bytes: u64,
    pub(super) duration_ms: u64,
    pub(super) contact_ms: Option<u64>,
    pub(super) return_to_guard_ms: Option<u64>,
    pub(super) settle_ms: Option<u64>,
    timing_method: String,
    guard_key: String,
    fallen_key: Option<String>,
    cost: Cost,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failure(label: &'static str) -> ResourceFailure {
    ResourceFailure::Permanent(label)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn store_json(path: &Path, value: &impl Serialize) -> Result<(), ResourceFailure> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| failure("clip_metadata_invalid"))?;
    std::fs::write(path, bytes).map_err(|_| ResourceFailure::Transient("clip_store_failed"))
}

fn read_cost(directory: &Path) -> Result<Cost, ResourceFailure> {
    match std::fs::read(directory.join("cost.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| failure("clip_cost_unreadable")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Cost::default()),
        Err(_) => Err(failure("clip_cost_unreadable")),
    }
}

fn frame_prompt(fighter: &Fighter, fallen: bool) -> String {
    let pose = if fallen {
        "Now the same character lies fallen on the ground on their side after a final blow, limp, eyes closed, head toward screen-left, weapon dropped beside them; one figure only, no blood. The body lies where the feet stood, near the bottom of the frame, at the same scale as the reference."
    } else {
        "Re-pose and re-frame only. Ready fighting guard, knees flexed, weight balanced, weapon raised and ready, calm alert expression. Figure about 70 percent of frame height, feet near the bottom with a small margin."
    };
    format!(
        "{LOOK} Keep exactly the character in the reference. {CAMERA} {pose} Weapon: {}. {FRAMING} Transparent background with only the character, no floor, scenery, shadow or text.",
        fighter.weapon
    )
}

fn motion_prompt(spec: &CharacterClipSpec) -> String {
    let motion = match (spec.kind, spec.fighter.attack_style) {
        (ClipKind::Idle, _) => {
            "Subtle breathing in a ready guard; no strike, walking or new objects. Finish in exactly the starting guard for a seamless loop."
        }
        (ClipKind::LastHit, _) => {
            "One final blow hits the character: recoil, stagger, then fall limply onto their side. Weapon drops beside them. Settle completely in the supplied fallen end pose and remain still. No blood, no recovery."
        }
        (ClipKind::Attack, AttackStyle::Melee) => {
            "One real weapon strike toward screen-right: wind up, strike with the weapon's real cutting, thrusting or bludgeoning action, then return to exactly the starting ready guard. No target or other person appears."
        }
        (ClipKind::Attack, AttackStyle::Thrown) => {
            "One real throw toward screen-right. Wind up, visibly RELEASE the weapon from the hand; the weapon flies away to the right. Do not thrust while holding it. Then draw another matching weapon and return to exactly the starting ready guard. No target appears."
        }
        (ClipKind::Attack, AttackStyle::Ranged) => {
            "One real ranged weapon shot toward screen-right. Draw or cock the weapon, aim, visibly RELEASE its projectile, follow through, then return to exactly the starting ready guard. No target appears."
        }
        (ClipKind::Attack, AttackStyle::Spell) => {
            "One spell-casting gesture toward screen-right, then return to exactly the starting ready guard. No target appears."
        }
    };
    format!(
        "{CAMERA} {FRAMING} Uniform perfectly flat chroma green RGB 0,177,64, including the ground, no horizon, scenery, shadows, text or other people. Weapon: {}. {motion}",
        spec.fighter.weapon
    )
}

fn green_frame(bytes: &[u8]) -> Result<Vec<u8>, ResourceFailure> {
    let image = image::load_from_memory(bytes).map_err(|_| failure("clip_frame_undecodable"))?;
    if image.width() != 1024 || image.height() != 1536 || !image.color().has_alpha() {
        return Err(failure("clip_frame_dimensions_or_alpha"));
    }
    let mut plate = image::RgbaImage::from_pixel(1024, 1536, image::Rgba([0, 177, 64, 255]));
    image::imageops::overlay(&mut plate, &image.to_rgba8(), 0, 0);
    // Research used a smaller 2:3 input; retain full-size alpha and a bounded green input.
    let plate = image::DynamicImage::ImageRgba8(plate)
        .resize_exact(768, 1152, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(plate)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| failure("clip_frame_encode_failed"))?;
    Ok(output.into_inner())
}

#[derive(Clone)]
struct Generation {
    hub: Hub,
    frames: Option<Arc<dyn FrameProvider>>,
    videos: Option<Arc<dyn VideoProvider>>,
    directory: PathBuf,
    // Hub::render does not coalesce concurrent calls for one key, so callers of the same frame
    // key wait on its gate and the frame is drawn once. Other frames are drawn concurrently.
    frame_gates: Arc<Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl Generation {
    fn frame_gate(&self, key: &ResourceKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = locked(&self.frame_gates);
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key.as_str()).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key.as_str().to_owned(), Arc::downgrade(&gate));
        gate
    }

    async fn frame(
        &self,
        spec: &CharacterClipSpec,
        source: Arc<Depiction>,
        fallen: bool,
        explicit_retry: bool,
    ) -> Result<ResourceKey, ResourceFailure> {
        let frame_spec = FrameSpec {
            clip: spec.clone(),
            fallen,
        };
        let key = ResourceKey::of(&frame_spec);
        let directory = self.directory.join(key.as_str());
        let gate = self.frame_gate(&key);
        let _guard = gate.lock().await;
        let service = self.clone();
        let fighter = spec.fighter.clone();
        let frame_source = spec.clone();
        let work_key = key.clone();
        self.hub
            .render(
                &frame_spec,
                Box::pin(async move {
                    if stored_frame(&directory) {
                        return Ok(Generated::new(work_key.as_str()));
                    }
                    let provider = service
                        .frames
                        .as_ref()
                        .ok_or_else(|| failure("clip_image_provider_unavailable"))?;
                    if source.bytes.is_empty() || source.bytes.len() > 4 * 1024 * 1024 {
                        return Err(failure("clip_source_size_invalid"));
                    }
                    std::fs::create_dir_all(&directory)
                        .map_err(|_| failure("clip_store_failed"))?;
                    store_json(
                        &directory.join("frame.json"),
                        &json!({
                            "frame": if fallen { "fallen" } else { "guard" },
                            "source": frame_source,
                        }),
                    )?;
                    let mut cost = read_cost(&directory)?;
                    if cost.image_calls > 0 && !explicit_retry {
                        return Err(failure("clip_frame_prior_attempt_requires_explicit_retry"));
                    }
                    let known_previous = if cost.image_calls == 0 {
                        Some(0)
                    } else {
                        cost.image_micro_dollars
                    };
                    cost.image_calls = cost.image_calls.saturating_add(1);
                    store_json(&directory.join("cost.json"), &cost)?;
                    service.hub.ledger().reserve(Unit::ImageCall, 1).commit(1);
                    // The frame provider reports every failure alike, so a failed frame is
                    // recorded as of unknown outcome with its one-call estimate.
                    let billed = service
                        .hub
                        .ledger()
                        .begin_call(super::cost::clip_frame_call(CLIP_FRAME_MODEL, fallen), &[]);
                    let image = provider
                        .frame(&source, &frame_prompt(&fighter, fallen))
                        .await?;
                    billed.settle(super::cost::image_token_usage(image.usage));
                    // The cost file prices the reported tokens at the price file's rates, once.
                    cost.image_micro_dollars =
                        if image.usage == df_providers::ImageUsage::default() {
                            None
                        } else {
                            known_previous
                                .zip(
                                    service.hub.ledger().costs().quote_image_usage(
                                        CLIP_FRAME_MODEL.model_id(),
                                        image.usage,
                                    ),
                                )
                                .map(|(previous, price)| previous.saturating_add(price))
                        };
                    store_json(&directory.join("cost.json"), &cost)?;
                    let alpha = image.bytes;
                    let green = tokio::task::spawn_blocking(move || {
                        let green = green_frame(&alpha)?;
                        Ok::<_, ResourceFailure>((alpha, green))
                    })
                    .await
                    .map_err(|_| failure("clip_frame_worker_failed"))??;
                    std::fs::write(directory.join("alpha.png"), green.0)
                        .map_err(|_| failure("clip_store_failed"))?;
                    std::fs::write(directory.join("frame.png"), green.1)
                        .map_err(|_| failure("clip_store_failed"))?;
                    Ok(Generated {
                        handle: work_key.to_string(),
                        note: "home-angle green frame".to_owned(),
                    })
                }),
            )
            .await?;
        Ok(key)
    }

    async fn generate(
        &self,
        spec: CharacterClipSpec,
        source: Arc<Depiction>,
        explicit_retry: bool,
    ) -> Result<Generated, ResourceFailure> {
        let key = ResourceKey::of(&spec);
        let directory = self.directory.join(key.as_str());
        if let Some(metadata) = read_metadata(&directory).filter(|_| !explicit_retry) {
            return Ok(Generated {
                handle: key.to_string(),
                note: json!(metadata).to_string(),
            });
        }
        std::fs::create_dir_all(&directory).map_err(|_| failure("clip_store_failed"))?;
        store_json(&directory.join("spec.json"), &spec)?;
        // Recover dispatch evidence into this run's hub rather than automatically submitting
        // an incomplete paid attempt again after a restart. `dev clips request` is explicit.
        if read_cost(&directory)?.video_calls > 0 && !explicit_retry {
            return Err(failure("clip_prior_attempt_requires_explicit_retry"));
        }
        let provider = self
            .videos
            .as_ref()
            .ok_or_else(|| failure("clip_video_provider_unavailable"))?;
        let guard = self
            .frame(&spec, Arc::clone(&source), false, explicit_retry)
            .await?;
        let fallen = if spec.kind == ClipKind::LastHit {
            let alpha = std::fs::read(self.directory.join(guard.as_str()).join("alpha.png"))
                .map_err(|_| failure("clip_guard_unavailable"))?;
            Some(
                self.frame(
                    &spec,
                    Arc::new(Depiction {
                        bytes: alpha,
                        format: ImageFormat::Png,
                        revision: guard.to_string(),
                    }),
                    true,
                    explicit_retry,
                )
                .await?,
            )
        } else {
            None
        };
        let start = std::fs::read(self.directory.join(guard.as_str()).join("frame.png"))
            .map_err(|_| failure("clip_guard_unavailable"))?;
        let end_key = fallen.as_ref().unwrap_or(&guard);
        let end = std::fs::read(self.directory.join(end_key.as_str()).join("frame.png"))
            .map_err(|_| failure("clip_end_frame_unavailable"))?;
        std::fs::create_dir_all(&directory).map_err(|_| failure("clip_store_failed"))?;
        let mut cost = read_cost(&directory)?;
        let known_previous = if cost.video_calls == 0 {
            Some(0)
        } else {
            cost.video_micro_dollars
        };
        cost.video_calls = cost.video_calls.saturating_add(1);
        // The clip's price is the price file's rate for its seconds, in whole micro-dollars; a
        // model the file does not price leaves the amount unknown.
        let price = self.hub.ledger().costs().quote(
            super::cost::Provider::Fal,
            df_providers::KLING_O1_IMAGE_TO_VIDEO,
            &[(super::cost::Category::VideoSeconds, u64::from(CLIP_SECONDS))],
        );
        cost.video_micro_dollars = known_previous
            .zip(price)
            .map(|(previous, price)| previous.saturating_add(price));
        cost.video_cost_basis = Some("price_file".to_owned());
        store_json(&directory.join("cost.json"), &cost)?;
        self.hub.ledger().reserve(Unit::VideoClip, 1).commit(1);
        let billed = self.hub.ledger().begin_call(
            super::cost::video_call(
                df_providers::KLING_O1_IMAGE_TO_VIDEO,
                super::cost::Cause::CharacterClip,
                spec.kind.label(),
                u64::from(CLIP_SECONDS),
            ),
            &[],
        );
        let prompt = motion_prompt(&spec);
        // A timeout or a failure is of unknown outcome: the dropped call is booked at the clip's
        // length. A clip that came back was billed.
        let video = tokio::time::timeout(DEADLINE, provider.video(&key, &start, &end, &prompt))
            .await
            .map_err(|_| ResourceFailure::TimedOut)??;
        super::cost::settle_counted(
            billed,
            vec![(super::cost::Category::VideoSeconds, u64::from(CLIP_SECONDS))],
        );
        if video.bytes.is_empty() || video.bytes.len() > MAXIMUM_CLIP_BYTES {
            return Err(failure("clip_size_invalid"));
        }
        let path = directory.join("clip.mp4");
        match std::fs::remove_file(directory.join("metadata.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(failure("clip_store_failed")),
        }
        std::fs::write(&path, &video.bytes).map_err(|_| failure("clip_store_failed"))?;
        let kind = spec.kind;
        let mut metadata = tokio::task::spawn_blocking(move || measure_clip(&path, kind))
            .await
            .map_err(|_| failure("clip_measure_worker_failed"))??;
        metadata.guard_key = guard.to_string();
        metadata.fallen_key = fallen.map(|key| key.to_string());
        metadata.cost = cost;
        store_json(&directory.join("metadata.json"), &metadata)?;
        super::actor::record_audit_line(
            "gameplay.character_clips",
            format!(
                "CHARACTER_CLIP measured key={key} identity={} frames={} fps={} width={} height={} contact_ms={:?} guard_ms={:?} settle_ms={:?}",
                spec.fighter.identity,
                metadata.frame_count,
                metadata.fps,
                metadata.width,
                metadata.height,
                metadata.contact_ms,
                metadata.return_to_guard_ms,
                metadata.settle_ms
            ),
        );
        Ok(Generated {
            handle: key.to_string(),
            note: json!(metadata).to_string(),
        })
    }
}

fn read_metadata(directory: &Path) -> Option<ClipMetadata> {
    let metadata: ClipMetadata =
        serde_json::from_slice(&std::fs::read(directory.join("metadata.json")).ok()?).ok()?;
    let length = std::fs::metadata(directory.join("clip.mp4")).ok()?.len();
    (length > 0
        && length == metadata.bytes
        && metadata.frame_count > 0
        && metadata.fps.is_finite()
        && metadata.fps > 0.0)
        .then_some(metadata)
}

fn stored_frame(directory: &Path) -> bool {
    image::image_dimensions(directory.join("frame.png")).ok() == Some((768, 1152))
        && image::image_dimensions(directory.join("alpha.png")).ok() == Some((1024, 1536))
}

/// Native decoding is delegated to the installed ffmpeg executable; the bounded low-resolution
/// frames are analyzed in Rust. The original MP4's count and clock are independently inspected.
fn measure_clip(path: &Path, kind: ClipKind) -> Result<ClipMetadata, ResourceFailure> {
    let bytes = std::fs::read(path).map_err(|_| failure("clip_read_failed"))?;
    let df_providers::EncodedMediaFacts::Mp4 {
        width,
        height,
        frames,
        timebase,
    } = df_providers::NativeFalMediaProvider::inspect_video(&bytes)
        .map_err(|_| failure("clip_container_invalid"))?
    else {
        return Err(failure("clip_container_invalid"));
    };
    let duration_ms = u64::try_from(
        timebase
            .milliseconds_ceil()
            .map_err(|_| failure("clip_clock_invalid"))?,
    )
    .map_err(|_| failure("clip_clock_invalid"))?;
    if !(4500..=5500).contains(&duration_ms)
        || !(2..=180).contains(&frames)
        || width < 640
        || u64::from(width).abs_diff(u64::from(height) * 2 / 3) > 4
    {
        return Err(failure("clip_dimensions_or_duration_invalid"));
    }
    let mut decoder = std::process::Command::new("ffmpeg");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Clip analysis is background server work, with no interactive console.
        decoder.creation_flags(0x0800_0000);
    }
    let decoded = decoder
        .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
        .arg(path)
        .args([
            "-an",
            "-t",
            "6",
            "-frames:v",
            "180",
            "-vf",
            "scale=64:96",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|_| failure("clip_decoder_unavailable"))?;
    if !decoded.status.success() || decoded.stdout.len() != frames as usize * SAMPLE_PIXELS {
        return Err(failure("clip_decode_failed"));
    }
    let fps = frames as f64 * f64::from(timebase.ticks_per_second) / timebase.ticks as f64;
    let (contact_ms, return_to_guard_ms, settle_ms) = measure_motion(&decoded.stdout, fps, kind)?;
    Ok(ClipMetadata {
        frame_count: frames,
        fps,
        width,
        height,
        bytes: bytes.len() as u64,
        duration_ms,
        contact_ms,
        return_to_guard_ms,
        settle_ms,
        timing_method: "gray64x96-largest-frame-difference-v1".to_owned(),
        guard_key: String::new(),
        fallen_key: None,
        cost: Cost::default(),
    })
}

fn mean_difference(left: &[u8], right: &[u8]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / left.len() as f64
}

fn measure_motion(
    decoded: &[u8],
    fps: f64,
    kind: ClipKind,
) -> Result<(Option<u64>, Option<u64>, Option<u64>), ResourceFailure> {
    if !fps.is_finite()
        || fps <= 0.0
        || decoded.len() < SAMPLE_PIXELS * 2
        || !decoded.len().is_multiple_of(SAMPLE_PIXELS)
    {
        return Err(failure("clip_motion_invalid"));
    }
    let frames = decoded.chunks_exact(SAMPLE_PIXELS).collect::<Vec<_>>();
    let differences = frames
        .windows(2)
        .map(|pair| mean_difference(pair[0], pair[1]))
        .collect::<Vec<_>>();
    let (peak, magnitude) = differences
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .ok_or_else(|| failure("clip_motion_missing"))?;
    let at = |index: usize| (index as f64 / fps * 1000.0).round() as u64;
    // A quarter-second quiet window avoids calling one paused wind-up a settled fall.
    let quiet_frames = (fps * 0.25).ceil() as usize;
    let quiet_limit = (magnitude * 0.08).max(0.35);
    let tail_motion = differences
        .iter()
        .skip(peak + 1)
        .rposition(|value| *value > quiet_limit)
        .map_or(peak + 1, |offset| peak + 2 + offset);
    let settled =
        (differences.len().saturating_sub(tail_motion) >= quiet_frames).then_some(at(tail_motion));
    match kind {
        ClipKind::Idle => Ok((None, None, None)),
        ClipKind::Attack => {
            if *magnitude < 0.5 {
                return Err(failure("clip_attack_motion_missing"));
            }
            let first = frames
                .first()
                .ok_or_else(|| failure("clip_motion_missing"))?;
            let guard = (tail_motion..frames.len())
                .find(|index| {
                    mean_difference(first, frames[*index]) <= 3.0
                        && differences.len().saturating_sub(*index) >= quiet_frames
                })
                .map(at);
            let guard = guard.ok_or_else(|| failure("clip_attack_no_guard_return"))?;
            Ok((Some(at(peak + 1)), Some(guard), None))
        }
        ClipKind::LastHit => {
            if *magnitude < 0.5 {
                return Err(failure("clip_fall_motion_missing"));
            }
            let settled = settled.ok_or_else(|| failure("clip_fall_not_settled"))?;
            let first = frames
                .first()
                .ok_or_else(|| failure("clip_motion_missing"))?;
            let last = frames
                .last()
                .ok_or_else(|| failure("clip_motion_missing"))?;
            if mean_difference(first, last) <= 3.0 {
                return Err(failure("clip_fall_returns_to_guard"));
            }
            Ok((None, None, Some(settled)))
        }
    }
}

#[derive(Clone)]
struct Wanted {
    fighter: Fighter,
    kinds: Vec<ClipKind>,
    force: bool,
}

/// A resolved or missing canonical source, pinned to the complete fighter and revision.
#[derive(Clone)]
struct CachedSource {
    fighter: Fighter,
    revision: String,
    depiction: Option<Arc<Depiction>>,
}

pub(super) struct CharacterClips {
    generation: Generation,
    source: Source,
    wanted: Mutex<BTreeMap<String, Wanted>>,
    fighters: Mutex<BTreeMap<String, Fighter>>,
    source_cache: Mutex<BTreeMap<String, CachedSource>>,
    logged_misses: Mutex<BTreeSet<String>>,
}

impl CharacterClips {
    fn new(
        hub: Hub,
        frames: Option<Arc<dyn FrameProvider>>,
        videos: Option<Arc<dyn VideoProvider>>,
        source: Source,
        directory: PathBuf,
    ) -> Self {
        hub.set_limits(
            ResourceKind::CharacterClip,
            Limits {
                // Two concurrent last-hit jobs can serialize four 300s image calls before
                // their 900s video deadlines; leave room for file writes and measurement.
                timeout: Duration::from_secs(2400),
                attempts: 1,
            },
        );
        let checked = directory.clone();
        hub.set_validator(
            ResourceKind::CharacterClip,
            Arc::new(move |ready| {
                let path = checked.join(&ready.handle);
                read_metadata(&path).is_some() || stored_frame(&path)
            }),
        );
        Self {
            generation: Generation {
                hub,
                frames,
                videos,
                directory,
                frame_gates: Arc::new(Mutex::new(BTreeMap::new())),
            },
            source,
            wanted: Mutex::new(BTreeMap::new()),
            fighters: Mutex::new(BTreeMap::new()),
            source_cache: Mutex::new(BTreeMap::new()),
            logged_misses: Mutex::new(BTreeSet::new()),
        }
    }

    /// Drops cached misses so the next resolver tick retries once. Resolved sources keep
    /// their pinned revision; a changed fighter or explicit force is handled in `request`.
    pub(super) fn note_source_changed(&self) {
        locked(&self.source_cache).retain(|_, cached| cached.depiction.is_some());
    }

    pub(super) fn request(&self, fighter: Fighter, kind: Option<ClipKind>, force: bool) {
        locked(&self.fighters).insert(fighter.identity.clone(), fighter.clone());
        if force {
            locked(&self.source_cache).remove(&fighter.identity);
            locked(&self.logged_misses)
                .retain(|key| !key.starts_with(&format!("{}|", fighter.identity)));
        } else {
            let stale = locked(&self.source_cache)
                .get(&fighter.identity)
                .is_some_and(|cached| cached.fighter != fighter);
            if stale {
                locked(&self.source_cache).remove(&fighter.identity);
            }
        }
        let mut wanted = locked(&self.wanted);
        let kinds = kind.map_or_else(|| ClipKind::ALL.to_vec(), |kind| vec![kind]);
        match wanted.get_mut(&fighter.identity) {
            Some(known) if known.fighter == fighter => {
                for kind in kinds {
                    if !known.kinds.contains(&kind) {
                        known.kinds.push(kind);
                    }
                }
                known.force |= force;
            }
            _ => {
                wanted.insert(
                    fighter.identity.clone(),
                    Wanted {
                        fighter,
                        kinds,
                        force,
                    },
                );
            }
        }
    }

    fn admit(
        self: &Arc<Self>,
        fighter: &Fighter,
        kinds: &[ClipKind],
        force: bool,
        source: Arc<Depiction>,
        revision: &str,
    ) {
        for &kind in kinds {
            let spec = CharacterClipSpec::new(fighter.clone(), revision.to_owned(), kind);
            let key = ResourceKey::of(&spec);
            if force {
                match self.generation.hub.status(&key) {
                    super::resources::ResourceStatus::Ready(_)
                    | super::resources::ResourceStatus::Failed(..) => {
                        self.generation
                            .hub
                            .invalidate(super::resources::Invalidate::Key(&key));
                    }
                    _ => {}
                }
                // A deliberately forced retry can also retry a previously refused frame.
                for fallen in [false, true] {
                    let frame_key = ResourceKey::of(&FrameSpec {
                        clip: spec.clone(),
                        fallen,
                    });
                    if matches!(
                        self.generation.hub.status(&frame_key),
                        super::resources::ResourceStatus::Failed(..)
                    ) {
                        self.generation
                            .hub
                            .invalidate(super::resources::Invalidate::Key(&frame_key));
                    }
                }
            }
            // The registry keeps the maker; a weak owner avoids retaining the hub and
            // provider credentials in a registry -> maker -> hub reference cycle.
            let service = Arc::downgrade(self);
            let work_spec = spec.clone();
            let source = Arc::clone(&source);
            let maker: Maker = Arc::new(move || {
                let service = service.clone();
                let spec = work_spec.clone();
                let source = Arc::clone(&source);
                Box::pin(async move {
                    let service = service.upgrade().ok_or(ResourceFailure::Interrupted)?;
                    service.generation.generate(spec, source, force).await
                })
            });
            self.generation.hub.get_or_request(&spec, maker);
        }
    }

    /// Wait for a canonical source without making a failed clip key for an absent depiction.
    /// Resolved and missing sources are cached per complete fighter; only a changed
    /// fighter, explicit force, or `note_source_changed` re-admits a Source call.
    async fn resolve_sources(self: &Arc<Self>) {
        let pending = locked(&self.wanted).values().cloned().collect::<Vec<_>>();
        for wanted in pending {
            let fighter = wanted.fighter.clone();
            if !wanted.force
                && let Some(cached) = locked(&self.source_cache)
                    .get(&fighter.identity)
                    .filter(|cached| cached.fighter == fighter)
                    .cloned()
            {
                match cached.depiction {
                    Some(source) => {
                        let removed = {
                            let mut current = locked(&self.wanted);
                            if current
                                .get(&fighter.identity)
                                .is_some_and(|known| known.fighter != fighter)
                            {
                                continue;
                            }
                            current.remove(&fighter.identity)
                        };
                        if removed.is_some() {
                            self.admit(&fighter, &wanted.kinds, false, source, &cached.revision);
                        }
                    }
                    None => continue,
                }
                continue;
            }
            let source = (self.source)(&fighter).await;
            let Some(source) = source else {
                locked(&self.source_cache).insert(
                    fighter.identity.clone(),
                    CachedSource {
                        fighter: fighter.clone(),
                        revision: String::new(),
                        depiction: None,
                    },
                );
                let miss_key = format!("{}|{fighter:?}", fighter.identity);
                if locked(&self.logged_misses).insert(miss_key) {
                    super::actor::record_audit_line(
                        "gameplay.character_clips",
                        format!(
                            "CHARACTER_CLIP fallback identity={} reason=canonical_depiction_unavailable",
                            fighter.identity
                        ),
                    );
                }
                continue;
            };
            let revision = format!("{}:{}", source.revision, digest(&source.bytes));
            let source = Arc::new(source);
            locked(&self.source_cache).insert(
                fighter.identity.clone(),
                CachedSource {
                    fighter: fighter.clone(),
                    revision: revision.clone(),
                    depiction: Some(Arc::clone(&source)),
                },
            );
            let removed = {
                let mut current = locked(&self.wanted);
                if current
                    .get(&fighter.identity)
                    .is_some_and(|known| known.fighter != fighter)
                {
                    continue;
                }
                current.remove(&fighter.identity)
            };
            if removed.is_some() {
                self.admit(&fighter, &wanted.kinds, wanted.force, source, &revision);
            }
        }
    }

    pub(super) fn list(&self, fighter: &Fighter) -> Value {
        let subject = format!("character-clips:{}", fighter.identity);
        let resources = self.generation.hub.for_subject(&subject).into_iter().map(|row| {
            let directory = self.generation.directory.join(row.key.as_str());
            let metadata = read_metadata(&directory);
            let cost: Option<Cost> = std::fs::read(directory.join("cost.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let spec: Option<CharacterClipSpec> = std::fs::read(directory.join("spec.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let frame: Option<Value> = std::fs::read(directory.join("frame.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let kind = spec.map(|spec| json!(spec.kind)).or_else(|| frame.and_then(|frame| frame.get("frame").cloned()));
            let state = if matches!(self.generation.hub.status(&row.key), super::resources::ResourceStatus::Fallback(_)) { "fallback" } else { row.state };
            json!({ "key": row.key.as_str(), "state": state, "error": row.last_error,
                "kind": kind, "cost": cost, "timings": metadata,
                "directory": directory, "clip": directory.join("clip.mp4"),
                "guard_frame": directory.join("frame.png"), "alpha_frame": directory.join("alpha.png") })
        }).collect::<Vec<_>>();
        json!({"identity": fighter.identity, "name": fighter.name, "waiting_for_depiction": locked(&self.wanted).contains_key(&fighter.identity), "resources": resources})
    }

    pub(super) fn find(&self, state: &GameState, name: &str) -> Result<Fighter, &'static str> {
        let mut candidates = locked(&self.fighters).clone();
        for fighter in forecast_fighters(state) {
            candidates
                .entry(fighter.identity.clone())
                .or_insert(fighter);
        }
        let matches = candidates
            .into_values()
            .filter(|fighter| fighter.identity == name || fighter.name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [fighter] => Ok(fighter.clone()),
            [] => Err("fighter not found"),
            _ => Err("fighter name is ambiguous; use canonical identity"),
        }
    }
}

static INSTALLED: OnceLock<Mutex<Option<Arc<CharacterClips>>>> = OnceLock::new();

pub(super) fn service() -> Option<Arc<CharacterClips>> {
    locked(INSTALLED.get()?).clone()
}

pub(super) struct Worker {
    task: tokio::task::JoinHandle<()>,
    owner: std::sync::Weak<CharacterClips>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(installed) = INSTALLED.get() {
            let mut installed = locked(installed);
            if installed.as_ref().is_some_and(|service| {
                std::sync::Weak::ptr_eq(&Arc::downgrade(service), &self.owner)
            }) {
                installed.take();
            }
        }
    }
}

/// The source callback must return only the selected canonical full-body depiction.
pub(super) fn start(
    hub: Hub,
    source: Source,
    directory: PathBuf,
    live: bool,
    mut source_wake: Option<tokio::sync::watch::Receiver<u64>>,
) -> Result<Worker, std::io::Error> {
    let (frames, videos) = native_providers(live)?;
    let service = Arc::new(CharacterClips::new(hub, frames, videos, source, directory));
    *locked(INSTALLED.get_or_init(Default::default)) = Some(Arc::clone(&service));
    let owner = Arc::downgrade(&service);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                changed = async {
                    match source_wake.as_mut() {
                        Some(wake) => wake.changed().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if changed.is_ok() {
                        service.note_source_changed();
                    } else {
                        source_wake = None;
                        continue;
                    }
                }
            }
            service.resolve_sources().await;
        }
    });
    Ok(Worker { task, owner })
}

pub(super) fn request_fighters(fighters: impl IntoIterator<Item = Fighter>) {
    let Some(service) = service() else {
        return;
    };
    for fighter in fighters {
        service.request(fighter, None, false);
    }
}

/// A pure roster projection shared by the staging hook, forecast hook and development tools.
pub(super) fn fighter_for(
    state: &GameState,
    entity: EntityId,
    subject: String,
    weapon: df_rules::tactical::Weapon,
) -> Option<Fighter> {
    let world = state.entities.iter().find(|world| world.id == entity)?;
    let reviewed_figure = super::beat_script_runtime::scene(state)
        .ok()
        .and_then(|scene| {
            scene
                .cast
                .iter()
                .find(|member| member.id == subject)
                .and_then(|member| reviewed_figure(&member.sprite))
        });
    Some(Fighter {
        identity: super::journey::canonical_actor_ref(state, entity).ok()?,
        identity_revision: world.identity_revision.as_str().to_owned(),
        depiction_subject: subject,
        reviewed_figure,
        name: super::journey::name(state, entity).ok()?,
        weapon: weapon.id().to_owned(),
        attack_style: AttackStyle::weapon(weapon),
    })
}

pub(super) fn hero_subject(entity: EntityId) -> String {
    format!("hero:{}", digest(entity.as_bytes()))
}

/// Foresight requests party and fightable current cast. The roster's stat blocks and carried
/// weapons choose mechanics; this module does not classify player language.
pub(super) fn forecast_fighters(state: &GameState) -> Vec<Fighter> {
    let mut fighters = Vec::new();
    for character in &state.characters {
        // Match the accepted canonical depiction's gear contract, rather than the ordering
        // of backpack items (which can put a spare dagger before the depicted weapon).
        let archer = character.choices.iter().any(|choice| {
            choice.offer.as_str() == "build-package" && choice.selected.as_str() == "archer"
        });
        let weapon_id = if archer {
            "longbow".to_owned()
        } else {
            super::pack::hero_weapon_item()
        };
        let weapon = weapon_by_id(&weapon_id);
        if let Some(weapon) = weapon
            && let Some(fighter) = fighter_for(
                state,
                character.entity,
                hero_subject(character.entity),
                weapon,
            )
        {
            fighters.push(fighter);
        }
    }
    if let Ok(scene) = super::beat_script_runtime::scene(state) {
        for member in scene.cast {
            let Some(creature) =
                super::journey::scene_fight::creature_for(member.stat_block.as_deref())
            else {
                continue;
            };
            let Ok(entity) = super::journey::scene_fight::person_entity(&member.id) else {
                continue;
            };
            let Ok(spec) = df_rules::tactical::CombatantSpec::creature(
                1,
                creature.id,
                df_rules::tactical::Cell(0, 0),
                0,
            ) else {
                continue;
            };
            let Some(weapon) = spec.weapons.first().copied() else {
                continue;
            };
            // Before a fight the scene person need not have a world entity yet.
            let fighter =
                fighter_for(state, entity, member.id.clone(), weapon).unwrap_or_else(|| Fighter {
                    identity: format!("entity:{}", super::journey::hex(entity.as_bytes())),
                    identity_revision: format!("scene-person-{}-1", member.id),
                    depiction_subject: member.id.clone(),
                    reviewed_figure: reviewed_figure(&member.sprite),
                    name: member.name,
                    weapon: weapon.id().to_owned(),
                    attack_style: AttackStyle::weapon(weapon),
                });
            fighters.push(fighter);
        }
    }
    fighters
}

fn reviewed_figure(sprite: &str) -> Option<String> {
    let photo = df_render::art_contract::identity_photo(sprite)
        .or_else(|| df_render::art_contract::reviewed_photo(sprite))?;
    (photo.starts_with("sprite-") || photo.starts_with("figure-")).then(|| photo.to_owned())
}

fn weapon_by_id(id: &str) -> Option<df_rules::tactical::Weapon> {
    use df_rules::tactical::Weapon;
    [
        Weapon::Greatsword,
        Weapon::Flail,
        Weapon::Javelin,
        Weapon::Spear,
        Weapon::Longbow,
        Weapon::Shortbow,
        Weapon::Scimitar,
        Weapon::Shortsword,
        Weapon::LightCrossbow,
        Weapon::Dagger,
        Weapon::Sickle,
        Weapon::Club,
        Weapon::Bite,
        Weapon::Slam,
        Weapon::UnarmedStrike,
    ]
    .into_iter()
    .find(|weapon| weapon.id() == id)
}

/// The model the clip frames are edited with; the cost engine prices the frames by it.
const CLIP_FRAME_MODEL: df_providers::ImageEditModel = df_providers::ImageEditModel::Gpt25Sunburst;

impl FrameProvider for df_providers::NativeImageEditProvider {
    fn frame<'a>(
        &'a self,
        source: &'a Depiction,
        prompt: &'a str,
    ) -> ProviderFuture<'a, GeneratedImage> {
        Box::pin(async move {
            let references = [df_providers::ReferenceImage {
                format: source.format,
                bytes: &source.bytes,
            }];
            let edit = df_providers::ImageEdit {
                model: CLIP_FRAME_MODEL,
                prompt,
                references: &references,
                width: 1024,
                height: 1536,
                // Dev play-tests draw clip frames at medium; production keeps high.
                quality: match super::startup_configuration::ImageryProfile::from_environment() {
                    Ok(super::startup_configuration::ImageryProfile::Dev) => {
                        df_providers::ImageQuality::Medium
                    }
                    _ => df_providers::ImageQuality::High,
                },
                background: df_providers::ImageBackground::Transparent,
                output: ImageFormat::Png,
                moderation: df_providers::ImageModeration::Low,
            };
            self.edit(&edit, IMAGE_EDIT_TIMEOUT).await.map_err(|error| {
                super::actor::record_audit_line(
                    "gameplay.character_clips",
                    format!("CHARACTER_CLIP image_failed reason={error:?} fallback=cutout"),
                );
                failure("clip_image_failed")
            })
        })
    }
}

struct NativeVideo {
    provider: df_providers::NativeFalMediaProvider,
}

fn video_binding(
    key: &ResourceKey,
    start: &[u8],
    end: &[u8],
    prompt: &str,
) -> Result<df_provider_api::RequestBinding<df_providers::FalVideoBasis>, ResourceFailure> {
    use df_model::checkpoint::{
        AssetKind, AssetReference, Basis, ContentDigest, ExecutionMode, JobId,
    };
    use df_provider_api::{
        RequestBasis, RequestBinding, RequestIdentity, RequestModality, VideoSemantics,
    };
    use df_types::{OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision};
    let id_digest = Sha256::digest(key.as_str().as_bytes());
    let ids = id_digest
        .get(..16)
        .ok_or_else(|| failure("clip_request_identity"))?;
    let revision =
        |text: &str| RevisionLabel::new(Some(text)).map_err(|_| failure("clip_request_revision"));
    let frame = |bytes: &[u8], name: &str| -> Result<_, ResourceFailure> {
        Ok(df_providers::FalReferenceFrame {
            asset: AssetReference {
                key: revision(name)?,
                digest: ContentDigest(Sha256::digest(bytes).into()),
                byte_length: bytes.len() as u64,
                kind: AssetKind::Image,
            },
            locator: format!(
                "data:image/png;base64,{}",
                super::home_video::base64_encode(bytes)
            ),
        })
    };
    Ok(RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(ids)
                    .map_err(|_| failure("clip_request_identity"))?,
                run: RunId::from_bytes(ids).map_err(|_| failure("clip_request_identity"))?,
                revision: SessionRevision::new(
                    RecoveryEpoch::new(1).map_err(|_| failure("clip_request_identity"))?,
                    1,
                ),
            },
            job: JobId::from_bytes(ids).map_err(|_| failure("clip_request_identity"))?,
            operation: OperationId::from_bytes(ids)
                .map_err(|_| failure("clip_request_identity"))?,
            generation: 1,
        },
        semantic_basis: RequestBasis {
            modality: RequestModality::Video,
            source: ContentDigest(Sha256::digest(start).into()),
            rights_revision: revision("canonical-character-clips-v1")?,
            output_format_revision: revision("character-clip-v1")?,
            semantic: VideoSemantics {
                locale: None,
                prompt: prompt.to_owned(),
                reference_frames: vec![frame(start, "guard-frame")?, frame(end, "end-frame")?],
                output_profile: df_providers::FalVideoProfile {
                    model: df_providers::KLING_O1_IMAGE_TO_VIDEO.to_owned(),
                    duration_seconds: CLIP_SECONDS,
                    resolution: "1080p".to_owned(),
                    width: 1080,
                    height: 1620,
                    accepted_duration_min: Duration::from_millis(4500),
                    accepted_duration_max: Duration::from_millis(5500),
                    seed: 0,
                    safety: true,
                    prompt_expansion: false,
                },
            },
        },
        mode: ExecutionMode::Live,
        deadline: DEADLINE,
    })
}

impl VideoProvider for NativeVideo {
    fn video<'a>(
        &'a self,
        key: &'a ResourceKey,
        start: &'a [u8],
        end: &'a [u8],
        prompt: &'a str,
    ) -> ProviderFuture<'a, Video> {
        Box::pin(async move {
            use df_provider_api::{RequestLimits, RequestOwnerState, RequestUsage, VideoEventKind};
            use df_types::{Usage, UsageUnit};
            let started = Instant::now();
            let binding = video_binding(key, start, end, prompt)?;
            let owner = || RequestOwnerState {
                current: Some(&binding),
                elapsed: Duration::ZERO,
                cancelled: false,
            };
            let request = df_providers::FalVideoRequest::new(
                video_binding(key, start, end, prompt)?,
                b"",
                RequestUsage::new(
                    0,
                    Duration::from_secs(5),
                    Usage::new(5000, UsageUnit::VideoMillisecond),
                ),
                RequestLimits::new(
                    8 * 1024 * 1024,
                    0,
                    Duration::from_secs(10),
                    Usage::new(10_000, UsageUnit::VideoMillisecond),
                )
                .map_err(|_| failure("clip_request_limits"))?,
                owner(),
            )
            .map_err(|_| failure("clip_request_invalid"))?;
            let provider_error = |error: df_providers::FalMediaError| {
                super::actor::record_audit_line(
                    "gameplay.character_clips",
                    format!(
                        "CHARACTER_CLIP video_failed key={key} reason={:?} request={:?} http={:?} fallback=cutout",
                        error.reason, error.request_id, error.http
                    ),
                );
                failure("clip_video_failed")
            };
            let mut job = self
                .provider
                .submit_video(&request, owner(), std::future::pending())
                .await
                .map_err(provider_error)?;
            let request_id = job.request_id().as_str().to_owned();
            super::actor::record_audit_line(
                "gameplay.character_clips",
                format!(
                    "CHARACTER_CLIP accepted key={key} request={request_id} ms={} model={}",
                    started.elapsed().as_millis(),
                    df_providers::KLING_O1_IMAGE_TO_VIDEO,
                ),
            );
            loop {
                match job
                    .status(owner(), std::future::pending())
                    .await
                    .map_err(provider_error)?
                {
                    df_providers::FalQueueObservation::Completed { .. } => break,
                    df_providers::FalQueueObservation::Pending(_) => {
                        tokio::time::sleep(Duration::from_secs(4)).await
                    }
                    _ => return Err(failure("clip_video_not_completed")),
                }
            }
            let event = job
                .result(owner(), std::future::pending())
                .await
                .map_err(provider_error)?
                .complete(owner())
                .map_err(provider_error)?
                .into_video_event()
                .map_err(provider_error)?;
            let VideoEventKind::Completed { output, .. } = event.kind else {
                return Err(failure("clip_video_no_output"));
            };
            let [clip] = output.as_slice() else {
                return Err(failure("clip_video_output_count"));
            };
            super::actor::record_audit_line(
                "gameplay.character_clips",
                format!(
                    "CHARACTER_CLIP provider_completed key={key} request={request_id} ms={} bytes={}",
                    started.elapsed().as_millis(),
                    clip.bytes().len(),
                ),
            );
            Ok(Video {
                bytes: clip.bytes().to_vec(),
            })
        })
    }
}

type NativeProviders = (
    Option<Arc<dyn FrameProvider>>,
    Option<Arc<dyn VideoProvider>>,
);

fn native_providers(live: bool) -> Result<NativeProviders, std::io::Error> {
    if !live
        || !super::media_policy::gates().allows(super::media_policy::Capability::CharacterVideo)
    {
        // Cached clips remain readable, but no clip-only image frames or video are generated.
        return Ok((None, None));
    }
    let image_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    let fal_key = std::env::var("FAL_KEY")
        .or_else(|_| std::env::var("DF_FAL_KEY"))
        .ok()
        .filter(|key| !key.is_empty());
    if fal_key.is_none() {
        return Ok((None, None));
    }
    let frames = image_key
        .map(|key| {
            df_providers::NativeImageEditProvider::new(
                df_providers::ImageEditConfig::live(),
                &key,
                IMAGE_EDIT_TIMEOUT,
            )
            .map(|provider| Arc::new(provider) as Arc<dyn FrameProvider>)
            .map_err(|_| std::io::Error::other("character clip image configuration invalid"))
        })
        .transpose()?;
    let videos = if let Some(key) = fal_key {
        // One pin per fal media host with all its addresses, as the home video does; a pin per
        // address exceeds the media fetcher's limit of 16 and refuses the provider.
        let pins = super::home_video::resolve_pins();
        if pins.is_empty() {
            None
        } else {
            let config =
                df_providers::FalMediaConfig::live(df_providers::FalMediaFamily::Video, pins)
                    .with_video_model(df_providers::KLING_O1_IMAGE_TO_VIDEO)
                    .map_err(|_| std::io::Error::other("character clip video model invalid"))?;
            let limits = df_providers::ProviderLimits::new(
                Duration::from_secs(170),
                8 * 1024 * 1024,
                MAXIMUM_CLIP_BYTES,
            )
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .chunks(65_536)
            .build();
            let provider = df_providers::registered_fal_media_provider(config, &key, limits)
                .map_err(|_| std::io::Error::other("character clip video configuration invalid"))?;
            Some(Arc::new(NativeVideo { provider }) as Arc<dyn VideoProvider>)
        }
    } else {
        None
    };
    Ok((frames, videos))
}

#[cfg(test)]
#[path = "character_clips_tests.rs"]
mod tests;
