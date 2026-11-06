//! Video in the media library: what is accepted, what is refused and why, and `Range` answers.

use std::io::Cursor;
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use df_playground::media::MediaEntry;
use df_playground::media::MediaKind;
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::post_json;
use crate::dev::{DevError, ReplyError, checked_file_size, run_dev};
use crate::media_ingest::{
    IngestError, MAXIMUM_MEDIA_BYTES, MAXIMUM_MEDIA_FILES, MAXIMUM_UPLOAD_BODY_BYTES,
    MAXIMUM_VIDEO_DURATION, MediaLimits, UploadKind, check_content, check_room,
};
use crate::test_support::{Running, STARTER_FILES, post_request, raw_exchange, start};
use crate::video_container::{DurationCause, FoundContent, VideoFormat};
use crate::video_fixtures::{
    SyntheticFile, WebmInfo, mp4_ffmpeg_default, mp4_of_seconds, mp4_of_size, mp4_of_size_prefix,
    mp4_with_brand, mp4_without_movie_header, synthetic_mp4_with_moov_last, webm_ffmpeg_style,
    webm_libwebm_style, webm_of_seconds, webm_with,
};

const SENTENCE: &[u8] = b"0123456789abcdefghij";
const MIB: usize = 1024 * 1024;

fn words(line: &str) -> Vec<String> {
    crate::command::split_words(line).expect("the line splits")
}

fn check(name: &str, bytes: &[u8]) -> Result<(), IngestError> {
    let upload = UploadKind::from_name(name).expect("a media name");
    check_content(&mut Cursor::new(bytes), upload, &MediaLimits::DEFAULT)
}

async fn add(running: &Running, name: &str, bytes: &[u8]) -> (StatusCode, String) {
    let body = json!({"name": name, "data_base64": STANDARD.encode(bytes)}).to_string();
    let response = post_json(running.port, "/dev/playground/media", &body)
        .await
        .expect("answers");
    (response.status, response.body)
}

fn library_directory(running: &Running) -> std::path::PathBuf {
    let directory = running.playground_path("assets/media");
    std::fs::create_dir_all(&directory).expect("makes the directory");
    directory
}

/// A file of `length` bytes that is all zeros and takes no disk to hold.
fn sparse_file(path: &std::path::Path, head: &[u8], length: u64) {
    use std::io::Write;
    let mut file = std::fs::File::create(path).expect("creates");
    file.write_all(head).expect("writes the head");
    file.set_len(length).expect("sizes");
}

struct Answer {
    head: String,
    body: Vec<u8>,
}

impl Answer {
    fn status(&self) -> &str {
        self.head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("has a status")
    }

    fn header(&self, name: &str) -> Option<&str> {
        let prefix = format!("{name}: ");
        self.head
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
    }
}

async fn send(running: &Running, method: &str, name: &str, extra_headers: &str) -> Answer {
    let request = format!(
        "{method} /playground/media/{name} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         {extra_headers}Connection: close\r\n\r\n"
    );
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", running.port))
        .await
        .expect("connects");
    stream.write_all(request.as_bytes()).await.expect("sends");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("reads");
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("has a head");
    Answer {
        head: String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase(),
        body: raw[split + 4..].to_vec(),
    }
}

async fn fetch(running: &Running, name: &str, extra_headers: &str) -> Answer {
    send(running, "GET", name, extra_headers).await
}

async fn with_sentence(running: &Running) -> String {
    add(running, "sentence.png", SENTENCE).await;
    fetch(running, "sentence.png", "")
        .await
        .header("etag")
        .expect("has an etag")
        .to_owned()
}

fn unreadable(refusal: &IngestError) -> DurationCause {
    match refusal {
        IngestError::DurationUnreadable { cause, .. } => *cause,
        other => panic!("expected an unreadable duration, got {other:?}"),
    }
}

// ---- ingest checks on the bytes

#[test]
fn a_real_mp4_and_a_real_webm_pass_the_content_checks() {
    assert_eq!(check("a.mp4", &mp4_of_seconds(3)), Ok(()));
    assert_eq!(check("a.webm", &webm_of_seconds(3)), Ok(()));
    assert_eq!(check("b.mp4", &mp4_ffmpeg_default(10, MIB)), Ok(()));
    assert_eq!(check("b.webm", &webm_ffmpeg_style(10)), Ok(()));
    assert_eq!(check("c.webm", &webm_libwebm_style(10, true)), Ok(()));
}

