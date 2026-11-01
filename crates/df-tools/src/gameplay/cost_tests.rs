use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use df_protocol::common as rpc;
use df_providers::{NativeHttpRefusal, ProviderFailureClass, SpeechAdapterReason, TextTokenUsage};
use serde_json::json;

use super::*;
use crate::gameplay::resources::SpendLedger;
use crate::voice_line::{VoiceLine, display_lines, fight_voice_line};
use df_commerce::tariff;

const TABLE: &str = r#"{
  "schema": "dungeonflux.pricing/1",
  "currency": "USD",
  "prices": [
    {"provider": "openai", "model": "gpt-6-luna", "category": "text_input", "usd": "0.10", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-6-luna", "category": "text_cached_input", "usd": "0.01", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-6-luna", "category": "text_cache_write", "usd": "0.125", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-6-luna", "category": "text_output", "usd": "0.50", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "text_input", "usd": "5", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_input", "usd": "8", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_output", "usd": "30", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "elevenlabs", "model": "eleven_flash_v2_5", "category": "characters", "usd": "0.04", "per": 1000, "as_of": "2026-10-24", "source": "https://example.test/p", "note": "flash"}
  ]
}"#;

/// A Dungeon Master fight line, built as the server builds it.
fn narration() -> VoiceLine {
    fight_voice_line(&rpc::FightLine {
        audio_url: String::new(),
        audio_part_urls: Vec::new(),
        role: rpc::TableRole::Narrator as i32,
        hero_index: 0,
        speaker: String::new(),
        text: "The torches gutter.".to_owned(),
    })
    .expect("a narration line")
}

/// A hero's battle cry, voiced as its named speaker.
fn hero_cry(hero: &str) -> VoiceLine {
    fight_voice_line(&rpc::FightLine {
        audio_url: String::new(),
        audio_part_urls: Vec::new(),
        role: rpc::TableRole::Hero as i32,
        hero_index: 1,
        speaker: hero.to_owned(),
        text: "For the harbour!".to_owned(),
    })
    .expect("a hero cry")
}

/// A cast member's conversation reply, read from the display's table log as the server reads it.
fn cast_reply(actor: &str, name: &str) -> VoiceLine {
    let display = rpc::DisplayGameplayView {
        journey: Some(rpc::JourneyView {
            table_log: vec![rpc::TableLine {
                audio_url: String::new(),
                audio_part_urls: Vec::new(),
                sequence: 1,
                actor_id: actor.to_owned(),
                speaker: name.to_owned(),
                text: "Well met.".to_owned(),
                role: rpc::TableRole::Npc as i32,
                kind: rpc::TableKind::Speech as i32,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut lines = display_lines(&display);
    assert_eq!(lines.len(), 1, "one reply line");
    lines.pop().expect("a reply line")
}

fn table() -> PriceTable {
    PriceTable::parse(TABLE).expect("the test table parses")
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("df-cost-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

fn opened_book(session: &str, log: Option<&std::path::Path>) -> CostBook {
    let book = CostBook::with_session(session);
    book.open(
        "df_gameplay_demo_test",
        log,
        table(),
        PricingStatus::NotLoaded,
        1_000,
    )
    .expect("the book opens");
    book
}

fn opened_ledger(log: Option<&std::path::Path>) -> Arc<SpendLedger> {
    let ledger = Arc::new(SpendLedger::default());
    ledger
        .costs()
        .open(
            "df_gameplay_demo_test",
            log,
            table(),
            PricingStatus::NotLoaded,
            1_000,
        )
        .expect("the book opens");
    ledger
}

#[derive(Clone, Copy)]
enum CostImageReply {
    Reported { text: u64, input: u64, output: u64 },
    Silent,
    Refused(u16),
    AmbiguousRateLimit,
    Quota,
    Unknown,
}

struct CostImageEndpoint {
    address: std::net::SocketAddr,
    reply: Arc<Mutex<CostImageReply>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl CostImageEndpoint {
    fn new(width: u32, height: u32, reply: CostImageReply) -> Self {
        use image::ImageEncoder;
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let reply = Arc::new(Mutex::new(reply));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (replies, captured, stopped) = (reply.clone(), requests.clone(), stop.clone());
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &vec![100; (width * height * 3) as usize],
                width,
                height,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &vec![100; (width * height * 4) as usize],
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let (jpeg_encoded, png_encoded) = (cost_base64(&jpeg), cost_base64(&png));
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            while !stopped.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("cost fixture accept: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() <= 4 * 1024 * 1024);
                    if let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..split]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&bytes).into_owned();
                let line = request.lines().next().unwrap().to_owned();
                {
                    let mut seen = captured.lock().unwrap();
                    assert!(seen.len() < 256, "cost fixture request bound exceeded");
                    seen.push(line.clone());
                }
                let mode = *replies.lock().unwrap();
                if line.starts_with("POST ") {
                    if matches!(mode, CostImageReply::Unknown) {
                        continue;
                    }
                    if let CostImageReply::Refused(_)
                    | CostImageReply::AmbiguousRateLimit
                    | CostImageReply::Quota = mode
                    {
                        let status = if let CostImageReply::Refused(status) = mode {
                            status
                        } else {
                            429
                        };
                        let body = if matches!(mode, CostImageReply::Quota) {
                            r#"{"error":{"type":"insufficient_quota","code":"insufficient_quota"}}"#
                        } else if matches!(mode, CostImageReply::AmbiguousRateLimit) {
                            r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"},"request_id":"accepted-job"}"#
                        } else if status == 429 {
                            r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}"#
                        } else if status >= 500 {
                            r#"{"error":{"type":"server_error","code":"server_error"}}"#
                        } else {
                            r#"{"error":{"type":"invalid_request_error"}}"#
                        };
                        let retry_after = if matches!(mode, CostImageReply::Refused(429)) {
                            "Retry-After: 0\r\n"
                        } else {
                            ""
                        };
                        write!(stream,"HTTP/1.1 {status} Refused\r\nContent-Type: application/json\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                        continue;
                    }
                }
                let path = line.split(' ').nth(1).unwrap();
                if path == "/image.jpg" {
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",jpeg.len()).unwrap();
                    stream.write_all(&jpeg).unwrap();
                    continue;
                }
                let body = if path.starts_with("/v1/images/") {
                    let mut body = json!({"data":[{"b64_json":jpeg_encoded}]});
                    if let CostImageReply::Reported {
                        text,
                        input,
                        output,
                    } = mode
                    {
                        body["usage"] = json!({"input_tokens_details":{"text_tokens":text,"image_tokens":input},"output_tokens":output});
                    }
                    body.to_string()
                } else if path == "/v1/flux-3-image" {
                    json!({"polling_url":format!("http://{address}/v1/get_result?id=cost-bfl"),"cost":3}).to_string()
                } else if path.starts_with("/v1/get_result?") {
                    json!({"status":"Ready","result":{"sample":format!("http://{address}/image.jpg")}}).to_string()
                } else if path.starts_with("/fal-ai/") && line.starts_with("POST ") {
                    json!({"request_id":"cost-fal","status_url":format!("https://queue.fal.run{path}/requests/cost-fal/status"),"response_url":format!("https://queue.fal.run{path}/requests/cost-fal/response")}).to_string()
                } else if path.starts_with("/fal-ai/") && path.ends_with("/status") {
                    r#"{"status":"COMPLETED"}"#.to_owned()
                } else if path.starts_with("/fal-ai/") && path.ends_with("/response") {
                    let uri = if path.contains("birefnet") {
                        format!("data:image/png;base64,{png_encoded}")
                    } else {
                        format!("data:image/jpeg;base64,{jpeg_encoded}")
                    };
                    json!({"images":[{"url":uri}]}).to_string()
                } else {
                    panic!("unexpected cost fixture request: {line}")
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            address,
            reply,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn set(&self, reply: CostImageReply) {
        *self.reply.lock().unwrap() = reply;
    }
    fn posts(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .cloned()
            .collect()
    }
}
impl Drop for CostImageEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
fn cost_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        result.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn cost_image_patch(
    address: std::net::SocketAddr,
    backend: df_imagery::BackendId,
    width: u32,
    height: u32,
    quality: df_imagery::GenerationQuality,
    alpha: bool,
) -> df_imagery::RuntimePatch {
    use df_imagery::*;
    let mut patch = RuntimePatch::empty();
    patch.mode = Some(GlobalMode::Live);
    patch.default_backend = Some(backend);
    patch.accounting_mode = Some(AccountingMode::RecordOnly);
    patch.backend_enabled = [Some(false); BackendId::COUNT];
    patch.backend_enabled[backend.index()] = Some(true);
    patch.backend_endpoint[backend.index()] = Some(Some(
        ApiEndpoint::new(&format!("http://{address}")).unwrap(),
    ));
    patch.quality_provider_quality = Some(Some(quality));
    patch.retry_max_attempts = Some(1);
    for purpose in [
        Purpose::ScenePlate,
        Purpose::ItemCloseUp,
        Purpose::Expression,
        Purpose::AnimationFrame,
    ] {
        patch.purpose_model_override[purpose.index()] = Some(None);
    }
    let dimensions = Dimensions::new(width, height).unwrap();
    let delivered = RasterOutputSpec::new(
        dimensions,
        if alpha {
            AlphaHandling::PreserveAlpha
        } else {
            AlphaHandling::Opaque
        },
        if alpha {
            Encoding::Png
        } else {
            Encoding::Jpeg {
                quality: LossyQuality::new(90).unwrap(),
            }
        },
    )
    .unwrap();
    let name = OutputProfileName::new("cost-test").unwrap();
    patch.default_output_profile = Some(name.clone());
    patch.output_profiles = Some(vec![NamedOutputProfile::new(
        name,
        OutputProfile::provider(dimensions, delivered),
    )]);
    patch
}

fn assert_cost_replay(outcome: df_imagery::RequestOutcome, expected: &df_imagery::ImageDemandView) {
    let replay = match outcome {
        df_imagery::RequestOutcome::Replayed(view) => view,
        df_imagery::RequestOutcome::Reused(view)
            if expected.execution == df_imagery::ImageExecutionState::Ready =>
        {
            view
        }
        other => panic!("same immutable image operation was not retained: {other:?}"),
    };
    assert_eq!(replay.handle, expected.handle);
    assert_eq!(replay.job, expected.job);
    assert_eq!(replay.interest, expected.interest);
}

fn cost_image_request(
    context: &crate::gameplay::imagery::central::ConsumerContext,
    operation: u8,
    purpose: df_imagery::Purpose,
    references: Vec<df_imagery::SourceRef>,
) -> df_imagery::ImageRequest {
    use df_imagery::*;
    let operation = df_types::OperationId::from_bytes(&[operation; 16]).unwrap();
    let audience = match &context.owner {
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
        ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
    };
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a plate", 1).unwrap(),
            references,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::Regenerate { nonce: operation },
        urgency: Urgency::Visible,
        audience,
        expected_source_revision: context.expected_source_revision,
        output: context.config.output_profile_for(purpose).unwrap(),
    };
    request.identity.fingerprint = context.service.canonical_fingerprint(&request).unwrap();
    request
}

async fn unified_cost_image(
    fixture: &crate::gameplay::image_test_support::ImageHostFixture,
    endpoint: &CostImageEndpoint,
    operation: u8,
    purpose: df_imagery::Purpose,
    references: Vec<df_imagery::SourceRef>,
    expected_calls: u64,
) -> (df_imagery::ImageDemandView, df_imagery::ImageRequest) {
    use df_imagery::*;
    let context = fixture.imagery().consumer_context().await.unwrap();
    let request = cost_image_request(&context, operation, purpose, references);
    let mut view = match context
        .service
        .request(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("actual unified request refused: {other:?}"),
    };
    let expected = *endpoint.reply.lock().unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let terminal = match expected {
            CostImageReply::Unknown
            | CostImageReply::Refused(_)
            | CostImageReply::AmbiguousRateLimit
            | CostImageReply::Quota => matches!(
                view.execution,
                ImageExecutionState::Unknown | ImageExecutionState::Uncertain
            ),
            CostImageReply::Reported { .. } | CostImageReply::Silent => {
                view.execution == ImageExecutionState::Ready
            }
        };
        if terminal && fixture.metering_port().totals().calls == expected_calls {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "actual unified cost request did not finish: {view:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
        view = context
            .service
            .read(&context.principal, &view.handle)
            .await
            .unwrap();
    }
    assert_cost_replay(
        context
            .service
            .request(&context.principal, &request)
            .await
            .unwrap(),
        &view,
    );
    assert_eq!(
        fixture.metering_port().totals().calls,
        expected_calls,
        "request replay cannot create another charge identity"
    );
    (view, request)
}

fn state_of(book: &CostBook) -> MutexGuard<'_, BookState> {
    locked(&book.state)
}

#[test]
fn a_priced_line_is_exact_nano_dollars_rounded_half_up_to_a_micro_dollar() {
    let price = |usd: &str, per: u64| tariff::Price {
        nano_usd: tariff::parse_nano_usd(usd).expect("rate"),
        per,
        as_of: String::new(),
        source: String::new(),
    };
    let jev = price("0.042", 1_000_000);
    assert_eq!(jev.micro_dollars_half_up(1_500), 63);
    assert_eq!(jev.micro_dollars_half_up(11), 0, "0.462 rounds down");
    assert_eq!(jev.micro_dollars_half_up(12), 1, "0.504 rounds up");
    let half = tariff::Price {
        nano_usd: 500,
        ..price("0", 1)
    };
    assert_eq!(
        half.micro_dollars_half_up(1),
        1,
        "exactly one half rounds up"
    );
    assert_eq!(half.micro_dollars_half_up(0), 0);
    let scribe = price("0.22", 3_600);
    assert_eq!(scribe.micro_dollars_half_up(3_600), 220_000);
    assert_eq!(scribe.micro_dollars_half_up(10), 611, "611.1 micro-dollars");
    let huge = tariff::Price {
        nano_usd: u64::MAX,
        ..price("0", 1)
    };
    assert_eq!(
        huge.micro_dollars_half_up(u64::MAX),
        u64::MAX,
        "saturates instead of wrapping"
    );
}

#[test]
fn dollar_text_parses_exactly_or_not_at_all() {
    let parse_nano_usd = tariff::parse_nano_usd;
    assert_eq!(parse_nano_usd("5"), Some(5_000_000_000));
    assert_eq!(parse_nano_usd("0.000000001"), Some(1));
    assert_eq!(parse_nano_usd("30.00"), Some(30_000_000_000));
    for bad in [
        "",
        ".5",
        "5.",
        "-1",
        "1e3",
        "0.0000000001",
        " 1",
        "1,5",
        "0x1",
    ] {
        assert_eq!(parse_nano_usd(bad), None, "{bad:?}");
    }
    assert_eq!(usd(1), "0.000001");
    assert_eq!(usd(12_345_678), "12.345678");
    assert_eq!(prices::nano_usd_text(42_000_000), "0.042");
    assert_eq!(prices::nano_usd_text(5_000_000_000), "5");
}

#[test]
fn every_enum_label_is_its_serialized_name() {
    for provider in Provider::ALL {
        assert_eq!(json!(provider), json!(provider.label()));
        assert_eq!(Provider::parse(provider.label()), Some(provider));
    }
    for category in Category::ALL {
        assert_eq!(json!(category), json!(category.label()));
        assert_eq!(Category::parse(category.label()), Some(category));
    }
    for cause in Cause::ALL {
        assert_eq!(json!(cause), json!(cause.label()));
        assert_eq!(Cause::parse(cause.label()), Some(cause));
    }
    for source in [
        UsageSource::Reported,
        UsageSource::Estimated,
        UsageSource::Unknown,
    ] {
        assert_eq!(json!(source), json!(source.label()));
    }
}

#[test]
fn a_malformed_price_file_is_refused_whole_with_the_entry_named() {
    let entry = r#"{"provider": "openai", "model": "m", "category": "text_input", "usd": "1", "per": 1, "as_of": "2026-10-24", "source": "https://example.test"}"#;
    let file = |prices: &str| {
        format!(r#"{{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [{prices}]}}"#)
    };
    assert_eq!(
        PriceTable::parse(&file(entry)).map(|table| table.len()),
        Ok(1)
    );
    assert_eq!(PriceTable::parse(&file("")).map(|table| table.len()), Ok(0));
    let refused = |text: String| PriceTable::parse(&text).map(|_| ()).unwrap_err();
    assert_eq!(
        refused(file(entry).replace("pricing/1", "pricing/2")),
        PricingError::Schema
    );
    assert_eq!(
        refused(file(entry).replace("USD", "EUR")),
        PricingError::Currency
    );
    for (from, to, reason) in [
        ("\"openai\"", "\"acme\"", "provider"),
        ("text_input", "tokens", "category"),
        ("\"m\"", "\"\"", "model"),
        ("\"m\"", "\"two words\"", "model"),
        ("\"1\"", "\"1e3\"", "usd"),
        ("\"per\": 1", "\"per\": 0", "per"),
        ("2026-10-24", "24/10/2026", "as_of"),
        ("https://example.test", "http://example.test", "source"),
    ] {
        assert_eq!(
            refused(file(&entry.replace(from, to))),
            PricingError::Entry { index: 0, reason },
            "{from} -> {to}"
        );
    }
    assert_eq!(
        refused(file(&format!("{entry}, {entry}"))),
        PricingError::Duplicate { index: 1 }
    );
    assert!(matches!(
        refused(file(&entry.replace("\"per\"", "\"extra\": 1, \"per\""))),
        PricingError::Json(_)
    ));
}

#[test]
fn a_missing_or_refused_price_file_leaves_every_line_unpriced_and_says_so() {
    let directory = scratch("load");
    let missing = directory.join("absent.json");
    let (table, status) = load_prices(&missing);
    assert_eq!(table.len(), 0);
    assert_eq!(status, PricingStatus::Missing { path: missing });
    let broken = directory.join("broken.json");
    std::fs::write(&broken, "{").expect("write");
    let (table, status) = load_prices(&broken);
    assert_eq!(table.len(), 0);
    assert!(matches!(status, PricingStatus::Refused { .. }));
}

#[test]
fn the_checked_in_price_file_loads_and_prices_every_priced_route() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/pricing.json"
    ));
    let (table, status) = load_prices(&path);
    assert!(
        matches!(status, PricingStatus::Loaded { .. }),
        "content/pricing.json must load: {status:?}"
    );
    for route in ROUTES {
        if route.metering != Metering::Priced {
            continue;
        }
        for category in route.categories {
            assert!(
                table
                    .lookup(route.provider, route.model, *category, 0)
                    .is_some(),
                "{} {} has no {} price",
                route.provider.label(),
                route.model,
                category.label()
            );
        }
    }
}

#[test]
fn every_provider_client_has_a_billed_route() {
    // The generic HTTP transport and its response metadata do not select a billed route.
    const NOT_CLIENTS: [&str; 6] = [
        "NativeHttp",
        "NativeHttpFailure",
        "NativeHttpLimits",
        "NativeHttpMetadata",
        "NativeHttpResponse",
        "NativeSpeechObservation",
    ];
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../df-providers/src");
    let mut clients = BTreeSet::new();
    for entry in std::fs::read_dir(directory).expect("df-providers sources") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source");
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("pub struct ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            if (name.starts_with("Native") || name.ends_with("Provider"))
                && !NOT_CLIENTS.contains(&name.as_str())
            {
                clients.insert(name);
            }
        }
    }
    let routed: BTreeSet<String> = ROUTES.iter().map(|route| route.client.to_owned()).collect();
    assert!(
        clients.len() >= 13,
        "the scan found the clients: {clients:?}"
    );
    assert_eq!(
        clients, routed,
        "every df_providers client needs a cost::ROUTES entry, and every entry a client"
    );
}

#[test]
fn the_client_metered_routes_have_no_pending_admission_entry() {
    assert!(
        ROUTES
            .iter()
            .all(|route| route.metering == Metering::Priced)
    );
}

#[tokio::test]
async fn an_image_call_is_priced_from_the_reported_tokens_like_the_provider_formula() {
    let ledger = opened_ledger(None);
    let endpoint = CostImageEndpoint::new(
        1024,
        640,
        CostImageReply::Reported {
            text: 1000,
            input: 2000,
            output: 3000,
        },
    );
    let patch = cost_image_patch(
        endpoint.address,
        df_imagery::BackendId::OpenAi,
        1024,
        640,
        df_imagery::GenerationQuality::Low,
        false,
    );
    let Some(fixture) = crate::gameplay::image_test_support::ImageHostFixture::open(
        endpoint.address,
        patch,
        port::MeteringPort::from_ledger(ledger.clone()),
    )
    .await
    else {
        return;
    };
    let (view, _) = unified_cost_image(
        &fixture,
        &endpoint,
        31,
        df_imagery::Purpose::ScenePlate,
        vec![],
        1,
    )
    .await;
    assert_eq!(view.execution, df_imagery::ImageExecutionState::Ready);
    {
        let state = state_of(ledger.costs());
        assert_eq!(
            state.session_totals.micro_usd,
            1000 * 5 + 2000 * 8 + 3000 * 30
        );
        assert_eq!(state.session_totals.estimated_events, 0);
        assert_eq!(
            state.by_line_micro(
                Provider::OpenAi,
                "gpt-image-2.5-flare",
                Category::ImageInput
            ),
            16_000
        );
    }
    let calls = fixture.metering_port().recent();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].call_id.starts_with("img-"));
    assert_eq!(calls[0].cause, "scene_plate");
    assert_eq!(calls[0].detail, "scene_plate/generation");
    assert_eq!(endpoint.posts().len(), 1);
    fixture.close().await;
}

impl BookState {
    fn by_line_micro(&self, provider: Provider, model: &str, category: Category) -> u64 {
        self.session_totals
            .by_line
            .get(&(provider, model.to_owned(), category))
            .map_or(0, |total| total.micro_usd)
    }
}

#[tokio::test]
async fn image_errors_without_billing_proof_retain_the_unknown_charge_without_resending() {
    use df_imagery::execution::FailureCause;
    for (operation, reply, expected_cause) in [
        (41, CostImageReply::Refused(400), None),
        (
            42,
            CostImageReply::Quota,
            Some(FailureCause::QuotaExhausted),
        ),
        (
            43,
            CostImageReply::Refused(503),
            Some(FailureCause::Provider),
        ),
        (
            44,
            CostImageReply::AmbiguousRateLimit,
            Some(FailureCause::RateLimited),
        ),
        (
            45,
            CostImageReply::Unknown,
            Some(FailureCause::TransportLost),
        ),
    ] {
        let ledger = opened_ledger(None);
        let endpoint = CostImageEndpoint::new(1024, 640, reply);
        let mut patch = cost_image_patch(
            endpoint.address,
            df_imagery::BackendId::OpenAi,
            1024,
            640,
            df_imagery::GenerationQuality::Low,
            false,
        );
        patch.retry_max_attempts = Some(2);
        let Some(fixture) = crate::gameplay::image_test_support::ImageHostFixture::open(
            endpoint.address,
            patch,
            port::MeteringPort::from_ledger(ledger.clone()),
        )
        .await
        else {
            return;
        };
        let (view, _) = unified_cost_image(
            &fixture,
            &endpoint,
            operation,
            df_imagery::Purpose::ItemCloseUp,
            vec![],
            1,
        )
        .await;
        assert_eq!(view.failure, expected_cause);
        {
            let state = state_of(ledger.costs());
            assert_eq!(state.session_totals.events, 1);
            assert_eq!(state.session_totals.unknown_events, 1);
            assert_eq!(state.session_totals.unpriced_events, 0);
            assert_eq!(state.session_totals.micro_usd, 4 * 5 + 170 * 30);
            assert_eq!(state.session_totals.by_cause[&Cause::ItemCloseUp].events, 1);
        }
        let calls = fixture.metering_port().recent();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].source, port::MeteredSource::Unknown);
        assert!(!calls[0].usable);
        let (database, connection) = fixture
            .database()
            .connect(tokio_postgres::NoTls)
            .await
            .unwrap();
        let driver = tokio::spawn(connection);
        let rows = database
            .query(
                "SELECT d.book_call_id,d.dispatch_state,d.settlement,u.evidence, \
                 u.units=u.maxima AS full_hold,octet_length(u.raw_output)>0 AS retained_error \
                 FROM imagery.image_dispatch_attempt d JOIN imagery.image_dispatch_usage u \
                 USING(dispatch_id) ORDER BY d.dispatch_seq,u.revision",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<_, String>("book_call_id"), calls[0].call_id);
        assert_eq!(rows[0].get::<_, String>("dispatch_state"), "unknown");
        assert_eq!(
            rows[0].get::<_, String>("settlement"),
            "unknown_outstanding"
        );
        assert_eq!(rows[0].get::<_, String>("evidence"), "unknown");
        assert!(rows[0].get::<_, bool>("full_hold"));
        assert!(rows[0].get::<_, bool>("retained_error"));
        drop(database);
        driver.await.unwrap().unwrap();
        assert_eq!(
            endpoint.posts().len(),
            1,
            "ambiguous, quota, invalid and provider failures cannot authorize a fresh send"
        );
        fixture.close().await;
    }
}

