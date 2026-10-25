//! Hosted SAM segmentation of a scene plate on fal (PLAY-V09, `planning/sam-3-1-research.md`).
//!
//! One image and one short noun phrase (`person`, `head`) go to fal's queue endpoint
//! `fal-ai/sam-3-1/image-rle`; every instance of that phrase comes back as a run-length mask with
//! a score. The adapter owns the whole queue exchange: one submit, status polls until the request
//! completes, one result read. It never resends, never follows a redirect, accepts only status
//! and result URLs that name this request on fal's queue host, and never logs or places the
//! picture, the prompt or the credential in an error.
//!
//! The mask is as large as the picture that was sent (the reply carries no size), so the caller
//! passes the picture's width and height and the adapter checks every run against them.
//!
//! The call has a price, not a cap, and the price is not kept here: the caller's cost book reads it
//! from the price file by [`SegmentModel::endpoint`]. A call that was sent is charged whatever its
//! outcome ([`FalSegmentError::was_sent`]).

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::native_http::NativeHttp;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{HttpMethod, HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const QUEUE_ORIGIN: &str = "https://queue.fal.run";
const MAXIMUM_PICTURE_BYTES: usize = 3 * 1024 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_PROMPT_CHARACTERS: usize = 48;
/// fal accepts 1 to 32 masks per call.
pub const MAXIMUM_MASKS: u8 = 32;
/// The largest picture side the adapter accepts; keeps every pixel index inside a `u32`.
const MAXIMUM_SIDE: u32 = 8192;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(400);

/// The hosted segmentation models the adapter knows. Each is one fal endpoint with one price.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentModel {
    /// `fal-ai/sam-3-1/image-rle`: the owner's choice.
    Sam31,
    /// `fal-ai/sam-3/image-rle`: the same stills at half the price, kept for comparison runs.
    /// Prices live in `content/pricing.json`, read by the cost book; this client holds none.
    Sam3,
}

impl SegmentModel {
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::Sam31 => "fal-ai/sam-3-1/image-rle",
            Self::Sam3 => "fal-ai/sam-3/image-rle",
        }
    }

    /// A short stable name for audit lines and cache keys.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sam31 => "sam-3.1",
            Self::Sam3 => "sam-3",
        }
    }
}

/// A box in whole pixels: `left` and `top` inclusive, `right` and `bottom` exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelBox {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// One run of a mask: `length` set pixels from the flat row-major index `start`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaskRun {
    pub start: u32,
    pub length: u32,
}

/// Why a mask was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskError {
    /// The picture size is zero or beyond the adapter's bound.
    Size,
    /// The text is not whole `start length` pairs of unsigned integers.
    Syntax,
    /// A run is empty, runs past the picture, or does not start after the previous run ends.
    Runs,
    /// More runs than a picture of this height can plausibly need.
    TooManyRuns,
}

/// A segmentation mask as the sorted, disjoint runs of its set pixels over a picture of `width`
/// by `height`. fal's `rle` strings are `start length` pairs over the flattened picture in row
/// order, with **1-based** starts (the run-length convention of Kaggle masks): the pixel index is
/// `y * width + x`, a run's first pixel is `start - 1`. Measured on fal's 2560 by 1440 answers
/// (2026-10-17): read 0-based, 510 runs of one person end one pixel into the next row; read
/// 1-based, none does and each run ends at its row's edge. [`MaskRows::runs`] holds 0-based starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaskRows {
    width: u32,
    height: u32,
    runs: Vec<MaskRun>,
}

