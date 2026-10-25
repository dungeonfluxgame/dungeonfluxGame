//! Context tiers, carried remainders, per-category maxima, follow-on calls, the meter of the
//! choice, judge and locator clients, and the metering port.

use std::sync::Mutex;

use df_providers::{
    AnsweredUsage, BflImageConfig, FalImageConfig, MeteredClient, NativeBflImageProvider,
    NativeFalImageProvider, SentCall,
};

use super::*;
use crate::gameplay::resources::SpendLedger;
use port::MeteringPort;

fn file(rows: &str) -> String {
    format!(r#"{{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [{rows}]}}"#)
}

fn row(model: &str, category: &str, usd: &str, per: u64, extra: &str) -> String {
    format!(
        r#"{{"provider": "openai", "model": "{model}", "category": "{category}", "usd": "{usd}", "per": {per}, {extra} "as_of": "2026-10-24", "source": "https://example.test/p"}}"#
    )
}

/// The checked-in price file, opened on a fresh ledger.
fn shipped_ledger() -> Arc<SpendLedger> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/pricing.json"
    ));
    let (prices, status) = load_prices(&path);
    assert!(
        matches!(status, PricingStatus::Loaded { .. }),
        "content/pricing.json must load: {status:?}"
    );
    let ledger = Arc::new(SpendLedger::default());
    ledger
        .costs()
        .open("df_gameplay_demo_test", None, prices, status, 1_000)
        .expect("the book opens");
    ledger
}

#[test]
fn a_context_tier_prices_calls_up_to_it_and_the_unbounded_row_prices_longer_ones() {
    let table = PriceTable::parse(&file(
        &[
            row(
                "m",
                "text_input",
                "0.10",
                1_000_000,
                r#""context_max_tokens": 272000,"#,
            ),
            row("m", "text_input", "0.20", 1_000_000, ""),
            row(
                "m",
                "text_output",
                "0.50",
                1_000_000,
                r#""context_max_tokens": 272000,"#,
            ),
        ]
        .join(","),
    ))
    .expect("a tiered table");
    assert_eq!(table.len(), 3);
    let nano = |context: u64, category: Category| {
        table
            .lookup(Provider::OpenAi, "m", category, context)
            .map(|price| price.nano_usd)
    };
    assert_eq!(nano(0, Category::TextInput), Some(100_000_000));
    assert_eq!(
        nano(272_000, Category::TextInput),
        Some(100_000_000),
        "the tier's own maximum is inside it"
    );
    assert_eq!(nano(272_001, Category::TextInput), Some(200_000_000));
    assert_eq!(nano(272_000, Category::TextOutput), Some(500_000_000));
    assert_eq!(
        nano(272_001, Category::TextOutput),
        None,
        "a context above every stored tier of a category is unpriced, not priced at a rate that does not cover it"
    );
    // The call's context is its input tokens, cached and written included.
    let (lines, total) = price_carrying(
        &table,
        &mut BTreeMap::new(),
        Provider::OpenAi,
        "m",
        &[
            (Category::TextInput, 250_000),
            (Category::TextCachedInput, 30_000),
            (Category::TextOutput, 1_000),
        ],
    );
    assert_eq!(
        lines.iter().map(|line| line.micro_usd).collect::<Vec<_>>(),
        [Some(50_000), None, None],
        "300K of context: the long input rate, no cached rate stored, no output rate above 272K"
    );
    assert_eq!(total, 50_000);
}

#[test]
fn a_tier_is_validated_like_any_row_and_a_duplicate_is_per_tier() {
    let tier = r#""context_max_tokens": 1000,"#;
    // The same key in two tiers is two rows; in one tier it is a duplicate, named by its index.
    let two = [
        row("m", "text_input", "1", 1, tier),
        row("m", "text_input", "2", 1, ""),
    ]
    .join(",");
    assert_eq!(
        PriceTable::parse(&file(&two)).map(|table| table.len()),
        Ok(2)
    );
    let duplicated = [
        row("m", "text_input", "1", 1, tier),
        row("m", "text_input", "2", 1, ""),
        row("m", "text_input", "3", 1, tier),
    ]
    .join(",");
    assert_eq!(
        PriceTable::parse(&file(&duplicated)).map(|_| ()),
        Err(PricingError::Duplicate { index: 2 })
    );
    let malformed = [
        row("m", "text_input", "1", 1, tier),
        row("m", "text_output", "1e3", 1, ""),
    ]
    .join(",");
    assert_eq!(
        PriceTable::parse(&file(&malformed)).map(|_| ()),
        Err(PricingError::Entry {
            index: 1,
            reason: "usd"
        }),
        "a bad row is named by its place in the file, whatever its tier"
    );
}

