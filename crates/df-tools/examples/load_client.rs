//! A TV and five phones using generated RPC over the gameplay server's WebSocket tunnel.
//! Every join and scripted action requires a confirmed server outcome.

#[path = "../src/load_presentation.rs"]
mod load_presentation;
#[path = "../src/load_transport.rs"]
mod load_transport;

use df_protocol::common as rpc;
use rpc::{action_service_client::ActionServiceClient, room_service_client::RoomServiceClient};
use std::{io, time::Duration};
use tokio::time::Instant;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AudioKey {
    Table(u64),
    Fight(u64, usize),
    Closing(u64, usize),
}

struct Table {
    display: load_transport::Connection,
    display_view: tokio::sync::watch::Receiver<Option<rpc::ViewMessage>>,
    phones: Vec<Phone>,
    display_binding: String,
}

struct Phone {
    transport: load_transport::Connection,
    joined: rpc::JoinedRoom,
    view: tokio::sync::watch::Receiver<Option<rpc::ViewMessage>>,
}

fn operation(sequence: u64) -> rpc::OperationId {
    let mut value = vec![0x4c; 16];
    value[..8].copy_from_slice(&sequence.to_le_bytes());
    rpc::OperationId { value: Some(value) }
}

fn request<T>(body: T, binding: &str) -> Result<tonic::Request<T>, Error> {
    let mut request = tonic::Request::new(body);
    request
        .metadata_mut()
        .insert("x-df-local-binding", binding.parse()?);
    request.set_timeout(Duration::from_secs(20));
    Ok(request)
}

