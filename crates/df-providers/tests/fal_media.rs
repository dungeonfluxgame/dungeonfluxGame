#![cfg(not(target_arch = "wasm32"))]

use std::{
    future,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use df_model::checkpoint::{AssetKind, AssetReference, Basis, ContentDigest, ExecutionMode, JobId};
use df_provider_api::{
    ImageEventKind, ImageSemantics, RequestBasis, RequestBinding, RequestIdentity, RequestLimits,
    RequestModality, RequestOwnerState, RequestUsage, SoundEventKind, SoundSemantics,
    VideoEventKind, VideoSemantics,
};
use df_providers::{
    EncodedMediaFacts, FalCancellationObservation, FalCdnPin, FalImageProfile, FalImageRequest,
    FalLoopFrames, FalMediaConfig, FalMediaFamily, FalMediaReason, FalQueueObservation,
    FalReferenceFrame, FalSoundProfile, FalSoundRequest, FalVideoProfile, FalVideoRequest,
    MediaTimebase, NativeFalMediaProvider, NativeHttpLimits, NativeHttpRefusal,
    registered_fal_media_provider,
};
use df_types::{
    OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage, UsageUnit,
};
use serde_json::{Value, json};

type ImageBasis = RequestBinding<RequestBasis<ImageSemantics<String, FalImageProfile>>>;
type VideoBasis =
    RequestBinding<RequestBasis<VideoSemantics<String, FalVideoProfile, FalReferenceFrame>>>;
type SoundBasis =
    RequestBinding<RequestBasis<SoundSemantics<String, FalSoundProfile, Option<FalLoopFrames>>>>;

const ID: &str = "media_fixture_01";
const PNG_URL: &str = "https://v3.fal.media/output/image.png";
const VIDEO_URL: &str = "https://v3.fal.media/output/video.mp4";
const SOUND_URL: &str = "https://v3.fal.media/output/audio.wav";

#[derive(Clone)]
struct Reply {
    status: u16,
    mime: &'static str,
    body: Vec<u8>,
    extra_headers: &'static str,
    declared: Option<usize>,
    chunked: bool,
    terminal: bool,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            mime: "application/json",
            body: serde_json::to_vec(&value).unwrap(),
            extra_headers: "",
            declared: None,
            chunked: false,
            terminal: true,
        }
    }
    fn binary(mime: &'static str, body: Vec<u8>) -> Self {
        Self {
            mime,
            body,
            ..Self::json(json!({}))
        }
    }
}

#[derive(Debug)]
struct Captured {
    head: String,
    body: Vec<u8>,
}
struct Fixture {
    address: SocketAddr,
    captured: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let output = captured.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let join = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut replies = replies.into_iter();
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::yield_now();
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut data = Vec::new();
                let end = loop {
                    let mut block = [0; 1024];
                    let count = socket.read(&mut block).unwrap();
                    assert!(count > 0 && data.len() + count <= 65_536);
                    data.extend_from_slice(&block[..count]);
                    if let Some(end) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8(data[..end].to_vec()).unwrap();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while data.len() - end < length {
                    let mut block = [0; 1024];
                    let count = socket.read(&mut block).unwrap();
                    assert!(count > 0 && data.len() + count <= 65_536);
                    data.extend_from_slice(&block[..count]);
                }
                output.lock().unwrap().push(Captured {
                    head,
                    body: data[end..].to_vec(),
                });
                let reply = replies.next().expect("unexpected repeated request");
                let framing = if reply.chunked {
                    "Transfer-Encoding: chunked\r\n".to_owned()
                } else {
                    format!(
                        "Content-Length: {}\r\n",
                        reply.declared.unwrap_or(reply.body.len())
                    )
                };
                let header = format!(
                    "HTTP/1.1 {} fixture\r\nContent-Type: {}\r\n{}{}Connection: close\r\n\r\n",
                    reply.status, reply.mime, framing, reply.extra_headers
                );
                if socket.write_all(header.as_bytes()).is_ok() {
                    if reply.chunked {
                        for chunk in reply.body.chunks(17) {
                            if write!(socket, "{:x}\r\n", chunk.len())
                                .and_then(|()| socket.write_all(chunk))
                                .and_then(|()| socket.write_all(b"\r\n"))
                                .is_err()
                            {
                                break;
                            }
                        }
                        if reply.terminal {
                            let _ = socket.write_all(b"0\r\n\r\n");
                        }
                    } else {
                        let _ = socket.write_all(&reply.body);
                    }
                }
            }
        });
        Self {
            address,
            captured,
            stop,
            join: Some(join),
        }
    }
    fn finish(&mut self) -> Vec<Captured> {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
        std::mem::take(&mut *self.captured.lock().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            if !thread::panicking() {
                join.join().unwrap();
            } else {
                let _ = join.join();
            }
        }
    }
}

fn limits() -> NativeHttpLimits {
    NativeHttpLimits {
        connect_timeout: Duration::from_secs(2),
        read_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(4),
        maximum_request_bytes: 16_384,
        maximum_response_bytes: 2_097_152,
        maximum_header_bytes: 4096,
        maximum_chunks: 65_536,
    }
}
fn provider(family: FalMediaFamily, fixture: &Fixture) -> NativeFalMediaProvider {
    registered_fal_media_provider(
        FalMediaConfig::loopback_fixture(family, fixture.address, fixture.address).unwrap(),
        "fixture-private-key",
        limits(),
    )
    .unwrap()
}
fn binding<S>(modality: RequestModality, semantic: S) -> RequestBinding<RequestBasis<S>> {
    RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(&[1; 16]).unwrap(),
                run: RunId::from_bytes(&[2; 16]).unwrap(),
                revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 2),
            },
            job: JobId::from_bytes(&[3; 16]).unwrap(),
            operation: OperationId::from_bytes(&[4; 16]).unwrap(),
            generation: 5,
        },
        semantic_basis: RequestBasis {
            modality,
            source: ContentDigest([6; 32]),
            rights_revision: RevisionLabel::new(Some("fixture-rights")).unwrap(),
            output_format_revision: RevisionLabel::new(Some("fixture-format")).unwrap(),
            semantic,
        },
        mode: ExecutionMode::Live,
        deadline: Duration::from_secs(8),
    }
}
fn owner<S>(binding: &RequestBinding<RequestBasis<S>>) -> RequestOwnerState<'_, RequestBasis<S>> {
    RequestOwnerState {
        current: Some(binding),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}