#[tokio::test]
async fn strict_image_submit_declines_retry_fresh_dispatches_only_to_the_frozen_limit() {
    let ledger = opened_ledger(None);
    let endpoint = CostImageEndpoint::new(1024, 640, CostImageReply::Refused(429));
    let mut patch = cost_image_patch(
        endpoint.address,
        df_imagery::BackendId::OpenAi,
        1024,
        640,
        df_imagery::GenerationQuality::Low,
        false,
    );
    patch.retry_max_attempts = Some(2);
    patch.retry_backoff_secs = Some(0);
    patch.retry_deadline_secs = Some(20);
    let Some(fixture) = crate::gameplay::image_test_support::ImageHostFixture::open(
        endpoint.address,
        patch,
        port::MeteringPort::from_ledger(ledger.clone()),
    )
    .await
    else {
        return;
    };
    let (view, request) = unified_cost_image(
        &fixture,
        &endpoint,
        46,
        df_imagery::Purpose::ItemCloseUp,
        vec![],
        2,
    )
    .await;
    assert_eq!(
        view.failure,
        Some(df_imagery::execution::FailureCause::RateLimited)
    );
    {
        let state = state_of(ledger.costs());
        assert_eq!(state.session_totals.events, 2);
        assert_eq!(state.session_totals.unknown_events, 2);
        assert_eq!(state.session_totals.unpriced_events, 0);
        assert_eq!(state.session_totals.micro_usd, 2 * (4 * 5 + 170 * 30));
    }
    let calls = fixture.metering_port().recent();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].call_id, calls[1].call_id);
    assert!(
        calls
            .iter()
            .all(|call| call.source == port::MeteredSource::Unknown && !call.usable)
    );
    let (database, connection) = fixture
        .database()
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    let driver = tokio::spawn(connection);
    let rows = database.query(
        "SELECT d.dispatch_id,d.lineage_parent,d.book_call_id,d.dispatch_state,d.dispatch_seq, \
         d.settlement,d.failure_cause,u.evidence,u.units=u.maxima AS full_hold, \
         o.max_attempts,o.usage_revision=u.revision AND c.usage_revision=u.revision AS same_receipt, \
         convert_from(u.raw_output,'UTF8')::jsonb->>'schema_revision' AS raw_schema \
         FROM imagery.image_dispatch_attempt d JOIN imagery.image_dispatch_usage u USING(dispatch_id) \
         JOIN imagery.image_decline_observation o ON o.dispatch_id=d.dispatch_id \
         JOIN imagery.image_decline_checkpoint c ON c.dispatch_id=d.dispatch_id \
         ORDER BY d.dispatch_seq,u.revision",
        &[],
    ).await.unwrap();
    assert_eq!(rows.len(), 2);
    let first: Vec<u8> = rows[0].get("dispatch_id");
    assert!(
        rows[0]
            .get::<_, Option<Vec<u8>>>("lineage_parent")
            .is_none()
    );
    assert_eq!(
        rows[1].get::<_, Option<Vec<u8>>>("lineage_parent"),
        Some(first)
    );
    assert_eq!(
        rows[1].get::<_, i32>("dispatch_seq"),
        rows[0].get::<_, i32>("dispatch_seq") + 1
    );
    let mut actual_ids = BTreeSet::new();
    for row in rows {
        let dispatch: [u8; 16] = row.get::<_, Vec<u8>>("dispatch_id").try_into().unwrap();
        let id: String = row.get("book_call_id");
        assert_eq!(id, df_imagery::cost::pinned_call_id(&dispatch));
        actual_ids.insert(id);
        assert_eq!(row.get::<_, String>("dispatch_state"), "declined");
        assert_eq!(row.get::<_, String>("settlement"), "unknown_outstanding");
        assert_eq!(row.get::<_, String>("failure_cause"), "rate_limited");
        assert_eq!(row.get::<_, String>("evidence"), "unknown");
        assert_eq!(row.get::<_, i64>("max_attempts"), 2);
        assert_eq!(row.get::<_, String>("raw_schema"), "2");
        assert!(row.get::<_, bool>("full_hold"));
        assert!(row.get::<_, bool>("same_receipt"));
    }
    assert_eq!(
        actual_ids,
        calls.iter().map(|call| call.call_id.clone()).collect()
    );
    drop(database);
    driver.await.unwrap().unwrap();
    let context = fixture.imagery().consumer_context().await.unwrap();
    for _ in 0..8 {
        assert_cost_replay(
            context
                .service
                .request(&context.principal, &request)
                .await
                .unwrap(),
            &view,
        );
    }
    drop(context);
    assert_eq!(
        endpoint.posts().len(),
        2,
        "the frozen attempt budget permits no third send"
    );
    assert_eq!(fixture.metering_port().totals().calls, 2);
    fixture.close().await;
}

