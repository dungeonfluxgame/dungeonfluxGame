use super::*;

fn line() -> TimingEvent {
    TimingEvent {
        kind: "line",
        reason: "read",
        id: 29,
        scene: 7,
        at_ms: 10_000,
        slot: Some(2),
        table_sequence: Some(41),
        chars: 89,
        read_ms: 8_285,
        opened_ms: Some(1_000),
        voice_start_ms: None,
        voice_end_ms: None,
        closed_ms: Some(10_000),
        crossfade_ms: 0,
        visual_source: None,
        component: None,
        component_phase: None,
        visibility: None,
        visible_held_ms: None,
        lag_ms: None,
    }
}

#[test]
fn display_staging_preserves_older_events_and_transfers_its_loss_counter() {
    DISPLAY_CAPTURE.with_borrow_mut(|(events, lost)| {
        events.clear();
        *lost = 0;
    });
    for id in 0..MAXIMUM_TIMING_EVENTS as u64 {
        capture_display(TimingEvent { id, ..line() }).unwrap();
    }
    assert_eq!(capture_display(line()), Err(TimingError::Capacity));
    let (events, lost) = drain_display_capture();
    assert_eq!(events.len(), MAXIMUM_TIMING_BATCH_EVENTS);
    assert_eq!(events[0].id, 0);
    assert_eq!(lost, 1);
    assert_eq!(drain_display_capture().1, 0);
    DISPLAY_CAPTURE.with_borrow_mut(|(events, _)| events.clear());
}

#[test]
fn timing_batch_keeps_its_identity_and_bytes_until_matching_receipt() {
    let mut buffer = TimingBuffer::default();
    buffer.push(line()).unwrap();
    let first = buffer.batch().unwrap();
    buffer.push(TimingEvent { id: 30, ..line() }).unwrap();
    assert_eq!(buffer.batch().unwrap(), first);
    assert!(!buffer.acknowledge(first.sequence + 1));
    assert_eq!(buffer.batch().unwrap(), first);
    assert!(buffer.acknowledge(first.sequence));
    let second = buffer.batch().unwrap();
    assert_eq!(second.sequence, first.sequence + 1);
    assert_eq!(decode(&second.bytes).unwrap()[0].id, 30);
}

#[test]
fn a_gap_without_any_retained_event_is_uploaded_once_after_acknowledgment() {
    let mut buffer = TimingBuffer::default();
    assert_eq!(buffer.batch(), Err(TimingError::Empty));
    buffer.report_loss(2);
    let gap = buffer.batch().unwrap();
    assert_eq!(gap.records, 0);
    assert!(gap.bytes.is_empty());
    assert_eq!(gap.dropped, 2);
    assert_eq!(buffer.batch().unwrap(), gap);
    assert!(buffer.acknowledge(gap.sequence));
    assert_eq!(buffer.batch(), Err(TimingError::Empty));
}

#[test]
fn timing_buffer_bounds_batches_and_reports_loss_without_evicting_pending() {
    let mut buffer = TimingBuffer::default();
    for _ in 0..MAXIMUM_TIMING_EVENTS {
        buffer.push(line()).unwrap();
    }
    let first = buffer.batch().unwrap();
    assert_eq!(first.records, MAXIMUM_TIMING_BATCH_EVENTS);
    assert!(first.bytes.len() <= MAXIMUM_TIMING_BATCH_BYTES);
    assert_eq!(buffer.push(line()), Err(TimingError::Capacity));
    assert_eq!(buffer.dropped(), 1);
    assert_eq!(buffer.batch().unwrap(), first);
}

#[test]
fn timing_allowlist_rejects_text_unknown_keys_and_invalid_measurements() {
    assert_eq!(
        encode(&[TimingEvent {
            reason: "private words",
            ..line()
        }]),
        Err(TimingError::Invalid)
    );
    assert_eq!(
        encode(&[TimingEvent {
            opened_ms: Some(11_000),
            ..line()
        }]),
        Err(TimingError::Invalid)
    );
    let bytes = encode(&[line()]).unwrap();
    assert_eq!(decode(&bytes).unwrap(), vec![line()]);
    let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    request.resource_logs[0].scope_logs[0].log_records[0].body = Some(AnyValue {
        value: Some(Value::StringValue("private".into())),
    });
    assert_eq!(decode(&request.encode_to_vec()), Err(TimingError::Invalid));
}