fn image_basis() -> ImageBasis {
    binding(
        RequestModality::Image,
        ImageSemantics {
            locale: None,
            prompt: "A lantern-lit hall".into(),
            references: Vec::new(),
            output_profile: FalImageProfile {
                model: "fal-ai/flux/schnell".into(),
                width: 2,
                height: 2,
                images: 1,
                steps: 4,
                seed: 7,
                safety: true,
                format: "png".into(),
            },
        },
    )
}
fn video_basis() -> VideoBasis {
    binding(
        RequestModality::Video,
        VideoSemantics {
            locale: None,
            prompt: "The hall camera moves slowly".into(),
            reference_frames: vec![FalReferenceFrame {
                asset: AssetReference {
                    key: RevisionLabel::new(Some("frame")).unwrap(),
                    digest: ContentDigest([8; 32]),
                    byte_length: 100,
                    kind: AssetKind::Image,
                },
                locator: PNG_URL.into(),
            }],
            output_profile: FalVideoProfile {
                model: "fal-ai/wan-25-preview/image-to-video".into(),
                duration_seconds: 5,
                resolution: "480p".into(),
                width: 640,
                height: 480,
                accepted_duration_min: Duration::from_secs(5),
                accepted_duration_max: Duration::from_secs(5),
                seed: 7,
                safety: true,
                prompt_expansion: false,
            },
        },
    )
}
fn sound_basis() -> SoundBasis {
    binding(
        RequestModality::Sound,
        SoundSemantics {
            locale: None,
            intent: "Quiet hall ambience".into(),
            loop_metadata: Some(FalLoopFrames {
                first: 0,
                end_exclusive: 8000,
            }),
            output_profile: FalSoundProfile {
                model: "fal-ai/stable-audio-25/text-to-audio".into(),
                duration_seconds: 1,
                steps: 50,
                seed: 7,
                safety: true,
                sample_rate: 8000,
                channels: 1,
                bits_per_sample: 16,
                accepted_duration_min: Duration::from_secs(1),
                accepted_duration_max: Duration::from_secs(1),
            },
        },
    )
}
fn image_request(current: &ImageBasis) -> FalImageRequest {
    FalImageRequest::new(
        binding(
            RequestModality::Image,
            current.semantic_basis.semantic.clone(),
        ),
        b"",
        RequestUsage::new(
            0,
            Duration::ZERO,
            Usage::new(
                u128::from(current.semantic_basis.semantic.output_profile.images),
                UsageUnit::Image,
            ),
        ),
        RequestLimits::new(16_384, 0, Duration::ZERO, Usage::new(4, UsageUnit::Image)).unwrap(),
        owner(current),
    )
    .unwrap()
}
fn video_request(current: &VideoBasis) -> FalVideoRequest {
    FalVideoRequest::new(
        binding(
            RequestModality::Video,
            current.semantic_basis.semantic.clone(),
        ),
        b"",
        RequestUsage::new(
            0,
            Duration::from_secs(5),
            Usage::new(5000, UsageUnit::VideoMillisecond),
        ),
        RequestLimits::new(
            16_384,
            0,
            Duration::from_secs(10),
            Usage::new(10_000, UsageUnit::VideoMillisecond),
        )
        .unwrap(),
        owner(current),
    )
    .unwrap()
}
fn sound_request(current: &SoundBasis) -> FalSoundRequest {
    FalSoundRequest::new(
        binding(
            RequestModality::Sound,
            current.semantic_basis.semantic.clone(),
        ),
        b"",
        RequestUsage::new(
            0,
            Duration::from_secs(1),
            Usage::new(1000, UsageUnit::AudioMillisecond),
        ),
        RequestLimits::new(
            16_384,
            0,
            Duration::from_secs(2),
            Usage::new(2000, UsageUnit::AudioMillisecond),
        )
        .unwrap(),
        owner(current),
    )
    .unwrap()
}
fn submit(family: FalMediaFamily) -> Reply {
    let model = match family {
        FalMediaFamily::Image => "fal-ai/flux/schnell",
        FalMediaFamily::Video => "fal-ai/wan-25-preview/image-to-video",
        FalMediaFamily::Sound => "fal-ai/stable-audio-25/text-to-audio",
    };
    let prefix = format!("https://queue.fal.run/{model}/requests/{ID}");
    Reply::json(
        json!({"request_id":ID,"status_url":format!("{prefix}/status"),
        "response_url":format!("{prefix}/response"),"cancel_url":format!("{prefix}/cancel")}),
    )
}
fn completed() -> Reply {
    Reply::json(json!({"request_id":ID,"status":"COMPLETED","metrics":{"inference_time":0.5}}))
}
fn image_result() -> Reply {
    Reply::json(json!({"images":[{"url":PNG_URL,"content_type":"image/png","width":2,"height":2}]}))
}
fn video_result() -> Reply {
    Reply::json(
        json!({"video":{"url":VIDEO_URL,"content_type":"video/mp4","width":640,"height":480,"duration":5.0,"num_frames":1,"fps":0.2}}),
    )
}
fn sound_result() -> Reply {
    Reply::json(json!({"audio":{"url":SOUND_URL,"content_type":"audio/wav","duration":1.0}}))
}