#[test]
fn a_text_call_splits_cached_and_written_input_from_plain_input() {
    let usage = TextTokenUsage {
        input_tokens: 1_000,
        output_tokens: 200,
        total_tokens: 1_200,
        cached_input_tokens: 400,
        cache_write_tokens: Some(100),
        reasoning_output_tokens: 50,
    };
    let reported = text_usage(&usage);
    assert_eq!(reported.source, UsageSource::Reported);
    assert_eq!(
        reported.quantities,
        vec![
            (Category::TextInput, 500),
            (Category::TextCachedInput, 400),
            (Category::TextCacheWrite, 100),
            (Category::TextOutput, 200),
        ]
    );
    let (lines, total) = price_carrying(
        &table(),
        &mut BTreeMap::new(),
        Provider::OpenAi,
        "gpt-6-luna",
        &reported.quantities,
    );
    assert_eq!(
        lines.iter().map(|line| line.micro_usd).collect::<Vec<_>>(),
        [Some(50), Some(4), Some(13), Some(100)],
        "12.5 micro-dollars of cache write round up to 13"
    );
    assert_eq!(total, 167);
    let without_write = text_usage(&TextTokenUsage {
        cache_write_tokens: None,
        ..usage
    });
    assert!(
        !without_write
            .quantities
            .iter()
            .any(|(category, _)| *category == Category::TextCacheWrite)
    );
}

#[test]
fn voice_clips_are_attributed_by_speaker_and_counted_characters_are_flagged_estimated() {
    let narrated = voice_call("m", &narration());
    assert_eq!(
        (narrated.cause, narrated.detail.as_str()),
        (Cause::NarrationVoice, "narrator")
    );
    let cried = voice_call("m", &hero_cry("Mira"));
    assert_eq!(
        (cried.cause, cried.detail.as_str()),
        (Cause::CharacterVoice, "Mira")
    );
    let replied = voice_call("m", &cast_reply("bandit", "Dain Rusk"));
    assert_eq!(
        (replied.cause, replied.detail.as_str()),
        (Cause::CharacterVoice, "bandit"),
        "a cast reply is attributed to its actor id"
    );
    let ledger = opened_ledger(None);
    ledger
        .begin_call(voice_call("eleven_flash_v2_5", &narration()), &[])
        .settle(Usage::estimated(vec![(Category::Characters, 250)]));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.micro_usd, 10_000);
    assert_eq!(state.session_totals.estimated_events, 1);
}

#[test]
fn a_line_without_a_price_is_recorded_unpriced_and_never_refused() {
    let ledger = opened_ledger(None);
    ledger
        .begin_call(text_call("gpt-9-unknown", "npc_reply"), &[])
        .settle(Usage::reported(vec![(Category::TextInput, 5_000)]));
    let report = ledger.costs().report(2_000);
    assert_eq!(report["session"]["events"], json!(1));
    assert_eq!(report["session"]["micro_usd"], json!(0));
    assert_eq!(report["session"]["unpriced_events"], json!(1));
    assert_eq!(
        report["session"]["by_line"][0]["unpriced_quantity"],
        json!(5_000)
    );
}

#[test]
fn an_unsettled_call_is_recorded_with_its_estimate_as_of_unknown_outcome() {
    let ledger = opened_ledger(None);
    drop(ledger.begin_call(
        text_call("gpt-6-luna", "npc_reply"),
        &[(Category::TextInput, 2_000)],
    ));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(state.session_totals.micro_usd, 200);
    drop(state);
    ledger
        .begin_call(
            text_call("gpt-6-luna", "npc_reply"),
            &[(Category::TextInput, 2_000)],
        )
        .not_billed();
    assert_eq!(
        state_of(ledger.costs()).session_totals.events,
        1,
        "not billed: nothing"
    );
}

#[test]
fn events_are_numbered_in_record_order_per_session() {
    let book = opened_book("s1", None);
    let usage = || Usage::reported(vec![(Category::TextInput, 1_000)]);
    book.meter(text_call("gpt-6-luna", "a"), usage(), 2_000);
    book.meter(text_call("gpt-6-luna", "a"), usage(), 2_001);
    let state = state_of(&book);
    assert_eq!(state.session_totals.events, 2, "two calls are two events");
    assert_eq!(state.high_water, BTreeMap::from([("s1".to_owned(), 2)]));
    assert_eq!(split_call_id("s1-2"), Some(("s1", 2)));
    for bad in ["s1", "-3", "s1-0", "s1-x", "s1-"] {
        assert_eq!(split_call_id(bad), None, "{bad:?}");
    }
}

#[test]
fn the_dedupe_state_is_one_number_per_session_however_long_the_run() {
    let directory = scratch("bound");
    let log = directory.join("cost.jsonl");
    for session in ["s1", "s2", "s3"] {
        let book = opened_book(session, Some(&log));
        for _ in 0..1_000 {
            book.meter(
                voice_call("eleven_flash_v2_5", &narration()),
                Usage::estimated(vec![(Category::Characters, 10)]),
                2_000,
            );
        }
        assert!(book.flush());
        assert_eq!(
            state_of(&book).high_water.len(),
            1 + ["s1", "s2", "s3"]
                .iter()
                .position(|s| *s == session)
                .expect("listed")
        );
    }
    let reopened = opened_book("s4", Some(&log));
    let state = state_of(&reopened);
    assert_eq!(state.run_totals.events, 3_000);
    assert_eq!(state.duplicate_calls, 0);
    assert_eq!(
        state.high_water,
        BTreeMap::from([
            ("s1".to_owned(), 1_000),
            ("s2".to_owned(), 1_000),
            ("s3".to_owned(), 1_000),
        ]),
        "3,000 replayed events keep three numbers"
    );
}

#[test]
fn every_breakdown_sums_to_its_total() {
    let book = opened_book("s1", None);
    let specs = [
        text_call("gpt-6-luna", "npc_reply"),
        text_call("gpt-6-luna", "place_creation"),
        voice_call("eleven_flash_v2_5", &narration()),
        voice_call("eleven_flash_v2_5", &hero_cry("Mira")),
        text_call("unpriced-model", "npc_reply"),
    ];
    for index in 0..200_u64 {
        let spec = specs[(index % 5) as usize].clone();
        let quantities = if spec.provider == Provider::ElevenLabs {
            vec![(Category::Characters, 17 * index + 3)]
        } else {
            vec![
                (Category::TextInput, 97 * index + 11),
                (Category::TextCachedInput, 13 * index),
                (Category::TextOutput, 7 * index + 1),
            ]
        };
        book.meter(spec, Usage::reported(quantities), 2_000 + index);
    }
    let state = state_of(&book);
    for totals in [&state.run_totals, &state.session_totals] {
        assert_eq!(totals.events, 200);
        let lines: u64 = totals.by_line.values().map(|total| total.micro_usd).sum();
        let causes: u64 = totals.by_cause.values().map(|total| total.micro_usd).sum();
        let details: u64 = totals.by_detail.values().map(|total| total.micro_usd).sum();
        let cause_events: u64 = totals.by_cause.values().map(|total| total.events).sum();
        assert!(totals.micro_usd > 0);
        assert_eq!(lines, totals.micro_usd);
        assert_eq!(causes, totals.micro_usd);
        assert_eq!(details, totals.micro_usd);
        assert_eq!(cause_events, totals.events);
    }
}

#[test]
fn concurrent_calls_are_each_counted_exactly_once_and_logged_once() {
    let directory = scratch("concurrent");
    let log = directory.join("cost.jsonl");
    let ledger = opened_ledger(Some(&log));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let ledger = Arc::clone(&ledger);
            std::thread::spawn(move || {
                for _ in 0..100 {
                    ledger
                        .begin_call(text_call("gpt-6-luna", "npc_reply"), &[])
                        .settle(Usage::reported(vec![(Category::TextInput, 1_000)]));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("thread");
    }
    assert!(ledger.costs().flush());
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.events, 800);
    assert_eq!(state.session_totals.micro_usd, 800 * 100);
    drop(state);
    let text = std::fs::read_to_string(&log).expect("log");
    let ids: HashSet<String> = text
        .lines()
        .map(|line| {
            serde_json::from_str::<CostEvent>(line)
                .expect("event")
                .call_id
        })
        .collect();
    assert_eq!(text.lines().count(), 800);
    assert_eq!(ids.len(), 800);
}

#[test]
fn the_run_log_round_trips_byte_for_byte_and_the_run_totals_survive_a_restart() {
    let directory = scratch("reopen");
    let log = directory.join("cost.jsonl");
    let first = opened_book("s1", Some(&log));
    first.meter(
        text_call("gpt-6-luna", "npc_reply"),
        text_usage(&TextTokenUsage {
            input_tokens: 9_000,
            output_tokens: 700,
            total_tokens: 9_700,
            cached_input_tokens: 4_000,
            cache_write_tokens: None,
            reasoning_output_tokens: 0,
        }),
        2_000,
    );
    first.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 321)]),
        3_000,
    );
    first.meter(
        text_call("unpriced-model", "odd detail with spaces"),
        Usage::reported(vec![(Category::TextOutput, 5)]),
        4_000,
    );
    assert!(first.flush());
    let before = first.report(10_000)["run"].clone();
    drop(first);

    let text = std::fs::read_to_string(&log).expect("log");
    for line in text.lines() {
        let event: CostEvent = serde_json::from_str(line).expect("event");
        assert_eq!(serde_json::to_string(&event).expect("json"), line);
    }

    let second = CostBook::with_session("s2");
    let opened = second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            20_000,
        )
        .expect("reopens");
    assert_eq!((opened.replayed, opened.malformed), (3, 0));
    assert_eq!(second.report(30_000)["run"], before);
    assert_eq!(second.report(30_000)["session"]["events"], json!(0));
    second.meter(
        voice_call("eleven_flash_v2_5", &cast_reply("bandit", "Dain Rusk")),
        Usage::estimated(vec![(Category::Characters, 1_000)]),
        21_000,
    );
    assert!(second.flush());
    drop(second);
    let third = CostBook::with_session("s3");
    let opened = third
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            40_000,
        )
        .expect("reopens");
    assert_eq!(opened.replayed, 4);
    assert_eq!(
        opened.run_micro_usd,
        before["micro_usd"].as_u64().expect("micro") + 40_000
    );
}

#[test]
fn a_damaged_or_repeated_log_line_is_skipped_and_counted() {
    let directory = scratch("damaged");
    let log = directory.join("cost.jsonl");
    let book = opened_book("s1", Some(&log));
    book.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 1_000)]),
        2_000,
    );
    assert!(book.flush());
    drop(book);
    let line = std::fs::read_to_string(&log).expect("log");
    let tampered = line.replace("],\"micro_usd\":40000}", "],\"micro_usd\":1}");
    std::fs::write(&log, format!("{line}{line}{{not json\n{tampered}")).expect("write");
    let reopened = CostBook::with_session("s2");
    let opened = reopened
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            3_000,
        )
        .expect("reopens");
    assert_eq!((opened.replayed, opened.malformed), (1, 2));
    assert_eq!(opened.run_micro_usd, 40_000);
    assert_eq!(state_of(&reopened).duplicate_calls, 1, "the repeated line");
}

#[test]
fn calls_metered_before_the_book_opens_are_priced_and_written_when_it_opens() {
    let directory = scratch("pending");
    let log = directory.join("cost.jsonl");
    let book = CostBook::with_session("s1");
    book.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 1_000)]),
        500,
    );
    assert_eq!(state_of(&book).session_totals.events, 0, "held until open");
    book.open(
        "df_gameplay_demo_test",
        Some(&log),
        table(),
        PricingStatus::NotLoaded,
        1_000,
    )
    .expect("opens");
    assert_eq!(state_of(&book).session_totals.micro_usd, 40_000);
    assert!(book.flush());
    let event: CostEvent =
        serde_json::from_str(std::fs::read_to_string(&log).expect("log").trim_end())
            .expect("event");
    assert_eq!(event.run, "df_gameplay_demo_test");
    assert!(
        book.open("again", None, table(), PricingStatus::NotLoaded, 2_000)
            .is_err(),
        "a book opens once"
    );
}

#[test]
fn the_cost_line_has_one_documented_shape() {
    let book = opened_book("s1", None);
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![
            (Category::TextInput, 1_000),
            (Category::TextOutput, 200),
        ]),
        2_000,
    );
    let (lines, micro_usd) = price_carrying(
        &table(),
        &mut BTreeMap::new(),
        Provider::ElevenLabs,
        "eleven_flash_v2_5",
        &[(Category::Characters, 250), (Category::Calls, 1)],
    );
    let event = CostEvent {
        call_id: "s1-2".to_owned(),
        at_ms: 3_000,
        run: "df_gameplay_demo_test".to_owned(),
        session: "s1".to_owned(),
        provider: Provider::ElevenLabs,
        model: "eleven_flash_v2_5".to_owned(),
        cause: Cause::NarrationVoice,
        detail: word("Narrator Line"),
        source: UsageSource::Estimated,
        lines,
        micro_usd,
        usable: true,
    };
    assert_eq!(
        cost_line(&event, 10_200, 10_200),
        "COST call=s1-2 provider=elevenlabs model=eleven_flash_v2_5 cause=narration_voice \
         detail=narrator-line source=estimated usd=0.010000 \
         lines=characters:250@0.010000,calls:1@unpriced run_usd=0.010200 session_usd=0.010200"
    );
}