#[test]
fn visual_source_round_trips_closed_sources_state_and_asset_hashes() {
    let event = TimingEvent::visual_source(
        17,
        23,
        100,
        VisualSourceDetails {
            requested: VisualSource::ScenePlate,
            effective: VisualSource::SceneArtwork,
            shot: VisualShot::Establishing,
            plate_state: VisualPlateState::Failed,
            requested_asset: Some(31),
            effective_asset: Some(37),
        },
    );
    let bytes = encode(std::slice::from_ref(&event)).unwrap();
    let [decoded] = decode(&bytes).unwrap().try_into().unwrap();
    assert_eq!(decoded, event);
    let line = decoded.log_line("DISPLAY", 200, "producer", 3);
    assert!(line.contains("requested_source=scene_plate"));
    assert!(line.contains("effective_source=scene_artwork"));
    assert!(line.contains("plate_state=failed"));
    assert!(line.contains("requested_asset_id=31"));
    assert!(line.contains("effective_asset_id=37"));
}

#[test]
fn component_lifecycle_round_trips_closed_visibility_and_duration() {
    let mut event = TimingEvent::component_lifecycle(
        41,
        23,
        8_250,
        PresentationComponent::SpeechBubble,
        ComponentPhase::Hidden,
        EffectiveVisibility::Occluded,
        "successor",
    );
    event.opened_ms = Some(2_000);
    event.closed_ms = Some(8_250);
    event.read_ms = 5_500;
    event.visible_held_ms = Some(4_125);
    let bytes = encode(std::slice::from_ref(&event)).unwrap();
    assert_eq!(decode(&bytes).unwrap(), vec![event.clone()]);

    let line = event.log_line("DISPLAY", 99_000, "sha256:producer", 3);
    assert!(line.contains("component=speech_bubble"));
    assert!(line.contains("component_phase=hidden"));
    assert!(line.contains("visibility=occluded"));
    assert!(line.contains("read_ms=5500"));
    assert!(line.contains("visible_held_ms=4125"));
    assert_eq!(
        PresentationComponent::from_key("check_card"),
        Some(PresentationComponent::CheckCard)
    );
    assert_eq!(
        PresentationComponent::from_key("dice"),
        Some(PresentationComponent::Dice)
    );
    assert_eq!(
        PresentationComponent::from_key("party_hud"),
        Some(PresentationComponent::PartyHud)
    );
    assert_eq!(
        PresentationComponent::from_key("roll_card"),
        Some(PresentationComponent::RollCard)
    );
    assert_eq!(
        PresentationComponent::from_key("die"),
        Some(PresentationComponent::Die)
    );
    assert_eq!(
        PresentationComponent::from_key("combat_banner"),
        Some(PresentationComponent::CombatBanner)
    );
    assert_eq!(
        PresentationComponent::from_key("turn_change"),
        Some(PresentationComponent::TurnChange)
    );
    assert_eq!(
        PresentationComponent::from_key("battlefield"),
        Some(PresentationComponent::Battlefield)
    );
    assert_eq!(PresentationComponent::from_key("secret-component"), None);
    assert_eq!(
        ComponentPhase::from_key("readable"),
        Some(ComponentPhase::Readable)
    );
    assert_eq!(
        EffectiveVisibility::from_key("hidden"),
        Some(EffectiveVisibility::Hidden)
    );
}

#[test]
fn battle_component_kinds_build_valid_lifecycle_events() {
    use ComponentPhase::{Closing, Hidden, Opened, Readable};

    let components = [
        (
            PresentationComponent::RollCard,
            "roll_card",
            &[Opened, Readable, Closing, Hidden][..],
        ),
        (
            PresentationComponent::Die,
            "die",
            &[Opened, Readable, Closing, Hidden][..],
        ),
        (
            PresentationComponent::CombatBanner,
            "combat_banner",
            &[Opened, Readable, Hidden][..],
        ),
        (
            PresentationComponent::TurnChange,
            "turn_change",
            &[Opened, Readable, Hidden][..],
        ),
        (
            PresentationComponent::Battlefield,
            "battlefield",
            &[Opened, Readable, Hidden][..],
        ),
    ];

    for (component, key, phases) in components {
        for phase in phases {
            let hidden = *phase == Hidden;
            let event = TimingEvent::component_lifecycle(
                41,
                23,
                8_250,
                component,
                *phase,
                if hidden {
                    EffectiveVisibility::Hidden
                } else {
                    EffectiveVisibility::Visible
                },
                if hidden { "ended" } else { "ready" },
            );

            assert_eq!(event.component, Some(component));
            assert_eq!(event.component.unwrap().key(), key);
            assert_eq!(event.component_phase, Some(*phase));
            assert_eq!(event.at_ms, 8_250);
            assert!(encode(&[event]).is_ok());
        }
    }
}