fn crc(data: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in data {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}
fn png_chunk(kind: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut data = (payload.len() as u32).to_be_bytes().to_vec();
    data.extend(kind);
    data.extend(payload);
    data.extend(crc(&data[4..]).to_be_bytes());
    data
}
fn png() -> Vec<u8> {
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = 2u32.to_be_bytes().to_vec();
    header.extend(2u32.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    data.extend(png_chunk(b"IHDR", &header));
    // Two RGB rows, each preceded by the PNG no-filter byte; stored DEFLATE block.
    let rows = [0u8; 14];
    let mut compressed = vec![0x78, 0x01, 0x01, 14, 0, 0xf1, 0xff];
    compressed.extend(rows);
    compressed.extend(0x000e_0001u32.to_be_bytes());
    data.extend(png_chunk(b"IDAT", &compressed));
    data.extend(png_chunk(b"IEND", &[]));
    data
}
fn wave(frames: u32) -> Vec<u8> {
    let length = frames * 2;
    let mut data = b"RIFF".to_vec();
    data.extend((36 + length).to_le_bytes());
    data.extend(b"WAVEfmt ");
    data.extend(16u32.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(8000u32.to_le_bytes());
    data.extend(16000u32.to_le_bytes());
    data.extend(2u16.to_le_bytes());
    data.extend(16u16.to_le_bytes());
    data.extend(b"data");
    data.extend(length.to_le_bytes());
    data.resize(data.len() + length as usize, 0);
    data
}
fn iso(kind: &[u8], payload: Vec<u8>) -> Vec<u8> {
    let mut data = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    data.extend(kind);
    data.extend(payload);
    data
}
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn mp4() -> Vec<u8> {
    mp4_with_mappings(false)
}
fn mp4_with_mappings(multiple: bool) -> Vec<u8> {
    let ftyp = iso(
        b"ftyp",
        [b"isom".as_slice(), &[0; 4], b"isom", b"avc1"].concat(),
    );
    let sample = [0, 0, 0, 2, 0x65, 0x80];
    let mdat = iso(b"mdat", sample.repeat(if multiple { 3 } else { 1 }));
    let offset = (ftyp.len() + 8) as u32;
    let mut tkhd = vec![0; 84];
    tkhd[3] = 3;
    tkhd[12..16].copy_from_slice(&1u32.to_be_bytes());
    tkhd[20..24].copy_from_slice(&5000u32.to_be_bytes());
    tkhd[40..76].copy_from_slice(&words(&[65536, 0, 0, 0, 65536, 0, 0, 0, 0x40000000]));
    tkhd[76..80].copy_from_slice(&(640u32 << 16).to_be_bytes());
    tkhd[80..84].copy_from_slice(&(480u32 << 16).to_be_bytes());
    let mut mdhd = words(&[0, 0, 0, 1000, 5000]);
    mdhd.extend([0; 4]);
    let mut hdlr = words(&[0, 0]);
    hdlr.extend(b"vide");
    hdlr.extend([0; 12]);
    hdlr.extend(b"Video\0");
    let mut avc1 = vec![0; 78];
    avc1[7] = 1;
    avc1[24..26].copy_from_slice(&640u16.to_be_bytes());
    avc1[26..28].copy_from_slice(&480u16.to_be_bytes());
    avc1[28..32].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1[32..36].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1[40..42].copy_from_slice(&1u16.to_be_bytes());
    avc1[74..76].copy_from_slice(&24u16.to_be_bytes());
    avc1[76..78].copy_from_slice(&0xffffu16.to_be_bytes());
    avc1.extend(iso(
        b"avcC",
        vec![
            1, 66, 0, 30, 0xff, 0xe1, 0, 4, 0x67, 66, 0, 30, 1, 0, 2, 0x68, 0x80,
        ],
    ));
    let mut stsd = words(&[0, 1]);
    stsd.extend(iso(b"avc1", avc1));
    let stbl = iso(
        b"stbl",
        [
            iso(b"stsd", stsd),
            iso(
                b"stts",
                if multiple {
                    words(&[0, 2, 1, 2000, 2, 1500])
                } else {
                    words(&[0, 1, 1, 5000])
                },
            ),
            iso(
                b"stsc",
                if multiple {
                    words(&[0, 2, 1, 2, 1, 2, 1, 1])
                } else {
                    words(&[0, 1, 1, 1, 1])
                },
            ),
            iso(
                b"stsz",
                words(&[0, sample.len() as u32, if multiple { 3 } else { 1 }]),
            ),
            iso(
                b"stco",
                if multiple {
                    words(&[0, 2, offset, offset + 12])
                } else {
                    words(&[0, 1, offset])
                },
            ),
        ]
        .concat(),
    );
    let mut dref = words(&[0, 1]);
    dref.extend(iso(b"url ", vec![0, 0, 0, 1]));
    let minf = iso(
        b"minf",
        [
            iso(b"vmhd", words(&[1, 0, 0])),
            iso(b"dinf", iso(b"dref", dref)),
            stbl,
        ]
        .concat(),
    );
    let mdia = iso(
        b"mdia",
        [iso(b"mdhd", mdhd), iso(b"hdlr", hdlr), minf].concat(),
    );
    let mut mvhd = vec![0; 100];
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&5000u32.to_be_bytes());
    let moov = iso(
        b"moov",
        [
            iso(b"mvhd", mvhd),
            iso(b"trak", [iso(b"tkhd", tkhd), mdia].concat()),
        ]
        .concat(),
    );
    [ftyp, mdat, moov].concat()
}

async fn image_completion(
    binary: Reply,
) -> Result<df_providers::FalMediaCompletion, df_providers::FalMediaError> {
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Image),
        completed(),
        image_result(),
        binary,
    ]);
    let provider = provider(FalMediaFamily::Image, &fixture);
    let current = image_basis();
    let request = image_request(&current);
    let mut job = provider
        .submit_image(&request, owner(&current), future::pending())
        .await?;
    job.status(owner(&current), future::pending()).await?;
    let prepared = job.result(owner(&current), future::pending()).await?;
    let output = prepared.complete(owner(&current));
    let captured = fixture.finish();
    assert_eq!(captured.len(), 4);
    assert!(captured[3].head.starts_with("GET /output/image.png "));
    assert!(
        !captured[3]
            .head
            .to_ascii_lowercase()
            .contains("authorization")
    );
    output
}

