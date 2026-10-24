//! Plate-owned SAM 3.1 masks. Identity comes from measured heads or explicit staging order,
//! never from SAM's score order. A durable sent marker prevents a cancelled call being paid again.

use std::{path::PathBuf, sync::OnceLock, time::Duration};

use df_providers::{
    FalSegmentConfig, HeadBox, LocatedHead, MaskRows, NativeFalSegmenter, SegmentInstance,
    SegmentModel, SegmentRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ImageFailure, ImageKey, Imagery, Mode, locked, read_cache_file, vision_picture};
use crate::gameplay::resources::{Generated, ResourceFailure, ResourceKind, ResourceSpec, Unit};

const DEADLINE: Duration = Duration::from_secs(20);
const REVISION: u32 = 1;

#[derive(Default)]
pub(super) struct FigureMaskCache {
    pub(super) provider: OnceLock<NativeFalSegmenter>,
    // Coalesce one plate without making unrelated cached plates wait for its provider call.
    gates: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
    >,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredMask {
    index: u32,
    score: f32,
    runs: Vec<[u32; 2]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredScan {
    revision: u32,
    model: String,
    image_sha256: String,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    /// `sent` includes interrupted/unknown outcomes; none of these states permits resubmission.
    outcome: String,
    masks: Vec<StoredMask>,
}

impl StoredScan {
    fn instances(&self) -> Result<Vec<SegmentInstance>, ImageFailure> {
        if self.revision != REVISION
            || self.model != SegmentModel::Sam31.label()
            || self.outcome != "complete"
            || self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || self.source_width == 0
            || self.source_height == 0
            || self.masks.len() > usize::from(df_providers::MAXIMUM_MASKS)
        {
            return Err(ImageFailure::BadOutput);
        }
        self.masks
            .iter()
            .map(|mask| {
                if !mask.score.is_finite() || !(0.0..=1.0).contains(&mask.score) {
                    return Err(ImageFailure::BadOutput);
                }
                Ok(SegmentInstance {
                    index: mask.index,
                    score: mask.score,
                    mask: MaskRows::from_runs(
                        self.width,
                        self.height,
                        mask.runs
                            .iter()
                            .map(|[start, length]| df_providers::MaskRun {
                                start: *start,
                                length: *length,
                            })
                            .collect(),
                    )
                    .map_err(|_| ImageFailure::BadOutput)?,
                })
            })
            .collect()
    }
}

/// An explicit left-to-right slot from the plate's authored staging facts. The caller must pass
/// the whole named cast, not one locator batch. Extras are never named by their mask alone.
pub(in crate::gameplay) struct StagedFigure<'a> {
    pub(in crate::gameplay) id: &'a str,
    pub(in crate::gameplay) hero: bool,
}

/// Audience-safe figure geometry. Boxes and crown points use intrinsic source pixels; the
/// permille crown is provided for the existing plate projection and camera transform.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(in crate::gameplay) struct FigureGeometry {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) mask_index: u32,
    #[serde(default)]
    pub(in crate::gameplay) mask_url: String,
    pub(in crate::gameplay) source_width: u32,
    pub(in crate::gameplay) source_height: u32,
    pub(in crate::gameplay) bounds: [u32; 4],
    pub(in crate::gameplay) head_top: [u32; 2],
    pub(in crate::gameplay) head_top_permille: [u32; 2],
    pub(in crate::gameplay) free_left: bool,
    pub(in crate::gameplay) inferred: bool,
}

struct SegmentsSpec<'a>(&'a str);