#[test]
fn a_text_file_named_mp4_an_svg_and_a_mismatched_type_are_refused_naming_what_was_found() {
    let text = check("a.mp4", b"hello there").expect_err("refused");
    assert_eq!(
        text,
        IngestError::NotThatVideoType {
            claimed: VideoFormat::Mp4,
            found: FoundContent::PlainText
        }
    );
    assert!(
        text.to_string().contains("name says mp4"),
        "{}",
        text.to_string()
    );
    assert!(
        text.to_string().contains("plain text"),
        "{}",
        text.to_string()
    );
    assert_eq!(text.status(), StatusCode::BAD_REQUEST);
    let svg = check("a.mp4", b"<svg onload='x()'/>").expect_err("refused");
    assert_eq!(
        svg,
        IngestError::NotThatVideoType {
            claimed: VideoFormat::Mp4,
            found: FoundContent::SvgOrXml
        }
    );
    assert!(svg.to_string().contains("SVG"), "{}", svg.to_string());
    let mp4_as_webm = check("a.webm", &mp4_of_seconds(2)).expect_err("refused");
    assert_eq!(
        mp4_as_webm,
        IngestError::NotThatVideoType {
            claimed: VideoFormat::Webm,
            found: FoundContent::Mp4Video
        }
    );
    assert!(
        mp4_as_webm.to_string().contains("name says webm")
            && mp4_as_webm.to_string().contains("an mp4"),
        "{}",
        mp4_as_webm.to_string()
    );
    let webm_as_mp4 = check("a.mp4", &webm_of_seconds(2)).expect_err("refused");
    assert_eq!(
        webm_as_mp4,
        IngestError::NotThatVideoType {
            claimed: VideoFormat::Mp4,
            found: FoundContent::WebmVideo
        }
    );
    assert!(webm_as_mp4.to_string().contains("a webm video"));
}

#[test]
fn the_defaults_are_two_gibibytes_twelve_hours_and_ten_thousand_files() {
    assert_eq!(MAXIMUM_MEDIA_BYTES, 2 * 1024 * 1024 * 1024);
    assert_eq!(MAXIMUM_VIDEO_DURATION, Duration::from_secs(12 * 60 * 60));
    assert_eq!(MAXIMUM_MEDIA_FILES, 10_000);
    assert_eq!(
        MAXIMUM_UPLOAD_BODY_BYTES,
        MAXIMUM_MEDIA_BYTES.div_ceil(3) * 4 + 4096
    );
}

#[test]
fn a_file_of_exactly_the_size_limit_passes_and_one_byte_more_is_refused_for_every_kind() {
    let limit = MAXIMUM_MEDIA_BYTES;
    let limits = MediaLimits::DEFAULT;
    for (name, kind) in [("a.png", MediaKind::Image), ("a.wav", MediaKind::Audio)] {
        let upload = UploadKind::from_name(name).expect("a media name");
        let mut at_limit = SyntheticFile::new(vec![1], Vec::new(), limit, 0);
        assert_eq!(
            check_content(&mut at_limit, upload, &limits),
            Ok(()),
            "{kind:?}"
        );
        let mut over = SyntheticFile::new(vec![1], Vec::new(), limit + 1, 0);
        let refusal = check_content(&mut over, upload, &limits).expect_err("refused");
        assert_eq!(
            refusal,
            IngestError::TooLarge {
                maximum_bytes: limit
            }
        );
        assert_eq!(refusal.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            refusal.to_string().contains("2 GiB"),
            "{}",
            refusal.to_string()
        );
    }
    let video = UploadKind::from_name("a.mp4").expect("a media name");
    let mut at_limit = synthetic_mp4_with_moov_last(limit);
    assert_eq!(check_content(&mut at_limit, video, &limits), Ok(()));
    assert!(
        at_limit.bytes_read < 1024,
        "{} bytes read",
        at_limit.bytes_read
    );
    let mut over = synthetic_mp4_with_moov_last(limit + 1);
    assert_eq!(
        check_content(&mut over, video, &limits),
        Err(IngestError::TooLarge {
            maximum_bytes: limit
        })
    );
    assert_eq!(
        over.bytes_read, 0,
        "an oversize file is refused from its length alone"
    );
}

