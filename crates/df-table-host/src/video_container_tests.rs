//! Reading a video's type and duration from its bytes.

use std::io::Cursor;
use std::time::Duration;

use crate::media_ingest::MediaLimits;
use crate::video_container::{DurationCause, FoundContent, VideoError, VideoFormat, inspect_video};
use crate::video_fixtures::{
    SyntheticFile, WebmInfo, mp4_ffmpeg_default, mp4_ffmpeg_faststart, mp4_of_seconds,
    mp4_version_one, mp4_with_brand, mp4_with_large_size_mdat_before_moov,
    mp4_with_moov_past_the_end, mp4_with_mvhd_past_the_moov, mp4_with_open_ended_mdat_before_moov,
    mp4_with_open_ended_mdat_last, mp4_without_movie_header, synthetic_mp4_with_moov_last,
    webm_doc_type_head, webm_ffmpeg_style, webm_info_first, webm_libwebm_style, webm_of_seconds,
    webm_with, webm_with_info_past_the_end, webm_with_timecode_scale_bytes,
};

const MIB: usize = 1024 * 1024;

fn inspect(bytes: &[u8], format: VideoFormat) -> Result<Duration, VideoError> {
    inspect_video(&mut Cursor::new(bytes), format, &MediaLimits::DEFAULT).map(|info| info.duration)
}

fn duration_of(bytes: &[u8], format: VideoFormat) -> Duration {
    inspect(bytes, format).expect("readable")
}

fn unreadable_cause(bytes: &[u8], format: VideoFormat) -> DurationCause {
    match inspect(bytes, format) {
        Err(VideoError::DurationUnreadable { cause }) => cause,
        other => panic!("expected an unreadable duration, got {other:?}"),
    }
}

fn found(bytes: &[u8], format: VideoFormat) -> FoundContent {
    match inspect(bytes, format) {
        Err(VideoError::WrongContent { found }) => found,
        other => panic!("expected wrong content, got {other:?}"),
    }
}

fn webm_with_ticks(ticks: f64) -> Vec<u8> {
    webm_with(
        "webm",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: Some(ticks),
        },
        8,
    )
}

#[test]
fn a_minimal_mp4_reports_its_duration_from_the_movie_header() {
    assert_eq!(
        duration_of(&mp4_of_seconds(5), VideoFormat::Mp4),
        Duration::from_secs(5)
    );
}

#[test]
fn a_version_one_movie_header_with_64_bit_times_is_read() {
    assert_eq!(
        duration_of(&mp4_version_one(90_000, 450_000), VideoFormat::Mp4),
        Duration::from_secs(5)
    );
}

#[test]
fn a_duration_is_rounded_up_to_the_next_millisecond() {
    assert_eq!(
        duration_of(&mp4_with_brand(b"isom", 600, 100), VideoFormat::Mp4),
        Duration::from_millis(167)
    );
}

#[test]
fn every_known_mp4_brand_is_accepted() {
    for brand in [
        b"isom", b"iso2", b"mp41", b"mp42", b"avc1", b"M4V ", b"dash",
    ] {
        let bytes = mp4_with_brand(brand, 1000, 2000);
        assert!(
            inspect(&bytes, VideoFormat::Mp4).is_ok(),
            "{}",
            String::from_utf8_lossy(brand)
        );
    }
}

#[test]
fn an_mp4_audio_brand_is_refused_and_named() {
    let bytes = mp4_with_brand(b"M4A ", 1000, 2000);
    let content = found(&bytes, VideoFormat::Mp4);
    assert_eq!(content, FoundContent::IsoMediaBrand(*b"M4A "));
    assert!(content.to_string().contains("M4A "), "{content}");
}

#[test]
fn a_webm_reports_its_duration_with_the_default_and_a_stated_timecode_scale() {
    assert_eq!(
        duration_of(&webm_of_seconds(7), VideoFormat::Webm),
        Duration::from_secs(7)
    );
    let microsecond_ticks = webm_with(
        "webm",
        &WebmInfo {
            timecode_scale: Some(1000),
            duration_ticks: Some(3_000_000.0),
        },
        8,
    );
    assert_eq!(
        duration_of(&microsecond_ticks, VideoFormat::Webm),
        Duration::from_secs(3)
    );
}