#[test]
fn component_events_require_closed_fields_and_bounded_measurements() {
    let event = TimingEvent::component_lifecycle(
        1,
        2,
        10,
        PresentationComponent::Subtitle,
        ComponentPhase::Readable,
        EffectiveVisibility::Visible,
        "ready",
    );
    assert!(encode(std::slice::from_ref(&event)).is_ok());
    assert_eq!(
        encode(&[TimingEvent {
            component_phase: None,
            ..event.clone()
        }]),
        Err(TimingError::Invalid)
    );
    assert_eq!(
        encode(&[TimingEvent {
            read_ms: 120_001,
            ..event.clone()
        }]),
        Err(TimingError::Invalid)
    );
    assert_eq!(
        encode(&[TimingEvent {
            visible_held_ms: Some(MAXIMUM_PAGE_MS + 1),
            ..event
        }]),
        Err(TimingError::Invalid)
    );
}

#[test]
fn component_clock_counts_only_confirmed_readable_intervals() {
    let mut clock = ComponentClock::default();
    assert_eq!(
        clock.observe(
            100,
            ComponentPhase::Opened,
            EffectiveVisibility::Transparent,
        ),
        0
    );
    assert_eq!(
        clock.observe(250, ComponentPhase::Readable, EffectiveVisibility::Visible,),
        0
    );
    assert_eq!(
        clock.observe(
            1_000,
            ComponentPhase::Readable,
            EffectiveVisibility::Occluded,
        ),
        750
    );
    assert_eq!(
        clock.observe(
            1_500,
            ComponentPhase::Readable,
            EffectiveVisibility::Visible,
        ),
        750
    );
    assert_eq!(
        clock.observe(2_000, ComponentPhase::Hidden, EffectiveVisibility::Hidden,),
        1_250
    );
    assert_eq!(clock.opened_ms(), Some(100));
    assert_eq!(clock.visible_held_ms(3_000), 1_250);
}

#[test]
fn visual_source_rejects_missing_or_uncatalogued_details() {
    assert_eq!(
        encode(&[TimingEvent::lifecycle("visual-source", "changed", 1, 2, 3,)]),
        Err(TimingError::Invalid)
    );
    let event = TimingEvent::visual_source(
        17,
        23,
        100,
        VisualSourceDetails {
            requested: VisualSource::ScenePlate,
            effective: VisualSource::SceneArtwork,
            shot: VisualShot::Establishing,
            plate_state: VisualPlateState::Failed,
            requested_asset: Some(31),
            effective_asset: Some(37),
        },
    );
    let bytes = encode(&[event]).unwrap();
    let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    let record = &mut request.resource_logs[0].scope_logs[0].log_records[0];
    let source = record
        .attributes
        .iter_mut()
        .find(|field| field.key == "effective_source")
        .unwrap();
    source.value.as_mut().unwrap().value = Some(Value::StringValue("private-url".into()));
    assert_eq!(decode(&request.encode_to_vec()), Err(TimingError::Invalid));
}