fn attribute(page: &str, name: &str) -> Result<String, Error> {
    page.split_once(&format!("{name}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value.to_owned())
        .ok_or_else(|| io::Error::other("display bootstrap attribute missing").into())
}

fn decode_hex(value: &str) -> Result<Vec<u8>, Error> {
    if value.len() != 32 || !value.is_ascii() {
        return Err(io::Error::other("display run id invalid").into());
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
        .collect()
}

fn character(index: usize) -> Result<rpc::CharacterSelection, Error> {
    Ok(rpc::CharacterSelection {
        name: ["Brynn", "Vale", "Maren", "Sigrun", "Brakka"]
            .get(index)
            .ok_or_else(|| io::Error::other("phone index invalid"))?
            .to_string(),
        choices: [
            ("portrait", "portrait-hero-1"),
            ("species", "dwarf"),
            ("class", "fighter-1"),
            ("background", "soldier"),
            ("array", "stalwart"),
            ("alignment", "lawful-good"),
            ("language-1", "dwarvish"),
            ("language-2", "elvish"),
            ("skill-1", "perception"),
            ("skill-2", "survival"),
            ("gaming-set", "dice"),
            ("equipment", "fighter-a-soldier-a"),
            ("style-masteries", "defense-greatsword-flail-javelin"),
            ("allied-ties", "join-order"),
        ]
        .into_iter()
        .map(|(group, option)| rpc::JourneyChoice {
            group_id: group.to_owned(),
            option_id: option.to_owned(),
        })
        .collect(),
        ..Default::default()
    })
}

fn choose_offer<'a>(
    view: &'a rpc::PlayerGameplayView,
    all_ready: bool,
    completed: &std::collections::BTreeSet<i32>,
    all_played: bool,
) -> Option<&'a rpc::GameplayActionOffer> {
    use rpc::GameplayActionKind as Kind;
    if view
        .journey
        .as_ref()
        .and_then(|journey| journey.combat.as_ref())
        .is_some_and(|combat| combat.released_sequence < combat.latest_sequence)
    {
        return None;
    }
    if !all_played
        && let Some(offer) = view
            .offers
            .iter()
            .find(|offer| offer.action_kind == Kind::EndTurn as i32)
    {
        return Some(offer);
    }
    [
        Kind::CreateCharacter,
        Kind::BeginStory,
        Kind::AskCourier,
        Kind::DefendCourier,
        Kind::GreatswordAttack,
        Kind::DeclineReaction,
        Kind::EndTurn,
        Kind::ExamineHarborSeal,
        Kind::ChooseHarborScene,
        Kind::ShortRest,
    ]
    .into_iter()
    .find_map(|kind| {
        if (kind == Kind::BeginStory && !all_ready)
            || (is_one_shot(kind as i32) && completed.contains(&(kind as i32)))
        {
            return None;
        }
        view.offers
            .iter()
            .find(|offer| offer.action_kind == kind as i32)
    })
}

fn is_one_shot(kind: i32) -> bool {
    use rpc::GameplayActionKind as Kind;
    [
        Kind::BeginStory,
        Kind::ExamineHarborSeal,
        Kind::ChooseHarborScene,
        Kind::AskCourier,
        Kind::DefendCourier,
    ]
    .iter()
    .any(|candidate| *candidate as i32 == kind)
}

fn select_inputs(offer: &rpc::GameplayActionOffer) -> Result<Vec<rpc::JourneyChoice>, Error> {
    if offer.action_kind == rpc::GameplayActionKind::GreatswordAttack as i32
        && !offer
            .input_groups
            .iter()
            .any(|group| group.group_id == "tactical-choice")
    {
        return Ok(Vec::new());
    }
    offer
        .input_groups
        .iter()
        .map(|group| {
            let option = group
                .options
                .first()
                .ok_or_else(|| io::Error::other("empty input group"))?;
            Ok(rpc::JourneyChoice {
                group_id: group.group_id.clone(),
                option_id: option.option_id.clone(),
            })
        })
        .collect()
}

async fn act(
    phone: &Phone,
    index: usize,
    sequence: u64,
    all_ready: bool,
    completed: &std::collections::BTreeSet<i32>,
    all_played: bool,
    tv_finished: u64,
) -> Result<Option<i32>, Error> {
    let Some(message) = phone.view.borrow().clone() else {
        return Ok(None);
    };
    let Some(rpc::view_message::Audience::Player(view)) = message.audience else {
        return Err(io::Error::other("phone received wrong audience").into());
    };
    if view
        .journey
        .as_ref()
        .and_then(|journey| journey.combat.as_ref())
        .is_some_and(|combat| combat.latest_sequence > tv_finished)
    {
        return Ok(None);
    }
    let Some(offer) = choose_offer(&view, all_ready, completed, all_played) else {
        return Ok(None);
    };
    let mut body = rpc::SubmitActionRequest {
        session_id: phone.joined.session_id.clone(),
        run_id: phone.joined.run_id.clone(),
        observed_revision: message.revision,
        operation_id: Some(operation(sequence)),
        offer_id: offer.offer_id.clone(),
        action_kind: offer.action_kind,
        destination: offer.destinations.first().map_or(0, |v| v.destination),
        ..Default::default()
    };
    if offer.action_kind == rpc::GameplayActionKind::CreateCharacter as i32 {
        body.character = Some(character(index)?);
    }
    body.action_choices = select_inputs(offer)?;
    let reply = ActionServiceClient::new(phone.transport.channel.clone())
        .submit(request(body, &phone.joined.local_binding)?)
        .await?
        .into_inner();
    match reply.outcome {
        Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) => {
            match receipt.outcome {
                Some(rpc::decision_receipt::Outcome::Accepted(_)) => {
                    println!(
                        "LOADCLIENT accepted phone={index} action={} operation={sequence} unix_ms={}",
                        offer.action_kind,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_millis()
                    );
                    Ok(Some(offer.action_kind))
                }
                other => Err(io::Error::other(format!("load action rejected: {other:?}")).into()),
            }
        }
        other => Err(io::Error::other(format!("load operation uncommitted: {other:?}")).into()),
    }
}

fn is_ready(phone: &Phone) -> bool {
    match phone
        .view
        .borrow()
        .as_ref()
        .and_then(|v| v.audience.as_ref())
    {
        Some(rpc::view_message::Audience::Player(view)) => view
            .journey
            .as_ref()
            .is_some_and(|j| j.own_character.is_some()),
        _ => false,
    }
}