#[test]
fn text_an_svg_a_picture_and_the_other_video_type_are_refused_naming_what_was_found() {
    assert_eq!(
        found(b"just some words in a file", VideoFormat::Mp4),
        FoundContent::PlainText
    );
    assert_eq!(
        found(
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            VideoFormat::Mp4
        ),
        FoundContent::SvgOrXml
    );
    assert_eq!(
        found(b"<!DOCTYPE html><title>x</title>", VideoFormat::Mp4),
        FoundContent::Html
    );
    assert_eq!(
        found(b"\x89PNG\r\n\x1a\nrest", VideoFormat::Webm),
        FoundContent::Png
    );
    assert_eq!(
        found(&webm_of_seconds(2), VideoFormat::Mp4),
        FoundContent::WebmVideo
    );
    assert_eq!(
        found(&mp4_of_seconds(2), VideoFormat::Webm),
        FoundContent::Mp4Video
    );
    assert_eq!(
        found(&[0, 1, 2, 3, 0xFF, 0xFE], VideoFormat::Mp4),
        FoundContent::Binary
    );
    assert_eq!(FoundContent::Mp4Video.to_string(), "an mp4 video");
    assert_eq!(FoundContent::PlainText.to_string(), "plain text");
}

#[test]
fn a_text_file_with_a_multi_byte_character_across_the_description_prefix_is_still_plain_text() {
    let mut text = vec![b'a'; 511];
    text.extend_from_slice("\u{e9}tail".as_bytes());
    assert_eq!(text[511], 0xC3);
    assert_eq!(found(&text, VideoFormat::Mp4), FoundContent::PlainText);
}

#[test]
fn an_ebml_file_that_is_not_webm_is_refused_with_its_document_type() {
    let matroska = webm_with(
        "matroska",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: Some(1000.0),
        },
        8,
    );
    let content = found(&matroska, VideoFormat::Webm);
    assert_eq!(content, FoundContent::EbmlDocType(b"matroska".to_vec()));
    assert!(content.to_string().contains("matroska"), "{content}");
}

#[test]
fn a_huge_doc_type_claim_is_refused_with_a_short_name_after_reading_a_few_bytes() {
    let length = 64 * MIB as u64;
    let claimed = 60 * MIB;
    let mut file = SyntheticFile::new(webm_doc_type_head(claimed), Vec::new(), length, 0xFF);
    let refusal =
        inspect_video(&mut file, VideoFormat::Webm, &MediaLimits::DEFAULT).expect_err("refused");
    let VideoError::WrongContent { found } = refusal else {
        panic!("expected wrong content, got {refusal:?}");
    };
    let FoundContent::EbmlDocType(doc_type) = &found else {
        panic!("expected a document type, got {found:?}");
    };
    assert_eq!(doc_type.len(), 24);
    assert!(found.to_string().len() < 100, "{found}");
    assert!(file.bytes_read < 200, "{} bytes read", file.bytes_read);
}

#[test]
fn a_doc_type_that_starts_with_webm_but_is_longer_is_not_webm() {
    let longer = webm_with(
        "webmwebm",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: Some(1000.0),
        },
        8,
    );
    assert_eq!(
        found(&longer, VideoFormat::Webm),
        FoundContent::EbmlDocType(b"webmwebm".to_vec())
    );
}

#[test]
fn a_container_whose_duration_cannot_be_read_is_refused_with_the_reason() {
    assert_eq!(
        unreadable_cause(&mp4_without_movie_header(), VideoFormat::Mp4),
        DurationCause::NoMovieHeader
    );
    assert_eq!(
        unreadable_cause(&mp4_with_brand(b"isom", 0, 5), VideoFormat::Mp4),
        DurationCause::ZeroTimescale
    );
    for units in [0, u32::MAX] {
        assert_eq!(
            unreadable_cause(&mp4_with_brand(b"isom", 1000, units), VideoFormat::Mp4),
            DurationCause::ZeroOrUnknownDuration
        );
    }
    let no_duration = webm_with(
        "webm",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: None,
        },
        8,
    );
    assert_eq!(
        unreadable_cause(&no_duration, VideoFormat::Webm),
        DurationCause::NoDuration
    );
}