impl ResourceSpec for SegmentsSpec<'_> {
    const KIND: ResourceKind = ResourceKind::PlateSegments;
    const CONTRACT_VERSION: u32 = REVISION;

    fn canonical(&self) -> String {
        serde_json::json!({
            "plate": self.0, "model": "sam-3.1", "concept": "person",
            "endpoint": SegmentModel::Sam31.endpoint(), "max_masks": df_providers::MAXIMUM_MASKS,
            "encoding": "rle", "score_floor": 0,
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("plate:{}", self.0)
    }

    fn chrome(&self) -> df_render::art_contract::Neutral {
        df_render::art_contract::Neutral::Nothing
    }
}

impl FigureMaskCache {
    pub(super) fn configure(&self) {
        // The launcher hands fal's credential over as FAL_KEY; DF_FAL_KEY serves direct runs.
        let Ok(key) = std::env::var("FAL_KEY").or_else(|_| std::env::var("DF_FAL_KEY")) else {
            return;
        };
        match NativeFalSegmenter::new(FalSegmentConfig::live(SegmentModel::Sam31), &key, DEADLINE) {
            Ok(provider) => {
                let _first = self.provider.set(provider);
            }
            Err(error) => diagnostic(
                "configuration",
                "",
                serde_json::json!({"error": error.label()}),
            ),
        }
    }

    fn gate_for(&self, key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut gates = locked(&self.gates);
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key).and_then(std::sync::Weak::upgrade) {
            return gate;
        }
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key.to_owned(), std::sync::Arc::downgrade(&gate));
        gate
    }

    async fn read_note(path: PathBuf) -> Result<Option<StoredScan>, ImageFailure> {
        tokio::task::spawn_blocking(move || {
            // An existing but corrupt note is a refusal, not permission to spend again.
            if !path.try_exists()? {
                return Ok(None);
            }
            let bytes = read_cache_file(&path)
                .ok_or_else(|| std::io::Error::other("unreadable segmentation note"))?;
            let scan =
                serde_json::from_slice::<StoredScan>(&bytes).map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(Some(scan))
        })
        .await
        .map_err(|_| ImageFailure::Interrupted)?
        .map_err(|_| ImageFailure::BadOutput)
    }

    async fn read_or_scan(
        &self,
        path: PathBuf,
        key: &str,
        bytes: &[u8],
        format: df_providers::ImageFormat,
        engine: &super::Engine,
    ) -> Result<StoredScan, ImageFailure> {
        let gate = self.gate_for(key);
        let (cached, _owner) = match Self::read_note(path.clone()).await? {
            Some(scan) => (Some(scan), None),
            None => {
                let owner = gate.lock().await;
                (Self::read_note(path.clone()).await?, Some(owner))
            }
        };
        if let Some(scan) = cached {
            diagnostic(
                "cache_hit",
                key,
                serde_json::json!({"cache_outcome": scan.outcome}),
            );
            let image = bytes.to_vec();
            return tokio::task::spawn_blocking(move || {
                if scan.image_sha256 != format!("{:x}", Sha256::digest(&image)) {
                    return Err(ImageFailure::BadOutput);
                }
                scan.instances()?;
                Ok(scan)
            })
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        }
        let provider = self.provider.get().ok_or(ImageFailure::Off)?;
        let image_sha256 = format!("{:x}", Sha256::digest(bytes));
        let bytes = bytes.to_vec();
        let (jpeg, width, height, source_width, source_height) =
            tokio::task::spawn_blocking(move || {
                let jpeg =
                    vision_picture(&bytes, format, 1536).map_err(|_| ImageFailure::BadOutput)?;
                let source = image::ImageReader::new(std::io::Cursor::new(&bytes))
                    .with_guessed_format()
                    .map_err(|_| ImageFailure::BadOutput)?
                    .into_dimensions()
                    .map_err(|_| ImageFailure::BadOutput)?;
                let picture = image::ImageReader::new(std::io::Cursor::new(&jpeg))
                    .with_guessed_format()
                    .map_err(|_| ImageFailure::BadOutput)?
                    .into_dimensions()
                    .map_err(|_| ImageFailure::BadOutput)?;
                Ok::<_, ImageFailure>((jpeg, picture.0, picture.1, source.0, source.1))
            })
            .await
            .map_err(|_| ImageFailure::Interrupted)??;
        if jpeg.len() > 3 * 1024 * 1024 {
            return Err(ImageFailure::InvalidRequest);
        }
        let _permit = engine
            .permits
            .acquire()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        let mut scan = StoredScan {
            revision: REVISION,
            model: provider.model().label().to_owned(),
            image_sha256,
            width,
            height,
            source_width,
            source_height,
            outcome: "sent".to_owned(),
            masks: Vec::new(),
        };
        // create_new is also a cross-process guard: a second server cannot submit the same
        // plate while this process has a call in flight. A crash preserves the unknown charge.
        let marker = serde_json::to_vec(&scan).map_err(|_| ImageFailure::BadOutput)?;
        let marker_path = path.clone();
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(marker_path)?;
            file.write_all(&marker)?;
            file.sync_all()
        })
        .await
        .map_err(|_| ImageFailure::Interrupted)?
        .map_err(|_| ImageFailure::Provider)?;
        let price = provider.model().price_micro_dollars();
        engine.ledger.reserve(Unit::SegmentCall, 1).commit(1);
        // The request below is locally validated here. Charge at dispatch so cancellation does
        // not refund an unknown provider outcome. There is no cost cap.
        engine
            .micro_dollars
            .fetch_add(price, std::sync::atomic::Ordering::SeqCst);
        diagnostic(
            "sent",
            key,
            serde_json::json!({"model": scan.model, "micro_dollars": price}),
        );
        let result = provider
            .segment(
                &SegmentRequest {
                    jpeg: &jpeg,
                    width,
                    height,
                    prompt: "person",
                    max_masks: df_providers::MAXIMUM_MASKS,
                },
                DEADLINE,
            )
            .await;
        match result {
            Ok(answer) => {
                scan.outcome = "complete".to_owned();
                scan.masks = answer
                    .instances
                    .iter()
                    .map(|instance| StoredMask {
                        index: instance.index,
                        score: instance.score,
                        runs: instance
                            .mask
                            .runs()
                            .iter()
                            .map(|run| [run.start, run.length])
                            .collect(),
                    })
                    .collect();
                diagnostic(
                    "complete",
                    key,
                    serde_json::json!({
                        "count": scan.masks.len(), "elapsed_ms": answer.elapsed.as_millis(),
                        "request_id": answer.request_id, "micro_dollars": price,
                    }),
                );
            }
            Err(error) => {
                scan.outcome = error.label().to_owned();
                diagnostic(
                    "failed",
                    key,
                    serde_json::json!({"error": error.label(), "micro_dollars": price}),
                );
            }
        }
        let note = serde_json::to_vec(&scan).map_err(|_| ImageFailure::BadOutput)?;
        let staging = path.with_extension(format!("{}.tmp", std::process::id()));
        tokio::task::spawn_blocking(move || super::write_cache_file(&staging, &path, &note))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::Provider)?;
        scan.instances()?;
        Ok(scan)
    }
}

