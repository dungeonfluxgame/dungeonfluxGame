//! One segmentation of one picture through the owner's local Forge studio (SAM 3.1), with the
//! same request, masks and errors as the fal door.
//!
//! Forge saves a segmentation as one PNG of the kept matches, not one mask per match, and it keeps
//! the `top` k matches by score. So the scan stages the picture once and asks for the top 1, which
//! also says how many matches there are, then the top 2, 3 and so on; match k is the pixels of the
//! top k that are not in the top k - 1 (where two matches overlap, the better one keeps the
//! pixels). SAM's inference on one staged picture is deterministic, so the rankings agree. It is
//! one Forge job per instance, each a couple of seconds once the model is loaded, and the scan asks
//! for at most as many as Forge reports scores for (20).
//!
//! A picture with no match fails in Forge with the engine's own answer, so it is a `Failure`, as
//! a fal refusal is. Nothing is retried; a job Forge reports as interrupted is `Unknown` and stays
//! for Forge's Recovery panel.

use std::time::{Duration, Instant};

use df_providers::{
    ForgeCause, ForgeClient, ForgeEngine, ForgeError, ForgeJobKind, ForgeMedia, NativeHttpCause,
    SendState,
};
use df_types::saturating_millis;

use super::{
    AnalysisMask, AnalysisMaskRun, AnalysisMaskSet, AnalysisUnit, MAXIMUM_MASKS, MAXIMUM_TIMEOUT,
    SegmentCause, SegmentError, SegmentPhases, SegmentRequest, Segmented,
};

/// The match confidence Forge keeps a match above (its default, which its own scans use).
const CONFIDENCE: f64 = 0.5;
/// A mask pixel above this grey level is the match (Forge writes white on black).
const MASK_THRESHOLD: u8 = 127;

/// Scans `request.source` through `client` within `timeout` and answers the masks at the picture's
/// size with the time each phase took: staging the picture, the first job, and the rest.
pub async fn segment_with_forge(
    client: &ForgeClient,
    request: SegmentRequest,
    timeout: Duration,
) -> Result<Segmented, SegmentError> {
    if !(1..=MAXIMUM_MASKS).contains(&request.maximum_masks)
        || timeout.is_zero()
        || timeout > MAXIMUM_TIMEOUT
    {
        return Err(SegmentError::unsent(SegmentCause::Configuration));
    }
    let until = tokio::time::Instant::now() + timeout;
    let started = Instant::now();
    let unsent = |error: ForgeError| SegmentError {
        cause: cause_of(error.cause),
        send_state: SendState::Unsent,
    };
    client
        .prepare(ForgeEngine::Segmentation, until)
        .await
        .map_err(unsent)?;
    let reference = client
        .stage_reference(&request.source, until)
        .await
        .map_err(unsent)?;
    let prepare_ms = saturating_millis(started.elapsed());
    let sent = |error: ForgeError| SegmentError {
        cause: cause_of(error.cause),
        send_state: SendState::KnownSent,
    };
    let job = |top: u32| {
        serde_json::json!({
            "prompt": request.prompt.key(),
            "references": [reference.as_str()],
            "output": "mask",
            "confidence": CONFIDENCE,
            "select": "top",
            "max_instances": top,
        })
    };
    let started = Instant::now();
    let first = client
        .render(ForgeJobKind::Segmentation, job(1), until)
        .await
        .map_err(sent)?;
    let malformed = SegmentError {
        cause: SegmentCause::Malformed,
        send_state: SendState::KnownSent,
    };
    let matches = first.matches.ok_or(malformed)?;
    let wanted = request
        .maximum_masks
        .min(matches.found)
        .min(u32::try_from(matches.scores.len()).unwrap_or(u32::MAX));
    let mut previous = mask_pixels(
        client
            .asset(&first.output, ForgeMedia::Png, until)
            .await
            .map_err(sent)?,
    )
    .await
    .ok_or(malformed)?;
    let submit_ms = saturating_millis(started.elapsed());
    let started = Instant::now();
    let mut instances = vec![instance(0, &matches.scores, None, &previous).ok_or(malformed)?];
    for top in 2..=wanted {
        let render = client
            .render(ForgeJobKind::Segmentation, job(top), until)
            .await
            .map_err(sent)?;
        let current = mask_pixels(
            client
                .asset(&render.output, ForgeMedia::Png, until)
                .await
                .map_err(sent)?,
        )
        .await
        .ok_or(malformed)?;
        if (current.width, current.height) != (previous.width, previous.height) {
            return Err(malformed);
        }
        instances
            .push(instance(top - 1, &matches.scores, Some(&previous), &current).ok_or(malformed)?);
        previous = current;
    }
    let masks = AnalysisMaskSet {
        prompt: request.prompt,
        width_px: previous.width,
        height_px: previous.height,
        instances,
    };
    masks.validate().map_err(|_| malformed)?;
    Ok(Segmented {
        masks,
        phases: SegmentPhases {
            prepare_ms,
            submit_ms,
            resume_ms: saturating_millis(started.elapsed()),
        },
    })
}

