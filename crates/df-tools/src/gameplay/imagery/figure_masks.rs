//! Plate figure geometry over actual service-owned SAM results and published focus derivatives.
//! Gameplay retains assignment rules; the runtime owns providers, paid state, raster work and replay.

use super::{ImageFailure, ImageKey, Imagery, analysis};
use crate::gameplay::image_references::PresentationAnnotation;
use df_imagery::{
    AlphaHandling, Dimensions, Encoding, ImageTransform, Purpose, PurposedUse, RasterOutputSpec,
    SegmentationPreparation, SegmentationPrompt, TransformSource, TransformSpec, VariantRole,
};
use df_providers::{HeadBox, LocatedHead, MaskRows, SegmentInstance};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct FocusMask {
    index: u32,
    score: f32,
    runs: Vec<[u32; 2]>,
}
struct MaskScan {
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    masks: Vec<FocusMask>,
}
impl MaskScan {
    fn instances(&self) -> Result<Vec<SegmentInstance>, ImageFailure> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || self.source_width == 0
            || self.source_height == 0
            || self.masks.len() > 32
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

#[derive(Deserialize, Serialize)]
struct AssignedFigure {
    geometry: FigureGeometry,
    derivative: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct FigureProjection {
    source_sha256: String,
    figures: Vec<AssignedFigure>,
}

fn focus_output(scan: &MaskScan) -> Result<RasterOutputSpec, ImageFailure> {
    let scale = 768.0 / f64::from(scan.source_width.max(scan.source_height).max(768));
    // The fixed fit bounds both rounded dimensions by 768.
    let width = (f64::from(scan.source_width) * scale).round().max(1.0) as u32;
    let height = (f64::from(scan.source_height) * scale).round().max(1.0) as u32;
    RasterOutputSpec::new(
        Dimensions::new(width, height).map_err(|_| ImageFailure::BadOutput)?,
        AlphaHandling::PreserveAlpha,
        Encoding::Png,
    )
    .map_err(|_| ImageFailure::BadOutput)
}

impl Imagery {
    async fn plate_masks(
        &self,
        key: &ImageKey,
    ) -> Result<
        (
            analysis::Observation<Vec<df_imagery::AnalysisMaskSet>>,
            MaskScan,
            String,
        ),
        ImageFailure,
    > {
        let reference = self.reference(key).await?;
        let header =
            df_imagery::source_header(&reference.bytes).map_err(|_| ImageFailure::BadOutput)?;
        let observed = analysis::segments(
            self,
            key,
            SegmentationPreparation::FigureMask,
            vec![SegmentationPrompt::Person],
            32,
        )
        .await?;
        if reference.canonical_source() != Some(&observed.source) || observed.value.len() != 1 {
            return Err(ImageFailure::BadOutput);
        }
        let set = observed.value.first().ok_or(ImageFailure::BadOutput)?;
        if set.prompt != SegmentationPrompt::Person {
            return Err(ImageFailure::BadOutput);
        }
        let scan = MaskScan {
            width: set.width_px,
            height: set.height_px,
            source_width: header.dimensions.width_px(),
            source_height: header.dimensions.height_px(),
            masks: set
                .instances
                .iter()
                .map(|mask| FocusMask {
                    index: mask.index,
                    score: mask.score.get(),
                    runs: mask
                        .runs
                        .iter()
                        .map(|run| [run.start, run.length])
                        .collect(),
                })
                .collect(),
        };
        scan.instances()?;
        Ok((observed, scan, reference.sha256))
    }