#[test]
fn the_checked_in_file_prices_the_long_context_input_and_leaves_the_rest_of_it_unpriced() {
    let ledger = shipped_ledger();
    let usage = TextTokenUsage {
        input_tokens: 300_000,
        output_tokens: 1_000,
        total_tokens: 301_000,
        cached_input_tokens: 0,
        cache_write_tokens: None,
        reasoning_output_tokens: 0,
    };
    ledger
        .begin_call(text_call("gpt-6-luna", "npc_reply"), &[])
        .settle(text_usage(&usage));
    let state = state_of(ledger.costs());
    assert_eq!(
        state.by_line_micro(Provider::OpenAi, "gpt-6-luna", Category::TextInput),
        60_000,
        "300K input tokens at 0.20 per million"
    );
    assert_eq!(
        state.session_totals.unpriced_events, 1,
        "the output above 272K has no stored rate"
    );
    drop(state);
    let short = TextTokenUsage {
        input_tokens: 272_000,
        output_tokens: 1_000,
        total_tokens: 273_000,
        ..usage
    };
    ledger
        .begin_call(text_call("gpt-6-luna", "npc_reply"), &[])
        .settle(text_usage(&short));
    let state = state_of(ledger.costs());
    assert_eq!(
        state.by_line_micro(Provider::OpenAi, "gpt-6-luna", Category::TextInput),
        60_000 + 27_200,
        "272K input tokens at 0.10 per million"
    );
    assert_eq!(state.session_totals.unpriced_events, 1);
}

#[test]
fn sub_micro_prices_are_carried_between_calls_instead_of_rounded_each_time() {
    // gpt-6-luna output is half a micro-dollar per token.
    let book = opened_book("s1", None);
    let output = |tokens: u64| Usage::reported(vec![(Category::TextOutput, tokens)]);
    for _ in 0..3 {
        book.meter(text_call("gpt-6-luna", "a"), output(1), 2_000);
    }
    assert_eq!(
        state_of(&book).session_totals.micro_usd,
        2,
        "1, 0, 1: three halves are one and a half, rounded once"
    );
    for _ in 0..997 {
        book.meter(text_call("gpt-6-luna", "a"), output(1), 2_000);
    }
    assert_eq!(
        state_of(&book).session_totals.micro_usd,
        500,
        "a thousand one-token calls are 500 micro-dollars, not 1,000"
    );
    // A call of 7 tokens is 3.5 exactly; the carry after 1,000 half-micro tokens is zero.
    book.meter(text_call("gpt-6-luna", "a"), output(7), 2_001);
    assert_eq!(state_of(&book).session_totals.micro_usd, 504);
    // Another model's line carries its own remainder.
    book.meter(text_call("gpt-6.1-sol", "a"), output(1), 2_002);
    assert_eq!(state_of(&book).session_totals.micro_usd, 504);
}

#[test]
fn a_quote_sums_the_exact_lines_and_rounds_once() {
    let table = table();
    let quote =
        |quantities: &[(Category, u64)]| table.quote(Provider::OpenAi, "gpt-6-luna", quantities);
    // 5 input tokens are 0.5 and 1 output token is 0.5: one micro-dollar, not two.
    assert_eq!(
        quote(&[(Category::TextInput, 5), (Category::TextOutput, 1)]),
        Some(1)
    );
    assert_eq!(quote(&[(Category::TextInput, 4)]), Some(0));
    assert_eq!(
        quote(&[(Category::TextInput, 5), (Category::Calls, 1)]),
        None,
        "a quote with an unpriced line is no quote"
    );
    let ledger = opened_ledger(None);
    assert_eq!(
        ledger.costs().quote(
            Provider::OpenAi,
            "gpt-image-2.5-flare",
            &[(Category::ImageOutput, 170)]
        ),
        Some(5_100)
    );
}

#[test]
fn a_dropped_picture_call_is_booked_in_the_routes_priced_categories() {
    let ledger = shipped_ledger();
    let route = image_route();
    let mut edit = plate_edit();
    edit.quality = ImageQuality::High;
    edit.width = 1024;
    edit.height = 1024;
    let spec = image_call(
        &route,
        ImageEditModel::Gpt25Flare,
        ImageKind::ScenePlate,
        ImagePriority::Foreground,
        "dev",
        &edit,
    );
    assert_eq!(
        spec.maxima,
        vec![
            (Category::TextInput, 4),
            (Category::ImageInput, 0),
            (Category::ImageOutput, 4_160),
        ],
        "the high-quality square of the published token table"
    );
    // The caller's own estimate (imagery sends one call) is not what is booked.
    drop(ledger.begin_call(spec, &[(Category::Calls, 1)]));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(state.session_totals.unpriced_events, 0);
    assert_eq!(state.session_totals.micro_usd, 4 * 5 + 4_160 * 30);
}