impl Imagery {
    async fn plate_masks(&self, key: &ImageKey) -> Result<StoredScan, ImageFailure> {
        let imagery = self.clone();
        let image_key = key.clone();
        self.hub()
            .render(
                &SegmentsSpec(key.as_str()),
                Box::pin(async move {
                    imagery
                        .scan_plate_masks(&image_key)
                        .await
                        .map(|_| Generated::new(image_key.as_str()))
                        .map_err(|failure| ResourceFailure::Permanent(failure.label()))
                }),
            )
            .await
            .map_err(|failure| match failure {
                ResourceFailure::TimedOut => ImageFailure::DeadlineExceeded,
                _ => ImageFailure::from_label(failure.label()).unwrap_or(ImageFailure::Interrupted),
            })?;
        self.scan_plate_masks(key).await
    }

    async fn scan_plate_masks(&self, key: &ImageKey) -> Result<StoredScan, ImageFailure> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Err(ImageFailure::Off);
        };
        let format = locked(&self.inner.registry)
            .by_key
            .get(key.as_str())
            .copied()
            .ok_or(ImageFailure::InvalidRequest)?
            .format;
        let (bytes, stored) = engine
            .read_cached(key.as_str(), format)
            .await
            .ok_or(ImageFailure::InvalidRequest)?;
        let path = engine
            .cache_dir
            .join(format!("{}.sam31-person-v1.json", key.as_str()));
        let scan = engine
            .figure_masks
            .read_or_scan(path, key.as_str(), &bytes, stored, engine)
            .await;
        if let Err(failure) = scan {
            diagnostic(
                "unavailable",
                key.as_str(),
                serde_json::json!({"failure": failure.label()}),
            );
        }
        scan
    }

    /// Run once after all head batches with the photo check's hero count. Only set
    /// staging_order_known for an explicitly authored left-to-right picture order. Otherwise
    /// inference requires one unique remaining identity. The sidecar exposes assigned boxes.
    pub(in crate::gameplay) async fn figure_geometry(
        &self,
        key: &ImageKey,
        heads: &[LocatedHead],
        staged: &[StagedFigure<'_>],
        hero_count: usize,
        staging_order_known: bool,
    ) -> Result<Vec<FigureGeometry>, ImageFailure> {
        let scan = self.plate_masks(key).await?;
        let mut figures = match_figures(&scan, heads, staged, hero_count, staging_order_known)?;
        diagnostic(
            "assignments",
            key.as_str(),
            serde_json::json!({"figures": figures, "staging_order_known": staging_order_known,
                "hero_count": hero_count}),
        );
        for person in staged {
            if !figures.iter().any(|figure| figure.id == person.id) {
                focus_diagnostic("missing", key.as_str(), person.id, 0, Some("unassigned"));
                diagnostic(
                    "miss",
                    key.as_str(),
                    serde_json::json!({"person": person.id}),
                );
            }
        }
        if let Mode::On { engine, .. } = &self.inner.mode {
            let gate = engine.figure_masks.gate_for(key.as_str());
            let _owner = gate.lock().await;
            let path = engine
                .cache_dir
                .join(format!("{}.figures-v1.json", key.as_str()));
            let staging = path.with_extension(format!("{}.tmp", std::process::id()));
            let stored = figures.clone();
            let cache_dir = engine.cache_dir.clone();
            let plate_key = key.as_str().to_owned();
            figures = tokio::task::spawn_blocking(move || {
                let mut stored = stored;
                let instances = scan.instances()?;
                for figure in &mut stored {
                    let result =
                        cache_focus_mask(&cache_dir, &plate_key, &scan, &instances, figure);
                    match result {
                        Ok(()) => {
                            figure.mask_url = format!(
                                "/gameplay/imagery/{plate_key}/figure/{}",
                                figure.mask_index
                            );
                        }
                        Err(failure) => focus_diagnostic(
                            "missing",
                            &plate_key,
                            &figure.id,
                            figure.mask_index,
                            Some(failure.label()),
                        ),
                    }
                }
                let note = serde_json::to_vec(&stored).map_err(|_| ImageFailure::BadOutput)?;
                super::write_cache_file(&staging, &path, &note)
                    .map_err(|_| ImageFailure::Provider)?;
                Ok::<_, ImageFailure>(stored)
            })
            .await
            .map_err(|_| ImageFailure::Interrupted)??;
        }
        Ok(figures)
    }

    /// Read only derived, assigned masks; an HTTP request never starts a SAM/provider job.
    pub(super) async fn focus_mask(&self, key: &str, index: u32) -> Option<bytes::Bytes> {
        let missing = || {
            focus_diagnostic("missing", key, "unassigned", index, None);
            None
        };
        let Some(key) = ImageKey::parse(key) else {
            return missing();
        };
        let Mode::On { engine, .. } = &self.inner.mode else {
            return missing();
        };
        if !locked(&self.inner.registry)
            .by_key
            .contains_key(key.as_str())
        {
            return missing();
        }
        let directory = engine.cache_dir.clone();
        let audit_key = key.clone();
        match tokio::task::spawn_blocking(move || {
            let figures =
                read_cache_file(&directory.join(format!("{}.figures-v1.json", key.as_str())))
                    .and_then(|bytes| serde_json::from_slice::<Vec<FigureGeometry>>(&bytes).ok());
            let person = figures.as_ref().and_then(|figures| {
                figures
                    .iter()
                    .find(|figure| figure.mask_index == index && !figure.mask_url.is_empty())
            });
            let bytes = person
                .and_then(|_| read_cache_file(&focus_mask_path(&directory, key.as_str(), index)));
            focus_diagnostic(
                if bytes.is_some() { "served" } else { "missing" },
                key.as_str(),
                person.map_or("unassigned", |figure| figure.id.as_str()),
                index,
                None,
            );
            bytes.map(bytes::Bytes::from)
        })
        .await
        {
            Ok(bytes) => bytes,
            Err(_) => {
                focus_diagnostic(
                    "missing",
                    audit_key.as_str(),
                    "unavailable",
                    index,
                    Some("interrupted"),
                );
                None
            }
        }
    }
}