#[test]
fn the_dev_cost_report_has_a_pinned_shape() {
    let book = CostBook::with_session("s1");
    book.open(
        "df_gameplay_demo_test",
        None,
        PriceTable::parse(&TABLE.replace(
            r#"{"provider": "openai", "model": "gpt-image-2.5-flare", "category": "text_input", "usd": "5", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_input", "usd": "8", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
    {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_output", "usd": "30", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
"#,
            "",
        ))
        .expect("trimmed table"),
        PricingStatus::Loaded {
            path: PathBuf::from("pricing.json"),
            entries: 5,
        },
        1_000,
    )
    .expect("opens");
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![
            (Category::TextInput, 1_000),
            (Category::TextOutput, 200),
        ]),
        2_000,
    );
    book.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 250)]),
        3_000,
    );
    let mut report = book.report(1_000 + 3_600_000);
    let routes = report
        .as_object_mut()
        .and_then(|object| object.remove("routes"))
        .expect("routes");
    assert_eq!(routes.as_array().map(Vec::len), Some(ROUTES.len()));
    assert_eq!(
        routes[0],
        json!({"client": "NativeImageEditProvider", "provider": "openai",
            "model": "gpt-image-2.5-flare",
            "categories": ["text_input", "image_input", "image_output"],
            "metering": "priced", "reason": ""})
    );
    let price = |model: &str, category: &str, usd: &str, per: u64| {
        json!({"provider": if model == "eleven_flash_v2_5" { "elevenlabs" } else { "openai" },
            "model": model, "category": category, "usd": usd, "per": per,
            "context_max_tokens": 0,
            "as_of": "2026-10-24", "source": "https://example.test/p"})
    };
    let line = |provider: &str, model: &str, category: &str, quantity: u64, micro: u64| {
        json!({"provider": provider, "model": model, "category": category, "quantity": quantity,
            "unpriced_quantity": 0, "micro_usd": micro, "usd": usd(micro)})
    };
    let totals = json!({
        "usd": "0.010200",
        "micro_usd": 10_200,
        "events": 2,
        "estimated_events": 1,
        "unknown_events": 0,
        "unpriced_events": 0,
        "unusable_events": 0,
        "by_line": [
            line("openai", "gpt-6-luna", "text_input", 1_000, 100),
            line("openai", "gpt-6-luna", "text_output", 200, 100),
            line("elevenlabs", "eleven_flash_v2_5", "characters", 250, 10_000),
        ],
        "by_cause": [
            {"cause": "narration_voice", "events": 1, "micro_usd": 10_000, "usd": "0.010000"},
            {"cause": "storyteller", "events": 1, "micro_usd": 200, "usd": "0.000200"},
        ],
        "top_spenders": [
            {"cause": "narration_voice", "detail": "narrator", "events": 1, "micro_usd": 10_000,
                "usd": "0.010000"},
            {"cause": "storyteller", "detail": "npc_reply", "events": 1, "micro_usd": 200,
                "usd": "0.000200"},
        ],
    });
    let mut run = totals.clone();
    run["id"] = json!("df_gameplay_demo_test");
    let mut session = totals;
    session["id"] = json!("s1");
    session["opened_at_ms"] = json!(1_000);
    session["usd_per_wall_hour"] = json!("0.010200");
    assert_eq!(
        report,
        json!({
            "pricing": {"status": "loaded", "path": "pricing.json", "entries": 5},
            // By provider, model, category and tier, as text.
            "prices": [
                price("eleven_flash_v2_5", "characters", "0.04", 1_000),
                price("gpt-6-luna", "text_cache_write", "0.125", 1_000_000),
                price("gpt-6-luna", "text_cached_input", "0.01", 1_000_000),
                price("gpt-6-luna", "text_input", "0.1", 1_000_000),
                price("gpt-6-luna", "text_output", "0.5", 1_000_000),
            ],
            "log": {"path": null, "status": "off", "write_failures": 0, "malformed_lines": 0,
                "duplicate_calls": 0, "pending_lost": 0},
            "run": run,
            "session": session,
        })
    );
}

#[test]
fn a_bill_is_compared_line_by_line_with_the_run() {
    let book = opened_book("s1", None);
    book.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 1_000)]),
        2_000,
    );
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![(Category::TextInput, 1_000_000)]),
        2_000,
    );
    let compared = book
        .compare_bill(
            "provider,model,category,usd\n\
             openai,gpt-6-luna,text_input,0.07\n\
             openai,gpt-6-luna,text_input,0.02\n\
             openai,gpt-image-2.5-flare,image_input,14.852\n",
        )
        .expect("compares");
    assert_eq!(
        compared,
        json!({"rows": [
            {"provider": "openai", "model": "gpt-6-luna", "category": "text_input",
                "ours_usd": "0.100000", "bill_usd": "0.090000", "delta_usd": "0.010000"},
            {"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_input",
                "ours_usd": "0.000000", "bill_usd": "14.852000", "delta_usd": "-14.852000"},
        ]}),
        "repeated bill rows add up; a provider the bill does not cover is left out"
    );
    for bad in [
        "",
        "model,usd\n",
        "provider,model,category,usd\nopenai,m,text_input\n",
        "provider,model,category,usd\nacme,m,text_input,1\n",
        "provider,model,category,usd\nopenai,m,tokens,1\n",
        "provider,model,category,usd\nopenai,m,text_input,0.0000001\n",
    ] {
        assert!(book.compare_bill(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn opening_a_run_loads_the_prices_names_the_log_and_reports_a_refused_file_loudly() {
    let root = scratch("open-run");
    std::fs::create_dir_all(root.join("content")).expect("content");
    std::fs::write(root.join("content/pricing.json"), TABLE).expect("prices");
    let ledger = Arc::new(SpendLedger::default());
    let lines = open_run(&ledger, &root, "df_gameplay_demo_cost_test");
    assert!(lines.contains("COST pricing loaded entries=8"), "{lines}");
    assert!(
        lines.contains("COST on run=df_gameplay_demo_cost_test"),
        "{lines}"
    );
    assert!(current().is_some_and(|installed| Arc::ptr_eq(&installed, &ledger)));
    for _ in 0..200 {
        ledger
            .begin_call(voice_call("eleven_flash_v2_5", &narration()), &[])
            .settle(Usage::estimated(vec![(Category::Characters, 1_000)]));
    }
    // The shutdown step drains the writer: no flush, and nothing queued is lost.
    close_run();
    assert!(current().is_none(), "the ledger is uninstalled at shutdown");
    let log = root.join("development/runs/df_gameplay_demo_cost_test/cost.jsonl");
    assert_eq!(
        std::fs::read_to_string(log).expect("log").lines().count(),
        200
    );
    assert_eq!(
        ledger.costs().report(now_ms())["log"]["status"],
        json!("closed")
    );

    std::fs::write(root.join("content/pricing.json"), "{\"schema\": 1}").expect("prices");
    let refused = Arc::new(SpendLedger::default());
    let lines = open_run(&refused, &root, "not a database name");
    assert!(lines.contains("COST pricing REFUSED"), "{lines}");
    assert!(lines.contains("run=unnamed log=off"), "{lines}");
    refused
        .begin_call(voice_call("eleven_flash_v2_5", &narration()), &[])
        .settle(Usage::estimated(vec![(Category::Characters, 1_000)]));
    let report = refused.costs().report(now_ms());
    assert_eq!(
        report["session"]["events"],
        json!(1),
        "a refused table never blocks a call"
    );
    assert_eq!(report["session"]["unpriced_events"], json!(1));
    close_run();
    assert!(current().is_none());
}

#[test]
fn a_log_that_cannot_be_opened_leaves_the_book_counting_in_memory_and_says_why() {
    let directory = scratch("unwritable");
    let blocker = directory.join("blocker");
    std::fs::write(&blocker, "a file where the run directory should be").expect("blocker");
    let book = CostBook::with_session("s1");
    let opened = book
        .open(
            "df_gameplay_demo_test",
            Some(&blocker.join("cost.jsonl")),
            table(),
            PricingStatus::NotLoaded,
            1_000,
        )
        .expect("the book opens in memory");
    let error = opened.log_error.expect("the log failure is reported");
    assert!(
        error.starts_with("read:") || error.starts_with("write:"),
        "{error}"
    );
    book.meter(
        voice_call("eleven_flash_v2_5", &narration()),
        Usage::estimated(vec![(Category::Characters, 1_000)]),
        2_000,
    );
    let report = book.report(3_000);
    assert_eq!(
        report["session"]["events"],
        json!(1),
        "counted, not held back"
    );
    assert_eq!(report["session"]["micro_usd"], json!(40_000));
    assert_eq!(report["log"]["status"], json!(error));
    assert_eq!(report["log"]["pending_lost"], json!(0));
}

#[test]
fn closing_the_log_writes_every_queued_line_and_keeps_counting_in_memory() {
    let directory = scratch("close");
    let log = directory.join("cost.jsonl");
    let book = opened_book("s1", Some(&log));
    for _ in 0..500 {
        book.meter(
            text_call("gpt-6-luna", "npc_reply"),
            Usage::reported(vec![(Category::TextInput, 1_000)]),
            2_000,
        );
    }
    book.close();
    assert_eq!(
        std::fs::read_to_string(&log).expect("log").lines().count(),
        500
    );
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![(Category::TextInput, 1_000)]),
        3_000,
    );
    let report = book.report(4_000);
    assert_eq!(report["session"]["events"], json!(501));
    assert_eq!(report["log"]["status"], json!("closed"));
    assert_eq!(
        std::fs::read_to_string(&log).expect("log").lines().count(),
        500
    );
    book.close();
}

#[test]
fn a_speech_request_refused_before_billable_work_is_not_billed() {
    for reason in [
        SpeechAdapterReason::InvalidInput,
        SpeechAdapterReason::InvalidCredential,
        SpeechAdapterReason::Owner(df_provider_api::RequestError::DeadlineExceeded),
        SpeechAdapterReason::Transport(NativeHttpRefusal::RequestTooLarge),
        SpeechAdapterReason::Transport(NativeHttpRefusal::ProviderDisabled),
        SpeechAdapterReason::Supplier(ProviderFailureClass::InvalidRequest),
        SpeechAdapterReason::Supplier(ProviderFailureClass::Denied),
        SpeechAdapterReason::Supplier(ProviderFailureClass::Capacity),
        SpeechAdapterReason::Supplier(ProviderFailureClass::Unavailable),
    ] {
        assert!(!speech_failure_may_be_billed(reason), "{reason:?}");
    }
    for reason in [
        SpeechAdapterReason::Transport(NativeHttpRefusal::Deadline),
        SpeechAdapterReason::Transport(NativeHttpRefusal::Unknown),
        SpeechAdapterReason::Supplier(ProviderFailureClass::Unknown),
        SpeechAdapterReason::EmptyAudio,
    ] {
        assert!(speech_failure_may_be_billed(reason), "{reason:?}");
    }
    let ledger = opened_ledger(None);
    let counted = [(Category::Characters, 250)];
    let call = ledger.begin_call(voice_call("eleven_flash_v2_5", &narration()), &counted);
    settle_speech_failure(
        call,
        SpeechAdapterReason::Supplier(ProviderFailureClass::Capacity),
    );
    assert_eq!(
        state_of(ledger.costs()).session_totals.events,
        0,
        "a 429 is not billed"
    );
    let call = ledger.begin_call(voice_call("eleven_flash_v2_5", &narration()), &counted);
    settle_speech_failure(
        call,
        SpeechAdapterReason::Transport(NativeHttpRefusal::Deadline),
    );
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(
        state.session_totals.micro_usd, 10_000,
        "the counted characters"
    );
}

#[tokio::test]
async fn a_clip_frame_is_priced_from_its_tokens_and_a_silent_usage_stands_for_its_maxima() {
    let ledger = metering::shipped_ledger();
    let endpoint = CostImageEndpoint::new(1024, 1024, CostImageReply::Silent);
    let mut patch = cost_image_patch(
        endpoint.address,
        df_imagery::BackendId::OpenAi,
        1024,
        1024,
        df_imagery::GenerationQuality::Medium,
        false,
    );
    patch.purpose_model_override[df_imagery::Purpose::AnimationFrame.index()] = Some(Some(
        df_imagery::ModelName::new("gpt-image-2.5-sunburst").unwrap(),
    ));
    let Some(fixture) = crate::gameplay::image_test_support::ImageHostFixture::open(
        endpoint.address,
        patch,
        port::MeteringPort::from_ledger(ledger.clone()),
    )
    .await
    else {
        return;
    };
    unified_cost_image(
        &fixture,
        &endpoint,
        51,
        df_imagery::Purpose::AnimationFrame,
        vec![],
        1,
    )
    .await;
    let silent = fixture.metering_port().recent().pop().unwrap();
    assert_eq!(silent.model, "gpt-image-2.5-sunburst");
    assert_eq!(silent.cause, "clip_frame");
    assert_eq!(silent.detail, "animation_frame/generation");
    assert_eq!(silent.source, port::MeteredSource::Estimated);
    assert_eq!(
        silent.units,
        vec![
            port::MeteredUnits {
                category: "text_input".to_owned(),
                quantity: 4
            },
            port::MeteredUnits {
                category: "image_input".to_owned(),
                quantity: 0
            },
            port::MeteredUnits {
                category: "image_output".to_owned(),
                quantity: 1056
            }
        ]
    );
    endpoint.set(CostImageReply::Reported {
        text: 1,
        input: 2,
        output: 3,
    });
    unified_cost_image(
        &fixture,
        &endpoint,
        52,
        df_imagery::Purpose::AnimationFrame,
        vec![],
        2,
    )
    .await;
    let reported = fixture.metering_port().recent().pop().unwrap();
    assert_eq!(reported.source, port::MeteredSource::Reported);
    assert_eq!(
        reported
            .units
            .iter()
            .map(|unit| unit.quantity)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_ne!(silent.call_id, reported.call_id);
    fixture.close().await;
}

// The charge-site guard, its fixtures and the pinned list of unmetered charges.
#[path = "cost_charge_guard_tests.rs"]
mod charge_guard;

// Tiers, carried remainders, maxima, the provider meter and the metering port.
#[path = "cost_unsent_cleanup_tests.rs"]
mod unsent_cleanup;

#[path = "cost_metering_tests.rs"]
mod metering;

/// The ids of the rows of the "Cost engine backlog" section of the triage file.
fn backlog_rows(triage: &str) -> BTreeSet<&str> {
    let section = triage
        .split("\n## ")
        .find(|section| section.starts_with("Cost engine backlog"))
        .expect("the Cost engine backlog section");
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| "))
        .filter_map(|row| row.split(' ').next())
        .filter(|id| id.starts_with("C-B"))
        .collect()
}

/// Every backlog id (`C-B` and digits) mentioned in `text`, comments included.
fn backlog_mentions(text: &str) -> Vec<&str> {
    if !text.contains("C-B") {
        return Vec::new();
    }
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .filter(|word| {
            word.strip_prefix("C-B").is_some_and(|number| {
                !number.is_empty() && number.chars().all(|digit| digit.is_ascii_digit())
            })
        })
        .collect()
}

/// The `(source, id)` pairs among `sources` (name and text) whose id is not a row of the backlog
/// section.
fn unknown_citations<'a>(triage: &str, sources: &'a [(String, String)]) -> Vec<(&'a str, &'a str)> {
    let rows = backlog_rows(triage);
    let mut unknown = Vec::new();
    for (name, text) in sources {
        for id in backlog_mentions(text) {
            if !rows.contains(id) {
                unknown.push((name.as_str(), id));
            }
        }
    }
    unknown
}

/// Every `.rs` file under `crates/`, as (path under `crates/`, text), in path order. This test
/// file is left out: its fixtures cite ids that are not rows, on purpose.
fn workspace_sources() -> Vec<(String, String)> {
    fn visit(directory: &std::path::Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("sources") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "target") {
                    visit(&path, files);
                }
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();
    let mut sources = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(&root)
            .expect("under crates")
            .to_string_lossy()
            .replace('\\', "/");
        if name == "df-tools/src/gameplay/cost_tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{name} cannot be read: {error}"));
        sources.push((name, text));
    }
    sources
}