#[test]
fn a_dropped_bfl_call_is_booked_at_the_files_per_call_ceiling_until_credits_are_reported() {
    let ledger = shipped_ledger();
    let config = BflImageConfig::loopback_fixture("127.0.0.1:9".parse().expect("address"))
        .expect("loopback fixture");
    let route = ImageRoute::Bfl(
        NativeBflImageProvider::new(config, "test-credential", Duration::from_secs(1))
            .expect("provider"),
    );
    let spec = |_: ()| {
        image_call(
            &route,
            ImageEditModel::Gpt25Flare,
            ImageKind::ScenePlate,
            ImagePriority::Foreground,
            "dev",
            &plate_edit(),
        )
    };
    assert_eq!(spec(()).maxima, vec![(Category::Calls, 1)]);
    settle_image_call(
        ledger.begin_call(spec(()), &[]),
        &route,
        &Err(ImageFailure::DeadlineExceeded),
    );
    assert_eq!(state_of(ledger.costs()).session_totals.micro_usd, 80_000);
    // Credits the call reported replace the ceiling.
    settle_image_call(
        ledger.begin_call(spec(()), &[]),
        &route,
        &Ok((image(ImageUsage::default()), Some("3".to_owned()))),
    );
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.micro_usd, 80_000 + 30_000);
    assert_eq!(state.session_totals.unpriced_events, 0);
}

#[test]
fn a_flux_edit_is_billed_under_the_endpoint_that_made_it_and_a_cutout_is_a_second_event() {
    let ledger = shipped_ledger();
    let config = FalImageConfig::loopback_fixture("127.0.0.1:9".parse().expect("address"))
        .expect("loopback fixture");
    let route = ImageRoute::Flux(
        NativeFalImageProvider::new(config, "test-credential", Duration::from_secs(1))
            .expect("provider"),
    );
    assert_eq!(
        image_route_model(&route, ImageEditModel::Gpt25Flare, false),
        (Provider::Fal, df_providers::FLUX_ULTRA)
    );
    assert_eq!(
        image_route_model(&route, ImageEditModel::Gpt25Flare, true),
        (Provider::Fal, df_providers::FLUX_KONTEXT_MULTI)
    );
    let mut edit = plate_edit();
    edit.background = ImageBackground::Transparent;
    edit.output = ImageFormat::Png;
    let spec = image_call(
        &route,
        ImageEditModel::Gpt25Flare,
        ImageKind::CastExpression,
        ImagePriority::Foreground,
        "dev",
        &edit,
    );
    assert_eq!(spec.model, df_providers::FLUX_ULTRA);
    assert_eq!(spec.follow_ons.len(), 1);
    let result = Ok((image(ImageUsage::default()), None));
    settle_image_call(ledger.begin_call(spec.clone(), &[]), &route, &result);
    {
        let state = state_of(ledger.costs());
        assert_eq!(state.session_totals.events, 2, "the picture and its cutout");
        assert_eq!(state.session_totals.unpriced_events, 0);
        assert_eq!(state.session_totals.micro_usd, 60_000 + 10_000);
        assert_eq!(
            state.by_line_micro(
                Provider::Fal,
                df_providers::BACKGROUND_REMOVAL,
                Category::Images
            ),
            10_000
        );
    }
    // A call of unknown outcome records only the picture: the cutout runs after it succeeds.
    settle_image_call(
        ledger.begin_call(spec, &[]),
        &route,
        &Err(ImageFailure::DeadlineExceeded),
    );
    assert_eq!(state_of(ledger.costs()).session_totals.events, 3);
}