impl MaskRows {
    /// Reads fal's `start length` text (1-based starts). Runs must be ascending and disjoint and
    /// lie inside the picture; an empty text is an empty mask.
    pub fn parse(text: &str, width: u32, height: u32) -> Result<Self, MaskError> {
        let mut numbers = Vec::new();
        for token in text.split_ascii_whitespace() {
            numbers.push(token.parse::<u32>().map_err(|_| MaskError::Syntax)?);
        }
        if !numbers.len().is_multiple_of(2) {
            return Err(MaskError::Syntax);
        }
        let runs = numbers
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[start, length]| {
                Ok(MaskRun {
                    start: start.checked_sub(1).ok_or(MaskError::Runs)?,
                    length: *length,
                })
            })
            .collect::<Result<Vec<_>, MaskError>>()?;
        Self::from_runs(width, height, runs)
    }

    /// A mask from its runs (0-based starts), under the same rules as [`MaskRows::parse`].
    pub fn from_runs(width: u32, height: u32, runs: Vec<MaskRun>) -> Result<Self, MaskError> {
        if width == 0 || height == 0 || width > MAXIMUM_SIDE || height > MAXIMUM_SIDE {
            return Err(MaskError::Size);
        }
        // A person needs a couple of runs per row; this is far above any real mask.
        if runs.len() > height as usize * 64 {
            return Err(MaskError::TooManyRuns);
        }
        let total = u64::from(width) * u64::from(height);
        let mut next_free = 0u64;
        for run in &runs {
            let start = u64::from(run.start);
            let end = start + u64::from(run.length);
            if run.length == 0 || start < next_free || end > total {
                return Err(MaskError::Runs);
            }
            next_free = end;
        }
        Ok(Self {
            width,
            height,
            runs,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn runs(&self) -> &[MaskRun] {
        &self.runs
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// How many pixels are set.
    pub fn pixel_count(&self) -> u64 {
        self.runs.iter().map(|run| u64::from(run.length)).sum()
    }

    /// The set pixels row by row as `(y, x_first, x_end)`: a run that crosses a row boundary
    /// is split, so each item lies on one row. `x_end` is exclusive.
    pub fn row_segments(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        let width = u64::from(self.width);
        self.runs.iter().flat_map(move |run| {
            let first = u64::from(run.start);
            let end = first + u64::from(run.length);
            let first_row = first / width;
            let last_row = (end - 1) / width;
            (first_row..=last_row).map(move |row| {
                let row_start = row * width;
                let from = first.max(row_start) - row_start;
                let to = end.min(row_start + width) - row_start;
                (row as u32, from as u32, to as u32)
            })
        })
    }

    /// The tight box of the set pixels, `None` for an empty mask.
    pub fn bounds(&self) -> Option<PixelBox> {
        let mut found: Option<PixelBox> = None;
        for (y, from, to) in self.row_segments() {
            found = Some(match found {
                None => PixelBox {
                    left: from,
                    top: y,
                    right: to,
                    bottom: y + 1,
                },
                Some(known) => PixelBox {
                    left: known.left.min(from),
                    top: known.top,
                    right: known.right.max(to),
                    bottom: y + 1,
                },
            });
        }
        found
    }

    /// How many pixels both masks set. Both must cover the same picture size.
    pub fn overlap(&self, other: &Self) -> u64 {
        if self.width != other.width || self.height != other.height {
            return 0;
        }
        let (mut mine, mut theirs) = (self.runs.iter().peekable(), other.runs.iter().peekable());
        let mut shared = 0u64;
        while let (Some(a), Some(b)) = (mine.peek(), theirs.peek()) {
            let (a_start, b_start) = (u64::from(a.start), u64::from(b.start));
            let (a_end, b_end) = (a_start + u64::from(a.length), b_start + u64::from(b.length));
            shared += a_end.min(b_end).saturating_sub(a_start.max(b_start));
            if a_end <= b_end {
                mine.next();
            } else {
                theirs.next();
            }
        }
        shared
    }

    /// The mean column of the set pixels in the topmost `rows` rows of the mask, with that top
    /// row. For a head this is the point just above the crown. `None` for an empty mask.
    pub fn top_centre(&self, rows: u32) -> Option<(f32, u32)> {
        let top = self.bounds()?.top;
        let (mut sum, mut count) = (0u64, 0u64);
        for (y, from, to) in self.row_segments() {
            if y >= top + rows.max(1) {
                break;
            }
            let length = u64::from(to - from);
            // Sum of from..to.
            sum += (u64::from(from) + u64::from(to) - 1) * length / 2;
            count += length;
        }
        (count > 0).then(|| (sum as f32 / count as f32 + 0.5, top))
    }
}

/// What one call asks: the JPEG picture with its size, one noun phrase and the most masks wanted.
#[derive(Clone, Copy, Debug)]
pub struct SegmentRequest<'a> {
    pub jpeg: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// A short plain noun phrase such as `person` or `head`; fixed by the server, never player text.
    pub prompt: &'a str,
    /// 1 to [`MAXIMUM_MASKS`].
    pub max_masks: u8,
}

/// One instance fal found.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentInstance {
    /// fal's own index of the instance (answers come in score order).
    pub index: u32,
    pub score: f32,
    pub mask: MaskRows,
}

/// A completed call. Zero instances is a successful answer.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentAnswer {
    pub instances: Vec<SegmentInstance>,
    pub model: SegmentModel,
    /// fal's queue request id: a safe identifier for audit lines.
    pub request_id: String,
    /// Submit to result, end to end.
    pub elapsed: Duration,
    /// Status reads before the request completed.
    pub polls: u32,
}