#[test]
fn a_cited_backlog_id_that_is_not_a_row_is_reported() {
    let triage = "# Triage\n\n## Cost engine backlog (C-B1..C-B2)\n\n| Id | Item | Where |\n\
                  |---|---|---|\n| C-B1 | one | here |\n| C-B2 | two | there |\n\n\
                  ## Other\n\n| C-B7 | a row outside the section | x |\n";
    assert_eq!(backlog_rows(triage), BTreeSet::from(["C-B1", "C-B2"]));
    let sources = [
        (
            "df-tools/src/a.rs".to_owned(),
            "// TODO(C-B1): fine".to_owned(),
        ),
        (
            "df-providers/src/b.rs".to_owned(),
            "// see C-B99 and C-B7, then C-B2.".to_owned(),
        ),
    ];
    assert_eq!(
        unknown_citations(triage, &sources),
        [
            ("df-providers/src/b.rs", "C-B99"),
            ("df-providers/src/b.rs", "C-B7")
        ]
    );
    assert_eq!(
        backlog_mentions("TODO(C-B3): see C-B13) and C-Bn, C-B, C-B4x, C-B5."),
        ["C-B3", "C-B13", "C-B5"]
    );
}

/// Every id any source file of the workspace cites (`cost.rs` holds most: its comments and the
/// routes' reasons; the charge guard's pinned list holds the rest) is a row of the section.
#[test]
fn every_cited_backlog_id_is_a_row_of_the_cost_engine_backlog() {
    let triage = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../planning/issue-triage.md"
    ))
    .expect("planning/issue-triage.md");
    let sources = workspace_sources();
    assert_eq!(
        unknown_citations(&triage, &sources),
        Vec::<(&str, &str)>::new(),
        "cited but not a row of the Cost engine backlog in planning/issue-triage.md"
    );
}

// Pinned imagery cost projection: one stable id per dispatch, same-id correction.

/// A pinned call id for `n`: dispatch-derived in shape, deterministic for tests.
fn pinned_id(n: u64) -> String {
    format!("{PINNED_CALL_PREFIX}{n:032x}")
}

fn pinned_unit(category: &str, quantity: u64) -> port::MeteredUnits {
    port::MeteredUnits {
        category: category.to_owned(),
        quantity,
    }
}

/// The imagery route the pinned tests project: the checked-in test rates price it.
fn pinned_route() -> port::PortRoute {
    port::PortRoute {
        provider: "openai".to_owned(),
        model: "gpt-image-2.5-flare".to_owned(),
        cause: "scene_plate".to_owned(),
        detail: "pinned-test".to_owned(),
        maxima: vec![
            pinned_unit("text_input", 2_000_000),
            pinned_unit("image_output", 1_000_000),
        ],
    }
}

fn pinned_port(run: &str) -> port::MeteringPort {
    let port = port::MeteringPort::new();
    port.open(run, TABLE).expect("opens");
    port
}

fn pinned_spec() -> CallSpec {
    CallSpec::new(
        Provider::OpenAi,
        "gpt-image-2.5-flare",
        Cause::ScenePlate,
        "pinned-test",
    )
    .with_maxima(vec![
        (Category::TextInput, 2_000_000),
        (Category::ImageOutput, 1_000_000),
    ])
}

struct PinnedRecorder {
    started: Mutex<u64>,
    settled: Mutex<Vec<port::MeteredCall>>,
}

/// An unknown-outcome report with no quantities: the book stands it for the call's maxima.
fn unknown_usage() -> Usage {
    Usage {
        source: UsageSource::Unknown,
        quantities: Vec::new(),
        usable: true,
    }
}

impl port::MeterObserver for PinnedRecorder {
    fn started(&self, _start: &port::MeterStart) {
        *self.started.lock().expect("started") += 1;
    }

    fn settled(&self, call: &port::MeteredCall) {
        self.settled.lock().expect("settled").push(call.clone());
    }
}

#[test]
fn a_pinned_unknown_then_reported_correction_is_one_call_with_the_latest_evidence() {
    let port = pinned_port("run");
    let id = pinned_id(1);
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Ok(PinnedStart::Opened)
    );
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Unknown, &[], true, 1_000),
        Ok(PinnedCorrection::Applied)
    );
    // Unknown stands for the maxima: 2M text tokens at 5 plus 1M output tokens at 30.
    assert_eq!(
        port.totals(),
        port::MeterTotals {
            calls: 1,
            micro_usd: 40000000,
            unpriced_calls: 0,
        }
    );
    let recent = port.recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].call_id, id);
    assert!(!recent[0].correction);
    assert_eq!(recent[0].source, port::MeteredSource::Unknown);
    assert_eq!(recent[0].at_ms, 1_000);
    assert_eq!(
        port.correct_pinned_at(
            &id,
            1,
            port::MeteredSource::Reported,
            &[pinned_unit("image_output", 2_000_000)],
            true,
            2_000,
        ),
        Ok(PinnedCorrection::Applied)
    );
    assert_eq!(
        port.totals(),
        port::MeterTotals {
            calls: 1,
            micro_usd: 60000000,
            unpriced_calls: 0,
        },
        "the correction replaces the totals under the same id"
    );
    let recent = port.recent();
    assert_eq!(
        recent.len(),
        1,
        "the correction replaces the recent entry under the same id"
    );
    assert_eq!(recent[0].call_id, id);
    assert!(recent[0].correction);
    assert_eq!(recent[0].source, port::MeteredSource::Reported);
    assert_eq!(
        recent[0].units,
        vec![pinned_unit("image_output", 2_000_000)]
    );
}

#[test]
fn an_exact_replay_of_a_pinned_start_or_correction_changes_nothing() {
    let port = pinned_port("run");
    let recorder = Arc::new(PinnedRecorder {
        started: Mutex::new(0),
        settled: Mutex::new(Vec::new()),
    });
    port.observe(recorder.clone());
    let id = pinned_id(2);
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Ok(PinnedStart::Opened)
    );
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Ok(PinnedStart::ReplayedOpen)
    );
    let units = vec![pinned_unit("image_output", 1_000_000)];
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &units, true, 1_000),
        Ok(PinnedCorrection::Applied)
    );
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &units, true, 1_000),
        Ok(PinnedCorrection::Replayed)
    );
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(port.totals().calls, 1);
    assert_eq!(port.totals().micro_usd, 30000000);
    assert_eq!(*recorder.started.lock().expect("started"), 1);
    let settled = recorder.settled.lock().expect("settled");
    assert_eq!(settled.len(), 1, "a replay notifies nothing");
    assert_eq!(settled[0].call_id, id);
}

#[test]
fn a_newer_pinned_revision_replaces_totals_rollups_and_recent_under_the_same_id() {
    let port = pinned_port("run");
    let id = pinned_id(3);
    port.start_pinned(&id, &pinned_route()).expect("opens");
    port.correct_pinned_at(
        &id,
        0,
        port::MeteredSource::Reported,
        &[pinned_unit("image_output", 1_000_000)],
        true,
        1_000,
    )
    .expect("settles");
    assert_eq!(port.totals().micro_usd, 30000000);
    port.correct_pinned_at(
        &id,
        1,
        port::MeteredSource::Estimated,
        &[pinned_unit("text_input", 1_000_000)],
        true,
        2_000,
    )
    .expect("corrects");
    assert_eq!(
        port.totals(),
        port::MeterTotals {
            calls: 1,
            micro_usd: 5000000,
            unpriced_calls: 0,
        }
    );
    let recent = port.recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].source, port::MeteredSource::Estimated);
    assert_eq!(recent[0].units, vec![pinned_unit("text_input", 1_000_000)]);
}

#[test]
fn an_out_of_order_or_conflicting_pinned_revision_is_refused_and_keeps_state() {
    let port = pinned_port("run");
    let id = pinned_id(4);
    port.start_pinned(&id, &pinned_route()).expect("opens");
    let rev2 = vec![pinned_unit("image_output", 1_000_000)];
    port.correct_pinned_at(&id, 2, port::MeteredSource::Reported, &rev2, true, 1_000)
        .expect("settles");
    assert_eq!(
        port.correct_pinned_at(&id, 1, port::MeteredSource::Reported, &rev2, true, 2_000),
        Err(PinnedError::StaleRevision { stored: 2, got: 1 })
    );
    assert_eq!(
        port.correct_pinned_at(
            &id,
            2,
            port::MeteredSource::Reported,
            &[pinned_unit("image_output", 2_000_000)],
            true,
            2_000
        ),
        Err(PinnedError::RevisionConflict { revision: 2 })
    );
    assert_eq!(port.totals().calls, 1);
    assert_eq!(port.totals().micro_usd, 30000000);
    assert_eq!(port.recent()[0].units, rev2);
}

#[test]
fn a_pinned_call_with_unpriced_lines_keeps_them_visible_and_prices_the_rest() {
    let port = pinned_port("run");
    let id = pinned_id(5);
    let mut route = pinned_route();
    route.maxima = vec![
        pinned_unit("text_input", 1_000_000),
        pinned_unit("credits", 7),
    ];
    port.start_pinned(&id, &route).expect("opens");
    assert_eq!(
        port.correct_pinned_at(
            &id,
            0,
            port::MeteredSource::Reported,
            &[
                pinned_unit("text_input", 1_000_000),
                pinned_unit("credits", 7)
            ],
            true,
            1_000,
        ),
        Ok(PinnedCorrection::Applied)
    );
    // The priced line pays its exact price; the unpriced line counts zero but stays visible.
    assert_eq!(
        port.totals(),
        port::MeterTotals {
            calls: 1,
            micro_usd: 5000000,
            unpriced_calls: 1,
        }
    );
    let recent = port.recent();
    assert_eq!(recent.len(), 1);
    assert!(recent[0].unpriced);
    assert_eq!(recent[0].units.len(), 2);
}

#[test]
fn a_billed_but_unusable_pinned_call_stays_attributed_as_a_company_loss() {
    let ledger = opened_ledger(None);
    let id = pinned_id(6);
    ledger
        .costs()
        .start_pinned(&id, pinned_spec())
        .expect("opens");
    ledger
        .costs()
        .correct_pinned(
            &id,
            0,
            Usage::reported(vec![(Category::ImageOutput, 1_000_000)]).unusable(),
            1_000,
        )
        .expect("settles");
    let report = ledger.costs().report(2_000);
    assert_eq!(report["run"]["events"], 1);
    assert_eq!(report["run"]["micro_usd"], 30_000_000);
    assert_eq!(
        report["run"]["unusable_events"], 1,
        "the loss is attributed, never charged on"
    );
    assert_eq!(report["run"]["unpriced_events"], 0);
}

#[test]
fn a_proven_unsent_pinned_call_records_nothing_and_cannot_be_rebilled() {
    let port = pinned_port("run");
    let id = pinned_id(7);
    assert_eq!(
        port.record_pinned_unsent(&id),
        Err(PinnedError::UnknownCall),
        "unsent needs its start first, like a correction does"
    );
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &[], true, 1_000),
        Err(PinnedError::UnknownCall)
    );
    assert_eq!(
        port.start_pinned("", &pinned_route()),
        Err(PinnedError::UnknownCall),
        "an empty id never opens a call"
    );
    port.start_pinned(&id, &pinned_route()).expect("opens");
    assert_eq!(port.record_pinned_unsent(&id), Ok(PinnedUnsent::Recorded));
    assert_eq!(port.totals().calls, 0);
    assert!(port.recent().is_empty());
    assert_eq!(port.record_pinned_unsent(&id), Ok(PinnedUnsent::Replayed));
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &[], true, 2_000),
        Err(PinnedError::AlreadySettled),
        "a sent-proof call cannot settle what was proven unsent"
    );
}

#[test]
fn a_pinned_start_conflicts_on_a_different_route() {
    let port = pinned_port("run");
    let id = pinned_id(8);
    port.start_pinned(&id, &pinned_route()).expect("opens");
    let mut other = pinned_route();
    other.detail = "another-dispatch".to_owned();
    assert_eq!(
        port.start_pinned(&id, &other),
        Err(PinnedError::RouteConflict)
    );
    port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &[], true, 1_000)
        .expect("the first route still settles");
    assert_eq!(port.totals().calls, 1);
}

#[test]
fn a_pinned_correction_reaches_observers_reports_and_rollups_exactly_once() {
    let ledger = opened_ledger(None);
    let recorder = Arc::new(PinnedRecorder {
        started: Mutex::new(0),
        settled: Mutex::new(Vec::new()),
    });
    ledger.costs().observe(recorder.clone());
    let id = pinned_id(9);
    ledger
        .costs()
        .start_pinned(&id, pinned_spec())
        .expect("opens");
    ledger
        .costs()
        .correct_pinned(&id, 0, unknown_usage(), 1_000)
        .expect("settles unknown");
    let report = ledger.costs().report(1_500);
    assert_eq!(report["run"]["events"], 1);
    assert_eq!(report["run"]["unknown_events"], 1);
    ledger
        .costs()
        .correct_pinned(
            &id,
            1,
            Usage::estimated(vec![(Category::TextInput, 1_000_000)]),
            2_000,
        )
        .expect("corrects");
    assert_eq!(*recorder.started.lock().expect("started"), 1);
    let settled = recorder.settled.lock().expect("settled");
    assert_eq!(settled.len(), 2);
    assert_eq!(settled[0].call_id, id);
    assert!(!settled[0].correction);
    assert_eq!(settled[1].call_id, id);
    assert!(settled[1].correction, "the correction is identifiable");
    drop(settled);
    let report = ledger.costs().report(3_000);
    assert_eq!(report["run"]["events"], 1, "one call, never two");
    assert_eq!(report["run"]["micro_usd"], 5_000_000);
    assert_eq!(report["run"]["estimated_events"], 1);
    assert_eq!(report["run"]["unknown_events"], 0);
    let causes = report["run"]["by_cause"]
        .as_array()
        .expect("by_cause rollup");
    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0]["cause"], "scene_plate");
    assert_eq!(causes[0]["events"], 1);
    assert_eq!(causes[0]["micro_usd"], 5_000_000);
}

#[test]
fn a_reopened_book_replays_the_latest_pinned_revision_without_double_counting() {
    let directory = scratch("pinned-reopen");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(40);
    {
        let first = CostBook::with_session("s1");
        first
            .open(
                "df_gameplay_demo_test",
                Some(&log),
                table(),
                PricingStatus::NotLoaded,
                1_000,
            )
            .expect("opens");
        first.start_pinned(&id, pinned_spec()).expect("opens");
        first
            .correct_pinned(&id, 0, unknown_usage(), 1_000)
            .expect("settles unknown");
        first
            .correct_pinned(
                &id,
                1,
                Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
                2_000,
            )
            .expect("corrects");
        assert_eq!(state_of(&first).run_totals.events, 1);
        first.close();
    }
    let second = CostBook::with_session("s2");
    let opened = second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            3_000,
        )
        .expect("reopens");
    assert_eq!(opened.malformed, 0);
    assert_eq!(opened.replayed, 1, "only the latest revision replays");
    assert_eq!(state_of(&second).run_totals.events, 1);
    assert_eq!(state_of(&second).run_totals.micro_usd, 60000000);
    assert_eq!(
        second.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::AlreadySettled),
        "the replayed call keeps its identity"
    );
    assert_eq!(
        second.correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
            4_000,
        ),
        Ok(PinnedCorrection::Replayed)
    );
    assert_eq!(
        second.correct_pinned(&id, 0, unknown_usage(), 4_000),
        Err(PinnedError::StaleRevision { stored: 1, got: 0 })
    );
    second
        .correct_pinned(
            &id,
            2,
            Usage::reported(vec![(Category::TextInput, 1_000_000)]),
            4_000,
        )
        .expect("corrects");
    assert_eq!(state_of(&second).run_totals.events, 1);
    assert_eq!(state_of(&second).run_totals.micro_usd, 5000000);
}