/// A decoded mask: which pixels, row by row, are the matches.
struct MaskPixels {
    width: u32,
    height: u32,
    on: Vec<bool>,
}

/// Decodes a mask PNG on the blocking pool; `None` for one that does not read.
async fn mask_pixels(png: Vec<u8>) -> Option<MaskPixels> {
    tokio::task::spawn_blocking(move || {
        let grey = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .ok()?
            .to_luma8();
        let (width, height) = grey.dimensions();
        let on = grey
            .into_raw()
            .into_iter()
            .map(|level| level > MASK_THRESHOLD)
            .collect();
        Some(MaskPixels { width, height, on })
    })
    .await
    .ok()
    .flatten()
}

/// Match `index` (zero-based, best first): the pixels of `current` not in `previous`, as runs.
fn instance(
    index: u32,
    scores: &[f32],
    previous: Option<&MaskPixels>,
    current: &MaskPixels,
) -> Option<AnalysisMask> {
    let score = AnalysisUnit::new(*scores.get(usize::try_from(index).ok()?)?).ok()?;
    let mut runs: Vec<AnalysisMaskRun> = Vec::new();
    for (at, &on) in current.on.iter().enumerate() {
        let taken = previous.is_some_and(|previous| previous.on.get(at) == Some(&true));
        if !on || taken {
            continue;
        }
        let at = u32::try_from(at).ok()?;
        match runs.last_mut() {
            Some(run) if run.start + run.length == at => run.length += 1,
            _ => runs.push(AnalysisMaskRun {
                start: at,
                length: 1,
            }),
        }
    }
    Some(AnalysisMask { index, score, runs })
}

/// The scan's cause of a Forge failure.
fn cause_of(cause: ForgeCause) -> SegmentCause {
    match cause {
        ForgeCause::InvalidConfiguration => SegmentCause::Configuration,
        ForgeCause::InvalidRequest => SegmentCause::Picture,
        ForgeCause::Rejected
        | ForgeCause::EngineUnavailable
        | ForgeCause::JobNeedsAttention
        | ForgeCause::QueueWaitingMemory
        | ForgeCause::QueueInterrupted
        | ForgeCause::QueueNeedsAttention => SegmentCause::Refused,
        ForgeCause::Transport(NativeHttpCause::Deadline) | ForgeCause::Deadline => {
            SegmentCause::Expired
        }
        ForgeCause::Transport(NativeHttpCause::Cancelled) | ForgeCause::JobCancelled => {
            SegmentCause::Cancelled
        }
        ForgeCause::JobInterrupted => SegmentCause::Unknown,
        ForgeCause::Malformed => SegmentCause::Malformed,
        ForgeCause::Transport(_)
        | ForgeCause::Failed
        | ForgeCause::JobFailed
        | ForgeCause::Session => SegmentCause::Failure,
    }
}

#[cfg(test)]
mod forge_tests;