async fn join_table(url: &str) -> Result<Table, Error> {
    let page = load_transport::get(url, "/gameplay/display").await?;
    let page = std::str::from_utf8(&page)?;
    let binding = attribute(page, "data-local-binding")?;
    let run = decode_hex(&attribute(page, "data-run-id")?)?;
    let room = attribute(page, "data-room")?;
    let mut display = load_transport::Connection::connect(url).await?;
    let display_view = display
        .watch(
            rpc::WatchViewRequest {
                session_id: Some(rpc::SessionId {
                    value: Some(vec![0x41; 16]),
                }),
                run_id: Some(rpc::RunId { value: Some(run) }),
                client_binding_id: Some(rpc::ClientBindingId {
                    value: Some(vec![0x72; 16]),
                }),
                after_revision: None,
            },
            &binding,
        )
        .await?;
    let mut phones = Vec::new();
    for index in 0..5u8 {
        let mut transport = load_transport::Connection::connect(url).await?;
        let mut secret = vec![0x5a; 32];
        secret[0] = index;
        let mut join_request = tonic::Request::new(rpc::JoinRoomRequest {
            room_code: room.clone(),
            operation_id: Some(operation(u64::from(index) + 1)),
            join_secret: secret,
        });
        join_request.set_timeout(Duration::from_secs(20));
        let reply = tokio::time::timeout(
            Duration::from_secs(20),
            RoomServiceClient::new(transport.channel.clone()).join(join_request),
        )
        .await??
        .into_inner();
        let Some(rpc::join_room_response::Outcome::Joined(joined)) = reply.outcome else {
            return Err(io::Error::other("load join refused").into());
        };
        let joined = *joined;
        let view = transport
            .watch(
                rpc::WatchViewRequest {
                    session_id: joined.session_id.clone(),
                    run_id: joined.run_id.clone(),
                    client_binding_id: joined.client_binding_id.clone(),
                    after_revision: None,
                },
                &joined.local_binding,
            )
            .await?;
        phones.push(Phone {
            transport,
            joined,
            view,
        });
    }
    println!("LOADCLIENT joined displays=1 phones=5 url={url}");
    Ok(Table {
        display,
        display_view,
        phones,
        display_binding: binding,
    })
}

async fn fetch_audio(
    url: &str,
    key: AudioKey,
    primary: &str,
    parts: &[String],
    seen: &mut std::collections::BTreeSet<AudioKey>,
) -> Result<(), Error> {
    if primary.is_empty() || seen.contains(&key) {
        return Ok(());
    }
    if seen.len() >= 4096 {
        return Err(io::Error::other("fake audio identity bound exceeded").into());
    }
    let mut total_bytes = 0usize;
    for path in std::iter::once(primary).chain(parts.iter().map(String::as_str)) {
        let bytes = load_transport::get(url, path).await?;
        verify_silent_wav(&bytes)?;
        total_bytes += bytes.len();
    }
    seen.insert(key);
    println!(
        "LOADCLIENT audio id={key:?} parts={} bytes={total_bytes}",
        parts.len() + 1
    );
    Ok(())
}

async fn consume_display(
    url: &str,
    message: Option<rpc::ViewMessage>,
    audio_keys: &mut std::collections::BTreeSet<AudioKey>,
    last_phase: &mut Option<i32>,
) -> Result<bool, Error> {
    let Some(message) = message else {
        return Ok(false);
    };
    let rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(view)),
        ..
    } = message
    else {
        return Err(io::Error::other("display received wrong audience").into());
    };
    let Some(journey) = view.journey else {
        return Ok(false);
    };
    let changed = *last_phase != Some(journey.phase);
    if changed {
        println!(
            "LOADCLIENT phase value={} unix_ms={}",
            journey.phase,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
        );
        *last_phase = Some(journey.phase);
        let image = load_transport::get(url, &view.scene_asset).await?;
        image::load_from_memory(&image)?;
        println!(
            "LOADCLIENT image phase={} bytes={}",
            journey.phase,
            image.len()
        );
    }
    for line in journey.table_log {
        fetch_audio(
            url,
            AudioKey::Table(line.sequence),
            &line.audio_url,
            &line.audio_part_urls,
            audio_keys,
        )
        .await?;
    }
    if let Some(combat) = journey.combat {
        for outcome in combat
            .outcomes
            .iter()
            .chain(&combat.move_outcomes)
            .chain(&combat.death_save_outcomes)
        {
            for (index, line) in outcome.lines.iter().enumerate() {
                fetch_audio(
                    url,
                    AudioKey::Fight(outcome.sequence, index),
                    &line.audio_url,
                    &line.audio_part_urls,
                    audio_keys,
                )
                .await?;
            }
        }
        for (index, line) in combat.closing_lines.iter().enumerate() {
            fetch_audio(
                url,
                AudioKey::Closing(combat.latest_sequence, index),
                &line.audio_url,
                &line.audio_part_urls,
                audio_keys,
            )
            .await?;
        }
    }
    Ok(changed)
}