#[test]
fn unpersisted_pinned_calls_stay_correctable_and_backpressure_new_identities() {
    let book = opened_book("unpersisted", None);
    for n in 0..8 {
        book.start_pinned(&pinned_id(n), pinned_spec()).unwrap();
        book.correct_pinned(&pinned_id(n), 0, unknown_usage(), 1_000)
            .unwrap();
    }
    assert_eq!(
        book.start_pinned(&pinned_id(9), pinned_spec()),
        Err(PinnedError::ProjectionFull)
    );
    book.correct_pinned(
        &pinned_id(0),
        1,
        Usage::reported(vec![(Category::ImageOutput, 0)]),
        2_000,
    )
    .unwrap();
    assert_eq!(book.totals().calls, 8);
    assert_eq!(book.totals().micro_usd, 7 * 40_000_000);
}

#[test]
fn a_repeated_pinned_revision_with_the_same_usage_keeps_identical_money() {
    let book = opened_book("s1", None);
    let output = |tokens: u64| Usage::reported(vec![(Category::TextOutput, tokens)]);
    // One ordinary token at half a micro-dollar rounds to one, carrying minus a half.
    book.meter(text_call("gpt-6-luna", "a"), output(1), 2_000);
    assert_eq!(state_of(&book).session_totals.micro_usd, 1);
    // A pinned call prices in isolation: one token is one, and the ordinary carry is
    // untouched by the interleave.
    let id = pinned_id(300);
    let spec = CallSpec::new(
        Provider::OpenAi,
        "gpt-6-luna",
        Cause::Storyteller,
        "pinned-test",
    )
    .with_maxima(vec![(Category::TextOutput, 10)]);
    book.start_pinned(&id, spec).expect("opens");
    book.correct_pinned(&id, 0, output(1), 2_001)
        .expect("settles");
    assert_eq!(state_of(&book).session_totals.micro_usd, 2);
    // The ordinary carry still holds its minus half: the next ordinary token costs nothing.
    book.meter(text_call("gpt-6-luna", "a"), output(1), 2_002);
    assert_eq!(state_of(&book).session_totals.micro_usd, 2);
    // New revisions with the same usage keep identical money instead of pumping rounding.
    for revision in 1..4 {
        book.correct_pinned(&id, revision, output(1), 2_003 + revision)
            .expect("corrects");
        assert_eq!(state_of(&book).session_totals.micro_usd, 2);
    }
    // Corrections up and down move the totals exactly: three tokens are two micro-dollars.
    book.correct_pinned(&id, 4, output(3), 3_000)
        .expect("corrects up");
    assert_eq!(state_of(&book).session_totals.micro_usd, 3);
    book.correct_pinned(&id, 5, output(1), 3_001)
        .expect("corrects down");
    assert_eq!(state_of(&book).session_totals.micro_usd, 2);
    let state = state_of(&book);
    assert_eq!(state.session_totals.events, 3);
    let lines: u64 = state
        .session_totals
        .by_line
        .values()
        .map(|total| total.micro_usd)
        .sum();
    assert_eq!(lines, state.session_totals.micro_usd);
}

#[test]
fn a_rate_change_never_reprices_replayed_history_or_corrections() {
    let directory = scratch("pinned-rates");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(310);
    let first = CostBook::with_session("s1");
    first
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            1_000,
        )
        .expect("opens");
    first.start_pinned(&id, pinned_spec()).expect("opens");
    first
        .correct_pinned(
            &id,
            0,
            Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
            1_000,
        )
        .expect("settles");
    assert_eq!(state_of(&first).run_totals.micro_usd, 30000000);
    assert!(first.flush());
    drop(first);
    let dearer = PriceTable::parse(&TABLE.replace("\"usd\": \"30\"", "\"usd\": \"60\""))
        .expect("a dearer table");
    let second = CostBook::with_session("s2");
    let opened = second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            dearer,
            PricingStatus::NotLoaded,
            2_000,
        )
        .expect("reopens");
    assert_eq!(opened.replayed, 1);
    assert_eq!(
        state_of(&second).run_totals.micro_usd,
        30000000,
        "replay never reprices stored lines"
    );
    second
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
            3_000,
        )
        .expect("corrects");
    assert_eq!(
        state_of(&second).run_totals.micro_usd,
        30_000_000,
        "the correction retains the original frozen tariff"
    );
}

#[test]
fn an_unknown_report_after_a_lower_report_uses_the_route_maxima_not_the_lower_usage() {
    let directory = scratch("pinned-maxima");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(320);
    let first = opened_book("s1", Some(&log));
    first.start_pinned(&id, pinned_spec()).expect("opens");
    // Unknown stands for the route maxima: two million text tokens at five plus one
    // million output tokens at thirty micro-dollars per million.
    first
        .correct_pinned(&id, 0, unknown_usage(), 1_000)
        .expect("settles unknown");
    assert_eq!(state_of(&first).run_totals.micro_usd, 40000000);
    // A lower report replaces the totals but never the route maxima.
    first
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 100)]),
            2_000,
        )
        .expect("corrects down");
    assert_eq!(state_of(&first).run_totals.micro_usd, 3_000);
    first
        .correct_pinned(&id, 2, unknown_usage(), 3_000)
        .expect("unknown again");
    assert_eq!(
        state_of(&first).run_totals.micro_usd,
        40000000,
        "unknown uses the original maxima, not the prior lower usage"
    );
    assert!(first.flush());
    drop(first);
    // The maxima survive a restart too: the log carries them beside the event.
    let second = CostBook::with_session("s2");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            4_000,
        )
        .expect("reopens");
    second
        .correct_pinned(&id, 3, unknown_usage(), 5_000)
        .expect("unknown after reopen");
    assert_eq!(state_of(&second).run_totals.micro_usd, 40000000);
}

#[test]
fn a_correction_from_a_later_session_keeps_the_origin_session_and_leaves_the_new_session() {
    let directory = scratch("pinned-session");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(330);
    {
        let first = CostBook::with_session("s1");
        first
            .open(
                "df_gameplay_demo_test",
                Some(&log),
                table(),
                PricingStatus::NotLoaded,
                1_000,
            )
            .expect("opens");
        first.start_pinned(&id, pinned_spec()).expect("opens");
        first
            .correct_pinned(
                &id,
                0,
                Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
                1_000,
            )
            .expect("settles");
        assert_eq!(state_of(&first).run_totals.micro_usd, 60000000);
        assert!(first.flush());
    }
    let second = CostBook::with_session("s2");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            2_000,
        )
        .expect("reopens");
    assert_eq!(state_of(&second).session_totals.events, 0);
    second
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::TextInput, 1_000_000)]),
            3_000,
        )
        .expect("corrects");
    let state = state_of(&second);
    assert_eq!(state.run_totals.events, 1, "one call, never two");
    assert_eq!(state.run_totals.micro_usd, 5000000);
    assert_eq!(
        state.session_totals.events, 0,
        "the foreign correction adds no event to the new session"
    );
    assert_eq!(
        state.session_totals.micro_usd, 0,
        "the old cost is never subtracted from the unrelated new session"
    );
    let held = state.pinned.get(&id).expect("a live record");
    match held {
        PinnedSlot::Settled(record) => {
            assert_eq!(
                record.event.session, "s1",
                "origin attribution is preserved"
            )
        }
        _ => panic!("the corrected call stays settled"),
    }
    let live_run = state.run_totals.clone();
    drop(state);
    second.close();
    drop(second);
    // A fresh replay of the same log agrees with the live book fully.
    let third = CostBook::with_session("s3");
    third
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            4_000,
        )
        .expect("reopens");
    assert_eq!(state_of(&third).run_totals, live_run);
}

#[test]
fn a_conflicting_log_revision_keeps_the_first_line_and_counts_malformed() {
    let directory = scratch("pinned-conflict");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(340);
    {
        let first = opened_book("s1", Some(&log));
        first.start_pinned(&id, pinned_spec()).expect("opens");
        first
            .correct_pinned(
                &id,
                0,
                Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
                1_000,
            )
            .expect("settles");
        assert!(first.flush());
    }
    // A same-revision different payload appended behind the book's back.
    let text = std::fs::read_to_string(&log).expect("log");
    let mut conflict: serde_json::Value = serde_json::from_str(
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap(),
    )
    .expect("one revisioned line");
    conflict["event"]["lines"] =
        serde_json::json!([{"category": "text_input", "quantity": 1_000_000, "micro_usd": 5}]);
    conflict["event"]["micro_usd"] = serde_json::json!(5);
    std::fs::write(&log, format!("{text}{conflict}\n")).expect("conflict");
    let second = CostBook::with_session("s2");
    let opened = second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            2_000,
        )
        .expect("reopens");
    assert_eq!(
        opened.malformed, 1,
        "the same-revision different payload conflicts"
    );
    assert_eq!(state_of(&second).run_totals.events, 1);
    assert_eq!(
        state_of(&second).run_totals.micro_usd,
        30000000,
        "the first line stands; file order never silently wins"
    );
    assert_eq!(
        second.correct_pinned(
            &id,
            0,
            Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
            3_000,
        ),
        Ok(PinnedCorrection::Replayed)
    );
    // A settled line and an unsent marker for one id conflict the same way.
    let marker = serde_json::json!({"call_id": id, "pinned_unsent": true});
    std::fs::write(&log, format!("{text}{conflict}\n{marker}\n")).expect("marker");
    let third = CostBook::with_session("s3");
    let reopened = third
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            4_000,
        )
        .expect("reopens");
    assert_eq!(reopened.malformed, 2);
    assert_eq!(state_of(&third).run_totals.micro_usd, 30000000);
}

#[test]
fn a_revision_lost_before_the_log_still_corrects_under_the_same_id_after_restart() {
    let directory = scratch("pinned-lost");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(350);
    let live = CostBook::with_session("s1");
    live.open(
        "df_gameplay_demo_test",
        Some(&log),
        table(),
        PricingStatus::NotLoaded,
        1_000,
    )
    .expect("opens");
    live.start_pinned(&id, pinned_spec()).expect("opens");
    live.correct_pinned(&id, 0, unknown_usage(), 1_000)
        .expect("settles unknown");
    assert!(live.flush());
    live.close();
    assert_eq!(live.report(2_000)["log"]["status"], json!("closed"));
    // The log is closed, so this is a local acknowledgement only: nothing durable.
    live.correct_pinned(
        &id,
        1,
        Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
        2_000,
    )
    .expect("applies locally");
    let live_run = state_of(&live).run_totals.clone();
    assert_eq!(live_run.micro_usd, 60000000);
    drop(live);
    // Recovery replays the latest durable revision, and the journal's retry of the same id
    // corrects instead of double-counting.
    let second = CostBook::with_session("s2");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            3_000,
        )
        .expect("reopens");
    assert_eq!(state_of(&second).run_totals.micro_usd, 40000000);
    second
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
            4_000,
        )
        .expect("the retry lands");
    assert_eq!(state_of(&second).run_totals, live_run);
    assert_eq!(
        second.correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
            4_000,
        ),
        Ok(PinnedCorrection::Replayed)
    );
}

#[test]
fn a_truncated_log_replays_the_latest_durable_revision_and_the_retry_lands_once() {
    let directory = scratch("pinned-truncate");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(360);
    let live = CostBook::with_session("s1");
    live.open(
        "df_gameplay_demo_test",
        Some(&log),
        table(),
        PricingStatus::NotLoaded,
        1_000,
    )
    .expect("opens");
    live.start_pinned(&id, pinned_spec()).expect("opens");
    live.correct_pinned(&id, 0, unknown_usage(), 1_000)
        .expect("settles unknown");
    live.correct_pinned(
        &id,
        1,
        Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
        2_000,
    )
    .expect("corrects");
    assert!(live.flush());
    let live_run = state_of(&live).run_totals.clone();
    drop(live);
    // The newest append never reached the disk: durable state is the older revision.
    let text = std::fs::read_to_string(&log).expect("log");
    let kept = text
        .trim_end()
        .rsplit_once('\n')
        .map_or("", |(rest, _)| rest);
    std::fs::write(&log, format!("{kept}\n")).expect("truncate");
    let second = CostBook::with_session("s2");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            3_000,
        )
        .expect("reopens");
    assert_eq!(state_of(&second).run_totals.micro_usd, 40000000);
    second
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 2_000_000)]),
            4_000,
        )
        .expect("the retry lands");
    assert_eq!(state_of(&second).run_totals, live_run);
}

#[test]
fn a_durable_cold_pinned_call_still_corrects_after_many_cache_windows() {
    let directory = scratch("cold-correction");
    let log = directory.join("cost.jsonl");
    let book = opened_book("cold", Some(&log));
    for n in 0..40 {
        book.start_pinned(&pinned_id(n), pinned_spec()).unwrap();
        book.correct_pinned(&pinned_id(n), 0, unknown_usage(), 1_000)
            .unwrap();
        assert!(book.flush());
    }
    assert!(state_of(&book).pinned.len() <= PINNED_LIMIT);
    assert_eq!(
        book.start_pinned(&pinned_id(0), pinned_spec()),
        Ok(PinnedStart::AlreadySettled)
    );
    book.correct_pinned(
        &pinned_id(0),
        1,
        Usage::reported(vec![(Category::ImageOutput, 1)]),
        2_000,
    )
    .unwrap();
    assert_eq!(book.totals().calls, 40);
    assert_eq!(book.totals().micro_usd, 39 * 40_000_000 + 30);
    assert!(book.flush());
    book.close();
    let reopened = opened_book("restart", Some(&log));
    assert_eq!(reopened.totals(), book.totals());
    reopened
        .correct_pinned(&pinned_id(0), 2, unknown_usage(), 3_000)
        .unwrap();
    assert_eq!(reopened.totals().micro_usd, 40 * 40_000_000);
}

#[test]
fn pinned_starts_past_a_full_window_of_open_calls_are_refused() {
    let port = pinned_port("run");
    for n in 0..8 {
        port.start_pinned(&pinned_id(600 + n), &pinned_route())
            .expect("opens");
    }
    assert_eq!(
        port.start_pinned(&pinned_id(700), &pinned_route()),
        Err(PinnedError::ProjectionFull),
        "open calls never retire to make room"
    );
    // Unpersisted terminal records stay retained; only a durable log can release a slot.
    port.correct_pinned_at(
        &pinned_id(600),
        0,
        port::MeteredSource::Reported,
        &[pinned_unit("image_output", 1_000_000)],
        true,
        1_000,
    )
    .expect("settles in place");
    assert_eq!(
        port.start_pinned(&pinned_id(700), &pinned_route()),
        Err(PinnedError::ProjectionFull)
    );
}