#[test]
fn a_usage_with_no_quantities_stands_for_the_calls_maxima_and_a_reported_one_stands_for_itself() {
    let ledger = opened_ledger(None);
    let spec = || {
        CallSpec::new(
            Provider::ElevenLabs,
            "eleven_flash_v2_5",
            Cause::NarrationVoice,
            "n",
        )
        .with_maxima(vec![(Category::Characters, 100)])
    };
    ledger
        .begin_call(spec(), &[(Category::Characters, 5)])
        .settle(Usage::estimated(Vec::new()));
    ledger
        .begin_call(spec(), &[])
        .settle(Usage::reported(vec![(Category::Characters, 40)]));
    drop(ledger.begin_call(spec(), &[(Category::Characters, 5)]));
    let state = state_of(ledger.costs());
    assert_eq!(
        state.session_totals.by_line[&(
            Provider::ElevenLabs,
            "eleven_flash_v2_5".to_owned(),
            Category::Characters
        )]
            .quantity,
        100 + 40 + 100,
        "the maxima stand for a silent usage and for a dropped call; a report stands for itself"
    );
    assert_eq!(state.session_totals.estimated_events, 1);
    assert_eq!(state.session_totals.unknown_events, 1);
}

/// The choice, judge and locator clients meter themselves through a [`LedgerMeter`].
#[test]
fn the_choice_client_is_metered_at_the_tokens_it_reports_else_at_its_requests_size() {
    let ledger = shipped_ledger();
    let meter = LedgerMeter::shared(&ledger, "approach");
    let call = |bytes: usize| SentCall {
        client: MeteredClient::Choice,
        model: "jev-latest",
        text_bytes: bytes,
        pictures: 0,
        maximum_output_tokens: 0,
    };
    meter.sent(call(2_000)).answered(AnsweredUsage {
        input_tokens: Some(1_000),
        output_tokens: None,
    });
    {
        let state = state_of(ledger.costs());
        assert_eq!(
            state.session_totals.estimated_events, 0,
            "the tokens were reported"
        );
        assert_eq!(
            state.session_totals.micro_usd, 42,
            "1,000 tokens at 0.042 per million"
        );
    }
    // No usage in the reply: the request's bytes at two per token, flagged estimated.
    meter.sent(call(2_001)).answered(AnsweredUsage::default());
    {
        let state = state_of(ledger.costs());
        assert_eq!(state.session_totals.estimated_events, 1);
        assert_eq!(state.session_totals.micro_usd, 42 + 42);
        let key = (
            Provider::TypeSafe,
            "jev-latest".to_owned(),
            Category::TextInput,
        );
        assert_eq!(state.session_totals.by_line[&key].quantity, 1_000 + 1_001);
    }
    meter.sent(call(2_000)).refused();
    assert_eq!(
        state_of(ledger.costs()).session_totals.events,
        2,
        "a refusal is not billed"
    );
    // A call that is dropped (a timeout cancels the future) may have been billed.
    drop(meter.sent(call(2_000)));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(state.session_totals.events, 3);
    assert_eq!(
        state.session_totals.by_cause[&Cause::Judgement].events,
        3,
        "every Jev call is a judgement"
    );
}

#[test]
fn the_judge_and_the_locator_are_metered_at_reported_tokens_else_at_their_maxima() {
    let ledger = shipped_ledger();
    let meter = LedgerMeter::shared(&ledger, "staging");
    let sent = |client: MeteredClient| SentCall {
        client,
        model: "gpt-5.5",
        text_bytes: 100,
        pictures: 2,
        maximum_output_tokens: 3_000,
    };
    // Reported: the pictures are inside the input tokens.
    meter
        .sent(sent(MeteredClient::PlateJudge))
        .answered(AnsweredUsage {
            input_tokens: Some(2_000),
            output_tokens: Some(100),
        });
    {
        let state = state_of(ledger.costs());
        assert_eq!(state.session_totals.estimated_events, 0);
        assert_eq!(state.session_totals.micro_usd, 2_000 * 5 + 100 * 30);
        assert_eq!(
            state.session_totals.by_cause[&Cause::PictureJudgement].events,
            1
        );
    }
    // Input reported, output not: the output is booked at the most the request allows.
    meter
        .sent(sent(MeteredClient::HeadLocator))
        .answered(AnsweredUsage {
            input_tokens: Some(1_000),
            output_tokens: None,
        });
    {
        let state = state_of(ledger.costs());
        assert_eq!(state.session_totals.estimated_events, 1);
        assert_eq!(
            state.session_totals.micro_usd,
            13_000 + 1_000 * 5 + 3_000 * 30
        );
        assert_eq!(
            state.session_totals.by_cause[&Cause::HeadLocation].events,
            1
        );
    }
    // Dropped: every declared category at its maximum (50 text tokens, 2 pictures, 3,000 out).
    let before = state_of(ledger.costs()).session_totals.micro_usd;
    drop(meter.sent(sent(MeteredClient::PlateJudge)));
    let state = state_of(ledger.costs());
    assert_eq!(state.session_totals.unknown_events, 1);
    assert_eq!(
        state.session_totals.micro_usd - before,
        50 * 5 + 2 * IMAGE_INPUT_TOKENS_PER_REFERENCE * 5 + 3_000 * 30,
        "the gpt-5.5 image_input row prices the pictures"
    );
    assert_eq!(state.session_totals.unpriced_events, 0);
}