#[tokio::test]
async fn image_normal_maps_explicit_fields_and_finishes_actual_bytes() {
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Image),
        completed(),
        image_result(),
        Reply::binary("image/png", png()),
    ]);
    let provider = provider(FalMediaFamily::Image, &fixture);
    let current = image_basis();
    let request = image_request(&current);
    let mut job = provider
        .submit_image(&request, owner(&current), future::pending())
        .await
        .unwrap();
    assert_eq!(job.request_id().as_str(), ID);
    assert!(matches!(
        job.status(owner(&current), future::pending())
            .await
            .unwrap(),
        FalQueueObservation::Completed { .. }
    ));
    let prepared = job
        .result(owner(&current), future::pending())
        .await
        .unwrap();
    let event = prepared
        .complete(owner(&current))
        .unwrap()
        .into_image_event()
        .unwrap();
    assert_eq!(event.usage, Usage::new(1, UsageUnit::Image));
    match event.kind {
        ImageEventKind::Completed { identity, output } => {
            assert_eq!(identity, current.identity);
            assert_eq!(output[0].bytes(), png());
            assert_eq!(
                output[0].facts(),
                EncodedMediaFacts::Png {
                    width: 2,
                    height: 2
                }
            );
        }
        _ => panic!("completion required"),
    }
    let captured = fixture.finish();
    assert_eq!(captured.len(), 4);
    assert!(captured[0].head.starts_with("POST /fal-ai/flux/schnell "));
    let sent: Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(
        sent,
        json!({"prompt":"A lantern-lit hall","image_size":{"width":2,"height":2},
        "num_images":1,"output_format":"png","num_inference_steps":4,"seed":7,"enable_safety_checker":true})
    );
    assert!(
        captured[0]
            .head
            .to_ascii_lowercase()
            .contains("x-fal-no-retry: 1")
    );
    assert!(
        !captured[3]
            .head
            .to_ascii_lowercase()
            .contains("authorization")
    );
}

#[tokio::test]
async fn video_normal_uses_first_frame_and_container_clock() {
    // Two chunks transition from two samples per chunk to one; total clock stays 5s.
    let mut result = video_result();
    let mut metadata: Value = serde_json::from_slice(&result.body).unwrap();
    metadata["video"]["num_frames"] = json!(3);
    metadata["video"]["fps"] = json!(0.6);
    result.body = serde_json::to_vec(&metadata).unwrap();
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Video),
        completed(),
        result,
        Reply::binary("video/mp4", mp4_with_mappings(true)),
    ]);
    let provider = provider(FalMediaFamily::Video, &fixture);
    let current = video_basis();
    let request = video_request(&current);
    let mut job = provider
        .submit_video(&request, owner(&current), future::pending())
        .await
        .unwrap();
    job.status(owner(&current), future::pending())
        .await
        .unwrap();
    let event = job
        .result(owner(&current), future::pending())
        .await
        .unwrap()
        .complete(owner(&current))
        .unwrap()
        .into_video_event()
        .unwrap();
    assert_eq!(event.usage, Usage::new(5000, UsageUnit::VideoMillisecond));
    match event.kind {
        VideoEventKind::Completed { identity, output } => {
            assert_eq!(identity, current.identity);
            assert_eq!(
                output[0].facts(),
                EncodedMediaFacts::Mp4 {
                    width: 640,
                    height: 480,
                    frames: 3,
                    timebase: MediaTimebase {
                        ticks: 5000,
                        ticks_per_second: 1000
                    }
                }
            );
        }
        _ => panic!("completion required"),
    }
    let captured = fixture.finish();
    let sent: Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(sent["image_url"], PNG_URL);
    assert_eq!(sent["duration"], "5");
    assert_eq!(sent["resolution"], "480p");
    assert_eq!(sent["enable_prompt_expansion"], false);
    assert!(sent.get("first_frame_url").is_none());
    assert_eq!(captured.len(), 4);
}

#[tokio::test]
async fn character_clip_kling_preserves_distinct_end_frame_and_single_frame_loops() {
    let model = df_providers::KLING_O1_IMAGE_TO_VIDEO;
    for distinct_end in [false, true] {
        let prefix = format!("https://queue.fal.run/{model}/requests/{ID}");
        let mut fixture = Fixture::new(vec![Reply::json(json!({
            "request_id": ID, "status_url": format!("{prefix}/status"),
            "response_url": format!("{prefix}/response"), "cancel_url": format!("{prefix}/cancel"),
        }))]);
        let config = FalMediaConfig::loopback_fixture(
            FalMediaFamily::Video,
            fixture.address,
            fixture.address,
        )
        .unwrap()
        .with_video_model(model)
        .unwrap();
        let provider = registered_fal_media_provider(config, "fixture-key", limits()).unwrap();
        let mut current = video_basis();
        current.semantic_basis.semantic.output_profile.model = model.to_owned();
        let end_url = "https://v3.fal.media/output/fallen.png";
        if distinct_end {
            let mut end = current.semantic_basis.semantic.reference_frames[0].clone();
            end.locator = end_url.to_owned();
            end.asset.digest = ContentDigest([9; 32]);
            current.semantic_basis.semantic.reference_frames.push(end);
        }
        let request = video_request(&current);
        provider
            .submit_video(&request, owner(&current), future::pending())
            .await
            .unwrap();
        let captured = fixture.finish();
        assert_eq!(captured.len(), 1);
        let body: Value = serde_json::from_slice(&captured[0].body).unwrap();
        assert_eq!(body["start_image_url"], PNG_URL);
        assert_eq!(
            body["end_image_url"],
            if distinct_end { end_url } else { PNG_URL }
        );
        assert_eq!(body["duration"], "5");
        assert!(body.get("image_url").is_none());
    }
}