fn focus_mask_path(directory: &std::path::Path, key: &str, index: u32) -> PathBuf {
    directory.join(format!("{key}.figure-{index}-v1.png"))
}

fn cache_focus_mask(
    directory: &std::path::Path,
    key: &str,
    scan: &StoredScan,
    instances: &[SegmentInstance],
    figure: &FigureGeometry,
) -> Result<(), ImageFailure> {
    let path = focus_mask_path(directory, key, figure.mask_index);
    if path.try_exists().map_err(|_| ImageFailure::Provider)? {
        return Ok(());
    }
    let instance = instances
        .iter()
        .find(|instance| instance.index == figure.mask_index)
        .ok_or(ImageFailure::BadOutput)?;
    let bytes = raster_focus_mask(scan, &instance.mask)?;
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    super::write_cache_file(&staging, &path, &bytes).map_err(|_| ImageFailure::Provider)?;
    focus_diagnostic("made", key, &figure.id, figure.mask_index, None);
    Ok(())
}

fn raster_focus_mask(scan: &StoredScan, mask: &MaskRows) -> Result<Vec<u8>, ImageFailure> {
    let longest = scan.source_width.max(scan.source_height);
    if longest == 0 || mask.width() == 0 || mask.height() == 0 {
        return Err(ImageFailure::BadOutput);
    }
    let scale = 768.0 / f64::from(longest.max(768));
    let width = (f64::from(scan.source_width) * scale).round().max(1.0) as u32;
    let height = (f64::from(scan.source_height) * scale).round().max(1.0) as u32;
    let mut alpha = image::GrayImage::new(width, height);
    let project = |pixel: u32, output: u32, input: u32| {
        ((u64::from(pixel) * u64::from(output) + u64::from(input / 2)) / u64::from(input)) as u32
    };
    for (row, from, to) in mask.row_segments() {
        for y in project(row, height, mask.height())..project(row + 1, height, mask.height()) {
            for x in project(from, width, mask.width())..project(to, width, mask.width()) {
                alpha.put_pixel(x, y, image::Luma([255]));
            }
        }
    }
    // Three Gaussian sigmas cover about 1.5% of the plate height on either side of the edge.
    let alpha = image::imageops::blur(&alpha, (height as f32 * 0.005).max(0.5));
    let rgba = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([255, 255, 255, alpha.get_pixel(x, y).0[0]])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| ImageFailure::BadOutput)?;
    Ok(png.into_inner())
}

fn focus_diagnostic(stage: &str, key: &str, person: &str, index: u32, failure: Option<&str>) {
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.imagery.focus_mask",
        format!("PLATE focus_mask stage={stage} key={key} person={person}"),
        serde_json::json!({"stage": stage, "key": key, "person": person, "mask_index": index,
            "failure": failure})
        .as_object()
        .cloned()
        .unwrap_or_default(),
        &[],
    );
}

fn diagnostic(outcome: &str, key: &str, value: serde_json::Value) {
    let mut fields = value.as_object().cloned().unwrap_or_default();
    fields.insert("image_key".into(), serde_json::json!(key));
    fields.insert("outcome".into(), serde_json::json!(outcome));
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.imagery.figure_masks",
        format!("FIGURE_MASKS image_key={key} outcome={outcome} {value}"),
        fields,
        &[],
    );
}

fn contains_head(mask: &MaskRows, head: HeadBox) -> u64 {
    let width = mask.width() as f32;
    let height = mask.height() as f32;
    let (left, right) = (
        (head.left * width).floor() as u32,
        (head.right * width).ceil() as u32,
    );
    let (top, bottom) = (
        (head.top * height).floor() as u32,
        (head.bottom * height).ceil() as u32,
    );
    let centre_x = ((head.left + head.right) * width / 2.0).floor() as u32;
    let centre_y = ((head.top + head.bottom) * height / 2.0).floor() as u32;
    if !mask
        .row_segments()
        .any(|(y, from, to)| y == centre_y && from <= centre_x && centre_x < to)
    {
        return 0;
    }
    mask.row_segments()
        .filter(|(y, _, _)| *y >= top && *y < bottom)
        .map(|(_, from, to)| u64::from(to.min(right).saturating_sub(from.max(left))))
        .sum()
}

fn crown(mask: &MaskRows, head: Option<HeadBox>) -> Option<(f32, u32)> {
    let Some(head) = head else {
        return mask.top_centre(3);
    };
    let left = (head.left * mask.width() as f32).floor() as u32;
    let right = (head.right * mask.width() as f32).ceil() as u32;
    let bottom = (head.bottom * mask.height() as f32).ceil() as u32;
    // Restrict to the measured head's horizontal column: a raised hand is not a crown.
    mask.row_segments().find_map(|(y, from, to)| {
        let (from, to) = (from.max(left), to.min(right));
        (y < bottom && from < to).then_some(((from as f32 + to as f32) / 2.0, y))
    })
}