#[test]
fn a_video_of_exactly_twelve_hours_passes_and_one_second_more_is_refused_naming_the_limit() {
    let twelve_hours_ms = 12 * 60 * 60 * 1000;
    assert_eq!(
        check("a.mp4", &mp4_with_brand(b"isom", 1000, twelve_hours_ms)),
        Ok(())
    );
    let over = check(
        "a.mp4",
        &mp4_with_brand(b"isom", 1000, twelve_hours_ms + 1000),
    )
    .expect_err("refused");
    assert!(matches!(over, IngestError::TooLong { .. }), "{over:?}");
    assert!(
        over.to_string().contains("43201.0 s"),
        "{}",
        over.to_string()
    );
    assert!(over.to_string().contains("43200 s"), "{}", over.to_string());
    let webm = |ticks: f64| {
        webm_with(
            "webm",
            &WebmInfo {
                timecode_scale: None,
                duration_ticks: Some(ticks),
            },
            8,
        )
    };
    assert_eq!(check("a.webm", &webm(43_200_000.0)), Ok(()));
    assert!(matches!(
        check("a.webm", &webm(43_201_000.0)),
        Err(IngestError::TooLong { .. })
    ));
}

#[test]
fn a_video_whose_duration_cannot_be_read_is_refused_with_the_reason() {
    let no_header = check("a.mp4", &mp4_without_movie_header()).expect_err("refused");
    assert_eq!(unreadable(&no_header), DurationCause::NoMovieHeader);
    assert!(
        no_header.to_string().contains("cannot be read") && no_header.to_string().contains("mvhd"),
        "{}",
        no_header.to_string()
    );
    let no_duration = webm_with(
        "webm",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: None,
        },
        8,
    );
    let refusal = check("a.webm", &no_duration).expect_err("refused");
    assert_eq!(unreadable(&refusal), DurationCause::NoDuration);
    assert!(
        refusal.to_string().contains("Duration"),
        "{}",
        refusal.to_string()
    );
    let fragmented = check("a.mp4", &mp4_with_brand(b"dash", 1000, 0)).expect_err("refused");
    assert_eq!(
        unreadable(&fragmented),
        DurationCause::ZeroOrUnknownDuration
    );
}

#[test]
fn a_picture_or_sound_is_only_size_checked() {
    assert_eq!(check("a.png", b"not really a picture"), Ok(()));
    assert_eq!(check("a.wav", b"RIFF"), Ok(()));
    assert_eq!(check("a.png", b""), Err(IngestError::Empty));
}

#[test]
fn a_video_name_carries_its_format_and_other_names_do_not() {
    assert_eq!(
        UploadKind::from_name("a.MP4"),
        Some(UploadKind::Video(VideoFormat::Mp4))
    );
    assert_eq!(
        UploadKind::from_name("a.webm"),
        Some(UploadKind::Video(VideoFormat::Webm))
    );
    assert_eq!(UploadKind::from_name("a.png"), Some(UploadKind::Image));
    assert_eq!(UploadKind::from_name("a.opus"), Some(UploadKind::Audio));
    assert_eq!(UploadKind::from_name("a.svg"), None);
    assert_eq!(UploadKind::from_name("../a.mp4"), None);
}

fn entries(count: usize, kind: MediaKind) -> Vec<MediaEntry> {
    (0..count)
        .map(|index| MediaEntry {
            name: format!("f{index}.mp4"),
            kind,
            bytes: 1,
        })
        .collect()
}

#[test]
fn the_library_has_room_for_the_ten_thousandth_file_but_not_the_next_and_replaces_freely() {
    let limits = MediaLimits::DEFAULT;
    let almost_full = entries(MAXIMUM_MEDIA_FILES - 1, MediaKind::Image);
    assert_eq!(check_room(&almost_full, "new.png", &limits), Ok(()));
    let full = entries(MAXIMUM_MEDIA_FILES, MediaKind::Image);
    assert_eq!(
        check_room(&full, "new.png", &limits),
        Err(IngestError::LibraryFull {
            maximum_files: MAXIMUM_MEDIA_FILES
        })
    );
    assert_eq!(check_room(&full, "f0.mp4", &limits), Ok(()));
}

#[test]
fn videos_have_no_count_limit_of_their_own() {
    let many_videos = entries(500, MediaKind::Video);
    assert_eq!(
        check_room(&many_videos, "one-more.mp4", &MediaLimits::DEFAULT),
        Ok(())
    );
}