struct Recorder {
    seen: Mutex<Vec<String>>,
    port: MeteringPort,
}

impl port::MeterObserver for Recorder {
    fn started(&self, start: &port::MeterStart) {
        let held: Vec<String> = start
            .maxima
            .iter()
            .map(|units| format!("{}={}", units.category, units.quantity))
            .collect();
        self.seen
            .lock()
            .expect("seen")
            .push(format!("start {} {}", start.cause, held.join(",")));
    }

    fn settled(&self, call: &port::MeteredCall) {
        // An observer may read the book: it runs after the book's lock is released.
        let totals = self.port.totals();
        self.seen.lock().expect("seen").push(format!(
            "settled {} {:?} at={} usd={} totals={}",
            call.call_id, call.source, call.at_ms, call.micro_usd, totals.calls
        ));
    }
}

fn route(cause: &str) -> port::PortRoute {
    port::PortRoute {
        provider: "openai".to_owned(),
        model: "gpt-6-luna".to_owned(),
        cause: cause.to_owned(),
        detail: "npc_reply".to_owned(),
        maxima: vec![
            port::MeteredUnits {
                category: "text_input".to_owned(),
                quantity: 1_000,
            },
            port::MeteredUnits {
                category: "text_output".to_owned(),
                quantity: 200,
            },
        ],
    }
}

#[test]
fn the_port_shows_a_call_when_it_starts_and_when_it_is_recorded_and_nothing_else() {
    let port = MeteringPort::new();
    port.open("run", TABLE).expect("opens");
    assert_eq!(port.open("again", TABLE), Err(port::PortError::AlreadyOpen));
    let recorder = Arc::new(Recorder {
        seen: Mutex::new(Vec::new()),
        port: port.clone(),
    });
    port.observe(recorder.clone());
    let call = port.start(&route("storyteller")).expect("a known route");
    call.settle_at(
        port::MeteredSource::Reported,
        &[
            port::MeteredUnits {
                category: "text_input".to_owned(),
                quantity: 1_000,
            },
            port::MeteredUnits {
                category: "text_output".to_owned(),
                quantity: 200,
            },
        ],
        5_000,
    )
    .expect("settles");
    let seen = recorder.seen.lock().expect("seen").clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0],
        "start storyteller text_input=1000,text_output=200,text_cached_input=0,text_cache_write=0"
    );
    assert!(seen[1].starts_with("settled "), "{}", seen[1]);
    assert!(
        seen[1].contains("Reported at=5000 usd=200 totals=1"),
        "{}",
        seen[1]
    );
    let recent = port.recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].units.len(), 2);
    assert!(!recent[0].unpriced);
    assert_eq!(
        port.totals(),
        port::MeterTotals {
            calls: 1,
            micro_usd: 200,
            unpriced_calls: 0
        }
    );
    // Refused: nothing is recorded. Dropped: of unknown outcome, at the maxima.
    port.start(&route("storyteller"))
        .expect("route")
        .not_billed();
    drop(port.start(&route("storyteller")).expect("route"));
    let recent = port.recent();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[1].source, port::MeteredSource::Unknown);
    assert_eq!(recent[1].units[0].quantity, 1_000);
}

#[test]
fn the_port_refuses_names_the_engine_does_not_know_and_keeps_a_bounded_record() {
    let port = MeteringPort::new();
    port.open("run", TABLE).expect("opens");
    let mut bad = route("storyteller");
    bad.provider = "acme".to_owned();
    assert!(matches!(
        port.start(&bad),
        Err(port::PortError::UnknownProvider(_))
    ));
    assert!(matches!(
        port.start(&route("sorcery")),
        Err(port::PortError::UnknownCause(_))
    ));
    let mut bad = route("storyteller");
    bad.maxima[0].category = "tokens".to_owned();
    assert!(matches!(
        port.start(&bad),
        Err(port::PortError::UnknownCategory(_))
    ));
    let call = port.start(&route("storyteller")).expect("route");
    let unknown_units = [port::MeteredUnits {
        category: "tokens".to_owned(),
        quantity: 1,
    }];
    assert!(matches!(
        call.settle(port::MeteredSource::Reported, &unknown_units),
        Err(port::PortError::UnknownCategory(_))
    ));
    assert!(matches!(
        MeteringPort::new().open("run", "{"),
        Err(port::PortError::Prices(_))
    ));
    for _ in 0..(port::RECENT_LIMIT + 50) {
        port.start(&route("storyteller"))
            .expect("route")
            .settle(port::MeteredSource::Estimated, &[])
            .expect("settles");
    }
    let recent = port.recent();
    assert_eq!(recent.len(), port::RECENT_LIMIT, "the record is bounded");
    assert_eq!(
        port.totals().calls,
        u64::try_from(port::RECENT_LIMIT + 50).expect("count") + 1,
        "the totals are not"
    );
}