async fn fetch_phone_images(url: &str, phone: &Phone, index: usize) -> Result<bool, Error> {
    let message = phone.view.borrow().clone();
    let Some(rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Player(view)),
        ..
    }) = message
    else {
        return Err(io::Error::other("phone received wrong audience").into());
    };
    let Some(journey) = view.journey else {
        return Ok(false);
    };
    let Some(character) = &journey.own_character else {
        return Ok(false);
    };
    if journey
        .party
        .iter()
        .filter(|member| member.character_ready)
        .count()
        != 5
    {
        return Ok(false);
    }
    let references = journey.party.len() + 1;
    let mut paths = std::collections::BTreeSet::new();
    paths.insert(character.portrait_uri.as_str());
    paths.extend(journey.party.iter().map(|member| member.portrait.as_str()));
    if paths.contains("") {
        return Err(io::Error::other("hero portrait reference missing").into());
    }
    let mut total_bytes = 0usize;
    for path in &paths {
        let bytes = load_transport::get(url, path).await?;
        image::load_from_memory(&bytes)?;
        total_bytes += bytes.len();
    }
    println!(
        "LOADCLIENT phone_images phone={index} refs={references} requests={} bytes={total_bytes}",
        paths.len()
    );
    Ok(true)
}

fn verify_silent_wav(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() != 32_044
        || &bytes[..4] != b"RIFF"
        || bytes[4..8] != 32_036u32.to_le_bytes()
        || &bytes[8..16] != b"WAVEfmt "
        || bytes[16..20] != 16u32.to_le_bytes()
        || bytes[20..22] != 1u16.to_le_bytes()
        || bytes[22..24] != 1u16.to_le_bytes()
        || bytes[24..28] != 8_000u32.to_le_bytes()
        || bytes[28..32] != 16_000u32.to_le_bytes()
        || bytes[32..34] != 2u16.to_le_bytes()
        || bytes[34..36] != 16u16.to_le_bytes()
        || &bytes[36..40] != b"data"
        || bytes[40..44] != 32_000u32.to_le_bytes()
        || bytes[44..].iter().any(|sample| *sample != 0)
    {
        return Err(
            io::Error::other("fake audio is not the expected silent two-second PCM WAV").into(),
        );
    }
    Ok(())
}

fn verify_release(submitted: u64, released: u64) -> Result<(), Error> {
    if released < submitted {
        return Err(io::Error::other("TV presentation was not released").into());
    }
    Ok(())
}