// ---- the library

#[tokio::test]
async fn a_real_mp4_and_webm_are_added_listed_as_video_and_served_with_their_type() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        add(&running, "hearth.mp4", &mp4_of_seconds(3)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        add(&running, "rain.webm", &webm_of_seconds(4)).await.0,
        StatusCode::OK
    );
    let listed = run_dev(running.port, &words("media")).await.expect("lists");
    assert!(
        listed.contains("hearth.mp4") && listed.contains("rain.webm") && listed.contains("video"),
        "{listed}"
    );
    let served = fetch(&running, "hearth.mp4", "").await;
    assert_eq!(served.status(), "200");
    assert_eq!(served.header("content-type"), Some("video/mp4"));
    assert_eq!(served.header("accept-ranges"), Some("bytes"));
    assert_eq!(served.body, mp4_of_seconds(3));
    assert_eq!(
        fetch(&running, "rain.webm", "")
            .await
            .header("content-type"),
        Some("video/webm")
    );
}

#[tokio::test]
async fn refused_uploads_say_why_and_write_nothing() {
    let running = start(&STARTER_FILES).await;
    for (name, bytes, reason) in [
        ("note.mp4", b"words".to_vec(), "plain text"),
        ("art.mp4", b"<svg/>".to_vec(), "SVG"),
        ("wrong.webm", mp4_of_seconds(2), "an mp4"),
        ("long.mp4", mp4_of_seconds(13 * 60 * 60), "46800.0 s"),
        ("blind.mp4", mp4_without_movie_header(), "cannot be read"),
    ] {
        let (status, body) = add(&running, name, &bytes).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert!(body.contains(reason), "{name}: {body}");
    }
    assert_eq!(
        run_dev(running.port, &words("media")).await.expect("lists"),
        "no media (add one with: media add FILE)"
    );
    let staged: Vec<_> = std::fs::read_dir(library_directory(&running))
        .expect("lists")
        .collect();
    assert!(staged.is_empty(), "a refused upload left {staged:?}");
}

#[tokio::test]
async fn a_video_larger_than_the_old_limits_is_accepted_through_the_route_and_ranges_into_it() {
    let running = start(&STARTER_FILES).await;
    let bytes = mp4_of_size(80 * MIB);
    assert_eq!(
        add(&running, "long-take.mp4", &bytes).await.0,
        StatusCode::OK
    );
    let served = fetch(
        &running,
        "long-take.mp4",
        "Range: bytes=67108864-67108871\r\n",
    )
    .await;
    assert_eq!(served.status(), "206");
    assert_eq!(served.body.len(), 8);
}

#[tokio::test]
async fn more_than_sixteen_videos_are_accepted() {
    let running = start(&STARTER_FILES).await;
    for index in 0..20 {
        let (status, body) = add(&running, &format!("clip{index}.mp4"), &mp4_of_seconds(1)).await;
        assert_eq!(status, StatusCode::OK, "{index}: {body}");
    }
}

#[tokio::test]
async fn a_request_that_declares_more_than_the_largest_add_is_refused_with_413_without_its_body() {
    let running = start(&STARTER_FILES).await;
    let declared = MAXIMUM_UPLOAD_BODY_BYTES + 1;
    let request = format!(
        "POST /dev/playground/media HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {declared}\r\nConnection: close\r\n\r\n"
    );
    let answer = raw_exchange(running.port, &request).await;
    assert!(
        answer.starts_with("HTTP/1.1 413"),
        "{}",
        answer.lines().next().unwrap_or("")
    );
    assert!(answer.contains("2 GiB"), "{answer}");
}

#[tokio::test]
async fn a_request_that_is_not_the_add_object_is_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    for body in [
        r#"{"name": 5}"#,
        r#"{"name":"a.png","data_base64":"aGk=","extra":1}"#,
        "not json",
    ] {
        let response = post_json(running.port, "/dev/playground/media", body)
            .await
            .expect("answers");
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(response.body.contains("not valid"), "{}", response.body);
    }
}

