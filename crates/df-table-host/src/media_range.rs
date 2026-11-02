//! `Range` request headers for the media route: which bytes of a file a request asks for.
//!
//! A browser needs `206` answers to start a video and to seek in it. Only a single `bytes=`
//! range is served; a header that is malformed, names another unit or asks for several ranges is
//! ignored, so the whole file is sent as for a request without one. The unit is matched without
//! regard to case (RFC 9110 14.1). A number too large for `u64` saturates: as a start it is past
//! the end of any file (`416`), as an end it is cut to the file, as a suffix length it is the
//! whole file. A range is answered in full: the file is streamed from its handle in chunks, so a
//! large range costs no memory.

/// An inclusive span of bytes, both ends inside the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ByteSpan {
    pub(crate) start: u64,
    pub(crate) end_inclusive: u64,
}

impl ByteSpan {
    pub(crate) fn length(self) -> u64 {
        self.end_inclusive
            .saturating_sub(self.start)
            .saturating_add(1)
    }
}

/// What to answer for a `Range` header against a file of known length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeRequest {
    /// No usable range: send the whole file with `200`.
    Whole,
    /// Send these bytes with `206`.
    Partial(ByteSpan),
    /// The range starts past the end: answer `416`.
    Unsatisfiable,
}

/// Reads a `Range` header value for a file of `length` bytes.
pub(crate) fn parse_range(header: &str, length: u64) -> RangeRequest {
    let Some((unit, spec)) = header.trim().split_once('=') else {
        return RangeRequest::Whole;
    };
    if !unit.eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangeRequest::Whole;
    };
    let (first, last) = (first.trim(), last.trim());
    let requested = match (first.is_empty(), last.is_empty()) {
        (true, true) => return RangeRequest::Whole,
        (true, false) => suffix_span(last, length),
        (false, _) => start_span(first, last, length),
    };
    match requested {
        Requested::Ignored => RangeRequest::Whole,
        Requested::Unsatisfiable => RangeRequest::Unsatisfiable,
        Requested::Span(span) => RangeRequest::Partial(span),
    }
}

/// A number written with digits only, saturating at `u64::MAX`: `u64::from_str` would also accept
/// a leading `+`, and fail on a number too large.
fn parse_digits(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    text.bytes().try_fold(0_u64, |value, byte| {
        let digit = char::from(byte).to_digit(10)?;
        Some(value.saturating_mul(10).saturating_add(u64::from(digit)))
    })
}

enum Requested {
    Ignored,
    Unsatisfiable,
    Span(ByteSpan),
}

/// `-n`: the last `n` bytes. Zero bytes cannot be satisfied; more than the file is all of it.
fn suffix_span(count: &str, length: u64) -> Requested {
    let Some(count) = parse_digits(count) else {
        return Requested::Ignored;
    };
    let Some(last_byte) = length.checked_sub(1) else {
        return Requested::Unsatisfiable;
    };
    if count == 0 {
        return Requested::Unsatisfiable;
    }
    Requested::Span(ByteSpan {
        start: length.saturating_sub(count),
        end_inclusive: last_byte,
    })
}

/// `a-b` and `a-`: from `a` to `b` (or the end), with `b` cut to the file.
fn start_span(first: &str, last: &str, length: u64) -> Requested {
    let Some(start) = parse_digits(first) else {
        return Requested::Ignored;
    };
    let requested_end = if last.is_empty() {
        None
    } else {
        match parse_digits(last) {
            Some(end) if end >= start => Some(end),
            _ => return Requested::Ignored,
        }
    };
    let Some(last_byte) = length.checked_sub(1) else {
        return Requested::Unsatisfiable;
    };
    if start > last_byte {
        return Requested::Unsatisfiable;
    }
    Requested::Span(ByteSpan {
        start,
        end_inclusive: requested_end.map_or(last_byte, |end| end.min(last_byte)),
    })
}