#[test]
fn a_retired_unsent_call_stays_terminal() {
    let directory = scratch("cold-unsent");
    let log = directory.join("cost.jsonl");
    let book = opened_book("unsent", Some(&log));
    let id = pinned_id(900);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.record_pinned_unsent(&id).unwrap();
    assert!(book.flush());
    for n in 0..24 {
        book.start_pinned(&pinned_id(1_000 + n), pinned_spec())
            .unwrap();
        book.correct_pinned(&pinned_id(1_000 + n), 0, unknown_usage(), 1_000)
            .unwrap();
        assert!(book.flush());
    }
    assert_eq!(
        book.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(book.record_pinned_unsent(&id), Ok(PinnedUnsent::Replayed));
    assert_eq!(
        book.correct_pinned(&id, 0, unknown_usage(), 2_000),
        Err(PinnedError::AlreadySettled)
    );
    assert_eq!(book.totals().calls, 24);
}

#[test]
fn a_proven_unsent_call_stays_terminal_through_recovery() {
    let directory = scratch("pinned-unsent");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(910);
    {
        let first = opened_book("s1", Some(&log));
        first.start_pinned(&id, pinned_spec()).expect("opens");
        assert_eq!(first.record_pinned_unsent(&id), Ok(PinnedUnsent::Recorded));
        assert!(first.flush());
    }
    let text = std::fs::read_to_string(&log).expect("log");
    assert!(
        text.contains("\"pinned_unsent\":true"),
        "the terminal marker is durable: {text}"
    );
    let second = CostBook::with_session("s2");
    let opened = second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            2_000,
        )
        .expect("reopens");
    assert_eq!(opened.replayed, 0, "the marker carries no money");
    assert_eq!(
        second.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(second.record_pinned_unsent(&id), Ok(PinnedUnsent::Replayed));
    assert_eq!(
        second.correct_pinned(&id, 0, unknown_usage(), 3_000),
        Err(PinnedError::AlreadySettled),
        "replay cannot reopen unsent as billed"
    );
    assert_eq!(state_of(&second).run_totals.events, 0);
}

#[test]
fn an_unusable_pinned_call_keeps_its_loss_through_recovery() {
    let directory = scratch("pinned-unusable");
    let log = directory.join("cost.jsonl");
    let id = pinned_id(920);
    {
        let first = opened_book("s1", Some(&log));
        first.start_pinned(&id, pinned_spec()).expect("opens");
        first
            .correct_pinned(
                &id,
                0,
                Usage::reported(vec![(Category::ImageOutput, 1_000_000)]).unusable(),
                1_000,
            )
            .expect("settles");
        assert!(first.flush());
    }
    let second = CostBook::with_session("s2");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            2_000,
        )
        .expect("reopens");
    let report = second.report(3_000);
    assert_eq!(report["run"]["events"], 1);
    assert_eq!(report["run"]["micro_usd"], 30_000_000);
    assert_eq!(report["run"]["unusable_events"], 1);
}

#[test]
fn a_subscription_report_keeps_its_token_units_visible_without_a_tariff() {
    let book = opened_book("s1", None);
    let spec = CallSpec::new(
        Provider::CodexSubscription,
        "codex-cli",
        Cause::Storyteller,
        "npc_reply",
    )
    .with_maxima(vec![
        (Category::Calls, 1),
        (Category::TextInput, 1_000),
        (Category::TextOutput, 200),
    ]);
    let usage = Usage::reported(vec![
        (Category::Calls, 1),
        (Category::TextInput, 1_000),
        (Category::TextOutput, 200),
    ]);
    assert_eq!(
        usage.quantities,
        vec![
            (Category::Calls, 1),
            (Category::TextInput, 1_000),
            (Category::TextOutput, 200),
        ]
    );
    book.meter(spec, usage, 2_000);
    let state = state_of(&book);
    assert_eq!(state.session_totals.events, 1);
    assert_eq!(
        state.session_totals.micro_usd, 0,
        "no tariff is invented for subscription units"
    );
    assert_eq!(state.session_totals.unpriced_events, 1);
    let stored = |category| {
        state.session_totals.by_line[&(
            Provider::CodexSubscription,
            "codex-cli".to_owned(),
            category,
        )]
            .quantity
    };
    assert_eq!(stored(Category::Calls), 1);
    assert_eq!(stored(Category::TextInput), 1_000);
    assert_eq!(stored(Category::TextOutput), 200);
}

#[test]
fn a_local_record_only_report_projects_unpriced_and_priced_lines_keep_their_amount() {
    let book = opened_book("s1", None);
    let local = CallSpec::new(
        Provider::Local,
        "local-transform",
        Cause::PlateEdit,
        "soften",
    )
    .with_maxima(vec![(Category::Calls, 1)]);
    book.meter(local, Usage::reported(vec![(Category::Calls, 1)]), 2_000);
    let mixed = CallSpec::new(
        Provider::OpenAi,
        "gpt-image-2.5-flare",
        Cause::ScenePlate,
        "pinned-mix",
    )
    .with_maxima(vec![
        (Category::TextInput, 1_000_000),
        (Category::Credits, 7),
    ]);
    book.meter(
        mixed,
        Usage::reported(vec![
            (Category::TextInput, 1_000_000),
            (Category::Credits, 7),
        ]),
        2_001,
    );
    let state = state_of(&book);
    assert_eq!(
        state.session_totals.micro_usd, 5000000,
        "the priced line keeps its amount beside unpriced ones"
    );
    assert_eq!(state.session_totals.unpriced_events, 2);
    assert_eq!(
        state.session_totals.by_line[&(
            Provider::OpenAi,
            "gpt-image-2.5-flare".to_owned(),
            Category::Credits,
        )]
            .unpriced_quantity,
        7,
        "the unpriced units stay visible"
    );
    // An invalid route is refused; an unknown price is projected, never zeroed.
    let port = pinned_port("run");
    let mut bad = pinned_route();
    bad.provider = "acme".to_owned();
    assert_eq!(
        port.start_pinned(&pinned_id(950), &bad),
        Err(PinnedError::UnknownProvider)
    );
    let mut bad = pinned_route();
    bad.maxima[0].category = "tokens".to_owned();
    assert_eq!(
        port.start_pinned(&pinned_id(951), &bad),
        Err(PinnedError::UnknownCategory)
    );
    let mut reasoning = pinned_route();
    reasoning
        .maxima
        .push(pinned_unit("text_reasoning_output", 40));
    port.start_pinned(&pinned_id(952), &reasoning)
        .expect("a real receipt category projects");
    port.correct_pinned_at(
        &pinned_id(952),
        0,
        port::MeteredSource::Reported,
        &[
            pinned_unit("text_input", 2_000_000),
            pinned_unit("text_reasoning_output", 40),
        ],
        true,
        1_000,
    )
    .expect("settles");
    assert_eq!(port.totals().micro_usd, 10_000_000);
    assert_eq!(port.totals().unpriced_calls, 1);
    assert_eq!(port.recent()[0].units.len(), 2);
}

#[test]
fn a_pinned_call_needs_an_open_book() {
    let port = port::MeteringPort::new();
    let id = pinned_id(50);
    assert_eq!(
        port.start_pinned(&id, &pinned_route()),
        Err(PinnedError::BookNotOpen)
    );
    assert_eq!(
        port.correct_pinned_at(&id, 0, port::MeteredSource::Reported, &[], true, 1_000),
        Err(PinnedError::BookNotOpen)
    );
    assert_eq!(
        port.record_pinned_unsent(&id),
        Err(PinnedError::BookNotOpen)
    );
}

#[test]
fn an_identical_revision_repairs_a_rejected_append_before_durable_acknowledgement() {
    let directory = scratch("rejected-append");
    let log = directory.join("cost.jsonl");
    let book = opened_book("first", Some(&log));
    let id = pinned_id(800);
    book.start_pinned(&id, pinned_spec()).unwrap();
    state_of(&book)
        .log
        .as_ref()
        .unwrap()
        .reject_next
        .store(true, Ordering::SeqCst);
    assert_eq!(
        book.correct_pinned(&id, 1, unknown_usage(), 2_000),
        Ok(PinnedCorrection::Applied)
    );
    assert!(!book.pinned_durable(&id, Some(1)).unwrap());
    assert_eq!(
        book.correct_pinned(&id, 1, unknown_usage(), 2_001),
        Ok(PinnedCorrection::Replayed)
    );
    assert!(book.flush());
    assert!(book.pinned_durable(&id, Some(1)).unwrap());
    book.close();
    let recovered = opened_book("second", Some(&log));
    assert_eq!(recovered.totals(), book.totals());
    assert_eq!(
        recovered.correct_pinned(&id, 1, unknown_usage(), 3_000),
        Ok(PinnedCorrection::Replayed)
    );
}

#[test]
fn an_identical_unsent_marker_repairs_a_rejected_append() {
    let directory = scratch("rejected-unsent");
    let log = directory.join("cost.jsonl");
    let book = opened_book("first", Some(&log));
    let id = pinned_id(801);
    book.start_pinned(&id, pinned_spec()).unwrap();
    state_of(&book)
        .log
        .as_ref()
        .unwrap()
        .reject_next
        .store(true, Ordering::SeqCst);
    assert_eq!(book.record_pinned_unsent(&id), Ok(PinnedUnsent::Recorded));
    assert!(!book.pinned_durable(&id, None).unwrap());
    assert_eq!(book.record_pinned_unsent(&id), Ok(PinnedUnsent::Replayed));
    assert!(book.flush());
    assert!(book.pinned_durable(&id, None).unwrap());
    book.close();
    let recovered = opened_book("second", Some(&log));
    assert_eq!(
        recovered.record_pinned_unsent(&id),
        Ok(PinnedUnsent::Replayed)
    );
    assert_eq!(
        recovered.correct_pinned(&id, 1, unknown_usage(), 2_000),
        Err(PinnedError::AlreadySettled)
    );
    assert_eq!(recovered.totals().calls, 0);
}

#[test]
fn malformed_ids_and_mutated_settled_or_unsent_routes_are_refused() {
    let book = opened_book("identity", None);
    for id in ["oops", "img-1", "img-0000000000000000000000000000000A"] {
        assert_eq!(
            book.start_pinned(id, pinned_spec()),
            Err(PinnedError::UnknownCall)
        );
    }
    for (n, unsent) in [(802, false), (803, true)] {
        let id = pinned_id(n);
        book.start_pinned(&id, pinned_spec()).unwrap();
        if unsent {
            book.record_pinned_unsent(&id).unwrap();
        } else {
            book.correct_pinned(&id, 1, unknown_usage(), 1_000).unwrap();
        }
        for field in 0..5 {
            let mut changed = pinned_spec();
            match field {
                0 => changed.provider = Provider::Bfl,
                1 => changed.model = "other".to_owned(),
                2 => changed.cause = Cause::Storyteller,
                3 => changed.detail = "other".to_owned(),
                _ => changed.maxima[0].1 += 1,
            }
            assert_eq!(
                book.start_pinned(&id, changed),
                Err(PinnedError::RouteConflict)
            );
        }
    }
}

#[test]
fn unknown_with_nonempty_or_zero_units_always_uses_frozen_maxima_after_recovery() {
    let directory = scratch("unknown-nonempty");
    let log = directory.join("cost.jsonl");
    let book = opened_book("first", Some(&log));
    let id = pinned_id(804);
    book.start_pinned(&id, pinned_spec()).unwrap();
    let unknown = |quantity| Usage {
        source: UsageSource::Unknown,
        quantities: vec![(Category::ImageOutput, quantity)],
        usable: true,
    };
    book.correct_pinned(&id, 1, unknown(1), 1_000).unwrap();
    assert_eq!(book.totals().micro_usd, 40_000_000);
    assert!(book.flush());
    book.close();
    let recovered = opened_book("second", Some(&log));
    recovered.correct_pinned(&id, 2, unknown(0), 2_000).unwrap();
    assert_eq!(recovered.totals().micro_usd, 40_000_000);
    recovered
        .correct_pinned(
            &id,
            3,
            Usage::reported(vec![(Category::ImageOutput, 0)]),
            3_000,
        )
        .unwrap();
    assert_eq!(recovered.totals().micro_usd, 0);
}

#[test]
fn replay_rejects_same_and_newer_revision_mutations_of_every_frozen_field() {
    let directory = scratch("frozen-replay");
    let log = directory.join("cost.jsonl");
    let book = opened_book("first", Some(&log));
    let id = pinned_id(805);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(&id, 1, unknown_usage(), 1_000).unwrap();
    assert!(book.flush());
    book.close();
    let text = fs::read_to_string(&log).unwrap();
    let original: Value = serde_json::from_str(
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap(),
    )
    .unwrap();
    let mut mutations = Vec::new();
    for revision in [1, 2] {
        for field in [
            "run",
            "session",
            "provider",
            "model",
            "cause",
            "detail",
            "maxima",
            "price_basis",
        ] {
            let mut changed = original.clone();
            changed["pinned_revision"] = json!(revision);
            match field {
                "provider" => changed["event"][field] = json!("bfl"),
                "cause" => changed["event"][field] = json!("storyteller"),
                "maxima" => changed[field][0][1] = json!(1),
                "price_basis" => changed[field]["prices"][0]["usd"] = json!("999"),
                _ => changed["event"][field] = json!("changed"),
            }
            mutations.push(changed.to_string());
        }
    }
    fs::write(&log, format!("{text}{}\n", mutations.join("\n"))).unwrap();
    let recovered = opened_book("second", Some(&log));
    assert_eq!(recovered.totals(), book.totals());
    assert_eq!(state_of(&recovered).malformed_lines, 16);
    recovered
        .correct_pinned(
            &id,
            2,
            Usage::reported(vec![(Category::ImageOutput, 1)]),
            2_000,
        )
        .unwrap();
    assert_eq!(recovered.totals().micro_usd, 30);
}

#[test]
fn a_frozen_opening_keeps_its_tariff_and_origin_after_restart_before_first_usage() {
    let directory = scratch("opening-tariff");
    let log = directory.join("cost.jsonl");
    let first = opened_book("origin", Some(&log));
    let id = pinned_id(806);
    first.start_pinned(&id, pinned_spec()).unwrap();
    first.close();
    let second = CostBook::with_session("restart");
    let dearer = PriceTable::parse(&TABLE.replace("\"usd\": \"30\"", "\"usd\": \"60\"")).unwrap();
    second
        .open(
            "reopened-display-label",
            Some(&log),
            dearer,
            PricingStatus::NotLoaded,
            2_000,
        )
        .unwrap();
    assert_eq!(
        second.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::ReplayedOpen)
    );
    second
        .correct_pinned(
            &id,
            1,
            Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
            3_000,
        )
        .unwrap();
    assert_eq!(second.totals().micro_usd, 30_000_000);
    assert_eq!(second.recent()[0].micro_usd, 30_000_000);
    assert_eq!(state_of(&second).session_totals.events, 0);
    assert!(second.flush());
    second.close();
    let third = opened_book("third", Some(&log));
    assert_eq!(
        third.totals(),
        second.totals(),
        "the new log preserves the original frozen run too"
    );
}

