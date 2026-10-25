use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use df_protocol::common as rpc;
use df_providers::{
    GeneratedImage, ImageBackground, ImageEdit, ImageEditConfig, ImageEditModel, ImageFormat,
    ImageModeration, ImageQuality, ImageUsage, NativeHttpRefusal, NativeImageEditProvider,
    ProviderFailureClass, SpeechAdapterReason, TextTokenUsage,
};
use serde_json::json;

use super::*;
use crate::gameplay::imagery::{ImageFailure, ImageKind, ImagePriority, ImageRoute};
use crate::gameplay::resources::SpendLedger;
use crate::voice_line::{VoiceLine, fight_voice_line};

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
        role: rpc::TableRole::Hero as i32,
        hero_index: 1,
        speaker: hero.to_owned(),
        text: "For the harbour!".to_owned(),
    })
    .expect("a hero cry")
}

/// A cast member's conversation reply, in the `npc-id:<id>|<name>` form the display uses.
fn cast_reply(actor: &str, name: &str) -> VoiceLine {
    VoiceLine {
        role: format!("npc-id:{actor}|{name}"),
        text: "Well met.".to_owned(),
    }
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

fn image_route() -> ImageRoute {
    let config = ImageEditConfig::loopback_fixture("127.0.0.1:9".parse().expect("address"))
        .expect("loopback fixture");
    ImageRoute::OpenAi(
        NativeImageEditProvider::new(config, "test-credential", Duration::from_secs(1))
            .expect("provider"),
    )
}

fn plate_edit() -> ImageEdit<'static> {
    ImageEdit {
        model: ImageEditModel::Gpt25Flare,
        prompt: "a plate",
        references: &[],
        width: 1024,
        height: 640,
        quality: ImageQuality::Low,
        background: ImageBackground::Opaque,
        output: ImageFormat::Jpeg,
        moderation: ImageModeration::Auto,
    }
}

fn image(usage: ImageUsage) -> GeneratedImage {
    GeneratedImage {
        bytes: vec![1, 2, 3],
        format: ImageFormat::Jpeg,
        width: 1024,
        height: 640,
        has_alpha_channel: false,
        usage,
    }
}

fn state_of(book: &CostBook) -> MutexGuard<'_, BookState> {
    locked(&book.state)
}

#[test]
fn a_priced_line_is_exact_nano_dollars_rounded_half_up_to_a_micro_dollar() {
    let jev = Price {
        nano_usd: parse_nano_usd("0.042").expect("rate"),
        per: 1_000_000,
        as_of: String::new(),
        source: String::new(),
    };
    assert_eq!(jev.micro_usd(1_500), 63);
    assert_eq!(jev.micro_usd(11), 0, "0.462 rounds down");
    assert_eq!(jev.micro_usd(12), 1, "0.504 rounds up");
    let half = Price {
        nano_usd: 500,
        per: 1,
        as_of: String::new(),
        source: String::new(),
    };
    assert_eq!(half.micro_usd(1), 1, "exactly one half rounds up");
    assert_eq!(half.micro_usd(0), 0);
    let scribe = Price {
        nano_usd: parse_nano_usd("0.22").expect("rate"),
        per: 3_600,
        as_of: String::new(),
        source: String::new(),
    };
    assert_eq!(scribe.micro_usd(3_600), 220_000);
    assert_eq!(scribe.micro_usd(10), 611, "611.1 micro-dollars");
    let huge = Price {
        nano_usd: u64::MAX,
        per: 1,
        as_of: String::new(),
        source: String::new(),
    };
    assert_eq!(
        huge.micro_usd(u64::MAX),
        u64::MAX,
        "saturates instead of wrapping"
    );
}

