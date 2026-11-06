//! The SAM scan: a library picture's instances through the imagery API, reduced to a layout whose
//! regions keep the names of the regions they overlap in the layout they replace.

use std::path::Path;

use df_imagery::oneshot_segment::{
    self, AnalysisMaskSet, SegmentConfig, SegmentError, SegmentRequest,
};
use df_scene_layout::{
    FULL_PERMILLE, PictureBinding, RegionGeometry, RegionRef, SceneLayout, SceneRegion, region_id,
};

use super::limits::SCAN_TIMEOUT;
use super::request::{LocateError, LocateRequest, ScanPrompt};
use super::storage::{lock_layout_writes, read_layout, read_picture, write_layout};
use super::validate::{check_label, check_masks, check_picture_name};

/// The environment variables the supplier key is read from, in order.
pub const FAL_KEY_VARIABLES: [&str; 2] = ["DF_FAL_KEY", "FAL_KEY"];
/// How much a new region must overlap an old one (intersection over union, in thousandths) to
/// keep its name.
pub const KEEP_LABEL_OVERLAP_PERMILLE: u16 = 500;
/// The share of the picture's height whose topmost rows are averaged for a head point.
const HEAD_ROWS_DIVISOR: u32 = 100;

fn permille(value: f64, whole: u32) -> u16 {
    let share = (value * f64::from(FULL_PERMILLE) / f64::from(whole.max(1))).round();
    share.clamp(0.0, f64::from(FULL_PERMILLE)) as u16
}

/// The layout of a mask set, left to right, and how many instances it left out: one region per
/// instance whose box has width and height in thousandths. An instance whose box rounds to no
/// width or height cannot be drawn or pointed at, so it is not a region; the scan records how many.
/// `picture` binds the layout to the scanned picture (its name and digest; its size is the scanned
/// size); each region's id comes from its mask, so the same mask keeps the same id.
pub fn layout_from(
    set: &AnalysisMaskSet,
    model: &str,
    prompt: ScanPrompt,
    picture: PictureBinding,
) -> (SceneLayout, usize) {
    let head_rows = (set.height_px / HEAD_ROWS_DIVISOR).max(1);
    let mut left_out = 0;
    let mut regions: Vec<SceneRegion> = Vec::new();
    for mask in &set.instances {
        // An instance with no pixels has no geometry: there is nothing to leave out.
        let Some(geometry) = oneshot_segment::instance_geometry(set, mask, head_rows) else {
            continue;
        };
        let runs: Vec<u8> = mask
            .runs
            .iter()
            .flat_map(|run| [run.start.to_be_bytes(), run.length.to_be_bytes()])
            .flatten()
            .collect();
        let measured = RegionGeometry {
            left_permille: permille(f64::from(geometry.left), set.width_px),
            top_permille: permille(f64::from(geometry.top), set.height_px),
            right_permille: permille(f64::from(geometry.right), set.width_px),
            bottom_permille: permille(f64::from(geometry.bottom), set.height_px),
            head_x_permille: permille(f64::from(geometry.head_x), set.width_px),
            head_y_permille: permille(f64::from(geometry.head_y), set.height_px),
        };
        let score_permille = (mask.score.get() * 1000.0).round().clamp(0.0, 1000.0) as u16;
        match SceneRegion::new(region_id(&runs), measured, score_permille) {
            Ok(region) => {
                regions.push(region.with_mask(oneshot_segment::mask_data_url(set, mask)));
            }
            // Recorded as a count in the scan's audit line (`left_out=`).
            Err(_) => left_out += 1,
        }
    }
    regions.sort_by_key(|region| (region.left_permille(), region.top_permille()));
    let picture = PictureBinding {
        width_px: set.width_px,
        height_px: set.height_px,
        ..picture
    };
    (
        SceneLayout::new(picture, model, prompt.key(), regions),
        left_out,
    )
}

/// Intersection over union of `a` and `b`, in thousandths.
fn overlap_permille(a: &SceneRegion, b: &SceneRegion) -> u16 {
    let area = |r: &SceneRegion| {
        u64::from(r.right_permille().saturating_sub(r.left_permille()))
            * u64::from(r.bottom_permille().saturating_sub(r.top_permille()))
    };
    let width = a
        .right_permille()
        .min(b.right_permille())
        .saturating_sub(a.left_permille().max(b.left_permille()));
    let height = a
        .bottom_permille()
        .min(b.bottom_permille())
        .saturating_sub(a.top_permille().max(b.top_permille()));
    let shared = u64::from(width) * u64::from(height);
    let union = area(a) + area(b) - shared;
    let overlap = (shared * u64::from(FULL_PERMILLE))
        .checked_div(union)
        .unwrap_or(0);
    // The intersection is never larger than the union.
    u16::try_from(overlap).map_or(FULL_PERMILLE, |overlap| overlap.min(FULL_PERMILLE))
}