/// What a prepaid admission is told when a call starts: every category its route declares has a
/// maximum, whoever built the call.
#[test]
fn every_metered_route_starts_with_a_maximum_for_every_category_it_declares() {
    use df_providers::{ImageEdit, ReferenceImage};

    struct Starts(Mutex<Vec<port::MeterStart>>);
    impl port::MeterObserver for Starts {
        fn started(&self, start: &port::MeterStart) {
            self.0.lock().expect("starts").push(start.clone());
        }

        fn settled(&self, _call: &port::MeteredCall) {}
    }

    let ledger = opened_ledger(None);
    let starts = Arc::new(Starts(Mutex::new(Vec::new())));
    ledger.costs().observe(starts.clone());
    let begin = |spec: CallSpec, estimate: &[(Category, u64)]| {
        drop(ledger.begin_call(spec, estimate));
    };

    // Pictures: OpenAI, BFL, FLUX with and without references.
    let references = [ReferenceImage {
        format: ImageFormat::Png,
        bytes: &[1, 2, 3],
    }];
    let with_references = ImageEdit {
        references: &references,
        ..plate_edit()
    };
    let openai = image_route();
    let bfl = ImageRoute::Bfl(
        NativeBflImageProvider::new(
            BflImageConfig::loopback_fixture("127.0.0.1:9".parse().expect("address")).expect("bfl"),
            "test-credential",
            Duration::from_secs(1),
        )
        .expect("provider"),
    );
    let flux = ImageRoute::Flux(
        NativeFalImageProvider::new(
            FalImageConfig::loopback_fixture("127.0.0.1:9".parse().expect("address")).expect("fal"),
            "test-credential",
            Duration::from_secs(1),
        )
        .expect("provider"),
    );
    for (route, edit) in [
        (&openai, plate_edit()),
        (&bfl, plate_edit()),
        (&flux, plate_edit()),
        (&flux, with_references),
    ] {
        let spec = image_call(
            route,
            ImageEditModel::Gpt25Flare,
            ImageKind::ScenePlate,
            ImagePriority::Foreground,
            "dev",
            &edit,
        );
        begin(spec, &[(Category::Calls, 1)]);
    }
    begin(clip_frame_call(ImageEditModel::Gpt25Sunburst, false), &[]);
    // Text and voice take their estimate from the caller.
    begin(
        text_call("gpt-6-luna", "npc_reply"),
        &[(Category::TextInput, 10), (Category::TextOutput, 5)],
    );
    begin(
        voice_call("eleven_flash_v2_5", &narration()),
        &[(Category::Characters, 19)],
    );
    // The other routes state their own.
    begin(stt_call(3), &[]);
    begin(embedding_call("text-embedding-3-small", 40), &[]);
    begin(segment_call("fal-ai/sam-3-1/image-rle", "person"), &[]);
    begin(music_layer_call("bed", 48), &[]);
    begin(ambience_call("set", 27), &[]);
    begin(
        video_call(
            df_providers::KLING_O1_IMAGE_TO_VIDEO,
            Cause::HomeVideo,
            "home",
            5,
        ),
        &[],
    );
    begin(world_call("marble-1.1", 1_580), &[]);
    // The clients that meter themselves.
    let meter = LedgerMeter::shared(&ledger, "test");
    for client in [
        MeteredClient::Choice,
        MeteredClient::PlateJudge,
        MeteredClient::HeadLocator,
    ] {
        drop(meter.sent(SentCall {
            client,
            model: if client == MeteredClient::Choice {
                "jev-latest"
            } else {
                "gpt-5.5"
            },
            text_bytes: 100,
            pictures: 1,
            maximum_output_tokens: 100,
        }));
    }

    let seen = starts.0.lock().expect("starts").clone();
    let mut covered = BTreeSet::new();
    for start in &seen {
        let route = ROUTES
            .iter()
            .find(|route| route.provider.label() == start.provider && route.model == start.model)
            .unwrap_or_else(|| panic!("{} {} has no route", start.provider, start.model));
        for category in route.categories {
            assert!(
                start
                    .maxima
                    .iter()
                    .any(|units| units.category == category.label()),
                "{} {} {} starts with no maximum for {}",
                start.provider,
                start.model,
                start.cause,
                category.label()
            );
        }
        covered.insert((route.provider, route.model));
    }
    // Every route starts at least once, but the cutout: it runs after a picture succeeds and is
    // no admission of its own.
    let routed: BTreeSet<_> = ROUTES
        .iter()
        .map(|route| (route.provider, route.model))
        .collect();
    let cutout = (Provider::Fal, df_providers::BACKGROUND_REMOVAL);
    assert_eq!(
        routed.difference(&covered).copied().collect::<Vec<_>>(),
        [cutout]
    );
}

