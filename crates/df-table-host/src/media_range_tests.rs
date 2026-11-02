//! Reading `Range` headers against a file length.

use crate::media_range::{ByteSpan, RangeRequest, parse_range};

fn partial(start: u64, end_inclusive: u64) -> RangeRequest {
    RangeRequest::Partial(ByteSpan {
        start,
        end_inclusive,
    })
}

#[test]
fn a_start_and_end_a_start_alone_and_a_suffix_each_name_their_bytes() {
    assert_eq!(parse_range("bytes=2-5", 20), partial(2, 5));
    assert_eq!(parse_range("bytes=10-", 20), partial(10, 19));
    assert_eq!(parse_range("bytes=-4", 20), partial(16, 19));
    assert_eq!(parse_range("bytes=0-0", 20), partial(0, 0));
    assert_eq!(parse_range(" bytes= 3 - 4 ", 20), partial(3, 4));
}

#[test]
fn an_end_past_the_file_is_cut_to_it_and_a_long_suffix_is_the_whole_file() {
    assert_eq!(parse_range("bytes=15-9999", 20), partial(15, 19));
    assert_eq!(parse_range("bytes=-9999", 20), partial(0, 19));
}

#[test]
fn a_start_at_or_past_the_end_and_an_empty_suffix_cannot_be_satisfied() {
    assert_eq!(parse_range("bytes=20-", 20), RangeRequest::Unsatisfiable);
    assert_eq!(parse_range("bytes=99-120", 20), RangeRequest::Unsatisfiable);
    assert_eq!(parse_range("bytes=-0", 20), RangeRequest::Unsatisfiable);
    assert_eq!(parse_range("bytes=0-", 0), RangeRequest::Unsatisfiable);
    assert_eq!(parse_range("bytes=-5", 0), RangeRequest::Unsatisfiable);
}

#[test]
fn several_ranges_garbage_another_unit_and_a_backwards_range_are_ignored() {
    for header in [
        "bytes=0-1,4-5",
        "bytes=",
        "bytes=-",
        "bytes=a-b",
        "bytes=+1-5",
        "bytes=1--5",
        "bytes=5-2",
        "items=0-1",
        "0-1",
        "",
        "bytes=99999999999999999999999-5",
    ] {
        assert_eq!(parse_range(header, 20), RangeRequest::Whole, "{header:?}");
    }
}

#[test]
fn a_number_too_large_for_u64_is_a_start_past_the_end_an_end_cut_to_the_file_or_a_whole_suffix() {
    let huge = "99999999999999999999999";
    assert_eq!(
        parse_range(&format!("bytes={huge}-"), 20),
        RangeRequest::Unsatisfiable
    );
    assert_eq!(parse_range(&format!("bytes=-{huge}"), 20), partial(0, 19));
    assert_eq!(parse_range(&format!("bytes=5-{huge}"), 20), partial(5, 19));
    assert_eq!(
        parse_range("bytes=18446744073709551615-", 20),
        RangeRequest::Unsatisfiable
    );
}

#[test]
fn the_unit_is_matched_without_regard_to_case() {
    for header in ["Bytes=0-3", "BYTES=0-3", "bYtEs=0-3"] {
        assert_eq!(parse_range(header, 20), partial(0, 3), "{header}");
    }
}

#[test]
fn a_range_is_answered_in_full_however_large_the_file() {
    let length = 5 * 1024 * 1024 * 1024_u64;
    assert_eq!(parse_range("bytes=0-", length), partial(0, length - 1));
    assert_eq!(
        parse_range("bytes=-1000", length),
        partial(length - 1000, length - 1)
    );
    assert_eq!(parse_range("bytes=5-9", length), partial(5, 9));
    assert_eq!(partial_length(parse_range("bytes=0-", length)), length);
}

fn partial_length(request: RangeRequest) -> u64 {
    match request {
        RangeRequest::Partial(span) => span.length(),
        other => panic!("expected a range, got {other:?}"),
    }
}