/// Why no answer came back. Safe to log: never carries the picture, prompt or credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalSegmentError {
    /// Failed local validation; nothing was sent and nothing is charged.
    InvalidRequest,
    Unauthorized,
    /// fal answered this HTTP status (a refused body is 422).
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// The queue reply named no usable request id or URLs.
    Queue,
    /// fal reported the request failed.
    Failed,
    /// The answer was not a well-formed segmentation result.
    Output,
}

impl FalSegmentError {
    /// Whether the request may have reached fal, so the call is charged whatever happened.
    pub const fn was_sent(self) -> bool {
        !matches!(self, Self::InvalidRequest)
    }

    /// Whether asking again later can succeed: a timeout, a transport fault, a rate limit or a
    /// server fault. A refused body, a bad key and a malformed answer are permanent.
    pub const fn is_transient(self) -> bool {
        match self {
            Self::Transport(refusal) => matches!(
                refusal,
                NativeHttpRefusal::Deadline | NativeHttpRefusal::Unknown
            ),
            Self::Provider(status) => status == 429 || status >= 500,
            Self::Failed => true,
            Self::InvalidRequest | Self::Unauthorized | Self::Queue | Self::Output => false,
        }
    }

    /// A short stable label for audit lines.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::Provider(_) => "provider_status",
            Self::Transport(NativeHttpRefusal::Deadline) => "deadline",
            Self::Transport(_) => "transport",
            Self::Queue => "queue_identity",
            Self::Failed => "provider_failed",
            Self::Output => "bad_output",
        }
    }
}

/// Where the adapter sends calls.
pub struct FalSegmentConfig {
    model: SegmentModel,
    fixture: Option<SocketAddr>,
    poll_interval: Duration,
}