async fn run(url: &str, seconds: u64, pace: Duration) -> Result<(), Error> {
    let Table {
        mut display,
        display_view,
        mut phones,
        display_binding,
    } = join_table(url).await?;
    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    let mut tv = load_presentation::Presentation::default();
    let mut lines = load_presentation::Presentation::default();
    let mut accepted = 0u64;
    let mut sequence = 100u64;
    let mut next_phone = 0usize;
    let mut completed = std::collections::BTreeSet::new();
    let mut action_counts = std::collections::BTreeMap::<i32, u64>::new();
    let mut phone_turns = [0u64; 5];
    let mut audio_keys = std::collections::BTreeSet::new();
    let mut last_phase = None;
    let mut image_fetches = 0u64;
    let mut portrait_phones = [false; 5];
    loop {
        let message = display_view.borrow().clone();
        if let Some(rpc::view_message::Audience::Display(view)) =
            message.as_ref().and_then(|view| view.audience.as_ref())
            && let Some(combat) = view
                .journey
                .as_ref()
                .and_then(|journey| journey.combat.as_ref())
        {
            tv.observe(combat)?;
        }
        if let Some(rpc::view_message::Audience::Display(view)) =
            message.as_ref().and_then(|view| view.audience.as_ref())
            && let Some(journey) = &view.journey
        {
            lines.observe_lines(&journey.table_log)?;
        }
        let now_ms = u64::try_from(start.elapsed().as_millis())?;
        let narrative = lines.advance(now_ms);
        if let Some((sequence, hold_ms)) = narrative.started {
            println!(
                "LOADCLIENT line_start sequence={sequence} hold_ms={hold_ms} elapsed_ms={now_ms}"
            );
        }
        if let Some(sequence) = narrative.finished {
            println!("LOADCLIENT line_finish sequence={sequence} elapsed_ms={now_ms}");
        }
        let progress = tv.advance(now_ms);
        if let Some((sequence, hold_ms)) = progress.closing_started {
            println!(
                "LOADCLIENT tv_closing_start sequence={sequence} hold_ms={hold_ms} elapsed_ms={now_ms}"
            );
        }
        if let Some((sequence, hold_ms)) = progress.started {
            println!(
                "LOADCLIENT tv_start sequence={sequence} hold_ms={hold_ms} elapsed_ms={now_ms}"
            );
        }
        if let Some(sequence) = progress.finished {
            let released =
                rpc::session_service_client::SessionServiceClient::new(display.channel.clone())
                    .report_presented(request(
                        rpc::PresentedRequest {
                            sequence,
                            display_instance: 1,
                            layout_readback_json: String::new(),
                        },
                        &display_binding,
                    )?)
                    .await?
                    .into_inner()
                    .released_sequence;
            verify_release(sequence, released)?;
            println!(
                "LOADCLIENT tv_finish sequence={sequence} released={released} elapsed_ms={now_ms} unix_ms={}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis()
            );
        }
        image_fetches +=
            u64::from(consume_display(url, message, &mut audio_keys, &mut last_phase).await?);
        if Instant::now() >= deadline && tv.idle() && lines.idle() {
            break;
        }
        let all_ready = phones.iter().all(is_ready);
        if all_ready {
            for (index, (phone, fetched)) in
                phones.iter().zip(portrait_phones.iter_mut()).enumerate()
            {
                if !*fetched {
                    *fetched = fetch_phone_images(url, phone, index).await?;
                }
            }
        }
        for offset in 0..phones.len() {
            if Instant::now() >= deadline || !lines.idle() || !tv.idle() {
                break;
            }
            let index = (next_phone + offset) % phones.len();
            let phone = phones
                .get(index)
                .ok_or_else(|| io::Error::other("phone missing"))?;
            if let Some(kind) = act(
                phone,
                index,
                sequence,
                all_ready,
                &completed,
                phone_turns.iter().all(|count| *count > 0),
                tv.finished(),
            )
            .await?
            {
                accepted += 1;
                *action_counts.entry(kind).or_default() += 1;
                completed.insert(kind);
                if kind != rpc::GameplayActionKind::CreateCharacter as i32 {
                    *phone_turns
                        .get_mut(index)
                        .ok_or_else(|| io::Error::other("phone counter missing"))? += 1;
                }
                next_phone = (index + 1) % phones.len();
                break;
            }
        }
        sequence += 1;
        tokio::time::sleep(pace).await;
        display.check()?;
        for phone in &mut phones {
            phone.transport.check()?;
        }
    }
    let ready = phones.iter().filter(|p| is_ready(p)).count();
    if ready != 5
        || accepted < 10
        || audio_keys.is_empty()
        || portrait_phones.iter().any(|fetched| !*fetched)
        || phone_turns.contains(&0)
        || !action_counts.contains_key(&(rpc::GameplayActionKind::GreatswordAttack as i32))
    {
        return Err(io::Error::other(format!(
            "incomplete workload ready={ready} accepted={accepted} audio={}",
            audio_keys.len()
        ))
        .into());
    }
    println!("LOADCLIENT actions counts={action_counts:?} phone_turns={phone_turns:?}");
    println!(
        "LOADCLIENT done displays=1 phones=5 ready={ready} accepted={accepted} audio={} images={image_fetches} portrait_phones=5",
        audio_keys.len()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut url = "http://127.0.0.1:63411".to_owned();
    let mut seconds = 600;
    let mut pace_ms = 4000;
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| io::Error::other("argument value missing"))?;
        match key.as_str() {
            "--url" => url = value,
            "--seconds" => seconds = value.parse()?,
            "--pace-ms" => pace_ms = value.parse()?,
            _ => return Err(io::Error::other("unknown load argument").into()),
        }
    }
    if !(1..=3600).contains(&seconds) || !(1000..=60000).contains(&pace_ms) {
        return Err(io::Error::other("load duration/pace out of bounds").into());
    }
    tokio::time::timeout(
        Duration::from_secs(seconds + 120),
        run(&url, seconds, Duration::from_millis(pace_ms)),
    )
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave_fixture() -> Vec<u8> {
        let mut bytes = Vec::from(*b"RIFF");
        bytes.extend(32_036u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8_000u32.to_le_bytes());
        bytes.extend(16_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(32_000u32.to_le_bytes());
        bytes.resize(32_044, 0);
        bytes
    }

    #[test]
    fn fetched_fake_audio_requires_complete_pcm_header_and_declared_lengths() {
        assert!(verify_silent_wav(&wave_fixture()).is_ok());
        assert!(verify_silent_wav(b"RIFF").is_err());
        let mut bytes = wave_fixture();
        bytes[40] = 1;
        assert!(verify_silent_wav(&bytes).is_err());
    }

    #[test]
    fn fetched_fake_audio_refuses_wrong_format_or_sample_rate() {
        for index in [8, 12, 16, 20, 22, 24, 28, 32, 34, 36] {
            let mut bytes = wave_fixture();
            bytes[index] ^= 1;
            assert!(verify_silent_wav(&bytes).is_err());
        }
    }

    #[test]
    fn fetched_fake_audio_refuses_a_non_silent_sample() {
        let mut bytes = wave_fixture();
        bytes[44] = 1;
        assert!(verify_silent_wav(&bytes).is_err());
    }

    #[test]
    fn character_fixtures_are_accepted_by_current_source_rules() {
        use df_rules::local_journey::{self as rules, JourneyRuleError};
        assert_eq!(
            rules::validate_character(&rules::CreationRules::srd_521(), "LoadHero1", &[]),
            Err(JourneyRuleError::InvalidName)
        );
        for index in 0..5 {
            let hero = character(index).unwrap();
            let choices: Vec<_> = hero
                .choices
                .iter()
                .filter(|choice| rules::BUILD_GROUPS.contains(&choice.group_id.as_str()))
                .map(|choice| (choice.group_id.as_str(), choice.option_id.as_str()))
                .collect();
            assert!(
                rules::validate_character(&rules::CreationRules::srd_521(), &hero.name, &choices)
                    .is_ok()
            );
        }
    }

    #[test]
    fn an_unreleased_display_ack_is_a_workload_failure() {
        assert!(verify_release(5, 4).is_err());
        assert!(verify_release(5, 5).is_ok());
        assert!(verify_release(5, 6).is_ok());
    }

    #[test]
    fn phone_actions_wait_for_the_server_presentation_release() {
        let view = rpc::PlayerGameplayView {
            offers: vec![rpc::GameplayActionOffer {
                action_kind: rpc::GameplayActionKind::EndTurn as i32,
                ..Default::default()
            }],
            journey: Some(rpc::JourneyView {
                combat: Some(rpc::CombatView {
                    latest_sequence: 5,
                    released_sequence: 4,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(choose_offer(&view, true, &Default::default(), true).is_none());
    }

    #[test]
    fn legacy_attack_toggles_are_not_sent_as_tactical_action_choices() {
        let mut offer = rpc::GameplayActionOffer {
            action_kind: rpc::GameplayActionKind::GreatswordAttack as i32,
            input_groups: vec![rpc::JourneyGroup {
                group_id: "graze".to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "no".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(select_inputs(&offer).unwrap().is_empty());
        offer.input_groups[0].group_id = "tactical-choice".to_owned();
        assert_eq!(select_inputs(&offer).unwrap()[0].option_id, "no");
    }

    #[test]
    fn combat_waits_for_every_phone_to_participate_before_attacking() {
        let view = rpc::PlayerGameplayView {
            offers: [
                rpc::GameplayActionKind::GreatswordAttack,
                rpc::GameplayActionKind::EndTurn,
            ]
            .into_iter()
            .map(|kind| rpc::GameplayActionOffer {
                action_kind: kind as i32,
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            choose_offer(&view, true, &Default::default(), false)
                .unwrap()
                .action_kind,
            rpc::GameplayActionKind::EndTurn as i32
        );
        assert_eq!(
            choose_offer(&view, true, &Default::default(), true)
                .unwrap()
                .action_kind,
            rpc::GameplayActionKind::GreatswordAttack as i32
        );
    }

    #[test]
    fn a_repeated_examine_offer_does_not_starve_scene_progression() {
        let view = rpc::PlayerGameplayView {
            offers: [
                rpc::GameplayActionKind::ExamineHarborSeal,
                rpc::GameplayActionKind::ChooseHarborScene,
            ]
            .into_iter()
            .map(|kind| rpc::GameplayActionOffer {
                action_kind: kind as i32,
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        let completed = [rpc::GameplayActionKind::ExamineHarborSeal as i32].into();
        assert_eq!(
            choose_offer(&view, true, &completed, true)
                .unwrap()
                .action_kind,
            rpc::GameplayActionKind::ChooseHarborScene as i32
        );
    }

    #[test]
    fn begin_story_waits_for_five_confirmed_heroes() {
        let view = rpc::PlayerGameplayView {
            offers: vec![rpc::GameplayActionOffer {
                action_kind: rpc::GameplayActionKind::BeginStory as i32,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(choose_offer(&view, false, &Default::default(), true).is_none());
        assert!(choose_offer(&view, true, &Default::default(), true).is_some());
    }

    #[test]
    fn operation_ids_are_distinct_for_distinct_script_steps() {
        assert_ne!(operation(100), operation(101));
        assert_eq!(operation(100), operation(100));
    }
}