#[test]
fn a_webm_duration_that_is_zero_negative_or_not_a_number_is_unknown_and_infinity_is_out_of_range() {
    for ticks in [f64::NAN, -5.0, 0.0] {
        assert_eq!(
            unreadable_cause(&webm_with_ticks(ticks), VideoFormat::Webm),
            DurationCause::ZeroOrUnknownDuration,
            "{ticks}"
        );
    }
    assert_eq!(
        unreadable_cause(&webm_with_ticks(f64::INFINITY), VideoFormat::Webm),
        DurationCause::DurationOutOfRange
    );
}

#[test]
fn a_webm_without_a_segment_or_an_info_says_which_is_missing() {
    let whole = webm_of_seconds(3);
    assert_eq!(
        unreadable_cause(&whole[..26], VideoFormat::Webm),
        DurationCause::NoSegment
    );
    assert_eq!(
        unreadable_cause(&whole[..40], VideoFormat::Webm),
        DurationCause::NoInfo
    );
}

#[test]
fn no_cut_of_a_valid_file_makes_the_reader_panic_and_a_cut_before_the_header_is_unreadable() {
    for original in [mp4_of_seconds(3), webm_of_seconds(3), webm_ffmpeg_style(3)] {
        for length in 0..original.len() {
            for format in [VideoFormat::Mp4, VideoFormat::Webm] {
                let cut = &original[..length];
                let _ = inspect(cut, format);
            }
        }
    }
    let mp4 = mp4_of_seconds(3);
    let cut_inside_moov = &mp4[..mp4.len() - 10];
    assert_eq!(
        unreadable_cause(cut_inside_moov, VideoFormat::Mp4),
        DurationCause::NoMovieBox
    );
}

#[test]
fn a_box_that_claims_a_huge_size_does_not_overflow() {
    let mut bytes = mp4_of_seconds(3);
    bytes.extend_from_slice(&[0, 0, 0, 1]);
    bytes.extend_from_slice(b"free");
    bytes.extend_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        duration_of(&bytes, VideoFormat::Mp4),
        Duration::from_secs(3)
    );
    let mut first_box_huge = vec![0xFF, 0xFF, 0xFF, 0xFF];
    first_box_huge.extend_from_slice(b"ftypisom");
    assert_eq!(
        unreadable_cause(&first_box_huge, VideoFormat::Mp4),
        DurationCause::NoMovieBox
    );
}

// ---- layouts real encoders write (the fixtures say which encoder each copies)

#[test]
fn an_mp4_with_the_moov_after_a_large_mdat_is_read_as_ffmpeg_writes_it_by_default() {
    let bytes = mp4_ffmpeg_default(9, 5 * MIB);
    assert_eq!(
        duration_of(&bytes, VideoFormat::Mp4),
        Duration::from_secs(9)
    );
}

#[test]
fn an_mp4_with_the_moov_before_the_mdat_is_read_as_ffmpeg_faststart_writes_it() {
    let bytes = mp4_ffmpeg_faststart(9, 64 * 1024);
    assert_eq!(
        duration_of(&bytes, VideoFormat::Mp4),
        Duration::from_secs(9)
    );
}

#[test]
fn a_64_bit_sized_mdat_before_the_moov_is_stepped_over() {
    let bytes = mp4_with_large_size_mdat_before_moov(4, 3 * MIB);
    assert_eq!(
        duration_of(&bytes, VideoFormat::Mp4),
        Duration::from_secs(4)
    );
}