#[tokio::test]
async fn the_raised_body_limit_belongs_to_the_add_route_alone() {
    let running = start(&STARTER_FILES).await;
    let large = format!(
        r#"{{"name":"a.png","padding":"{}"}}"#,
        "x".repeat(crate::limits::MAXIMUM_REQUEST_BYTES + 1024)
    );
    let request = post_request("/dev/playground/media/remove", &large, "");
    let answer = raw_exchange(running.port, &request).await;
    assert!(
        answer.starts_with("HTTP/1.1 413"),
        "{}",
        answer.lines().next().unwrap_or("")
    );
    let bytes = vec![7_u8; crate::limits::MAXIMUM_REQUEST_BYTES + 1];
    assert_eq!(
        add(&running, "wider-than-the-router-limit.png", &bytes)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_library_holds_ten_thousand_files_and_refuses_the_next_but_allows_a_replacement() {
    let running = start(&STARTER_FILES).await;
    let directory = library_directory(&running);
    for index in 0..MAXIMUM_MEDIA_FILES {
        std::fs::write(directory.join(format!("f{index}.png")), b"a picture").expect("writes");
    }
    let (status, body) = add(&running, "fourth.mp4", &mp4_of_seconds(1)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("10000 files"), "{body}");
    assert_eq!(
        add(&running, "f0.png", b"another picture").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn media_add_takes_a_video_path_and_refuses_a_fake_one() {
    let running = start(&STARTER_FILES).await;
    let directory = TempDir::new().expect("makes a directory");
    let good = directory.path().join("hearth.mp4");
    std::fs::write(&good, mp4_of_seconds(2)).expect("writes");
    let line = vec![
        "media".to_owned(),
        "add".to_owned(),
        good.to_string_lossy().into_owned(),
    ];
    run_dev(running.port, &line).await.expect("adds");
    assert!(
        run_dev(running.port, &words("media"))
            .await
            .expect("lists")
            .contains("hearth.mp4")
    );

    let fake = directory.path().join("fake.mp4");
    std::fs::write(&fake, b"not a video").expect("writes");
    let line = vec![
        "media".to_owned(),
        "add".to_owned(),
        fake.to_string_lossy().into_owned(),
    ];
    let error = run_dev(running.port, &line).await.expect_err("refused");
    // The library's refusals carry no cause yet; the sentence says what the bytes are.
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas { cause: None, .. },
                message,
            } if message.contains("plain text")
        ),
        "{error:?}"
    );
}

#[test]
fn a_local_file_is_refused_over_the_limit_before_any_of_it_is_read() {
    assert_eq!(
        checked_file_size(MAXIMUM_MEDIA_BYTES, MAXIMUM_MEDIA_BYTES),
        Ok(MAXIMUM_MEDIA_BYTES)
    );
    assert!(
        checked_file_size(MAXIMUM_MEDIA_BYTES + 1, MAXIMUM_MEDIA_BYTES)
            .expect_err("refused")
            .contains("larger than")
    );
}

#[tokio::test]
async fn media_add_sends_a_video_near_64_mib_through_the_dev_path_in_time() {
    // A sparse stand-in for a long ffmpeg faststart mp4: the head is real, the media is zeros.
    let running = start(&STARTER_FILES).await;
    let directory = TempDir::new().expect("makes a directory");
    let path = directory.path().join("long-take.mp4");
    let total = 64 * MIB - 1024;
    sparse_file(&path, &mp4_of_size_prefix(total), total as u64);
    let line = vec![
        "media".to_owned(),
        "add".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    let started = std::time::Instant::now();
    run_dev(running.port, &line).await.expect("adds");
    let elapsed = started.elapsed();
    let stored = std::fs::metadata(library_directory(&running).join("long-take.mp4"))
        .expect("is stored")
        .len();
    assert_eq!(stored, total as u64);
    assert!(elapsed < Duration::from_secs(60), "took {elapsed:?}");
}

// ---- Range

#[tokio::test]
async fn a_range_gives_206_with_the_right_bytes_and_headers() {
    let running = start(&STARTER_FILES).await;
    with_sentence(&running).await;
    for (range, content_range, bytes) in [
        ("bytes=2-5", "bytes 2-5/20", &b"2345"[..]),
        ("bytes=15-", "bytes 15-19/20", &b"fghij"[..]),
        ("bytes=-3", "bytes 17-19/20", &b"hij"[..]),
        ("bytes=0-0", "bytes 0-0/20", &b"0"[..]),
        ("bytes=19-19", "bytes 19-19/20", &b"j"[..]),
        ("bytes=15-999", "bytes 15-19/20", &b"fghij"[..]),
        ("bytes=-999", "bytes 0-19/20", SENTENCE),
        ("bytes=-99999999999999999999999", "bytes 0-19/20", SENTENCE),
        (
            "bytes=15-99999999999999999999999",
            "bytes 15-19/20",
            &b"fghij"[..],
        ),
        ("Bytes=2-5", "bytes 2-5/20", &b"2345"[..]),
        ("BYTES=2-5", "bytes 2-5/20", &b"2345"[..]),
    ] {
        let answer = fetch(&running, "sentence.png", &format!("Range: {range}\r\n")).await;
        assert_eq!(answer.status(), "206", "{range}");
        assert_eq!(
            answer.header("content-range"),
            Some(content_range),
            "{range}"
        );
        assert_eq!(
            answer.header("content-length"),
            Some(bytes.len().to_string().as_str()),
            "{range}"
        );
        assert_eq!(answer.header("accept-ranges"), Some("bytes"), "{range}");
        assert_eq!(answer.header("content-type"), Some("image/png"), "{range}");
        assert!(answer.header("etag").is_some(), "{range}");
        assert_eq!(answer.body, bytes, "{range}");
    }
}

#[tokio::test]
async fn a_range_that_starts_past_the_end_gives_416_with_the_length() {
    let running = start(&STARTER_FILES).await;
    with_sentence(&running).await;
    for range in [
        "bytes=20-",
        "bytes=500-600",
        "bytes=-0",
        "bytes=99999999999999999999999-",
    ] {
        let answer = fetch(&running, "sentence.png", &format!("Range: {range}\r\n")).await;
        assert_eq!(answer.status(), "416", "{range}");
        assert_eq!(
            answer.header("content-range"),
            Some("bytes */20"),
            "{range}"
        );
        assert!(answer.body.is_empty(), "{range}");
    }
}

#[tokio::test]
async fn a_range_that_starts_past_the_end_of_a_video_gives_416_with_the_length() {
    let running = start(&STARTER_FILES).await;
    let bytes = mp4_of_seconds(2);
    assert_eq!(add(&running, "hearth.mp4", &bytes).await.0, StatusCode::OK);
    let length = bytes.len();
    for range in [
        format!("bytes={length}-"),
        format!("bytes={}-", length + 1000),
    ] {
        let answer = fetch(&running, "hearth.mp4", &format!("Range: {range}\r\n")).await;
        assert_eq!(answer.status(), "416", "{range}");
        assert_eq!(
            answer.header("content-range"),
            Some(format!("bytes */{length}").as_str()),
            "{range}"
        );
        assert_eq!(answer.header("content-type"), Some("video/mp4"), "{range}");
        assert!(answer.body.is_empty(), "{range}");
    }
}

#[tokio::test]
async fn several_ranges_garbage_and_another_unit_are_served_as_a_normal_200() {
    let running = start(&STARTER_FILES).await;
    with_sentence(&running).await;
    for range in [
        "bytes=0-1,4-5",
        "bytes=banana",
        "pages=1-2",
        "bytes=5-2",
        "bytes=",
    ] {
        let answer = fetch(&running, "sentence.png", &format!("Range: {range}\r\n")).await;
        assert_eq!(answer.status(), "200", "{range}");
        assert_eq!(answer.body, SENTENCE, "{range}");
        assert!(answer.header("content-range").is_none(), "{range}");
    }
}

#[tokio::test]
async fn the_etag_and_not_modified_keep_working_with_a_range() {
    let running = start(&STARTER_FILES).await;
    let tag = with_sentence(&running).await;
    let unchanged = fetch(
        &running,
        "sentence.png",
        &format!("If-None-Match: {tag}\r\nRange: bytes=0-3\r\n"),
    )
    .await;
    assert_eq!(unchanged.status(), "304");
    assert!(unchanged.body.is_empty());
    assert_eq!(unchanged.header("etag"), Some(tag.as_str()));
    let matching = fetch(
        &running,
        "sentence.png",
        &format!("If-Range: {tag}\r\nRange: bytes=0-3\r\n"),
    )
    .await;
    assert_eq!(matching.status(), "206");
    assert_eq!(matching.body, b"0123");
}

#[tokio::test]
async fn an_if_range_that_does_not_match_gives_the_whole_file() {
    let running = start(&STARTER_FILES).await;
    with_sentence(&running).await;
    for version in ["\"stale\"", "Wed, 21 Oct 2015 07:28:00 GMT"] {
        let answer = fetch(
            &running,
            "sentence.png",
            &format!("If-Range: {version}\r\nRange: bytes=0-3\r\n"),
        )
        .await;
        assert_eq!(answer.status(), "200", "{version}");
        assert_eq!(answer.body, SENTENCE, "{version}");
    }
}

#[tokio::test]
async fn a_range_of_a_large_sparse_file_is_answered_in_full_from_the_requested_bytes() {
    let running = start(&STARTER_FILES).await;
    let directory = library_directory(&running);
    let file_bytes = 40 * MIB as u64;
    sparse_file(&directory.join("sparse.mp4"), b"", file_bytes);

    let kilobyte = fetch(&running, "sparse.mp4", "Range: bytes=1000-1999\r\n").await;
    assert_eq!(kilobyte.status(), "206");
    assert_eq!(kilobyte.body.len(), 1000);
    assert_eq!(
        kilobyte.header("content-range"),
        Some(format!("bytes 1000-1999/{file_bytes}").as_str())
    );

    let open_ended = fetch(&running, "sparse.mp4", "Range: bytes=10485760-\r\n").await;
    assert_eq!(open_ended.status(), "206");
    assert_eq!(open_ended.body.len() as u64, file_bytes - 10 * MIB as u64);
    assert_eq!(
        open_ended.header("content-range"),
        Some(format!("bytes 10485760-{}/{file_bytes}", file_bytes - 1).as_str())
    );

    let tail = fetch(&running, "sparse.mp4", "Range: bytes=-10\r\n").await;
    assert_eq!(
        tail.header("content-range"),
        Some(format!("bytes {}-{}/{file_bytes}", file_bytes - 10, file_bytes - 1).as_str())
    );

    let suffix = fetch(&running, "sparse.mp4", "Range: bytes=-4194304\r\n").await;
    assert_eq!(suffix.status(), "206");
    assert_eq!(suffix.body.len(), 4 * MIB);
    assert_eq!(
        suffix.header("content-range"),
        Some(
            format!(
                "bytes {}-{}/{file_bytes}",
                file_bytes - 4 * MIB as u64,
                file_bytes - 1
            )
            .as_str()
        )
    );
}

// ---- whole files and HEAD

#[tokio::test]
async fn a_get_without_a_range_streams_a_large_file_with_its_length() {
    let running = start(&STARTER_FILES).await;
    let directory = library_directory(&running);
    let file_bytes = 24 * MIB as u64 + 17;
    sparse_file(&directory.join("whole.mp4"), b"head", file_bytes);
    let answer = fetch(&running, "whole.mp4", "").await;
    assert_eq!(answer.status(), "200");
    assert_eq!(
        answer.header("content-length"),
        Some(file_bytes.to_string().as_str())
    );
    assert_eq!(answer.body.len() as u64, file_bytes);
    assert_eq!(&answer.body[..4], b"head");
    assert!(answer.body[4..].iter().all(|&byte| byte == 0));
}

#[tokio::test]
async fn a_head_answers_with_the_length_and_the_headers_of_a_get_and_no_body() {
    let running = start(&STARTER_FILES).await;
    let directory = library_directory(&running);
    let file_bytes = 40 * MIB as u64;
    sparse_file(&directory.join("big.mp4"), b"", file_bytes);
    let head = send(&running, "HEAD", "big.mp4", "").await;
    assert_eq!(head.status(), "200");
    assert_eq!(
        head.header("content-length"),
        Some(file_bytes.to_string().as_str())
    );
    assert_eq!(head.header("content-type"), Some("video/mp4"));
    assert_eq!(head.header("accept-ranges"), Some("bytes"));
    assert!(head.header("etag").is_some());
    assert!(head.body.is_empty());
    let get_tag = fetch(&running, "big.mp4", "Range: bytes=0-9\r\n")
        .await
        .header("etag")
        .map(str::to_owned);
    assert_eq!(head.header("etag").map(str::to_owned), get_tag);
    let ranged = send(&running, "HEAD", "big.mp4", "Range: bytes=0-9\r\n").await;
    assert_eq!(ranged.status(), "206");
    assert_eq!(ranged.header("content-length"), Some("10"));
    assert_eq!(
        ranged.header("content-range"),
        Some(format!("bytes 0-9/{file_bytes}").as_str())
    );
    assert!(ranged.body.is_empty());
}