fn match_figures(
    scan: &StoredScan,
    heads: &[LocatedHead],
    staged: &[StagedFigure<'_>],
    hero_count: usize,
    staging_order_known: bool,
) -> Result<Vec<FigureGeometry>, ImageFailure> {
    let masks = scan.instances()?;
    if hero_count > 6 {
        return Err(ImageFailure::InvalidRequest);
    }
    let whole_cast = !staged.is_empty();
    let mut heroes = 0;
    let mut staged: Vec<_> = staged
        .iter()
        .filter(|person| {
            if person.hero {
                heroes += 1;
                heroes <= hero_count
            } else {
                true
            }
        })
        .collect();
    let mut assigned: Vec<(String, usize, Option<HeadBox>, bool)> = Vec::new();
    // Largest overlaps claim first, rather than allowing head/response order to steal a mask.
    let mut candidates: Vec<_> = heads
        .iter()
        .enumerate()
        .flat_map(|(head_index, head)| {
            masks
                .iter()
                .enumerate()
                .filter_map(move |(mask_index, instance)| {
                    let pixels = contains_head(&instance.mask, head.head);
                    (pixels > 0).then_some((pixels, head_index, mask_index))
                })
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, head_index, mask_index) in candidates {
        let Some(head) = heads.get(head_index) else {
            continue;
        };
        if whole_cast && !staged.iter().any(|person| person.id == head.id) {
            continue;
        }
        if assigned
            .iter()
            .any(|(id, index, _, _)| id == &head.id || *index == mask_index)
        {
            continue;
        }
        assigned.push((head.id.clone(), mask_index, Some(head.head), false));
    }
    let mut ordered_masks: Vec<_> = masks
        .iter()
        .enumerate()
        .filter_map(|(index, instance)| {
            let bounds = instance.mask.bounds()?;
            Some((bounds.left + (bounds.right - bounds.left) / 2, index))
        })
        .collect();
    ordered_masks.sort_unstable();
    if !staging_order_known {
        // Without explicit spatial staging, only a unique remaining identity is safe to infer.
        // Reference-sheet order and model response order are not picture positions.
        let missing: Vec<_> = staged
            .iter()
            .filter(|person| !assigned.iter().any(|(id, _, _, _)| id == person.id))
            .collect();
        let available: Vec<_> = ordered_masks
            .iter()
            .filter(|(_, index)| !assigned.iter().any(|(_, used, _, _)| used == index))
            .collect();
        if let ([person], [(_, index)]) = (missing.as_slice(), available.as_slice()) {
            assigned.push((person.id.to_owned(), *index, None, true));
        }
        staged.clear();
    }
    let measured_ranks: Vec<_> = staged
        .iter()
        .filter_map(|person| {
            let (_, index, _, _) = assigned.iter().find(|(id, _, _, _)| id == person.id)?;
            ordered_masks
                .iter()
                .position(|(_, candidate)| candidate == index)
        })
        .collect();
    if measured_ranks.windows(2).any(|pair| pair[0] >= pair[1]) {
        // The photograph contradicted the staging order. Keep measured identities, but do not
        // name any extra person from those now-conflicting staging facts.
        staged.clear();
    }
    // Named measured people partition staging into intervals. Infer only intervals with exactly
    // as many masks as missing named slots: extra/background people remain anonymous.
    let mut previous_rank = None;
    let mut missing = Vec::new();
    for person in staged.iter().map(Some).chain(std::iter::once(None)) {
        let matched_rank = person
            .and_then(|person| assigned.iter().find(|(id, _, _, _)| id == person.id))
            .and_then(|(_, index, _, _)| {
                ordered_masks
                    .iter()
                    .position(|(_, candidate)| candidate == index)
            });
        if person.is_some() && matched_rank.is_none() {
            if let Some(person) = person {
                missing.push(person.id);
            }
            continue;
        }
        let end = matched_rank.unwrap_or(ordered_masks.len());
        let start = previous_rank.map_or(0, |rank| rank + 1);
        let available: Vec<_> = ordered_masks
            .iter()
            .enumerate()
            .filter(|(rank, (_, index))| {
                *rank >= start
                    && *rank < end
                    && !assigned.iter().any(|(_, used, _, _)| used == index)
            })
            .map(|(_, (_, index))| *index)
            .collect();
        if available.len() == missing.len() {
            for (id, index) in missing.iter().zip(available) {
                assigned.push(((*id).to_owned(), index, None, true));
            }
        }
        missing.clear();
        previous_rank = matched_rank;
    }
    let mut figures = Vec::new();
    for (id, index, head, inferred) in assigned {
        let Some(instance) = masks.get(index) else {
            continue;
        };
        let Some(bounds) = instance.mask.bounds() else {
            continue;
        };
        let Some((x, y)) = crown(&instance.mask, head) else {
            continue;
        };
        let source_x = |x: f32| (x * scan.source_width as f32 / scan.width as f32).round() as u32;
        let source_y =
            |y: u32| (u64::from(y) * u64::from(scan.source_height) / u64::from(scan.height)) as u32;
        let centre = (bounds.left as f32 + bounds.right as f32) / 2.0;
        let left_space = masks
            .iter()
            .filter_map(|other| other.mask.bounds())
            .filter(|other| other.right <= bounds.left)
            .map(|other| bounds.left - other.right)
            .min()
            .unwrap_or(bounds.left);
        let right_space = masks
            .iter()
            .filter_map(|other| other.mask.bounds())
            .filter(|other| other.left >= bounds.right)
            .map(|other| other.left - bounds.right)
            .min()
            .unwrap_or(scan.width - bounds.right);
        figures.push(FigureGeometry {
            id,
            mask_index: instance.index,
            mask_url: String::new(),
            source_width: scan.source_width,
            source_height: scan.source_height,
            bounds: [
                source_x(bounds.left as f32),
                source_y(bounds.top),
                source_x(bounds.right as f32),
                source_y(bounds.bottom),
            ],
            head_top: [source_x(x), source_y(y)],
            head_top_permille: [
                (x * 1000.0 / scan.width as f32).round() as u32,
                (u64::from(y) * 1000 / u64::from(scan.height)) as u32,
            ],
            free_left: left_space > right_space
                || (left_space == right_space && centre >= scan.width as f32 / 2.0),
            inferred,
        });
    }
    Ok(figures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn hs54_sam_gates_coalesce_only_the_same_plate() {
        let cache = FigureMaskCache::default();
        let first = cache.gate_for("first");
        let same = cache.gate_for("first");
        let other = cache.gate_for("other");
        assert!(Arc::ptr_eq(&first, &same));
        let _held = first.lock().await;
        assert!(same.try_lock().is_err());
        assert!(
            other.try_lock().is_ok(),
            "another plate never waits behind this SAM call"
        );
        drop(other);
        let _next = cache.gate_for("next");
        assert!(
            !locked(&cache.gates).contains_key("other"),
            "unused keyed locks are reclaimed"
        );
    }

    fn rectangle(left: u32, top: u32, right: u32, bottom: u32) -> StoredMask {
        StoredMask {
            index: left,
            score: 0.95,
            runs: (top..bottom)
                .map(|y| [y * 40 + left, right - left])
                .collect(),
        }
    }

    fn scan(masks: Vec<StoredMask>) -> StoredScan {
        StoredScan {
            revision: REVISION,
            model: "sam-3.1".into(),
            image_sha256: String::new(),
            width: 40,
            height: 20,
            source_width: 400,
            source_height: 200,
            outcome: "complete".into(),
            masks,
        }
    }

    fn head(id: &str, left: f32, top: f32, right: f32, bottom: f32) -> LocatedHead {
        LocatedHead {
            id: id.into(),
            head: HeadBox {
                left,
                top,
                right,
                bottom,
            },
        }
    }

    #[test]
    fn figure_masks_match_named_heads_by_pixels_not_sam_score_order() {
        let scan = scan(vec![rectangle(26, 3, 35, 19), rectangle(4, 2, 14, 20)]);
        let heads = [
            head("hero-1", 0.125, 0.1, 0.3, 0.3),
            head("npc-0", 0.7, 0.15, 0.85, 0.4),
        ];
        let matched = match_figures(&scan, &heads, &[], 0, true).unwrap();
        let hero = matched.iter().find(|figure| figure.id == "hero-1").unwrap();
        assert_eq!(hero.bounds, [40, 20, 140, 200]);
        assert_eq!(hero.head_top, [85, 20]);
        assert_eq!(hero.head_top_permille, [213, 100]);
        assert!(!hero.free_left);
        assert!(!hero.inferred);
        assert_eq!(
            matched
                .iter()
                .find(|figure| figure.id == "npc-0")
                .unwrap()
                .mask_index,
            26
        );
    }

    #[test]
    fn figure_masks_infer_a_missing_hero_from_whole_cast_staging_order() {
        let scan = scan(vec![rectangle(26, 3, 35, 19), rectangle(4, 2, 14, 20)]);
        let heads = [head("npc-0", 0.7, 0.15, 0.85, 0.4)];
        let staged = [
            StagedFigure {
                id: "hero-1",
                hero: true,
            },
            StagedFigure {
                id: "npc-0",
                hero: false,
            },
        ];
        let matched = match_figures(&scan, &heads, &staged, 1, true).unwrap();
        let hero = matched.iter().find(|figure| figure.id == "hero-1").unwrap();
        assert!(hero.inferred);
        assert_eq!(hero.bounds, [40, 20, 140, 200]);
        assert_eq!(hero.head_top, [90, 20]);
        assert!(
            !matched
                .iter()
                .find(|figure| figure.id == "npc-0")
                .unwrap()
                .inferred
        );
        let bounded = match_figures(&scan, &heads, &staged, 0, true).unwrap();
        assert!(!bounded.iter().any(|figure| figure.id == "hero-1"));
    }

    #[test]
    fn figure_masks_leave_extra_people_anonymous_when_order_is_ambiguous() {
        let scan = scan(vec![
            rectangle(2, 3, 8, 19),
            rectangle(12, 2, 18, 20),
            rectangle(26, 3, 35, 19),
        ]);
        let staged = [
            StagedFigure {
                id: "hero-1",
                hero: true,
            },
            StagedFigure {
                id: "npc-0",
                hero: false,
            },
        ];
        let heads = [head("npc-0", 0.7, 0.15, 0.85, 0.4)];
        let matched = match_figures(&scan, &heads, &staged, 1, true).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "npc-0");
    }

    #[test]
    fn figure_masks_without_spatial_staging_only_infer_one_unique_missing_person() {
        let scan = scan(vec![rectangle(26, 3, 35, 19), rectangle(4, 2, 14, 20)]);
        // This is reference-sheet order, deliberately reversed from the actual photograph.
        let cast = [
            StagedFigure {
                id: "npc-0",
                hero: false,
            },
            StagedFigure {
                id: "hero-1",
                hero: true,
            },
        ];
        let heads = [head("npc-0", 0.7, 0.15, 0.85, 0.4)];
        let matched = match_figures(&scan, &heads, &cast, 1, false).unwrap();
        assert!(matched.iter().any(|figure| figure.id == "hero-1"
            && figure.inferred
            && figure.bounds == [40, 20, 140, 200]));
        let ambiguous = match_figures(&scan, &[], &cast, 1, false).unwrap();
        assert!(
            ambiguous.is_empty(),
            "two anonymous masks cannot establish identity order"
        );
    }

    #[test]
    fn figure_masks_crown_ignores_a_raised_hand_outside_the_head_column() {
        let mut mask = rectangle(10, 5, 20, 20);
        mask.runs.insert(0, [1, 2]);
        let scan = scan(vec![mask]);
        let matched =
            match_figures(&scan, &[head("hero-1", 0.3, 0.25, 0.45, 0.5)], &[], 0, true).unwrap();
        assert_eq!(matched[0].bounds[1], 0);
        assert_eq!(matched[0].head_top, [150, 50]);
    }

    #[test]
    fn figure_masks_raster_png_keeps_plate_aspect_and_feathers_alpha() {
        let mut scan = scan(vec![rectangle(10, 5, 30, 15)]);
        scan.source_width = 2400;
        scan.source_height = 1200;
        let instances = scan.instances().unwrap();
        let png = raster_focus_mask(&scan, &instances[0].mask).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (768, 384));
        assert_eq!(decoded.get_pixel(384, 192).0, [255, 255, 255, 255]);
        assert_eq!(decoded.get_pixel(0, 0).0[3], 0);
        let edge = decoded.get_pixel(192, 192).0[3];
        assert!(edge > 0 && edge < 255, "alpha at feathered edge: {edge}");
        assert_eq!(decoded.get_pixel(180, 192).0[3], 0);
        assert_eq!(decoded.get_pixel(204, 192).0[3], 255);
        scan.source_width = 400;
        scan.source_height = 800;
        let portrait =
            image::load_from_memory(&raster_focus_mask(&scan, &instances[0].mask).unwrap())
                .unwrap();
        assert_eq!((portrait.width(), portrait.height()), (384, 768));
    }

    fn imagery(directory: PathBuf) -> Imagery {
        use crate::gameplay::startup_configuration::{
            ContentRating, ImageryConfiguration, ImageryCredential,
        };
        Imagery::new(
            ImageryConfiguration {
                credential: ImageryCredential::for_test("fixture-key"),
                model: df_providers::ImageEditModel::Gpt25Flare,
                quality: df_providers::ImageQuality::Medium,
                profile: Default::default(),
                interval: Duration::from_secs(20),
                deadline: Duration::from_secs(5),
                cache_dir: directory,
                rating: ContentRating::Mature,
            },
            df_providers::NativeImageEditProvider::new(
                df_providers::ImageEditConfig::live(),
                "fixture-key",
                Duration::from_secs(5),
            )
            .unwrap(),
            None,
        )
        .unwrap()
    }

    async fn register(imagery: &Imagery) -> ImageKey {
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(40, 20)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        imagery
            .register_reviewed(
                "plate:mask-fixture",
                super::super::ReferenceImage::new(
                    df_providers::ImageFormat::Jpeg,
                    bytes::Bytes::from(jpeg.into_inner()),
                ),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn figure_masks_loopback_matches_and_cached_plate_never_submits_again() {
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        let submits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&submits);
        let rle = |left, top, right, bottom| {
            rectangle(left, top, right, bottom)
                .runs
                .iter()
                .map(|[start, length]| format!("{} {length}", start + 1))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let result =
            serde_json::json!({"rle": [rle(26,3,35,19), rle(4,2,14,20)], "scores": [0.96,0.95]});
        let app = Router::new()
            .route("/fal-ai/sam-3-1/image-rle", post(move |Json(body): Json<serde_json::Value>| {
                counted.fetch_add(1, Ordering::SeqCst);
                assert_eq!(body["prompt"], "person");
                assert_eq!(body["max_masks"], 32);
                async { Json(serde_json::json!({
                    "request_id": "hs51-fixture",
                    "status_url": "https://queue.fal.run/fal-ai/sam-3-1/requests/hs51-fixture/status",
                    "response_url": "https://queue.fal.run/fal-ai/sam-3-1/requests/hs51-fixture",
                })) }
            }))
            .route("/fal-ai/sam-3-1/requests/hs51-fixture/status", get(|| async { Json(serde_json::json!({"status": "COMPLETED"})) }))
            .route("/fal-ai/sam-3-1/requests/hs51-fixture", get(move || { let result = result.clone(); async move { Json(result) } }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _stopped = stopped.await;
                })
                .await
                .unwrap();
        });
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS62")
            .join(format!(
                "mask-fixture-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let imagery = imagery(directory.clone());
        let Mode::On { engine, .. } = &imagery.inner.mode else {
            panic!("enabled fixture");
        };
        engine
            .figure_masks
            .provider
            .set(
                NativeFalSegmenter::new(
                    FalSegmentConfig::loopback_fixture(
                        SegmentModel::Sam31,
                        address,
                        Duration::from_millis(1),
                    )
                    .unwrap(),
                    "fixture-key",
                    Duration::from_secs(3),
                )
                .unwrap(),
            )
            .unwrap();
        let key = register(&imagery).await;
        let heads = [head("npc-0", 0.7, 0.15, 0.85, 0.4)];
        let staged = [
            StagedFigure {
                id: "hero-1",
                hero: true,
            },
            StagedFigure {
                id: "npc-0",
                hero: false,
            },
        ];
        let (first, second) = tokio::join!(
            imagery.figure_geometry(&key, &heads, &staged, 1, true),
            imagery.plate_masks(&key)
        );
        assert!(second.is_ok());
        let figures = first.unwrap();
        assert!(
            figures
                .iter()
                .any(|figure| figure.id == "hero-1" && figure.inferred)
        );
        assert!(figures.iter().any(|figure| figure.id == "npc-0"
            && !figure.inferred
            && figure.bounds == [26, 3, 35, 19]));
        let figure = figures.iter().find(|figure| figure.id == "npc-0").unwrap();
        assert_eq!(
            figure.mask_url,
            format!("{}/figure/{}", key.path(), figure.mask_index)
        );
        let cached_mask = focus_mask_path(&directory, key.as_str(), figure.mask_index);
        let mask_modified = std::fs::metadata(&cached_mask).unwrap().modified().unwrap();
        let response = super::super::serve_focus_mask(
            axum::extract::State(imagery.clone()),
            axum::extract::Path((key.as_str().to_owned(), figure.mask_index)),
            axum::http::HeaderMap::new(),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "image/png"
        );
        assert_eq!(
            response.headers()[axum::http::header::CACHE_CONTROL],
            super::super::CACHE_CONTROL
        );
        let etag = response.headers()[axum::http::header::ETAG].clone();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let png = image::load_from_memory(&body).unwrap();
        assert_eq!((png.width(), png.height()), (40, 20));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, etag);
        let unchanged = super::super::serve_focus_mask(
            axum::extract::State(imagery.clone()),
            axum::extract::Path((key.as_str().to_owned(), figure.mask_index)),
            headers,
        )
        .await;
        assert_eq!(unchanged.status(), axum::http::StatusCode::NOT_MODIFIED);
        assert!(imagery.focus_mask(key.as_str(), 999).await.is_none());
        assert!(imagery.focus_mask("../invalid", 0).await.is_none());
        assert_eq!(submits.load(Ordering::SeqCst), 1);
        assert_eq!(engine.ledger.snapshot(Unit::SegmentCall).committed, 1);
        assert_eq!(
            engine.micro_dollars.load(Ordering::SeqCst),
            SegmentModel::Sam31.price_micro_dollars()
        );
        // A fresh process-equivalent service has no provider configured and still reads masks.
        let restarted = self::imagery(directory.clone());
        let restarted_key = register(&restarted).await;
        assert_eq!(key.as_str(), restarted_key.as_str());
        assert_eq!(
            restarted
                .figure_geometry(&key, &heads, &staged, 1, true)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(submits.load(Ordering::SeqCst), 1);
        let cached_note = directory.join(format!("{}.sam31-person-v1.json", key.as_str()));
        assert_eq!(
            std::fs::metadata(cached_mask).unwrap().modified().unwrap(),
            mask_modified
        );
        assert!(matches!(
            engine
                .figure_masks
                .read_or_scan(
                    cached_note,
                    key.as_str(),
                    b"different plate bytes",
                    df_providers::ImageFormat::Jpeg,
                    engine
                )
                .await,
            Err(ImageFailure::BadOutput)
        ));
        let mut interrupted = scan(vec![]);
        interrupted.image_sha256 = format!("{:x}", Sha256::digest(b""));
        interrupted.outcome = "sent".into();
        let marker = directory.join("interrupted.sam31-person-v1.json");
        std::fs::write(&marker, serde_json::to_vec(&interrupted).unwrap()).unwrap();
        assert!(matches!(
            engine
                .figure_masks
                .read_or_scan(
                    marker,
                    "interrupted",
                    b"",
                    df_providers::ImageFormat::Jpeg,
                    engine
                )
                .await,
            Err(ImageFailure::BadOutput)
        ));
        let corrupt = directory.join("corrupt.sam31-person-v1.json");
        std::fs::write(&corrupt, b"broken note").unwrap();
        assert!(matches!(
            engine
                .figure_masks
                .read_or_scan(
                    corrupt,
                    "corrupt",
                    b"",
                    df_providers::ImageFormat::Jpeg,
                    engine
                )
                .await,
            Err(ImageFailure::BadOutput)
        ));
        assert_eq!(
            submits.load(Ordering::SeqCst),
            1,
            "unknown or corrupt cached outcomes never authorize another paid call"
        );
        stop.send(()).unwrap();
        server.await.unwrap();
        // Retain the exact fixture sidecars for coordinator inspection.
    }
}