/// An observer that keeps what it was told a call would use.
struct Starts(Mutex<Vec<port::MeterStart>>);

impl port::MeterObserver for Starts {
    fn started(&self, start: &port::MeterStart) {
        self.0.lock().expect("starts").push(start.clone());
    }

    fn settled(&self, _call: &port::MeteredCall) {}
}

#[test]
fn a_category_the_estimate_omits_starts_with_zero_for_it() {
    let ledger = opened_ledger(None);
    let starts = Arc::new(Starts(Mutex::new(Vec::new())));
    ledger.costs().observe(starts.clone());
    drop(ledger.begin_call(
        text_call("gpt-6-luna", "npc_reply"),
        &[(Category::TextInput, 10), (Category::TextOutput, 5)],
    ));
    let seen = starts.0.lock().expect("starts").clone();
    let maxima: Vec<(&str, u64)> = seen[0]
        .maxima
        .iter()
        .map(|units| (units.category.as_str(), units.quantity))
        .collect();
    assert_eq!(
        maxima,
        [
            ("text_input", 10),
            ("text_output", 5),
            ("text_cached_input", 0),
            ("text_cache_write", 0)
        ]
    );
}

#[test]
fn a_billed_call_whose_output_was_no_use_is_a_counted_loss_the_log_remembers() {
    let directory = scratch("unusable");
    let log = directory.join("cost.jsonl");
    let book = opened_book("s1", Some(&log));
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![(Category::TextInput, 1_000)]),
        2_000,
    );
    book.meter(
        text_call("gpt-6-luna", "npc_reply"),
        Usage::reported(vec![(Category::TextInput, 1_000)]).unusable(),
        3_000,
    );
    assert!(book.flush());
    let run = book.report(4_000)["run"].clone();
    assert_eq!(run["events"], 2);
    assert_eq!(run["unusable_events"], 1);
    assert_eq!(run["micro_usd"], 200, "an unusable call costs what it cost");
    assert!(
        book.recent()
            .iter()
            .map(|call| call.usable)
            .eq([true, false]),
        "the port shows which call was no use"
    );
    // The log keeps the flag only where it is false, and a restart reads it back.
    let text = std::fs::read_to_string(&log).expect("log");
    let flags: Vec<bool> = text
        .lines()
        .map(|line| line.contains("\"usable\":false"))
        .collect();
    assert_eq!(flags, [false, true]);
    drop(book);
    let reopened = CostBook::with_session("s2");
    reopened
        .open(
            "df_gameplay_demo_test",
            Some(&log),
            table(),
            PricingStatus::NotLoaded,
            9_000,
        )
        .expect("reopens");
    assert_eq!(reopened.report(10_000)["run"]["unusable_events"], 1);
    // The COST line names it, and only it.
    let event = |usable: bool| CostEvent {
        call_id: "s1-1".to_owned(),
        at_ms: 1,
        run: "r".to_owned(),
        session: "s1".to_owned(),
        provider: Provider::OpenAi,
        model: "m".to_owned(),
        cause: Cause::Embedding,
        detail: "d".to_owned(),
        source: UsageSource::Reported,
        lines: Vec::new(),
        micro_usd: 0,
        usable,
    };
    assert!(!cost_line(&event(true), 0, 0).contains("usable"));
    assert!(cost_line(&event(false), 0, 0).contains("source=reported usable=no usd="));
}

#[test]
fn a_ticket_settled_unusable_records_the_loss_and_the_port_can_say_so() {
    let ledger = shipped_ledger();
    let meter = LedgerMeter::shared(&ledger, "approach");
    meter
        .sent(SentCall {
            client: MeteredClient::Choice,
            model: "jev-latest",
            text_bytes: 400,
            pictures: 0,
            maximum_output_tokens: 0,
        })
        .unusable(AnsweredUsage {
            input_tokens: Some(200),
            output_tokens: None,
        });
    let report = ledger.costs().report(2);
    assert_eq!(report["run"]["unusable_events"], 1);
    assert_eq!(report["run"]["events"], 1);

    let port = MeteringPort::new();
    port.open("run", TABLE).expect("opens");
    port.start(&route("storyteller"))
        .expect("route")
        .settle_unusable(port::MeteredSource::Reported, &[])
        .expect("settles");
    assert!(!port.recent()[0].usable);
}