#[tokio::test]
async fn character_clip_second_reference_is_rejected_on_models_without_bookends() {
    let mut fixture = Fixture::new(Vec::new());
    let provider = provider(FalMediaFamily::Video, &fixture);
    let mut current = video_basis();
    let second = current.semantic_basis.semantic.reference_frames[0].clone();
    current
        .semantic_basis
        .semantic
        .reference_frames
        .push(second);
    let request = video_request(&current);
    let error = provider
        .submit_video(&request, owner(&current), future::pending())
        .await
        .err()
        .unwrap();
    assert_eq!(error.reason, FalMediaReason::Reference);
    assert!(fixture.finish().is_empty());
}

#[tokio::test]
async fn sound_normal_retains_pcm_frames_loop_bounds_and_usage() {
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Sound),
        completed(),
        sound_result(),
        Reply::binary("audio/wav", wave(8000)),
    ]);
    let provider = provider(FalMediaFamily::Sound, &fixture);
    let current = sound_basis();
    let request = sound_request(&current);
    let mut job = provider
        .submit_sound(&request, owner(&current), future::pending())
        .await
        .unwrap();
    job.status(owner(&current), future::pending())
        .await
        .unwrap();
    let event = job
        .result(owner(&current), future::pending())
        .await
        .unwrap()
        .complete(owner(&current))
        .unwrap()
        .into_sound_event()
        .unwrap();
    assert_eq!(event.usage, Usage::new(1000, UsageUnit::AudioMillisecond));
    match event.kind {
        SoundEventKind::Completed { output, .. } => {
            assert_eq!(
                output[0].facts(),
                EncodedMediaFacts::PcmWave {
                    sample_rate: 8000,
                    channels: 1,
                    bits_per_sample: 16,
                    frames: 8000,
                    timebase: MediaTimebase {
                        ticks: 8000,
                        ticks_per_second: 8000
                    }
                }
            );
        }
        _ => panic!("completion required"),
    }
    let captured = fixture.finish();
    let sent: Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(sent["seconds_total"], 1);
    assert_eq!(sent["num_inference_steps"], 50);
    assert!(sent.get("output_format").is_none());
    assert_eq!(captured.len(), 4);
}

#[tokio::test]
async fn invalid_profiles_payloads_and_modes_refuse_before_send() {
    let mut fixture = Fixture::new(Vec::new());
    let provider = provider(FalMediaFamily::Image, &fixture);
    for mutation in 0..5 {
        let mut current = image_basis();
        match mutation {
            0 => current.semantic_basis.semantic.output_profile.model = "other-model".into(),
            1 => current.semantic_basis.semantic.output_profile.format = "jpeg".into(),
            2 => current.semantic_basis.semantic.prompt = " ".into(),
            3 => current.semantic_basis.semantic.output_profile.steps = 0,
            _ => current.semantic_basis.semantic.output_profile.width = 0,
        }
        let request = image_request(&current);
        assert!(
            provider
                .submit_image(&request, owner(&current), future::pending())
                .await
                .is_err()
        );
    }
    let mut current = image_basis();
    current.mode = ExecutionMode::PreparedOnly;
    let request = FalImageRequest::new(
        binding_with_mode(&current),
        b"",
        RequestUsage::new(0, Duration::ZERO, Usage::new(1, UsageUnit::Image)),
        RequestLimits::new(1024, 0, Duration::ZERO, Usage::new(1, UsageUnit::Image)).unwrap(),
        owner(&current),
    )
    .unwrap();
    assert!(matches!(
        provider
            .submit_image(&request, owner(&current), future::pending())
            .await
            .unwrap_err()
            .reason,
        FalMediaReason::Profile
    ));
    assert!(fixture.finish().is_empty());
}
fn binding_with_mode(current: &ImageBasis) -> ImageBasis {
    let mut result = binding(
        RequestModality::Image,
        current.semantic_basis.semantic.clone(),
    );
    result.mode = current.mode;
    result
}

#[tokio::test]
async fn canonical_reference_count_kind_locator_and_current_digest_are_fenced() {
    let mut fixture = Fixture::new(Vec::new());
    let video_provider = provider(FalMediaFamily::Video, &fixture);
    for mutation in 0..4 {
        let mut current = video_basis();
        match mutation {
            0 => current.semantic_basis.semantic.reference_frames.clear(),
            1 => {
                let frame = current.semantic_basis.semantic.reference_frames[0].clone();
                current.semantic_basis.semantic.reference_frames.push(frame);
            }
            2 => {
                current.semantic_basis.semantic.reference_frames[0]
                    .asset
                    .kind = AssetKind::Audio
            }
            _ => {
                current.semantic_basis.semantic.reference_frames[0].locator =
                    "https://127.0.0.1/frame.png".into()
            }
        }
        let request = video_request(&current);
        assert!(matches!(
            video_provider
                .submit_video(&request, owner(&current), future::pending())
                .await
                .unwrap_err()
                .reason,
            FalMediaReason::Reference
        ));
    }
    let mut current = video_basis();
    let request = video_request(&current);
    current.semantic_basis.semantic.reference_frames[0]
        .asset
        .digest = ContentDigest([9; 32]);
    assert!(matches!(
        video_provider
            .submit_video(&request, owner(&current), future::pending())
            .await
            .unwrap_err()
            .reason,
        FalMediaReason::Owner(df_provider_api::RequestError::SemanticBasisMismatch)
    ));
    let image_provider = provider(FalMediaFamily::Image, &fixture);
    let mut current = image_basis();
    current.semantic_basis.semantic.references.push(
        video_basis().semantic_basis.semantic.reference_frames[0]
            .asset
            .clone(),
    );
    let request = image_request(&current);
    assert!(matches!(
        image_provider
            .submit_image(&request, owner(&current), future::pending())
            .await
            .unwrap_err()
            .reason,
        FalMediaReason::Reference
    ));
    assert!(fixture.finish().is_empty());
}