/// Gives each region of `new` the name of the old region it overlaps most, when that overlap is
/// at least `KEEP_LABEL_OVERLAP_PERMILLE` and no other new region took the name. Returns how many
/// names were kept.
pub fn keep_labels(old: &SceneLayout, new: &mut SceneLayout) -> usize {
    let mut kept = 0;
    for named in old
        .regions()
        .iter()
        .filter(|region| region.label().is_some())
    {
        let best = new
            .regions_mut()
            .iter_mut()
            .filter(|region| region.label().is_none())
            .map(|region| (overlap_permille(named, region), region))
            .filter(|(overlap, _)| *overlap >= KEEP_LABEL_OVERLAP_PERMILLE)
            .max_by_key(|(overlap, _)| *overlap);
        if let Some((_, region)) = best {
            region.set_label(named.label().map(str::to_owned));
            kept += 1;
        }
    }
    kept
}

/// Names region `index` of `layout` `label`, or clears its name. A name is unique in a layout, so
/// it moves off any other region that had it.
pub fn relabel(
    layout: &mut SceneLayout,
    reference: RegionRef<'_>,
    label: Option<&str>,
) -> Result<(), LocateError> {
    let Some(index) = layout.find(reference) else {
        return Err(LocateError::UnknownRegion {
            region: match reference {
                RegionRef::Number(number) => number.to_string(),
                RegionRef::Id(text) | RegionRef::Name(text) => text.to_owned(),
            },
            region_count: layout.regions().len(),
            names: layout.names().into_iter().map(str::to_owned).collect(),
        });
    };
    if let Some(label) = label {
        check_label(label)?;
        for region in layout.regions_mut() {
            if region.label() == Some(label) {
                region.set_label(None);
            }
        }
    }
    layout.regions_mut()[index].set_label(label.map(str::to_owned));
    Ok(())
}

/// What a scan stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    pub layout: SceneLayout,
    pub duration_ms: u64,
    pub kept_labels: usize,
    /// How many instances the scan found whose box rounds to no width or height in thousandths,
    /// left out of the layout ([`layout_from`]).
    pub left_out: usize,
    /// The layout this scan replaced could not be read, so none of its names were kept.
    pub old_layout_unreadable: bool,
}

/// Scans `picture` of `media` through the imagery API and stores its layout in `layouts`. Reading
/// the picture and storing the layout each run on the blocking pool (a step that stopped without
/// an answer is `Stopped`); the scan itself is awaited here.
pub async fn scan_picture(
    media: &Path,
    layouts: &Path,
    request: &LocateRequest,
) -> Result<Scanned, LocateError> {
    check_picture_name(&request.picture)?;
    let prompt = request.prompt.unwrap_or_default();
    let masks = check_masks(request.masks)?;
    let (media, picture) = (media.to_owned(), request.picture.clone());
    let (source, sha256) = tokio::task::spawn_blocking(move || {
        let source = read_picture(&media, &picture)?;
        let sha256 = oneshot_segment::picture_sha256(&source);
        Ok((source, sha256))
    })
    .await
    .unwrap_or(Err(LocateError::Stopped))?;
    let binding = PictureBinding {
        name: request.picture.clone(),
        sha256,
        width_px: 0,
        height_px: 0,
    };
    let credential =
        crate::environment::first_text(&FAL_KEY_VARIABLES).ok_or(LocateError::FalKeyMissing)?;
    let config = SegmentConfig {
        endpoint: oneshot_segment::DEFAULT_ENDPOINT.to_owned(),
        model: oneshot_segment::DEFAULT_MODEL.to_owned(),
        credential,
        timeout: SCAN_TIMEOUT,
    };
    let started = tokio::time::Instant::now();
    let set = oneshot_segment::segment(
        &config,
        SegmentRequest {
            source,
            prompt: prompt.segmentation(),
            maximum_masks: masks,
        },
    )
    .await
    .map_err(|error: SegmentError| LocateError::ScanFailed {
        cause: error.into(),
    })?;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (mut layout, left_out) = layout_from(&set, "sam-3.1", prompt, binding);
    let (layouts, picture) = (layouts.to_owned(), request.picture.clone());
    tokio::task::spawn_blocking(move || {
        let _writing = lock_layout_writes();
        // An old layout that does not read has no names anyone can use any more, and the scan is
        // already paid for: it is replaced, and the caller records that its names were lost.
        let (kept_labels, old_layout_unreadable) = match read_layout(&layouts, &picture) {
            Ok(Some(old)) => (keep_labels(&old, &mut layout), false),
            Ok(None) => (0, false),
            Err(_) => (0, true),
        };
        write_layout(&layouts, &picture, &layout)?;
        Ok(Scanned {
            layout,
            duration_ms,
            kept_labels,
            left_out,
            old_layout_unreadable,
        })
    })
    .await
    .unwrap_or(Err(LocateError::Stopped))
}