impl FalSegmentConfig {
    pub fn live(model: SegmentModel) -> Self {
        Self {
            model,
            fixture: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// A literal loopback socket for tests, polled every `poll_interval`.
    pub fn loopback_fixture(
        model: SegmentModel,
        address: SocketAddr,
        poll_interval: Duration,
    ) -> Result<Self, FalSegmentError> {
        if !address.ip().is_loopback() || address.port() == 0 || poll_interval.is_zero() {
            return Err(FalSegmentError::InvalidRequest);
        }
        Ok(Self {
            model,
            fixture: Some(address),
            poll_interval,
        })
    }
}

pub struct NativeFalSegmenter {
    model: SegmentModel,
    origin: String,
    credential: String,
    poll_interval: Duration,
    http: NativeHttp,
}

impl std::fmt::Debug for NativeFalSegmenter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeFalSegmenter")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl NativeFalSegmenter {
    /// `total_timeout` bounds each HTTP exchange of a call; [`NativeFalSegmenter::segment`]'s own
    /// deadline bounds the whole call.
    pub fn new(
        config: FalSegmentConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, FalSegmentError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(FalSegmentError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: MAXIMUM_PICTURE_BYTES * 2,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 4096,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| FalSegmentError::InvalidRequest)?;
        Ok(Self {
            model: config.model,
            origin: config.fixture.map_or_else(
                || QUEUE_ORIGIN.to_owned(),
                |address| format!("http://{address}"),
            ),
            credential: credential.to_owned(),
            poll_interval: config.poll_interval,
            http,
        })
    }

    pub fn model(&self) -> SegmentModel {
        self.model
    }

    /// Segments `request.prompt` in the picture. One submit, then status reads until fal reports
    /// completion or `deadline` passes, then one result read.
    pub async fn segment(
        &self,
        request: &SegmentRequest<'_>,
        deadline: Duration,
    ) -> Result<SegmentAnswer, FalSegmentError> {
        let body = request_body(request)?;
        let started = Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());

        let submitted = self
            .exchange(
                HttpMethod::Post,
                &format!("/{}", self.model.endpoint()),
                body,
                remaining(),
            )
            .await?;
        let queue = parse_queue_reply(&submitted)?;

        let mut polls = 0u32;
        loop {
            let status = self
                .exchange(HttpMethod::Get, &queue.status_path, Vec::new(), remaining())
                .await?;
            polls += 1;
            match read_status(&status)? {
                QueueStatus::Completed => break,
                QueueStatus::Waiting => {
                    let pause = self.poll_interval.min(remaining());
                    if pause.is_zero() {
                        return Err(FalSegmentError::Transport(NativeHttpRefusal::Deadline));
                    }
                    tokio::time::sleep(pause).await;
                }
            }
        }

        let result = self
            .exchange(HttpMethod::Get, &queue.result_path, Vec::new(), remaining())
            .await?;
        let instances = parse_instances(
            &json_body(&result)?,
            request.width,
            request.height,
            usize::from(request.max_masks),
        )?;
        Ok(SegmentAnswer {
            instances,
            model: self.model,
            request_id: queue.request_id,
            elapsed: started.elapsed(),
            polls,
        })
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        path: &str,
        body: Vec<u8>,
        remaining: Duration,
    ) -> Result<crate::native_http::NativeHttpResponse, FalSegmentError> {
        let request = HttpRequest::fal_native(
            method,
            &format!("{}{path}", self.origin),
            &self.credential,
            body,
        );
        let response = self
            .http
            .execute(request, remaining, std::future::pending())
            .await
            .map_err(|failure| FalSegmentError::Transport(failure.reason))?;
        match response.status {
            200 | 202 => Ok(response),
            401 | 403 => Err(FalSegmentError::Unauthorized),
            status => Err(FalSegmentError::Provider(status)),
        }
    }
}

fn request_body(request: &SegmentRequest<'_>) -> Result<Vec<u8>, FalSegmentError> {
    let prompt = request.prompt.trim();
    if request.jpeg.is_empty()
        || request.jpeg.len() > MAXIMUM_PICTURE_BYTES
        || request.width == 0
        || request.height == 0
        || request.width > MAXIMUM_SIDE
        || request.height > MAXIMUM_SIDE
        || prompt.is_empty()
        || prompt.chars().count() > MAXIMUM_PROMPT_CHARACTERS
        || !(1..=MAXIMUM_MASKS).contains(&request.max_masks)
    {
        return Err(FalSegmentError::InvalidRequest);
    }
    serde_json::to_vec(&json!({
        "image_url": format!("data:image/jpeg;base64,{}", STANDARD.encode(request.jpeg)),
        "prompt": prompt,
        "include_scores": true,
        "include_boxes": true,
        "return_multiple_masks": true,
        "max_masks": request.max_masks,
    }))
    .map_err(|_| FalSegmentError::InvalidRequest)
}

fn json_body(response: &crate::native_http::NativeHttpResponse) -> Result<Value, FalSegmentError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_depth: 16,
            maximum_nodes: 262_144,
        },
    )
    .map_err(|_| FalSegmentError::Output)
}

struct QueueReply {
    request_id: String,
    status_path: String,
    result_path: String,
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// The path of `url` on fal's queue host when it names request `id` (`…/requests/<id>` then
/// nothing or `suffix`). Any query, fragment, traversal or other host is refused. fal names a
/// request under its app (`fal-ai/sam-3-1/requests/<id>/status` for `fal-ai/sam-3-1/image-rle`).
fn confined_path<'a>(url: &'a str, id: &str, suffix: Option<&str>) -> Option<&'a str> {
    let tail = url.strip_prefix(QUEUE_ORIGIN)?.strip_prefix('/')?;
    if tail.contains(['?', '#']) || tail.contains("..") {
        return None;
    }
    let needle = format!("/requests/{id}");
    let position = tail.rfind(&needle)?;
    let (app, rest) = (&tail[..position], &tail[position + needle.len()..]);
    let app_ok = app.split('/').count() >= 2 && app.split('/').all(|part| !part.is_empty());
    let rest_ok = match suffix {
        Some(expected) => rest == format!("/{expected}"),
        None => rest.is_empty() || rest == "/response",
    };
    (app_ok && rest_ok)
        .then(|| url.strip_prefix(QUEUE_ORIGIN))
        .flatten()
}