    /// Assigns only named measured or uniquely inferable figures, then publishes their masks.
    pub(in crate::gameplay) async fn figure_geometry(
        &self,
        key: &ImageKey,
        heads: &[LocatedHead],
        staged: &[StagedFigure<'_>],
        hero_count: usize,
        staging_order_known: bool,
    ) -> Result<Vec<FigureGeometry>, ImageFailure> {
        let (observed, scan, source_sha256) = self.plate_masks(key).await?;
        let figures = match_figures(&scan, heads, staged, hero_count, staging_order_known)?;
        let output = focus_output(&scan)?;
        let mut assigned = Vec::with_capacity(figures.len());
        for mut figure in figures {
            let spec = TransformSpec {
                source: TransformSource::SegmentationMask {
                    analyzed: observed.source.clone(),
                    segmentation: observed.handle.clone(),
                    prompt: SegmentationPrompt::Person,
                    mask_index: figure.mask_index,
                    contract_revision: 1,
                },
                steps: Vec::<ImageTransform>::new(),
            };
            let result = match self
                .transform(
                    "figure-focus-mask",
                    PurposedUse {
                        purpose: Purpose::ReferencePrep,
                        role: VariantRole::Mask,
                    },
                    spec,
                    output,
                )
                .await
            {
                Ok(reference) => self.register_reviewed("figure-focus-mask", reference).await,
                Err(error) => Err(error),
            };
            let derivative = match result {
                Ok(derived) => {
                    figure.mask_url = format!(
                        "/gameplay/imagery/{}/figure/{}",
                        key.as_str(),
                        figure.mask_index
                    );
                    Some(derived.as_str().to_owned())
                }
                Err(error) => {
                    focus_diagnostic(
                        "missing",
                        key.as_str(),
                        &figure.id,
                        figure.mask_index,
                        Some(error.label()),
                    );
                    None
                }
            };
            assigned.push(AssignedFigure {
                geometry: figure,
                derivative,
            });
        }
        for person in staged {
            if !assigned
                .iter()
                .any(|figure| figure.geometry.id == person.id)
            {
                focus_diagnostic("missing", key.as_str(), person.id, 0, Some("unassigned"));
            }
        }
        let projection = FigureProjection {
            source_sha256,
            figures: assigned,
        };
        let bytes = serde_json::to_vec(&projection).map_err(|_| ImageFailure::BadOutput)?;
        self.write_presentation_annotation(key, PresentationAnnotation::Figures, bytes)
            .await?;
        let figures = projection
            .figures
            .into_iter()
            .map(|figure| figure.geometry)
            .collect::<Vec<_>>();
        diagnostic(
            "assignments",
            key.as_str(),
            serde_json::json!({"figures":figures,
            "staging_order_known":staging_order_known,"hero_count":hero_count}),
        );
        Ok(figures)
    }

    /// An HTTP read only reauthorizes existing assigned publications; it never starts work.
    pub(super) async fn focus_mask(&self, key: &str, index: u32) -> Option<bytes::Bytes> {
        let result = self.read_focus_mask(key, index).await;
        match result {
            Ok(bytes) => bytes,
            Err(error) => {
                focus_diagnostic("missing", key, "unavailable", index, Some(error.label()));
                None
            }
        }
    }

    async fn read_focus_mask(
        &self,
        key: &str,
        index: u32,
    ) -> Result<Option<bytes::Bytes>, ImageFailure> {
        let key = ImageKey::parse(key).ok_or(ImageFailure::InvalidRequest)?;
        let source = self.reference(&key).await?;
        let Some(bytes) = self
            .read_presentation_annotation(&key, PresentationAnnotation::Figures)
            .await?
        else {
            return Ok(None);
        };
        let projection: FigureProjection =
            serde_json::from_slice(&bytes).map_err(|_| ImageFailure::BadOutput)?;
        if projection.source_sha256 != source.sha256 {
            return Err(ImageFailure::BadOutput);
        }
        let mut figures = projection
            .figures
            .iter()
            .filter(|figure| figure.geometry.mask_index == index);
        let Some(figure) = figures.next() else {
            return Ok(None);
        };
        if figures.next().is_some() {
            return Err(ImageFailure::BadOutput);
        }
        let Some(derivative) = figure.derivative.as_deref() else {
            return Ok(None);
        };
        let derived = ImageKey::parse(derivative).ok_or(ImageFailure::BadOutput)?;
        let image = self.reference(&derived).await?;
        if image.format != df_providers::ImageFormat::Png {
            return Err(ImageFailure::BadOutput);
        }
        focus_diagnostic("served", key.as_str(), &figure.geometry.id, index, None);
        Ok(Some(image.bytes))
    }
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
    scan: &MaskScan,
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
    fn rectangle(left: u32, top: u32, right: u32, bottom: u32) -> FocusMask {
        FocusMask {
            index: left,
            score: 0.95,
            runs: (top..bottom)
                .map(|y| [y * 40 + left, right - left])
                .collect(),
        }
    }

    fn scan(masks: Vec<FocusMask>) -> MaskScan {
        MaskScan {
            width: 40,
            height: 20,
            source_width: 400,
            source_height: 200,
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
    #[tokio::test]
    async fn actual_analysis_coalesces_publishes_assigned_masks_and_reuses_after_restart() {
        use crate::gameplay::{cost::port::MeteringPort, image_test_support::ImageHostFixture};
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let submits = Arc::new(AtomicUsize::new(0));
        let counted = submits.clone();
        let rle = |left, top, right, bottom| {
            rectangle(left, top, right, bottom)
                .runs
                .iter()
                .map(|[start, length]| format!("{} {length}", start + 1))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let result =
            serde_json::json!({"rle":[rle(26,3,35,19),rle(4,2,14,20)],"scores":[0.96,0.95]});
        let app = Router::new()
            .route("/fal-ai/sam-3-1/image-rle",post(move |Json(body):Json<serde_json::Value>| {
                counted.fetch_add(1,Ordering::SeqCst);
                assert_eq!(body["prompt"],"person"); assert_eq!(body["max_masks"],32);
                async { Json(serde_json::json!({"request_id":"figure-fixture",
                    "status_url":"https://queue.fal.run/fal-ai/sam-3-1/requests/figure-fixture/status",
                    "response_url":"https://queue.fal.run/fal-ai/sam-3-1/requests/figure-fixture"})) }
            }))
            .route("/fal-ai/sam-3-1/requests/figure-fixture/status",get(||async{Json(serde_json::json!({"status":"COMPLETED"}))}))
            .route("/fal-ai/sam-3-1/requests/figure-fixture",get(move||{let result=result.clone();async move{Json(result)}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let costs = MeteringPort::new();
        costs.open("figure-mask-test",r#"{"schema":"dungeonflux.pricing/1","currency":"USD","prices":[{"provider":"fal","model":"fal-ai/sam-3-1/image-rle","category":"calls","usd":"0.01","per":1,"as_of":"2026-10-24","source":"https://fal.ai/models/fal-ai/sam-3-1/image-rle"}]}"#).unwrap();
        let mut patch = df_imagery::RuntimePatch::empty();
        patch.mode = Some(df_imagery::GlobalMode::Live);
        patch.analysis_segmentation = Some(
            df_imagery::SegmentationRoute {
                provider: df_imagery::SegmentationProvider::Fal,
                account: df_imagery::AccountHandle::new("fixture-segmentation").unwrap(),
                model: df_imagery::ModelName::new("fal-ai/sam-3-1/image-rle").unwrap(),
                endpoint: df_imagery::ApiEndpoint::new(&format!("http://{address}")).unwrap(),
            }
            .into(),
        );
        let Some(mut fixture) = ImageHostFixture::open(address, patch, costs.clone()).await else {
            stop.send(()).unwrap();
            server.await.unwrap();
            return;
        };
        let mut picture = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(40, 20)
            .write_to(&mut picture, image::ImageFormat::Png)
            .unwrap();
        let (key, _) = fixture
            .import_bytes(df_providers::ImageFormat::Png, picture.into_inner().into())
            .await
            .unwrap();
        let imagery = fixture.imagery().clone();
        assert!(imagery.focus_mask(key.as_str(), 0).await.is_none());
        assert_eq!(
            submits.load(Ordering::SeqCst),
            0,
            "HTTP reads never admit analysis"
        );
        let heads = [head("npc-0", 0.665, 0.15, 0.87, 0.4)];
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
        let (figures, scan) = tokio::join!(
            imagery.figure_geometry(&key, &heads, &staged, 1, true),
            imagery.plate_masks(&key)
        );
        assert!(scan.is_ok());
        let figures = figures.unwrap();
        assert!(
            figures
                .iter()
                .any(|figure| figure.id == "hero-1" && figure.inferred)
        );
        let figure = figures.iter().find(|figure| figure.id == "npc-0").unwrap();
        assert_eq!(figure.bounds, [26, 3, 35, 19]);
        assert!(!figure.inferred);
        assert_eq!(
            figure.mask_url,
            format!("{}/figure/{}", key.path(), figure.mask_index)
        );
        let first = imagery
            .focus_mask(key.as_str(), figure.mask_index)
            .await
            .unwrap();
        let png = image::load_from_memory(&first).unwrap().to_rgba8();
        assert_eq!(png.dimensions(), (40, 20));
        assert_eq!(png.get_pixel(30, 10).0, [255, 255, 255, 255]);
        assert_eq!(png.get_pixel(0, 0)[3], 0);
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        let grant = crate::gameplay::media_grant::sign(
            key.as_str(),
            crate::gameplay::media_grant::Audience::Imagery,
            crate::gameplay::media_grant::PrincipalAudience::Display,
            &fixture.display_credential(),
            expiry,
        )
        .unwrap();
        let uri: axum::http::Uri = format!("/?g={grant}").parse().unwrap();
        let response = super::super::serve_focus_mask(
            axum::extract::State(imagery.clone()),
            axum::extract::Path((key.as_str().to_owned(), figure.mask_index)),
            axum::http::HeaderMap::new(),
            uri.clone(),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::REFERRER_POLICY],
            "no-referrer"
        );
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "image/png"
        );
        let cache_control = response.headers()[axum::http::header::CACHE_CONTROL]
            .to_str()
            .unwrap();
        let maximum_age = cache_control
            .strip_prefix("private, max-age=")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!((1..=60).contains(&maximum_age));
        let etag = response.headers()[axum::http::header::ETAG].clone();
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
            first
        );
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, etag);
        let response = super::super::serve_focus_mask(
            axum::extract::State(imagery.clone()),
            axum::extract::Path((key.as_str().to_owned(), figure.mask_index)),
            headers,
            uri,
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::NOT_MODIFIED);
        assert!(imagery.focus_mask(key.as_str(), 999).await.is_none());
        assert!(imagery.focus_mask("../invalid", 0).await.is_none());
        assert_eq!(submits.load(Ordering::SeqCst), 1);
        assert_eq!(costs.totals().calls, 1);
        assert_eq!(costs.totals().micro_usd, 10_000);
        imagery
            .write_presentation_annotation(
                &key,
                PresentationAnnotation::Figures,
                b"corrupt projection".to_vec(),
            )
            .await
            .unwrap();
        assert!(
            imagery
                .focus_mask(key.as_str(), figure.mask_index)
                .await
                .is_none()
        );
        assert_eq!(
            submits.load(Ordering::SeqCst),
            1,
            "corrupt projection cannot authorize another supplier call"
        );
        imagery
            .figure_geometry(&key, &heads, &staged, 1, true)
            .await
            .unwrap();
        drop(imagery);
        fixture.restart().await;
        let imagery = fixture.imagery().clone();
        assert_eq!(
            imagery
                .focus_mask(key.as_str(), figure.mask_index)
                .await
                .unwrap(),
            first
        );
        assert_eq!(
            imagery
                .figure_geometry(&key, &heads, &staged, 1, true)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            submits.load(Ordering::SeqCst),
            1,
            "actual restart reuses durable analysis"
        );
        assert_eq!(
            costs.totals().calls,
            1,
            "replay/derivatives never duplicate supplier accounting"
        );
        drop(imagery);
        fixture.close().await;
        stop.send(()).unwrap();
        server.await.unwrap();
    }
}