#[test]
fn an_oversized_unterminated_cold_log_line_returns_corruption_without_changing_totals() {
    let directory = scratch("oversized-cold-line");
    let log = directory.join("cost.jsonl");
    let book = opened_book("bounded-cold", Some(&log));
    for n in 0..10 {
        book.start_pinned(&pinned_id(2_000 + n), pinned_spec())
            .unwrap();
        book.correct_pinned(&pinned_id(2_000 + n), 0, unknown_usage(), 1_000)
            .unwrap();
        assert!(book.flush());
    }
    let before = book.totals();
    assert!(!state_of(&book).pinned.contains_key(&pinned_id(2_000)));
    let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
    file.write_all(&vec![b'x'; pinned_storage::MAXIMUM_PINNED_LINE_BYTES + 1])
        .unwrap();
    file.sync_all().unwrap();
    assert_eq!(
        book.correct_pinned(&pinned_id(2_000), 1, unknown_usage(), 2_000),
        Err(PinnedError::CorruptLog)
    );
    assert_eq!(
        book.start_pinned(&pinned_id(3_000), pinned_spec()),
        Err(PinnedError::CorruptLog)
    );
    assert_eq!(
        book.pinned_durable(&pinned_id(2_000), Some(0)),
        Err(PinnedError::CorruptLog)
    );
    assert_eq!(book.totals(), before);
}

#[test]
fn a_start_that_cannot_fit_future_settlements_refuses_without_consuming_a_cache_slot() {
    let book = CostBook::with_session("bounded-opening");
    let run = "x".repeat(pinned_storage::MAXIMUM_PINNED_LINE_BYTES);
    book.open(&run, None, table(), PricingStatus::NotLoaded, 1_000)
        .unwrap();
    assert_eq!(
        book.start_pinned(&pinned_id(4_000), pinned_spec()),
        Err(PinnedError::ProjectionFull)
    );
    assert!(state_of(&book).pinned.is_empty());
    assert_eq!(book.totals().calls, 0);
}

#[test]
fn a_coherent_receipt_reads_the_exact_stored_revision_route_units_and_billed_unusable_amount() {
    let book = opened_book("receipt", None);
    let id = pinned_id(8_000);
    assert_eq!(book.read_pinned_receipt(&id).unwrap(), None);
    book.start_pinned(&id, pinned_spec()).unwrap();
    assert_eq!(book.read_pinned_receipt(&id).unwrap(), None);
    book.correct_pinned(
        &id,
        1,
        Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
        1_000,
    )
    .unwrap();
    book.correct_pinned(
        &id,
        2,
        Usage::reported(vec![(Category::ImageOutput, 2_000_000)]).unusable(),
        2_000,
    )
    .unwrap();
    let receipt = book.read_pinned_receipt(&id).unwrap().unwrap();
    assert_eq!(receipt.call_id, id);
    assert_eq!(receipt.revision.0, 2);
    assert_eq!(receipt.route.provider, "openai");
    assert_eq!(receipt.route.model, pinned_spec().model);
    assert_eq!(receipt.route.maxima.len(), 2);
    assert_eq!(receipt.evidence, df_imagery::cost::UsageEvidence::Reported);
    assert_eq!(
        receipt.units,
        [df_imagery::cost::CostUnits {
            category: "image_output".to_owned(),
            quantity: 2_000_000
        }]
    );
    assert!(!receipt.usable);
    assert_eq!(receipt.amount.unwrap().micros(), 60_000_000);
    assert_eq!(receipt.amount.unwrap().currency().as_str(), "USD");
    assert!(
        !receipt.durable,
        "memory-only is not a durable Book receipt"
    );
    assert_eq!(book.read_pinned_receipt(&pinned_id(8_001)).unwrap(), None);
}

#[test]
fn a_receipt_never_turns_missing_unpriced_or_proven_unsent_work_into_a_known_zero() {
    let book = opened_book("receipt-unknown", None);
    let id = pinned_id(8_010);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.record_pinned_unsent(&id).unwrap();
    assert_eq!(book.read_pinned_receipt(&id).unwrap(), None);
    let id = pinned_id(8_011);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(
        &id,
        1,
        Usage::reported(vec![
            (Category::TextInput, 1_000_000),
            (Category::Credits, 7),
        ]),
        1_000,
    )
    .unwrap();
    let receipt = book.read_pinned_receipt(&id).unwrap().unwrap();
    assert_eq!(
        receipt.amount, None,
        "a priced subtotal is not a known total"
    );
    assert_eq!(receipt.units.len(), 2);
    let id = pinned_id(8_012);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(
        &id,
        1,
        Usage::reported(vec![(Category::ImageOutput, 0)]),
        1_000,
    )
    .unwrap();
    let receipt = book.read_pinned_receipt(&id).unwrap().unwrap();
    assert_eq!(
        receipt.amount.unwrap().micros(),
        0,
        "explicit booked zero remains distinct"
    );
    assert_eq!(receipt.units[0].quantity, 0);
}

#[test]
fn a_cold_receipt_reads_original_money_across_retention_and_a_new_price_file() {
    let directory = scratch("coherent-receipt");
    let log = directory.join("cost.jsonl");
    let first = opened_book("receipt-first", Some(&log));
    for n in 0..12 {
        let id = pinned_id(8_100 + n);
        first.start_pinned(&id, pinned_spec()).unwrap();
        first
            .correct_pinned(
                &id,
                1,
                Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
                1_000,
            )
            .unwrap();
        assert!(first.flush());
    }
    assert!(!state_of(&first).pinned.contains_key(&pinned_id(8_100)));
    let receipt = first
        .read_pinned_receipt(&pinned_id(8_100))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.amount.unwrap().micros(), 30_000_000);
    assert!(receipt.durable);
    first.close();
    let dearer = PriceTable::parse(&TABLE.replace("\"usd\": \"30\"", "\"usd\": \"60\"")).unwrap();
    let second = CostBook::with_session("receipt-second");
    second
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            dearer,
            PricingStatus::NotLoaded,
            2_000,
        )
        .unwrap();
    let before = second.totals();
    let receipt = second
        .read_pinned_receipt(&pinned_id(8_100))
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.amount.unwrap().micros(),
        30_000_000,
        "read never reprices history"
    );
    assert_eq!(receipt.revision.0, 1);
    assert_eq!(
        second.totals(),
        before,
        "a receipt read records no additional money"
    );
}

#[test]
fn concurrent_corrections_never_mix_receipt_revision_quantities_and_money() {
    let book = Arc::new(opened_book("receipt-concurrent", None));
    let id = pinned_id(8_200);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(
        &id,
        1,
        Usage::reported(vec![(Category::ImageOutput, 1_000_000)]),
        1_000,
    )
    .unwrap();
    let start = Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|threads| {
        let writer_book = Arc::clone(&book);
        let writer_start = Arc::clone(&start);
        let writer_id = id.clone();
        let writer = threads.spawn(move || {
            writer_start.wait();
            for revision in 2..16 {
                writer_book
                    .correct_pinned(
                        &writer_id,
                        revision,
                        Usage::reported(vec![(Category::ImageOutput, revision * 1_000_000)]),
                        revision,
                    )
                    .unwrap();
            }
        });
        start.wait();
        for _ in 0..64 {
            let receipt = book.read_pinned_receipt(&id).unwrap().unwrap();
            assert_eq!(receipt.units[0].quantity, receipt.revision.0 * 1_000_000);
            assert_eq!(
                receipt.amount.unwrap().micros(),
                u128::from(receipt.revision.0) * 30_000_000
            );
        }
        writer.join().unwrap();
    });
}

#[test]
fn an_inconsistent_booked_sum_cannot_be_returned_as_a_verified_receipt_amount() {
    let book = opened_book("receipt-overflow", None);
    let id = pinned_id(8_300);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(
        &id,
        1,
        Usage::reported(vec![
            (Category::TextInput, u64::MAX),
            (Category::ImageOutput, u64::MAX),
        ]),
        1_000,
    )
    .unwrap();
    assert_eq!(book.read_pinned_receipt(&id), Err(PinnedError::CorruptLog));
}

#[test]
fn a_no_delivery_seal_closes_absent_and_open_calls_without_cost_or_late_start() {
    use df_imagery::cost::UnsentDurability;
    let book = opened_book("seal-absence", None);
    for (number, opened) in [(9_000, false), (9_001, true)] {
        let id = pinned_id(number);
        if opened {
            book.start_pinned(&id, pinned_spec()).unwrap();
        }
        let before = book.totals();
        let seal = book.seal_pinned_unsent(&id).unwrap();
        assert_eq!(seal.call_id, id);
        assert_eq!(seal.durability, UnsentDurability::Pending);
        assert_eq!(
            book.start_pinned(&id, pinned_spec()),
            Ok(PinnedStart::AlreadySettled)
        );
        assert_eq!(
            book.correct_pinned(&id, 1, unknown_usage(), 1_000),
            Err(PinnedError::AlreadySettled)
        );
        assert_eq!(book.read_pinned_receipt(&id).unwrap(), None);
        assert_eq!(book.totals(), before);
        let state = state_of(&book);
        let Some(PinnedSlot::Unsent(spec, _)) = state.pinned.get(&id) else {
            panic!("terminal seal missing");
        };
        assert_eq!(spec.is_some(), opened);
    }
    assert_eq!(book.totals().calls, 0);
    assert!(book.recent().is_empty());
}

#[test]
fn a_no_delivery_seal_refuses_closed_malformed_and_already_booked_calls() {
    let closed = CostBook::with_session("closed-seal");
    assert_eq!(
        closed.seal_pinned_unsent(&pinned_id(9_010)),
        Err(PinnedError::BookNotOpen)
    );
    let book = opened_book("settled-seal", None);
    assert_eq!(
        book.seal_pinned_unsent("img-bad"),
        Err(PinnedError::UnknownCall)
    );
    let id = pinned_id(9_011);
    book.start_pinned(&id, pinned_spec()).unwrap();
    book.correct_pinned(&id, 1, unknown_usage(), 1_000).unwrap();
    let before = book.totals();
    let receipt = book.read_pinned_receipt(&id).unwrap();
    assert_eq!(
        book.seal_pinned_unsent(&id),
        Err(PinnedError::AlreadySettled)
    );
    assert_eq!(book.totals(), before);
    assert_eq!(book.read_pinned_receipt(&id).unwrap(), receipt);
}

#[test]
fn a_no_delivery_seal_retries_rejected_append_and_remains_terminal_after_restart() {
    use df_imagery::cost::UnsentDurability;
    let directory = scratch("seal-rejected-append");
    let log = directory.join("cost.jsonl");
    let book = opened_book("seal-first", Some(&log));
    let id = pinned_id(9_020);
    state_of(&book)
        .log
        .as_ref()
        .unwrap()
        .reject_next
        .store(true, Ordering::SeqCst);
    assert_eq!(
        book.seal_pinned_unsent(&id).unwrap().durability,
        UnsentDurability::Pending
    );
    assert!(!book.pinned_durable(&id, None).unwrap());
    book.seal_pinned_unsent(&id).unwrap();
    assert!(book.flush());
    assert_eq!(
        book.seal_pinned_unsent(&id).unwrap().durability,
        UnsentDurability::Durable
    );
    assert_eq!(book.totals().calls, 0);
    book.close();
    let text = std::fs::read_to_string(&log).unwrap();
    let markers = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|line| line["call_id"] == id)
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0]["pinned_unsent"], true);
    assert!(markers[0]["spec"].is_null());
    assert!(markers[0].get("event").is_none());
    let recovered = opened_book("seal-second", Some(&log));
    let seal = recovered.seal_pinned_unsent(&id).unwrap();
    assert_eq!(seal.call_id, id);
    assert_eq!(seal.durability, UnsentDurability::Durable);
    assert_eq!(
        recovered.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(recovered.totals().calls, 0);
}

#[test]
fn sealed_absent_identities_reload_after_bounded_cache_eviction() {
    use df_imagery::cost::UnsentDurability;
    let directory = scratch("seal-retention");
    let log = directory.join("cost.jsonl");
    let book = opened_book("seal-retention", Some(&log));
    for number in 0..12 {
        book.seal_pinned_unsent(&pinned_id(9_100 + number)).unwrap();
        assert!(book.flush());
        assert!(state_of(&book).pinned.len() <= PINNED_LIMIT);
    }
    let id = pinned_id(9_100);
    assert!(!state_of(&book).pinned.contains_key(&id));
    assert_eq!(
        book.start_pinned(&id, pinned_spec()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(
        book.seal_pinned_unsent(&id).unwrap().durability,
        UnsentDurability::Durable
    );
    assert_eq!(book.totals().calls, 0);
}

#[test]
fn memory_only_seals_remain_pending_and_respect_the_existing_retention_bound() {
    use df_imagery::cost::UnsentDurability;
    let book = opened_book("seal-memory-bound", None);
    for number in 0..PINNED_LIMIT {
        let id = pinned_id(9_200 + u64::try_from(number).unwrap());
        assert_eq!(
            book.seal_pinned_unsent(&id).unwrap().durability,
            UnsentDurability::Pending
        );
    }
    assert_eq!(
        book.seal_pinned_unsent(&pinned_id(9_299)),
        Err(PinnedError::ProjectionFull)
    );
    assert_eq!(state_of(&book).pinned.len(), PINNED_LIMIT);
    assert_eq!(book.totals().calls, 0);
}

#[test]
fn an_actual_start_racing_a_no_delivery_seal_cannot_leave_an_open_call() {
    use df_imagery::cost::UnsentDurability;
    let directory = scratch("seal-race");
    for number in 0..4 {
        let log = directory.join(format!("cost-{number}.jsonl"));
        let book = Arc::new(opened_book("seal-race-first", Some(&log)));
        let id = pinned_id(9_300 + number);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|threads| {
            let starter = Arc::clone(&book);
            let start_id = id.clone();
            let start_barrier = Arc::clone(&barrier);
            let start = threads.spawn(move || {
                start_barrier.wait();
                starter.start_pinned(&start_id, pinned_spec())
            });
            let sealer = Arc::clone(&book);
            let seal_id = id.clone();
            let seal_barrier = Arc::clone(&barrier);
            let seal = threads.spawn(move || {
                seal_barrier.wait();
                sealer.seal_pinned_unsent(&seal_id)
            });
            barrier.wait();
            assert!(matches!(
                start.join().unwrap(),
                Ok(PinnedStart::Opened | PinnedStart::AlreadySettled)
            ));
            assert_eq!(seal.join().unwrap().unwrap().call_id, id);
        });
        assert!(book.flush());
        assert_eq!(
            book.seal_pinned_unsent(&id).unwrap().durability,
            UnsentDurability::Durable
        );
        assert_eq!(book.totals().calls, 0);
        book.close();
        let recovered = opened_book("seal-race-second", Some(&log));
        assert_eq!(
            recovered.start_pinned(&id, pinned_spec()),
            Ok(PinnedStart::AlreadySettled)
        );
        assert_eq!(recovered.totals().calls, 0);
    }
}