fn parse_queue_reply(
    response: &crate::native_http::NativeHttpResponse,
) -> Result<QueueReply, FalSegmentError> {
    let value = json_body(response).map_err(|_| FalSegmentError::Queue)?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| valid_request_id(id))
        .ok_or(FalSegmentError::Queue)?;
    let path = |field: &str, suffix: Option<&str>| {
        value
            .get(field)
            .and_then(Value::as_str)
            .and_then(|url| confined_path(url, id, suffix))
            .map(str::to_owned)
            .ok_or(FalSegmentError::Queue)
    };
    Ok(QueueReply {
        request_id: id.to_owned(),
        status_path: path("status_url", Some("status"))?,
        result_path: path("response_url", None)?,
    })
}

enum QueueStatus {
    Waiting,
    Completed,
}

fn read_status(
    response: &crate::native_http::NativeHttpResponse,
) -> Result<QueueStatus, FalSegmentError> {
    let value = json_body(response)?;
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(FalSegmentError::Failed);
    }
    match value.get("status").and_then(Value::as_str) {
        Some("IN_QUEUE" | "IN_PROGRESS") => Ok(QueueStatus::Waiting),
        Some("COMPLETED") => Ok(QueueStatus::Completed),
        Some("FAILED" | "ERROR" | "CANCELLED") => Err(FalSegmentError::Failed),
        _ => Err(FalSegmentError::Output),
    }
}