#[test]
fn timing_ingress_rejects_unknown_attributes_duplicates_resources_and_oversize() {
    let bytes = encode(&[line()]).unwrap();
    let request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    for field in [
        attribute("prompt", Value::StringValue("private".into())),
        attribute("at_ms", Value::IntValue(1)),
        attribute("slot", Value::IntValue(-1)),
    ] {
        let mut invalid = request.clone();
        invalid.resource_logs[0].scope_logs[0].log_records[0]
            .attributes
            .push(field);
        assert_eq!(decode(&invalid.encode_to_vec()), Err(TimingError::Invalid));
    }
    let mut invalid = request.clone();
    invalid.resource_logs.push(request.resource_logs[0].clone());
    assert_eq!(decode(&invalid.encode_to_vec()), Err(TimingError::Invalid));
    assert_eq!(
        decode(&vec![0; MAXIMUM_TIMING_BATCH_BYTES + 1]),
        Err(TimingError::Oversized)
    );
    assert_eq!(
        encode(&vec![line(); MAXIMUM_TIMING_BATCH_EVENTS + 1]),
        Err(TimingError::Capacity)
    );
}

#[test]
fn timing_log_distinguishes_no_voice_from_cut_voice_and_keeps_receive_time() {
    let text = line().log_line("DISPLAY", 123_456, "page", 1);
    assert!(text.contains("receive_unix_ms=123456"));
    assert!(text.contains("held_ms=9000"));
    assert!(text.contains("voice_start_ms=none"));
    assert!(text.contains("chars=89 read_ms=8285"));
    assert!(text.contains("reason=read"));
}

#[test]
fn hs48_capture_rejects_uncatalogued_close_causes_and_backdated_voice_reports() {
    for reason in ["dice", "check"] {
        assert_eq!(
            TimingBuffer::default().push(TimingEvent::lifecycle("line", reason, 1, 7, 2000)),
            Err(TimingError::Invalid)
        );
    }
    let mut voice = TimingEvent::lifecycle("voice-start", "ready", 1, 7, 950);
    voice.opened_ms = Some(1000);
    voice.voice_start_ms = Some(950);
    voice.table_sequence = Some(42);
    assert_eq!(
        TimingBuffer::default().push(voice.clone()),
        Err(TimingError::Invalid)
    );
    voice.at_ms = voice.at_ms.max(voice.opened_ms.unwrap());
    let mut buffer = TimingBuffer::default();
    buffer.push(voice).unwrap();
    let mut crossfade = TimingEvent::lifecycle("plate-crossfade", "started", 8, 7, 1200);
    crossfade.crossfade_ms = 1000;
    buffer.push(crossfade).unwrap();
    buffer
        .push(TimingEvent::lifecycle("line", "successor", 1, 7, 2000))
        .unwrap();
    let batch = buffer.batch().unwrap();
    let decoded = decode(&batch.bytes).unwrap();
    assert_eq!(decoded[0].voice_start_ms, Some(950));
    assert_eq!(decoded[0].opened_ms, Some(1000));
    assert_eq!(decoded[0].table_sequence, Some(42));
    assert_eq!(batch.dropped, 0);
}

/// Asserts each variant's key parses back to it, no two variants share a key and every key is
/// snake_case, so the wire keys cannot drift from the closed sets.
fn assert_keys_round_trip<T: Copy + Eq + std::fmt::Debug>(
    all: &[T],
    key: fn(T) -> &'static str,
    from_key: fn(&str) -> Option<T>,
) {
    let mut seen = std::collections::BTreeSet::new();
    for variant in all.iter().copied() {
        let text = key(variant);
        assert_eq!(from_key(text), Some(variant), "{text}");
        assert!(seen.insert(text), "duplicate key {text}");
        assert!(
            !text.is_empty()
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "{text} is not snake_case"
        );
    }
    assert_eq!(from_key("not_a_key"), None);
}

#[test]
fn every_closed_key_round_trips_and_is_unique() {
    assert_keys_round_trip(
        PresentationComponent::ALL,
        PresentationComponent::key,
        PresentationComponent::from_key,
    );
    assert_keys_round_trip(
        ComponentPhase::ALL,
        ComponentPhase::key,
        ComponentPhase::from_key,
    );
    assert_keys_round_trip(
        EffectiveVisibility::ALL,
        EffectiveVisibility::key,
        EffectiveVisibility::from_key,
    );
    assert_keys_round_trip(VisualSource::ALL, VisualSource::key, VisualSource::from_key);
    assert_keys_round_trip(VisualShot::ALL, VisualShot::key, VisualShot::from_key);
    assert_keys_round_trip(
        VisualPlateState::ALL,
        VisualPlateState::key,
        VisualPlateState::from_key,
    );
}
