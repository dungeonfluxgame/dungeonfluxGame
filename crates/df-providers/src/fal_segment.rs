//! Provider-neutral mask geometry and historical segmentation accounting labels.

use crate::NativeHttpRefusal;
use crate::output_fetch::MAXIMUM_IMAGE_SIDE_PX;

/// The most instances one segmentation answer may carry.
pub const MAXIMUM_MASKS: u8 = 32;

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
        if width == 0
            || height == 0
            || width > MAXIMUM_IMAGE_SIDE_PX
            || height > MAXIMUM_IMAGE_SIDE_PX
        {
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
        // Saturating: a caller may ask for every row (`u32::MAX`), which must not overflow.
        let end_row = top.saturating_add(rows.max(1));
        let (mut sum, mut count) = (0u64, 0u64);
        for (y, from, to) in self.row_segments() {
            if y >= end_row {
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

/// One instance fal found.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentInstance {
    /// fal's own index of the instance (answers come in score order).
    pub index: u32,
    pub score: f32,
    pub mask: MaskRows,
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

    /// `top + rows` overflowed for a request of every row: a panic in a debug build, and in a
    /// release build a wrapped bound that skipped every row and reported no centre.
    #[test]
    fn asking_for_every_row_averages_the_whole_mask_instead_of_overflowing() {
        let runs = rectangle(30, 8, 3, 12, 5);
        let head = MaskRows::from_runs(30, 12, runs).unwrap();
        let (x, y) = head.top_centre(u32::MAX).unwrap();
        assert_eq!(y, 3);
        assert!((x - 10.0).abs() < 0.01, "{x}");
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