#[test]
fn a_last_mdat_with_size_zero_runs_to_the_end_and_a_first_one_hides_the_moov() {
    assert_eq!(
        duration_of(&mp4_with_open_ended_mdat_last(6, 1024), VideoFormat::Mp4),
        Duration::from_secs(6)
    );
    assert_eq!(
        unreadable_cause(
            &mp4_with_open_ended_mdat_before_moov(6, 1024),
            VideoFormat::Mp4
        ),
        DurationCause::NoMovieBox
    );
}

#[test]
fn a_moov_or_mvhd_that_claims_more_than_holds_it_is_refused() {
    assert_eq!(
        unreadable_cause(&mp4_with_moov_past_the_end(10_000), VideoFormat::Mp4),
        DurationCause::NoMovieBox
    );
    assert_eq!(
        unreadable_cause(&mp4_with_mvhd_past_the_moov(10_000), VideoFormat::Mp4),
        DurationCause::NoMovieHeader
    );
}

#[test]
fn a_webm_written_by_ffmpeg_with_a_seek_head_and_a_64_bit_float_duration_is_read() {
    assert_eq!(
        duration_of(&webm_ffmpeg_style(12), VideoFormat::Webm),
        Duration::from_secs(12)
    );
}

#[test]
fn a_webm_written_by_libwebm_with_a_32_bit_float_duration_is_read_with_or_without_a_segment_size() {
    for live_segment in [false, true] {
        assert_eq!(
            duration_of(&webm_libwebm_style(8, live_segment), VideoFormat::Webm),
            Duration::from_secs(8),
            "live segment: {live_segment}"
        );
    }
}

#[test]
fn a_webm_with_info_straight_after_the_header_is_read() {
    assert_eq!(
        duration_of(&webm_info_first(3), VideoFormat::Webm),
        Duration::from_secs(3)
    );
}

#[test]
fn a_timecode_scale_written_in_more_than_eight_bytes_is_refused() {
    assert_eq!(
        unreadable_cause(&webm_with_timecode_scale_bytes(9), VideoFormat::Webm),
        DurationCause::InvalidTimecodeScale
    );
    assert_eq!(
        duration_of(&webm_with_timecode_scale_bytes(8), VideoFormat::Webm),
        Duration::from_nanos(5000)
    );
}

#[test]
fn an_info_that_claims_more_than_the_file_holds_is_read_as_far_as_the_file_goes() {
    let whole = webm_with_info_past_the_end(10_000, usize::MAX);
    assert_eq!(
        duration_of(&whole, VideoFormat::Webm),
        Duration::from_secs(5)
    );
    let cut_before_duration = webm_with_info_past_the_end(10_000, 5);
    assert_eq!(
        unreadable_cause(&cut_before_duration, VideoFormat::Webm),
        DurationCause::NoDuration
    );
}

// ---- a file of any size is read through a few bytes

#[test]
fn an_mp4_of_three_gigabytes_with_the_moov_last_is_read_through_a_few_bytes() {
    let length = 3 * 1024 * MIB as u64;
    let mut file = synthetic_mp4_with_moov_last(length);
    let info = inspect_video(&mut file, VideoFormat::Mp4, &MediaLimits::DEFAULT).expect("readable");
    assert_eq!(info.duration, Duration::from_secs(1));
    assert!(file.bytes_read < 1024, "{} bytes read", file.bytes_read);
}

#[test]
fn a_file_of_tiny_boxes_stops_at_the_element_limit_instead_of_walking_to_the_end() {
    let limits = MediaLimits {
        maximum_container_elements: 100,
        ..MediaLimits::DEFAULT
    };
    let mut bytes = Vec::from(&mp4_of_seconds(1)[..32]);
    for _ in 0..1000 {
        bytes.extend_from_slice(&[0, 0, 0, 8]);
        bytes.extend_from_slice(b"free");
    }
    let refusal = inspect_video(&mut Cursor::new(&bytes[..]), VideoFormat::Mp4, &limits)
        .expect_err("refused");
    assert_eq!(
        refusal,
        VideoError::DurationUnreadable {
            cause: DurationCause::TooManyElements
        }
    );
}