#[test]
fn missing_supplier_usage_estimates_choice_and_vision_at_their_required_amounts() {
    let choice = SentCall {
        client: MeteredClient::Choice,
        model: "jev-latest",
        text_bytes: 401,
        pictures: 0,
        maximum_output_tokens: 0,
    };
    let choice_spec = provider_meter::spec_of(&choice, "test");
    let choice_usage = provider_meter::answered_usage(
        MeteredClient::Choice,
        &choice_spec.maxima,
        AnsweredUsage::default(),
    );
    assert_eq!(choice_usage.source, UsageSource::Estimated);
    assert_eq!(
        choice_usage.quantities,
        [(Category::TextInput, 201)],
        "choice estimates request text bytes at two bytes per token"
    );

    for (client, text_bytes, pictures, output_tokens, expected) in [
        (
            MeteredClient::PlateJudge,
            101,
            2,
            3_000,
            vec![
                (Category::TextInput, 51),
                (Category::ImageInput, 2 * IMAGE_INPUT_TOKENS_PER_REFERENCE),
                (Category::TextOutput, 3_000),
            ],
        ),
        (
            MeteredClient::HeadLocator,
            99,
            1,
            2_000,
            vec![
                (Category::TextInput, 50),
                (Category::ImageInput, IMAGE_INPUT_TOKENS_PER_REFERENCE),
                (Category::TextOutput, 2_000),
            ],
        ),
    ] {
        let call = SentCall {
            client,
            model: "gpt-5.5",
            text_bytes,
            pictures,
            maximum_output_tokens: output_tokens,
        };
        let spec = provider_meter::spec_of(&call, "test");
        let usage = provider_meter::answered_usage(client, &spec.maxima, AnsweredUsage::default());
        assert_eq!(usage.source, UsageSource::Estimated);
        assert_eq!(usage.quantities, expected, "{client:?}");
        assert!(usage.quantities.iter().all(|(_, quantity)| *quantity > 0));
    }
}

/// What a failed request may have cost, by the failure the client reports.
#[test]
fn a_failed_request_may_have_been_billed_unless_the_supplier_refused_it() {
    use df_providers::{ElevenMusicError, FalSegmentError, NativeHttpRefusal, WorldLabsError};
    for (error, billed) in [
        (ElevenMusicError::InvalidRequest, false),
        (ElevenMusicError::Unauthorized, false),
        (ElevenMusicError::Provider(429), false),
        (ElevenMusicError::Provider(400), false),
        (ElevenMusicError::Provider(500), true),
        (ElevenMusicError::Provider(408), true),
        (
            ElevenMusicError::Transport(NativeHttpRefusal::Deadline),
            true,
        ),
        (
            ElevenMusicError::Transport(NativeHttpRefusal::RequestTooLarge),
            false,
        ),
        (ElevenMusicError::Output, true),
    ] {
        assert_eq!(music_error_may_be_billed(error), billed, "{error:?}");
    }
    for (error, billed) in [
        (FalSegmentError::InvalidRequest, false),
        (FalSegmentError::Unauthorized, false),
        (FalSegmentError::Provider(429), false),
        (FalSegmentError::Provider(503), true),
        (
            FalSegmentError::Transport(NativeHttpRefusal::Deadline),
            true,
        ),
        (FalSegmentError::Queue, true),
        (FalSegmentError::Failed, true),
        (FalSegmentError::Output, true),
    ] {
        assert_eq!(segment_error_may_be_billed(error), billed, "{error:?}");
    }
    for (error, billed) in [
        (WorldLabsError::InvalidRequest, false),
        (WorldLabsError::Unauthorized, false),
        (WorldLabsError::InsufficientCredits, false),
        (WorldLabsError::RateLimited, false),
        (WorldLabsError::ContentPolicy, false),
        (WorldLabsError::Http(422), false),
        (WorldLabsError::Http(502), true),
        (WorldLabsError::Timeout, true),
        (WorldLabsError::Output, true),
        (WorldLabsError::Transport, true),
    ] {
        assert_eq!(world_error_may_be_billed(error), billed, "{error:?}");
    }
}