#[tokio::test]
async fn owner_identity_cancel_and_deadline_fence_each_action_and_final_consume() {
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Image),
        completed(),
        image_result(),
        Reply::binary("image/png", png()),
    ]);
    let provider = provider(FalMediaFamily::Image, &fixture);
    let mut current = image_basis();
    let request = image_request(&current);
    let mut job = provider
        .submit_image(&request, owner(&current), future::pending())
        .await
        .unwrap();
    current.identity.generation += 1;
    assert!(matches!(
        job.status(owner(&current), future::pending())
            .await
            .unwrap_err()
            .reason,
        FalMediaReason::Owner(_)
    ));
    current.identity.generation -= 1;
    job.status(owner(&current), future::pending())
        .await
        .unwrap();
    let prepared = job
        .result(owner(&current), future::pending())
        .await
        .unwrap();
    let mut cancelled = owner(&current);
    cancelled.cancelled = true;
    let failure = prepared.complete(cancelled).unwrap_err();
    assert_eq!(failure.request_id.unwrap().as_str(), ID);
    assert_eq!(
        failure.observed_usage,
        Some(Usage::new(1, UsageUnit::Image))
    );
    let mut expired = owner(&current);
    expired.elapsed = current.deadline;
    assert!(matches!(
        provider
            .submit_image(&request, expired, future::pending())
            .await
            .unwrap_err()
            .reason,
        FalMediaReason::Owner(_)
    ));
    assert_eq!(fixture.finish().len(), 4);
}