/// The instances of a result: `rle` strings with `scores` (and fal's `metadata` carrying each
/// instance's own index). Empty masks are dropped; more instances than were asked for, a count
/// mismatch or a mask that does not fit the picture refuses the whole answer.
fn parse_instances(
    value: &Value,
    width: u32,
    height: u32,
    maximum: usize,
) -> Result<Vec<SegmentInstance>, FalSegmentError> {
    let masks = value
        .get("rle")
        .and_then(Value::as_array)
        .ok_or(FalSegmentError::Output)?;
    let scores = value
        .get("scores")
        .and_then(Value::as_array)
        .ok_or(FalSegmentError::Output)?;
    if masks.len() > maximum || scores.len() != masks.len() {
        return Err(FalSegmentError::Output);
    }
    let metadata = value.get("metadata").and_then(Value::as_array);
    if metadata.is_some_and(|rows| rows.len() != masks.len()) {
        return Err(FalSegmentError::Output);
    }
    let mut instances = Vec::with_capacity(masks.len());
    for (position, (mask, score)) in masks.iter().zip(scores).enumerate() {
        let score = score
            .as_f64()
            .filter(|score| (0.0..=1.0).contains(score))
            .ok_or(FalSegmentError::Output)? as f32;
        let mask = MaskRows::parse(mask.as_str().ok_or(FalSegmentError::Output)?, width, height)
            .map_err(|_| FalSegmentError::Output)?;
        if mask.is_empty() {
            continue;
        }
        let index = metadata
            .and_then(|rows| rows.get(position))
            .and_then(|row| row.get("index"))
            .and_then(Value::as_u64)
            .map_or(position as u32, |index| index as u32);
        instances.push(SegmentInstance { index, score, mask });
    }
    Ok(instances)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(width: u32, left: u32, top: u32, right: u32, bottom: u32) -> Vec<MaskRun> {
        (top..bottom)
            .map(|y| MaskRun {
                start: y * width + left,
                length: right - left,
            })
            .collect()
    }

    fn mask(width: u32, height: u32, box_: (u32, u32, u32, u32)) -> MaskRows {
        MaskRows::from_runs(
            width,
            height,
            rectangle(width, box_.0, box_.1, box_.2, box_.3),
        )
        .unwrap()
    }

    #[test]
    fn a_mask_text_decodes_to_runs_whose_pixels_and_bounds_are_exact() {
        // Width 10, 1-based starts: row 1 columns 2..5, then row 2 columns 3..6.
        let decoded = MaskRows::parse("13 3 24 3", 10, 5).unwrap();
        assert_eq!(decoded.pixel_count(), 6);
        assert_eq!(
            decoded.bounds(),
            Some(PixelBox {
                left: 2,
                top: 1,
                right: 6,
                bottom: 3
            })
        );
        assert_eq!(
            decoded.row_segments().collect::<Vec<_>>(),
            vec![(1, 2, 5), (2, 3, 6)]
        );
        assert!(MaskRows::parse("", 10, 5).unwrap().is_empty());
    }

    #[test]
    fn a_run_that_crosses_rows_is_split_and_bounds_span_the_full_width() {
        // 7 pixels from (row 0, column 3) cross into rows 1 and 2 of a width-4 picture.
        let decoded = MaskRows::parse("4 7", 4, 4).unwrap();
        assert_eq!(
            decoded.row_segments().collect::<Vec<_>>(),
            vec![(0, 3, 4), (1, 0, 4), (2, 0, 2)]
        );
        assert_eq!(
            decoded.bounds(),
            Some(PixelBox {
                left: 0,
                top: 0,
                right: 4,
                bottom: 3
            })
        );
    }

    #[test]
    fn malformed_masks_are_refused() {
        for (text, expected) in [
            ("1", MaskError::Syntax),
            ("1 2 3", MaskError::Syntax),
            ("a 2", MaskError::Syntax),
            ("-1 2", MaskError::Syntax),
            ("0 0", MaskError::Runs),
            ("0 1", MaskError::Runs),
            ("46 6", MaskError::Runs),
            ("10 5 12 3", MaskError::Runs),
            ("20 5 10 3", MaskError::Runs),
        ] {
            assert_eq!(MaskRows::parse(text, 10, 5), Err(expected), "{text}");
        }
        assert_eq!(MaskRows::parse("1 1", 0, 5), Err(MaskError::Size));
        assert_eq!(MaskRows::parse("1 1", 10, 9000), Err(MaskError::Size));
    }

    #[test]
    fn overlap_counts_the_pixels_two_masks_share() {
        let a = mask(20, 20, (2, 2, 10, 10));
        let b = mask(20, 20, (6, 6, 14, 14));
        assert_eq!(a.overlap(&b), 16);
        assert_eq!(b.overlap(&a), 16);
        assert_eq!(a.overlap(&a), a.pixel_count());
        assert_eq!(a.overlap(&mask(20, 20, (12, 12, 18, 18))), 0);
        assert_eq!(a.overlap(&mask(21, 20, (2, 2, 10, 10))), 0);
    }

    #[test]
    fn the_top_centre_is_the_mean_column_of_the_topmost_rows() {
        // A crown narrower than the shoulders: rows 3-4 columns 8..12, rows 5-9 columns 2..18.
        let mut runs = rectangle(30, 8, 3, 12, 5);
        runs.extend(rectangle(30, 2, 5, 18, 10));
        let head = MaskRows::from_runs(30, 12, runs).unwrap();
        let (x, y) = head.top_centre(2).unwrap();
        assert_eq!(y, 3);
        assert!((x - 10.0).abs() < 0.01, "{x}");
        assert!(MaskRows::parse("", 30, 12).unwrap().top_centre(2).is_none());
    }

    #[test]
    fn queue_urls_must_name_this_request_on_the_queue_host() {
        let id = "req_1";
        assert_eq!(
            confined_path(
                "https://queue.fal.run/fal-ai/sam-3-1/requests/req_1/status",
                id,
                Some("status")
            ),
            Some("/fal-ai/sam-3-1/requests/req_1/status")
        );
        assert!(
            confined_path(
                "https://queue.fal.run/fal-ai/sam-3-1/requests/req_1",
                id,
                None
            )
            .is_some()
        );
        for bad in [
            "https://evil.example/fal-ai/sam-3-1/requests/req_1/status",
            "http://queue.fal.run/fal-ai/sam-3-1/requests/req_1/status",
            "https://queue.fal.run/fal-ai/sam-3-1/requests/other/status",
            "https://queue.fal.run/fal-ai/sam-3-1/requests/req_1/status?x=1",
            "https://queue.fal.run/fal-ai/../requests/req_1/status",
            "https://queue.fal.run/requests/req_1/status",
            "https://queue.fal.run/fal-ai/sam-3-1/requests/req_1/cancel",
        ] {
            assert!(confined_path(bad, id, Some("status")).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_result_becomes_instances_with_fals_own_indices_and_empty_masks_dropped() {
        let value = json!({
            "rle": ["13 3 24 3", "", "1 4"],
            "scores": [0.96, 0.5, 0.87],
            "boxes": [],
            "metadata": [{"index": 2}, {"index": 0}, {"index": 1}],
        });
        let found = parse_instances(&value, 10, 5, 8).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].index, found[0].score), (2, 0.96));
        assert_eq!((found[1].index, found[1].mask.pixel_count()), (1, 4));
        let none = json!({"rle": [], "scores": []});
        assert!(parse_instances(&none, 10, 5, 8).unwrap().is_empty());
    }

    #[test]
    fn a_malformed_result_is_refused_whole() {
        for bad in [
            json!({"scores": [0.9]}),
            json!({"rle": ["1 1"]}),
            json!({"rle": ["1 1"], "scores": [0.9, 0.8]}),
            json!({"rle": ["1 1"], "scores": [1.5]}),
            json!({"rle": ["1 1"], "scores": ["x"]}),
            json!({"rle": ["1 99"], "scores": [0.9]}),
            json!({"rle": ["0 1"], "scores": [0.9]}),
            json!({"rle": [7], "scores": [0.9]}),
            json!({"rle": ["1 1", "3 1"], "scores": [0.9, 0.8]}),
            json!({"rle": ["1 1"], "scores": [0.9], "metadata": []}),
        ] {
            let maximum = if bad["rle"].as_array().is_some_and(|rows| rows.len() == 2) {
                1
            } else {
                8
            };
            assert_eq!(
                parse_instances(&bad, 10, 5, maximum),
                Err(FalSegmentError::Output),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_request_carries_the_picture_and_refuses_bad_input_before_anything_is_sent() {
        let ask = |jpeg: &'static [u8], prompt: &'static str, max_masks: u8| SegmentRequest {
            jpeg,
            width: 64,
            height: 36,
            prompt,
            max_masks,
        };
        let body = request_body(&ask(b"\xff\xd8jpeg", " person ", 8)).unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["prompt"], "person");
        assert_eq!(value["max_masks"], 8);
        assert_eq!(value["include_scores"], true);
        assert!(
            value["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        for bad in [
            ask(b"", "person", 8),
            ask(b"jpeg", "", 8),
            ask(b"jpeg", "person", 0),
            ask(b"jpeg", "person", 33),
            SegmentRequest {
                width: 0,
                ..ask(b"jpeg", "person", 8)
            },
        ] {
            assert_eq!(request_body(&bad), Err(FalSegmentError::InvalidRequest));
        }
        let long = "x".repeat(49);
        assert_eq!(
            request_body(&SegmentRequest {
                prompt: &long,
                ..ask(b"jpeg", "person", 8)
            }),
            Err(FalSegmentError::InvalidRequest)
        );
    }

    #[test]
    fn endpoints_and_failure_classes_follow_the_research() {
        assert_eq!(SegmentModel::Sam31.endpoint(), "fal-ai/sam-3-1/image-rle");
        assert!(!FalSegmentError::InvalidRequest.was_sent());
        assert!(FalSegmentError::Provider(500).was_sent());
        assert!(FalSegmentError::Provider(503).is_transient());
        assert!(FalSegmentError::Provider(429).is_transient());
        assert!(!FalSegmentError::Provider(422).is_transient());
        assert!(!FalSegmentError::Unauthorized.is_transient());
        assert!(FalSegmentError::Transport(NativeHttpRefusal::Deadline).is_transient());
        assert!(!FalSegmentError::Transport(NativeHttpRefusal::ResponseTooLarge).is_transient());
        assert!(!FalSegmentError::Output.is_transient());
    }
}