#[test]
fn dollar_text_parses_exactly_or_not_at_all() {
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
    assert_eq!(nano_usd_text(42_000_000), "0.042");
    assert_eq!(nano_usd_text(5_000_000_000), "5");
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
    for cause in [
        Cause::ScenePlate,
        Cause::PlateEdit,
        Cause::SceneRefresh,
        Cause::CharacterPortrait,
        Cause::PlacePortrait,
        Cause::CastExpression,
        Cause::ItemCloseUp,
        Cause::ClipFrame,
        Cause::Storyteller,
        Cause::NarrationVoice,
        Cause::CharacterVoice,
    ] {
        assert_eq!(json!(cause), json!(cause.label()));
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
                    .lookup(route.provider, route.model, *category)
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
    const NOT_CLIENTS: [&str; 2] = ["NativeHttpLimits", "NativeSpeechObservation"];
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
fn the_routes_not_metered_yet_are_exactly_the_tracked_backlog() {
    let pending: Vec<(&str, &str)> = ROUTES
        .iter()
        .filter(|route| matches!(route.metering, Metering::Pending(_)))
        .map(|route| (route.client, route.model))
        .collect();
    assert_eq!(
        pending,
        [
            ("NativeSpeechProvider", "scribe_v2"),
            ("NativeEmbeddingProvider", "text-embedding-3-small"),
            ("NativeFalSegmenter", "fal-ai/sam-3-1/image-rle"),
            ("NativeHeadLocator", "gpt-5.5"),
            ("NativePlateJudge", "gpt-5.5"),
            ("NativeElevenMusicProvider", "music_v2"),
            (
                "NativeFalMediaProvider",
                "fal-ai/kling-video/o1/image-to-video"
            ),
            ("NativeWorldLabs", "marble-1.1"),
            ("SystemOneChoiceProvider", "jev-latest"),
        ],
        "metering a route moves it out of this list (and out of the notes backlog)"
    );
}

#[test]
fn an_image_call_is_priced_from_the_reported_tokens_like_the_provider_formula() {
    let ledger = opened_ledger(None);
    let route = image_route();
    let spec = image_call(
        &route,
        ImageEditModel::Gpt25Flare,
        ImageKind::ScenePlate,
        ImagePriority::Foreground,
        "dev",
        &plate_edit(),
    );
    assert_eq!(spec.cause, Cause::ScenePlate);
    assert_eq!(spec.detail, "scene_plate/foreground/dev/low/1024x640/refs0");
    let usage = ImageUsage {
        text_input_tokens: 1_000,
        image_input_tokens: 2_000,
        output_tokens: 3_000,
    };
    let call = ledger.begin_call(spec.clone(), &[(Category::Calls, 1)]);
    settle_image_call(call, &route, &Ok((image(usage), None)));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.micro_usd, usage.micro_dollars());
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

impl BookState {
    fn by_line_micro(&self, provider: Provider, model: &str, category: Category) -> u64 {
        self.session_totals
            .by_line
            .get(&(provider, model.to_owned(), category))
            .map_or(0, |total| total.micro_usd)
    }
}

#[test]
fn a_rate_limited_image_call_is_not_billed_and_a_failed_one_is_of_unknown_outcome() {
    let ledger = opened_ledger(None);
    let route = image_route();
    let spec = image_call(
        &route,
        ImageEditModel::Gpt25Flare,
        ImageKind::ArtifactCloseUp,
        ImagePriority::Background,
        "production",
        &plate_edit(),
    );
    for failure in [ImageFailure::RateLimited, ImageFailure::InvalidRequest] {
        let call = ledger.begin_call(spec.clone(), &[(Category::Calls, 1)]);
        settle_image_call(call, &route, &Err(failure));
    }
    assert_eq!(state_of(ledger.costs()).session_totals.events, 0);
    let call = ledger.begin_call(spec, &[(Category::Calls, 1)]);
    settle_image_call(call, &route, &Err(ImageFailure::DeadlineExceeded));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.events, 1);
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(
        state.session_totals.unpriced_events, 1,
        "no price for a bare call"
    );
    assert_eq!(
        state
            .session_totals
            .by_cause
            .get(&Cause::ItemCloseUp)
            .map(|total| total.events),
        Some(1)
    );
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
    let (lines, total) = table().price(Provider::OpenAi, "gpt-6-luna", &reported.quantities);
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
    let (lines, micro_usd) = table().price(
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
            "model": "gpt-image-2.5-flare", "categories": ["text_input", "image_input", "image_output"],
            "metering": "priced", "reason": ""})
    );
    let price = |model: &str, category: &str, usd: &str, per: u64| {
        json!({"provider": if model == "eleven_flash_v2_5" { "elevenlabs" } else { "openai" },
            "model": model, "category": category, "usd": usd, "per": per,
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
            "prices": [
                price("gpt-6-luna", "text_input", "0.1", 1_000_000),
                price("gpt-6-luna", "text_cached_input", "0.01", 1_000_000),
                price("gpt-6-luna", "text_cache_write", "0.125", 1_000_000),
                price("gpt-6-luna", "text_output", "0.5", 1_000_000),
                price("eleven_flash_v2_5", "characters", "0.04", 1_000),
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

#[test]
fn a_clip_frame_is_priced_from_its_tokens_and_a_silent_usage_is_one_estimated_call() {
    let spec = clip_frame_call(ImageEditModel::Gpt25Sunburst, true);
    assert_eq!(
        (
            spec.provider,
            spec.model.as_str(),
            spec.cause,
            spec.detail.as_str()
        ),
        (
            Provider::OpenAi,
            "gpt-image-2.5-sunburst",
            Cause::ClipFrame,
            "fallen"
        )
    );
    assert_eq!(
        image_token_usage(ImageUsage::default()),
        Usage::estimated(vec![(Category::Calls, 1)])
    );
    assert_eq!(
        image_token_usage(ImageUsage {
            text_input_tokens: 1,
            image_input_tokens: 2,
            output_tokens: 3,
        }),
        Usage::reported(vec![
            (Category::TextInput, 1),
            (Category::ImageInput, 2),
            (Category::ImageOutput, 3),
        ])
    );
}

/// The charge sites that do not meter dollars yet, by file and unit: each is a backlog item.
const UNMETERED_CHARGES: [(&str, &str, &str); 11] = [
    ("gameplay/character_clips.rs", "VideoClip", "C-B1"),
    (
        "gameplay/creator_classification.rs",
        "CreatorClassificationCall",
        "C-B5",
    ),
    ("gameplay/home_video.rs", "VideoClip", "C-B1"),
    ("gameplay/imagery/figure_masks.rs", "SegmentCall", "C-B1"),
    ("gameplay/memory/embedder.rs", "EmbeddingToken", "C-B1"),
    ("gameplay/place_music/resource.rs", "MusicCall", "C-B1"),
    ("gameplay/place_music/resource.rs", "MusicCall", "C-B1"),
    ("gameplay/plate_segments/service.rs", "SegmentCall", "C-B1"),
    ("gameplay/plate_segments/service.rs", "TextToken", "C-B2"),
    ("gameplay/speech_input.rs", "SttSecond", "C-B1"),
    ("gameplay/splat_world.rs", "SplatCredit", "C-B1"),
];

#[test]
fn every_spend_charge_meters_its_dollars_or_is_a_tracked_backlog_item() {
    fn visit(directory: &std::path::Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("sources") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();
    let mut metered = Vec::new();
    let mut unmetered = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        let name = relative.rsplit('/').next().unwrap_or_default();
        // The ledger and the cost book themselves, the kit's generic charge, and tests.
        if matches!(
            relative.as_str(),
            "gameplay/resources.rs" | "gameplay/cost.rs" | "gameplay/resource_adapters.rs"
        ) || name.ends_with("_tests.rs")
            || name == "tests.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let unit = if line.contains("reserve(IMAGE_UNIT") {
                "ImageCall"
            } else {
                let Some(start) = line
                    .find("charge(Unit::")
                    .map(|at| at + "charge(Unit::".len())
                    .or_else(|| {
                        line.find("reserve(Unit::")
                            .map(|at| at + "reserve(Unit::".len())
                    })
                else {
                    continue;
                };
                let rest = &line[start..];
                &rest[..rest
                    .find(|character: char| !character.is_ascii_alphanumeric())
                    .unwrap_or(rest.len())]
            };
            let window = lines[index..lines.len().min(index + 10)].join("\n");
            if window.contains("begin_call(") {
                metered.push((relative.clone(), unit.to_owned()));
            } else {
                unmetered.push((relative.clone(), unit.to_owned()));
            }
        }
    }
    let expected: Vec<(String, String)> = UNMETERED_CHARGES
        .iter()
        .map(|(file, unit, _)| ((*file).to_owned(), (*unit).to_owned()))
        .collect();
    assert_eq!(
        unmetered, expected,
        "a charge with no begin_call within ten lines must meter its dollars or be listed with \
         its backlog item"
    );
    for (file, unit) in [
        ("gameplay/character_clips.rs", "ImageCall"),
        ("gameplay/imagery.rs", "ImageCall"),
        ("gameplay/storyteller.rs", "TextToken"),
        ("gameplay/voice.rs", "VoiceCharacter"),
    ] {
        assert!(
            metered.contains(&(file.to_owned(), unit.to_owned())),
            "{file} {unit} is metered: {metered:?}"
        );
    }
}