#[tokio::test]
async fn captured_queue_urls_refuse_cross_origin_id_and_substituted_paths() {
    for mutation in 0..4 {
        let mut reply = submit(FalMediaFamily::Image);
        let mut value: Value = serde_json::from_slice(&reply.body).unwrap();
        match mutation {
            0 => value["request_id"] = json!("invalid/id"),
            1 => value["response_url"] = json!("https://attacker.invalid/response"),
            2 => {
                value["status_url"] =
                    json!("https://queue.fal.run/fal-ai/flux/schnell/requests/other/status")
            }
            _ => {
                value["cancel_url"] = json!(format!(
                    "https://queue.fal.run/fal-ai/flux/schnell/requests/{ID}/cancel?x=1"
                ))
            }
        }
        reply.body = serde_json::to_vec(&value).unwrap();
        let mut fixture = Fixture::new(vec![reply]);
        let provider = provider(FalMediaFamily::Image, &fixture);
        let current = image_basis();
        let request = image_request(&current);
        assert!(
            provider
                .submit_image(&request, owner(&current), future::pending())
                .await
                .is_err()
        );
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test]
async fn pending_failed_unknown_status_never_publishes_result() {
    for status in ["IN_QUEUE", "IN_PROGRESS", "COMPLETED", "unrecognized"] {
        let mut value = json!({"request_id":ID,"status":status});
        if status == "COMPLETED" {
            value["error"] = json!("supplier failure");
        }
        let mut fixture = Fixture::new(vec![submit(FalMediaFamily::Image), Reply::json(value)]);
        let provider = provider(FalMediaFamily::Image, &fixture);
        let current = image_basis();
        let request = image_request(&current);
        let mut job = provider
            .submit_image(&request, owner(&current), future::pending())
            .await
            .unwrap();
        let result = job
            .status(owner(&current), future::pending())
            .await
            .unwrap();
        match status {
            "IN_QUEUE" | "IN_PROGRESS" => {
                assert!(matches!(result, FalQueueObservation::Pending(_)))
            }
            "COMPLETED" => assert!(matches!(result, FalQueueObservation::Failed { .. })),
            _ => assert!(matches!(result, FalQueueObservation::Unknown(_))),
        }
        assert!(matches!(
            job.result(owner(&current), future::pending())
                .await
                .unwrap_err()
                .reason,
            FalMediaReason::NotCompleted
        ));
        assert_eq!(fixture.finish().len(), 2);
    }
}

#[tokio::test]
async fn cancellation_uses_put_and_keeps_ambiguous_remote_liability() {
    for (http, status, expected) in [
        (
            202,
            "CANCELLATION_REQUESTED",
            FalCancellationObservation::Requested,
        ),
        (
            400,
            "ALREADY_COMPLETED",
            FalCancellationObservation::AlreadyCompleted,
        ),
        (404, "NOT_FOUND", FalCancellationObservation::NotFound),
    ] {
        let mut cancel = Reply::json(json!({"request_id":ID,"status":status}));
        cancel.status = http;
        let mut fixture = Fixture::new(vec![submit(FalMediaFamily::Image), cancel]);
        let provider = provider(FalMediaFamily::Image, &fixture);
        let current = image_basis();
        let request = image_request(&current);
        let mut job = provider
            .submit_image(&request, owner(&current), future::pending())
            .await
            .unwrap();
        assert_eq!(
            job.cancel(owner(&current), future::pending())
                .await
                .unwrap(),
            expected
        );
        assert_eq!(job.request_id().as_str(), ID);
        drop(job);
        let captured = fixture.finish();
        assert_eq!(captured.len(), 2);
        assert!(
            captured[1]
                .head
                .starts_with(&format!("PUT /fal-ai/flux/schnell/requests/{ID}/cancel "))
        );
    }
}

#[tokio::test]
async fn json_requires_unique_complete_eof_and_classifies_transport_failures() {
    for mutation in 0..7 {
        let mut reply = submit(FalMediaFamily::Image);
        match mutation {
            0 => reply.body.extend(b" trailing"),
            1 => reply.body = br#"{"request_id":"first","request_id":"second"}"#.to_vec(),
            2 => reply.mime = "text/html",
            3 => {
                reply.status = 302;
                reply.extra_headers = "Location: https://attacker.invalid/\r\n";
            }
            4 => {
                reply.chunked = true;
                reply.terminal = false;
            }
            5 => reply.status = 429,
            _ => reply.status = 503,
        }
        let mut fixture = Fixture::new(vec![reply]);
        let provider = provider(FalMediaFamily::Image, &fixture);
        let current = image_basis();
        let request = image_request(&current);
        let failure = provider
            .submit_image(&request, owner(&current), future::pending())
            .await
            .unwrap_err();
        assert_ne!(
            failure.failure_class(),
            df_providers::ProviderFailureClass::Cancelled
        );
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[tokio::test]
async fn binary_acquisition_rejects_partial_size_mime_redirect_and_credentials() {
    for mutation in 0..5 {
        let mut reply = Reply::binary("image/png", png());
        match mutation {
            0 => reply.declared = Some(reply.body.len() + 1),
            1 => reply.mime = "image/jpeg",
            2 => {
                reply.status = 302;
                reply.extra_headers = "Location: http://127.0.0.1:1/\r\n";
            }
            3 => reply.body.clear(),
            _ => {
                reply.chunked = true;
                reply.terminal = false;
            }
        }
        assert!(image_completion(reply).await.is_err());
    }
    assert!(FalCdnPin::new("v3.fal.media", &["127.0.0.1".parse().unwrap()]).is_err());
    assert!(FalCdnPin::new("other.invalid", &["8.8.8.8".parse().unwrap()]).is_err());
    assert!(FalCdnPin::new("v3.fal.media", &["8.8.8.8".parse().unwrap()]).is_ok());
}

#[tokio::test]
async fn png_crc_critical_animation_truncation_trailing_and_dimensions_are_checked() {
    for mutation in 0..6 {
        let mut data = png();
        match mutation {
            0 => data[29] ^= 1,
            1 => {
                let chunk = png_chunk(b"ABCD", &[]);
                data.splice(33..33, chunk);
            }
            2 => {
                let chunk = png_chunk(b"acTL", &[0; 8]);
                data.splice(33..33, chunk);
            }
            3 => {
                data.pop();
            }
            4 => data.push(0),
            _ => {
                let mut header = 3u32.to_be_bytes().to_vec();
                header.extend(2u32.to_be_bytes());
                header.extend([8, 2, 0, 0, 0]);
                data.splice(8..33, png_chunk(b"IHDR", &header));
            }
        }
        assert!(
            image_completion(Reply::binary("image/png", data))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn mp4_external_refs_sample_ranges_rotation_fragmentation_and_terminal_bytes_refuse() {
    for mutation in 0..6 {
        let mut data = mp4();
        match mutation {
            0 => {
                let p = data.windows(4).position(|w| w == b"url ").unwrap();
                data[p + 7] = 0;
            }
            1 => {
                let p = data.windows(4).position(|w| w == b"stco").unwrap();
                data[p + 12..p + 16].copy_from_slice(&0u32.to_be_bytes());
            }
            2 => {
                let p = data.windows(4).position(|w| w == b"tkhd").unwrap();
                data[p + 44..p + 48].copy_from_slice(&0u32.to_be_bytes());
            }
            3 => data.extend(iso(b"moof", Vec::new())),
            4 => {
                data.pop();
            }
            _ => data.push(0),
        }
        let mut fixture = Fixture::new(vec![
            submit(FalMediaFamily::Video),
            completed(),
            video_result(),
            Reply::binary("video/mp4", data),
        ]);
        let provider = provider(FalMediaFamily::Video, &fixture);
        let current = video_basis();
        let request = video_request(&current);
        let mut job = provider
            .submit_video(&request, owner(&current), future::pending())
            .await
            .unwrap();
        job.status(owner(&current), future::pending())
            .await
            .unwrap();
        assert!(
            job.result(owner(&current), future::pending())
                .await
                .is_err()
        );
        assert_eq!(fixture.finish().len(), 4);
    }
}

#[tokio::test]
async fn wave_alignment_compression_duplicate_truncation_duration_and_loop_refuse() {
    for mutation in 0..7 {
        let mut data = wave(8000);
        let mut current = sound_basis();
        match mutation {
            0 => data[20] = 3,
            1 => data[32] = 3,
            2 => {
                let fmt = data[12..36].to_vec();
                data.splice(36..36, fmt);
                let size = (data.len() - 8) as u32;
                data[4..8].copy_from_slice(&size.to_le_bytes());
            }
            3 => {
                data.pop();
            }
            4 => data.push(0),
            5 => data = wave(7999),
            _ => {
                current.semantic_basis.semantic.loop_metadata = Some(FalLoopFrames {
                    first: 0,
                    end_exclusive: 8001,
                })
            }
        }
        let mut fixture = Fixture::new(vec![
            submit(FalMediaFamily::Sound),
            completed(),
            sound_result(),
            Reply::binary("audio/wav", data),
        ]);
        let provider = provider(FalMediaFamily::Sound, &fixture);
        let request = sound_request(&current);
        let mut job = provider
            .submit_sound(&request, owner(&current), future::pending())
            .await
            .unwrap();
        job.status(owner(&current), future::pending())
            .await
            .unwrap();
        assert!(
            job.result(owner(&current), future::pending())
                .await
                .is_err()
        );
        assert_eq!(fixture.finish().len(), 4);
    }
}

#[tokio::test]
async fn actual_usage_ceil_and_overrun_are_retained_without_clipping() {
    assert_eq!(
        MediaTimebase {
            ticks: 8001,
            ticks_per_second: 8000
        }
        .milliseconds_ceil()
        .unwrap(),
        1001
    );
    let mut current = sound_basis();
    current
        .semantic_basis
        .semantic
        .output_profile
        .accepted_duration_max = Duration::from_millis(1100);
    let request = FalSoundRequest::new(
        binding(
            RequestModality::Sound,
            current.semantic_basis.semantic.clone(),
        ),
        b"",
        RequestUsage::new(
            0,
            Duration::from_secs(1),
            Usage::new(1000, UsageUnit::AudioMillisecond),
        ),
        RequestLimits::new(
            1024,
            0,
            Duration::from_secs(1),
            Usage::new(1000, UsageUnit::AudioMillisecond),
        )
        .unwrap(),
        owner(&current),
    )
    .unwrap();
    let mut result = sound_result();
    let mut value: Value = serde_json::from_slice(&result.body).unwrap();
    value["audio"].as_object_mut().unwrap().remove("duration");
    result.body = serde_json::to_vec(&value).unwrap();
    let mut fixture = Fixture::new(vec![
        submit(FalMediaFamily::Sound),
        completed(),
        result,
        Reply::binary("audio/wav", wave(8001)),
    ]);
    let provider = provider(FalMediaFamily::Sound, &fixture);
    let mut job = provider
        .submit_sound(&request, owner(&current), future::pending())
        .await
        .unwrap();
    job.status(owner(&current), future::pending())
        .await
        .unwrap();
    let failure = job
        .result(owner(&current), future::pending())
        .await
        .unwrap_err();
    assert_eq!(failure.reason, FalMediaReason::UsageOverrun);
    assert_eq!(
        failure.observed_usage,
        Some(Usage::new(1001, UsageUnit::AudioMillisecond))
    );
    assert_eq!(failure.request_id.unwrap().as_str(), ID);
    assert_eq!(fixture.finish().len(), 4);
}

#[tokio::test]
async fn dropped_job_and_cancelled_future_never_resubmit_or_fetch() {
    let mut fixture = Fixture::new(vec![submit(FalMediaFamily::Image)]);
    let provider = provider(FalMediaFamily::Image, &fixture);
    let current = image_basis();
    let request = image_request(&current);
    let job = provider
        .submit_image(&request, owner(&current), future::pending())
        .await
        .unwrap();
    assert_eq!(job.request_id().as_str(), ID);
    drop(job);
    let failed = provider
        .submit_image(&request, owner(&current), future::ready(()))
        .await
        .unwrap_err();
    assert_eq!(
        failed.reason,
        FalMediaReason::Transport(NativeHttpRefusal::Cancelled)
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[tokio::test]
async fn native_registry_hook_redacts_private_fields_and_keeps_dated_routes() {
    let mut fixture = Fixture::new(Vec::new());
    let provider = provider(FalMediaFamily::Image, &fixture);
    assert!(!format!("{provider:?}").contains("fixture-private-key"));
    let current = image_basis();
    let request = image_request(&current);
    let mut cancelled = owner(&current);
    cancelled.cancelled = true;
    let failure = provider
        .submit_image(&request, cancelled, future::pending())
        .await
        .unwrap_err();
    assert!(!format!("{failure:?}").contains("A lantern-lit hall"));
    assert_eq!(df_providers::candidate_routes().len(), 2);
    assert!(df_providers::candidate_for(df_providers::Capability::Video).is_err());
    assert!(fixture.finish().is_empty());
}

#[tokio::test]
async fn a_refused_submit_keeps_the_http_status_and_a_redacted_bounded_body() {
    let opaque = "A".repeat(200);
    let mut refused = Reply::json(json!({"detail": [{
        "msg": "image rejected, credential fixture-private-key",
        "input": opaque,
        "loc": ["body", "start_image_url"],
    }]}));
    refused.status = 422;
    let fixture = Fixture::new(vec![refused]);
    let provider = provider(FalMediaFamily::Video, &fixture);
    let current = video_basis();
    let request = video_request(&current);

    let error = provider
        .submit_video(&request, owner(&current), future::pending())
        .await
        .expect_err("a 422 answer is a refusal");

    assert_eq!(
        error.reason,
        FalMediaReason::Status,
        "a refused submit is a status failure, not a queue reply that lost its identity"
    );
    let http = error.http.expect("the supplier's answer is kept");
    assert_eq!(http.status, 422);
    assert!(http.body.contains("start_image_url"), "{}", http.body);
    assert!(http.body.contains("image rejected"), "{}", http.body);
    assert!(!http.body.contains("fixture-private-key"), "{}", http.body);
    assert!(http.body.contains("[credential]"), "{}", http.body);
    assert!(http.body.contains("[opaque]"), "{}", http.body);
    assert!(!http.body.contains("AAAAAAAA"), "{}", http.body);
    assert!(http.body.chars().count() <= 400);
}

#[test]
fn provider_limits_build_the_transport_limits_with_the_shared_defaults() {
    let limits = df_providers::ProviderLimits::new(Duration::from_secs(30), 1024, 4096).build();

    assert_eq!(limits.total_timeout, Duration::from_secs(30));
    assert_eq!(limits.read_timeout, Duration::from_secs(30));
    assert_eq!(limits.connect_timeout, Duration::from_secs(5));
    assert_eq!(limits.maximum_request_bytes, 1024);
    assert_eq!(limits.maximum_response_bytes, 4096);
    assert_eq!(limits.maximum_header_bytes, 16 * 1024);
    assert_eq!(limits.maximum_chunks, 4096);
    let short = df_providers::ProviderLimits::new(Duration::from_secs(2), 1, 1)
        .read_timeout(Duration::from_secs(1))
        .chunks(8)
        .build();
    assert_eq!(
        short.connect_timeout,
        Duration::from_secs(2),
        "never longer than the call"
    );
    assert_eq!(short.read_timeout, Duration::from_secs(1));
    assert_eq!(short.maximum_chunks, 8);
}
