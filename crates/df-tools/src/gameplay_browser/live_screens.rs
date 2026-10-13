//! Routes the accepted lobby and exploration views to the designed screens and owns their
//! mounts. The shared display gets a 16:9 stage; a player gets the df-ui lobby or
//! exploration surface. Phases not listed in [`takes_over`] keep their existing rendering.
use super::live_exploration::{
    ExplorationChoiceModel, ExplorationMember, ExplorationModel, ExplorationScreen,
    ExplorationSpeaker,
};
use super::live_lobby::{LobbyMember, LobbyModel, LobbyOfferModel, LobbyScreen};
use super::live_tv::{TvMember, TvMode, TvModel, TvSpeaker, TvStage};
use super::*;
use df_ui::{ConceptScene, ExplorationPortrait, JoinStage};

const HARBOR_ESTABLISHING: &str =
    "/assets/concept-art/establishing-harbor-canal-rowboat-bridge.webp";
const HARBOR_WHARF: &str = "/assets/ui/scenes/mara-harbor-v4.png";
const HARBOR_INN: &str = "/assets/concept-art/scene-tavern-barkeep-talk-rain.webp";
const WAITING_TEXT: &str = "Waiting for your companions to act.";
const CHOOSE_TEXT: &str = "Choose how your hero responds.";

/// The mounted live screens of one client. At most one display stage, or one player
/// surface, exists at a time; a phase change disposes the previous one first.
#[derive(Default)]
pub(super) struct LiveScreens {
    pub(super) lobby: Option<LobbyScreen>,
    pub(super) exploration: Option<ExplorationScreen>,
    pub(super) tv: Option<TvStage>,
    last_private_clue: Option<String>,
}

impl LiveScreens {
    fn is_empty(&self) -> bool {
        self.lobby.is_none() && self.exploration.is_none() && self.tv.is_none()
    }

    /// Disposes everything mounted, attempting each cleanup and returning the first failure.
    pub(super) fn dispose(&mut self) -> Result<(), JsValue> {
        let mut failure = None;
        if let Some(lobby) = self.lobby.take() {
            failure = failure.or(lobby.dispose().err());
        }
        if let Some(exploration) = self.exploration.take() {
            failure = failure.or(exploration.dispose().err());
        }
        if let Some(tv) = self.tv.take() {
            failure = failure.or(tv.dispose().err());
        }
        failure.map_or(Ok(()), Err)
    }
}

/// What a render pass knows about one accepted view of the phases handled here.
pub(super) struct SceneInput<'a> {
    pub(super) phase: rpc::JourneyPhase,
    pub(super) narration: &'a str,
    pub(super) scene: &'a rpc::GameplayScene,
    pub(super) journey: &'a rpc::JourneyView,
    pub(super) offers: &'a [rpc::GameplayActionOffer],
    pub(super) clue: &'a str,
}

fn is_new_private_clue(previous: Option<&str>, clue: &str) -> bool {
    previous == Some("") && !clue.is_empty()
}

/// The lobby, opening, dialogue and non-combat completion are rendered here. Combat and the
/// encounter aftermath keep their own rendering.
pub(super) fn takes_over(
    phase: rpc::JourneyPhase,
    journey: &rpc::JourneyView,
    scene: &rpc::GameplayScene,
) -> bool {
    match phase {
        rpc::JourneyPhase::Room | rpc::JourneyPhase::Opening | rpc::JourneyPhase::Dialogue => true,
        rpc::JourneyPhase::Complete => {
            journey.combat.is_none()
                || scene.destination == rpc::HarborDestination::HarborInn as i32
        }
        _ => false,
    }
}

/// Hands the view to the existing rendering: retires anything mounted here and clears it.
pub(super) fn release(client: &Rc<RefCell<Client>>, view: &Element) -> Result<(), JsValue> {
    let mounted = !client.borrow().live.is_empty();
    if !mounted {
        return Ok(());
    }
    let disposed = client.borrow_mut().live.dispose();
    view.set_text_content(None);
    disposed
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The join address the server offered for this room: its player page at the server's public
/// base address (`join_url`), which the path (`join_path`) must end.
fn join_address(journey: &rpc::JourneyView) -> Result<&str, JsValue> {
    let path_ok = journey.join_path.starts_with("/gameplay/player?room=");
    let url_ok =
        journey.join_url.starts_with("http") && journey.join_url.ends_with(&journey.join_path);
    if !path_ok || !url_ok {
        return Err(JsValue::from_str("join route invalid"));
    }
    Ok(&journey.join_url)
}

/// The join address as shown: the base address and path without the scheme.
// PLAY-Q01: the address now comes from the server, not from the page this browser loaded.
fn join_label(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .to_owned()
}

/// The host a player types the room code at, from the join address.
// PLAY-Q01
fn join_host(url: &str) -> String {
    join_label(url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn fallback_art(
    phase: rpc::JourneyPhase,
    scene: &rpc::GameplayScene,
) -> (&'static str, &'static str) {
    if phase == rpc::JourneyPhase::Room {
        return ("scene-lobby-harbor", HARBOR_ESTABLISHING);
    }
    if scene.destination == rpc::HarborDestination::HarborInn as i32 {
        return ("scene-harbor-inn", HARBOR_INN);
    }
    match phase {
        rpc::JourneyPhase::Opening => ("scene-wharf-arrival", HARBOR_WHARF),
        rpc::JourneyPhase::Dialogue => ("scene-whisper-in-the-rain", HARBOR_WHARF),
        _ => ("scene-courier-road", HARBOR_WHARF),
    }
}

/// Public asset paths are presentation data; only local asset URLs enter CSS/image sources.
pub(super) fn asset_url(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    (path.starts_with("assets/")
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_')))
    .then(|| format!("/{path}"))
}
pub(super) struct ServerImage {
    image: Element,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}
impl ServerImage {
    pub(super) fn install(
        image: Element,
        source: &str,
        fallback: Option<String>,
    ) -> Result<Self, JsValue> {
        let target = image.clone();
        let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if let Some(fallback) = &fallback {
                if target.get_attribute("src").as_ref() != Some(fallback) {
                    let _ = target.set_attribute("src", fallback);
                }
            } else {
                let _ = target.set_attribute("hidden", "");
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        image.add_event_listener_with_callback("error", callback.as_ref().unchecked_ref())?;
        image.set_attribute("src", source)?;
        image.remove_attribute("hidden")?;
        Ok(Self { image, callback })
    }
}
impl Drop for ServerImage {
    fn drop(&mut self) {
        let _ = self
            .image
            .remove_event_listener_with_callback("error", self.callback.as_ref().unchecked_ref());
    }
}

fn art(phase: rpc::JourneyPhase, scene: &rpc::GameplayScene) -> (String, &'static str) {
    let (stem, fallback) = fallback_art(phase, scene);
    let painting =
        asset_url(&scene.scene_asset).unwrap_or_else(|| format!("/assets/ui/generated/{stem}"));
    // PLAY-L01: a server-generated re-render of the painting leads when the view carries one; the
    // authored painting stays beneath it in the CSS layers should the image fail to load.
    let painting = match crate::presentation::imagery_url(&scene.live_backdrop) {
        Some(live) => format!("{live}),url({painting}"),
        None => painting,
    };
    (painting, fallback)
}

fn lobby_art_source(scene_asset: &str, live_backdrop: &str) -> Option<String> {
    crate::presentation::imagery_url(live_backdrop).or_else(|| asset_url(scene_asset))
}

fn validated_join_scene_asset(scene_asset: Option<&str>) -> Option<String> {
    scene_asset.and_then(asset_url)
}

fn speaker_portrait_stem(name: &str) -> Option<&'static str> {
    match name {
        "Courier" => Some("portrait-courier"),
        "Bandit" => Some("portrait-bandit"),
        _ => None,
    }
}

fn tv_model(input: &SceneInput<'_>) -> Result<TvModel, JsValue> {
    let lobby = input.phase == rpc::JourneyPhase::Room;
    let join_url = join_address(input.journey)?;
    let (art, art_fallback) = art(input.phase, input.scene);
    let speaker = (!input.journey.dialogue.is_empty()).then(|| TvSpeaker {
        name: if input.journey.speaker.is_empty() {
            "Narrator".to_owned()
        } else {
            input.journey.speaker.clone()
        },
        line: input.journey.dialogue.clone(),
        portrait: asset_url(&input.scene.speaker_portrait).or_else(|| {
            speaker_portrait_stem(&input.journey.speaker)
                .map(|stem| format!("/assets/ui/generated/{stem}"))
        }),
    });
    Ok(TvModel {
        mode: if lobby { TvMode::Lobby } else { TvMode::Scene },
        title: input.scene.title.clone(),
        lede: crate::presentation::first_sentence(input.narration),
        speaker,
        room_code: input.journey.room_code.clone(),
        join_host: join_host(join_url),
        party: input
            .journey
            .party
            .iter()
            .map(|member| TvMember {
                name: member.name.clone(),
                status: match (member.character_ready, lobby) {
                    (false, _) => "Creating a hero…".to_owned(),
                    (true, true) => "Ready".to_owned(),
                    (true, false) => {
                        format!("HP {}/{}", member.hit_points, member.maximum_hit_points)
                    }
                },
                ready: member.character_ready,
            })
            .collect(),
        art,
        art_fallback,
        cast: Vec::new(),
        place: input.scene.place.clone(),
        rules_note: input.scene.rules_note.clone(),
        choices: input.journey.table_choices.clone(),
        choice_hints_hidden: input.journey.choice_hints_hidden,
        family: crate::presentation::scene_family(&input.scene.scene_asset),
        scene_key: input.scene.title.clone(),
        home: input.journey.home.clone(),
        home_video: (!input.journey.home_video_path.is_empty())
            .then(|| input.journey.home_video_path.clone()),
        // PLAY-V09: the scene as one composed picture, when the server has made it.
        plate: crate::presentation::plate_of(input.scene),
        adventure_name: input.journey.adventure_name.clone(),
        narrated: !input.narration.trim().is_empty(),
        end: input.journey.adventure_ended.then(|| {
            crate::presentation::end_card(
                input.journey,
                &input.journey.adventure_title,
                input.narration,
            )
        }),
    })
}

fn party_members(journey: &rpc::JourneyView) -> (Vec<LobbyMember>, Vec<ExplorationMember>) {
    let mut lobby = Vec::new();
    let mut exploration = Vec::new();
    for (index, member) in journey.party.iter().enumerate() {
        let key = if member.participant_id.is_empty() {
            format!("seat-{index}")
        } else {
            hex(&member.participant_id)
        };
        let initial: String = member
            .name
            .chars()
            .next()
            .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect());
        lobby.push(LobbyMember {
            key: key.clone(),
            name: member.name.clone(),
            readiness: if member.character_ready {
                "Ready"
            } else {
                "Creating"
            },
        });
        exploration.push(ExplorationMember {
            key,
            name: member.name.clone(),
            role: if member.character_ready {
                format!("HP {}/{}", member.hit_points, member.maximum_hit_points)
            } else {
                "Creating a hero".to_owned()
            },
            sigil: initial,
        });
    }
    (lobby, exploration)
}

/// The generated icon stem shown beside a choice; presentation only.
fn choice_icon(action_kind: i32) -> &'static str {
    match rpc::GameplayActionKind::try_from(action_kind) {
        Ok(rpc::GameplayActionKind::AskCourier | rpc::GameplayActionKind::ExamineHarborSeal) => {
            "icon-investigate"
        }
        Ok(rpc::GameplayActionKind::EscortCourier | rpc::GameplayActionKind::DefendCourier) => {
            "icon-shield"
        }
        Ok(rpc::GameplayActionKind::BeginStory | rpc::GameplayActionKind::ChooseHarborScene) => {
            "icon-journey"
        }
        Ok(rpc::GameplayActionKind::ShortRest) => "icon-short-rest",
        _ => "icon-talk",
    }
}

/// Offers that need no extra typed input become scene choices; the rest keep their
/// existing controls in the dock. A travel with one destination needs no drop-down.
fn is_simple_offer(offer: &rpc::GameplayActionOffer) -> bool {
    offer.input_groups.is_empty()
        && offer.destinations.len() <= 1
        && !matches!(
            rpc::GameplayActionKind::try_from(offer.action_kind),
            Ok(rpc::GameplayActionKind::CreateCharacter
                | rpc::GameplayActionKind::GreatswordAttack)
        )
}

fn lobby_model(input: &SceneInput<'_>, client: &Client) -> Result<LobbyModel, JsValue> {
    let (members, _) = party_members(input.journey);
    let all_ready = !members.is_empty()
        && members.iter().all(|member| member.readiness == "Ready")
        && members.len() == 2;
    let begin = input
        .offers
        .iter()
        .find(|offer| offer.action_kind == rpc::GameplayActionKind::BeginStory as i32);
    let join_url = join_address(input.journey)?;
    Ok(LobbyModel {
        stage: JoinStage::Lobby,
        title: input.scene.title.clone(),
        narration: input.narration.to_owned(),
        scene_asset: lobby_art_source(&input.scene.scene_asset, &input.scene.live_backdrop),
        code: input.journey.room_code.clone(),
        join_url: join_label(join_url),
        join_label: "Join this room",
        panel_heading: "Your party gathers",
        panel_description: if all_ready {
            "Your party is ready. Step into the story together."
        } else {
            "Waiting for your companions to finish their heroes."
        },
        members,
        offer: begin.map(|offer| LobbyOfferModel {
            key: offer.offer_id.clone(),
            label: offer.label.clone(),
            enabled: !client.busy && (client.last_confirmed || client.last_request.is_none()),
            pending: client.busy,
        }),
    })
}

fn exploration_model(
    input: &SceneInput<'_>,
    client: &Client,
    show_clue: bool,
) -> Result<ExplorationModel, JsValue> {
    let binding = client
        .views
        .as_ref()
        .map(|views| views.scope().cache_scope().binding)
        .ok_or_else(|| JsValue::from_str("scene binding absent"))?;
    let (_, members) = party_members(input.journey);
    let unconfirmed = !client.busy && !client.last_confirmed && client.last_request.is_some();
    let ended = input.journey.adventure_ended;
    let choices: Vec<ExplorationChoiceModel> = if ended {
        Vec::new()
    } else {
        input
            .offers
            .iter()
            .filter(|offer| is_simple_offer(offer))
            .map(|offer| ExplorationChoiceModel {
                id: offer.offer_id.clone(),
                label: offer.label.clone(),
                icon: choice_icon(offer.action_kind),
                disabled_reason: choice_disabled_reason(client.busy, unconfirmed),
            })
            .collect()
    };
    let has_offers = !input.offers.is_empty();
    let whisper = if !show_clue {
        if has_offers {
            CHOOSE_TEXT
        } else {
            WAITING_TEXT
        }
        .to_owned()
    } else {
        input.clue.to_owned()
    };
    let art_scene = match fallback_art(input.phase, input.scene).0 {
        "scene-harbor-inn" => ConceptScene::HarborInn,
        "scene-wharf-arrival" => ConceptScene::WharfArrival,
        "scene-whisper-in-the-rain" => ConceptScene::WhisperInTheRain,
        _ => ConceptScene::CourierRoad,
    };
    let scene_speech = crate::presentation::newest_scene_speech(input.journey)
        .map(|(sequence, name, line)| (Some(sequence), name, line))
        .or_else(|| {
            (!input.journey.dialogue.trim().is_empty()).then_some((
                None,
                input.journey.speaker.as_str(),
                input.journey.dialogue.as_str(),
            ))
        });
    let speaker =
        if ended { None } else { scene_speech }.map(|(_, name, line)| ExplorationSpeaker {
            name: if name.is_empty() {
                "Narrator".to_owned()
            } else {
                name.to_owned()
            },
            context: input.scene.title.clone(),
            line: line.to_owned(),
            portrait: match name {
                "Courier" => Some(ExplorationPortrait::Courier),
                "Bandit" => Some(ExplorationPortrait::Bandit),
                "Vell" => Some(ExplorationPortrait::Vell),
                _ => None,
            },
        });
    // PLAY-V09: a ready scene plate is the scene picture at the top of the player page, cropped to
    // keep the speaker in view; the authored painting stays the fallback behind it.
    let plate = crate::presentation::plate_of(input.scene);
    Ok(ExplorationModel {
        plate_focus: plate.layout.map(|layout| layout.head_x),
        scene_asset: plate.url.or_else(|| asset_url(&input.scene.scene_asset)),
        portrait_asset: asset_url(&input.scene.speaker_portrait),
        has_whisper: show_clue,
        rules_note: input.scene.rules_note.clone(),
        binding,
        scene: art_scene,
        title: if ended {
            if input.journey.adventure_title.is_empty() {
                input.scene.title.clone()
            } else {
                input.journey.adventure_title.clone()
            }
        } else {
            input.scene.title.clone()
        },
        description: if ended {
            "The adventure is over. Thank you for playing.".to_owned()
        } else {
            input.narration.to_owned()
        },
        whisper,
        location: format!("Room {}", input.journey.room_code),
        scene_label: match input.phase {
            rpc::JourneyPhase::Opening => "Opening scene",
            rpc::JourneyPhase::Dialogue => "Conversation",
            _ => "The story continues",
        },
        members,
        speaker,
        heading: if ended {
            "The End"
        } else if choices.is_empty() {
            "The story waits"
        } else {
            "What do you do?"
        },
        choices,
        pending: client.busy.then_some("Waiting for the game server…"),
    })
}

fn choice_disabled_reason(busy: bool, unconfirmed: bool) -> Option<&'static str> {
    if busy {
        Some("Waiting for the game server…")
    } else if unconfirmed {
        Some("Confirm your last choice first.")
    } else {
        None
    }
}

/// A choice or lobby action chosen on a screen: the server's own current offer is sent
/// through the existing prepare/send path, or the choice is reported as ended.
pub(super) fn submit_offer(client: &Rc<RefCell<Client>>, offer_id: &str) {
    let offer = client
        .borrow()
        .views
        .as_ref()
        .and_then(|views| views.current())
        .and_then(|view| match &view.audience {
            Some(rpc::view_message::Audience::Player(player)) => player
                .offers
                .iter()
                .find(|offer| offer.offer_id == offer_id)
                .cloned(),
            _ => None,
        });
    let Some(offer) = offer else {
        feedback(
            &client.borrow(),
            "That offer has ended. Choose from the current view.",
        );
        return;
    };
    // A single destination is the only one the server offered, so it is sent as chosen.
    let destination = match offer.destinations.as_slice() {
        [only] => only.destination,
        _ => rpc::HarborDestination::Unspecified as i32,
    };
    match prepare(client, &offer, None, false, false, destination) {
        Ok(body) => send(client.clone(), body, false),
        Err(error) => feedback(&client.borrow(), &error),
    }
}

/// The player's area under the surface. It persists across renders so a draft and the open
/// "say something else" row survive; only the offer buttons are rebuilt.
fn ensure_dock(document: &Document, view: &Element) -> Result<Element, JsValue> {
    let dock = match view.query_selector(".live-dock")? {
        Some(dock) => dock,
        None => node(document, view, "div", "live-dock", "")?,
    };
    if dock.next_element_sibling().is_some() {
        view.append_child(&dock)?;
    }
    Ok(dock)
}

/// The typed-dialogue panel is collapsed behind one row until the player asks for it.
fn ensure_say_more(document: &Document, dock: &Element) -> Result<Element, JsValue> {
    if let Some(existing) = dock.query_selector("details.say-more")? {
        return Ok(existing);
    }
    let details = node(document, dock, "details", "say-more", "")?;
    let summary = node(document, &details, "summary", "", "")?;
    let icon = node(document, &summary, "img", "say-more-icon", "")?;
    icon.set_attribute("src", &format!("{}/icon-talk", live_tv::GENERATED))?;
    icon.set_attribute("alt", "")?;
    node(document, &summary, "span", "", "Say something else…")?;
    Ok(details)
}

/// Recovery controls: the toast when something needs attention, the menu items for the rest.
fn recovery(client: &Rc<RefCell<Client>>, dock: &Element) -> Result<(), JsValue> {
    recovery_prompt(client, dock)?;
    recovery_details(client, dock)
}

fn prepare_mount(client: &Rc<RefCell<Client>>, view: &Element) -> Result<(), JsValue> {
    if !client.borrow().live.is_empty() {
        return Ok(());
    }
    if let Some(scene) = &client.borrow().scene_delivery {
        scene
            .retire()
            .map_err(|_| JsValue::from_str("retired scene cleanup failed"))?;
    }
    view.set_text_content(None);
    Ok(())
}

/// Presents one accepted view of a phase [`takes_over`] returned true for.
pub(super) fn render(
    client: &Rc<RefCell<Client>>,
    view: &Element,
    input: &SceneInput<'_>,
) -> Result<(), JsValue> {
    prepare_mount(client, view)?;
    let document = client.borrow().document.clone();
    if client.borrow().role == "display" {
        return render_display(client, view, input);
    }
    let lobby = input.phase == rpc::JourneyPhase::Room;
    let show_clue = {
        let mut state = client.borrow_mut();
        if state.role == "player" {
            let show_clue =
                is_new_private_clue(state.live.last_private_clue.as_deref(), input.clue);
            state.live.last_private_clue = Some(input.clue.to_owned());
            show_clue
        } else {
            false
        }
    };
    // The other screen retires first so exactly one surface is ever mounted.
    let (retired_lobby, retired_exploration) = {
        let mut state = client.borrow_mut();
        (
            if lobby { None } else { state.live.lobby.take() },
            if lobby {
                state.live.exploration.take()
            } else {
                None
            },
        )
    };
    if let Some(retired) = retired_lobby {
        retired.dispose()?;
    }
    if let Some(retired) = retired_exploration {
        retired.dispose()?;
    }
    if lobby {
        let model = lobby_model(input, &client.borrow())?;
        LobbyScreen::present(client, view, &model)?;
    } else {
        let model = exploration_model(input, &client.borrow(), show_clue)?;
        ExplorationScreen::present(client, view, &model)?;
    }
    let dock = ensure_dock(&document, view)?;
    if lobby {
        dock.set_text_content(None);
    } else {
        let say = ensure_say_more(&document, &dock)?;
        dialogue_recovery::mount(client, &say, input.offers)?;
        if let Some(previous) = dock.query_selector(".scene-actions")? {
            previous.remove();
        }
        if let Some(own) = input.journey.own_character.as_ref() {
            let token = live_combat::sheet::portrait_token(&input.journey.party, own);
            live_combat::sheet::mount_inline(client, &dock, own, token)?;
        }
        let actions = node(&document, &dock, "div", "scene-actions", "")?;
        for offer in input.offers.iter().filter(|offer| !is_simple_offer(offer)) {
            action_button(client, &actions, offer.clone())?;
        }
    }
    recovery(client, &dock)
}

/// The cast of the view being drawn, from the one function that derives it.
pub(super) fn current_cast(client: &Rc<RefCell<Client>>) -> Vec<crate::presentation::StageActor> {
    client
        .borrow()
        .views
        .as_ref()
        .and_then(|views| views.current())
        .map(crate::presentation::scene_cast)
        .unwrap_or_default()
}

fn render_display(
    client: &Rc<RefCell<Client>>,
    view: &Element,
    input: &SceneInput<'_>,
) -> Result<(), JsValue> {
    let mut model = tv_model(input)?;
    model.cast = current_cast(client);
    if client.borrow().live.tv.is_none() {
        let document = client.borrow().document.clone();
        let stage = TvStage::mount(&document, view)?;
        client.borrow_mut().live.tv = Some(stage);
    }
    let dock = {
        let state = client.borrow();
        let Some(stage) = state.live.tv.as_ref() else {
            return Err(JsValue::from_str("display stage absent"));
        };
        stage.update(&model)?;
        stage.dock().clone()
    };
    recovery(client, &dock)
}

#[cfg(test)]
mod tests {
    use super::{
        choice_disabled_reason, is_new_private_clue, lobby_art_source, validated_join_scene_asset,
    };

    #[test]
    fn lobby_art_uses_the_server_scene_asset_when_it_is_a_local_asset_path() {
        let live_backdrop = format!("/gameplay/imagery/{}", "a".repeat(64));
        assert_eq!(
            lobby_art_source("assets/ui/generated/scene-place-guardhouse", ""),
            Some("/assets/ui/generated/scene-place-guardhouse".to_owned())
        );
        assert_eq!(
            lobby_art_source("assets/ui/generated/scene-place-guardhouse", &live_backdrop),
            Some(live_backdrop)
        );
        assert_eq!(
            lobby_art_source(
                "https://example.com/scene.webp",
                "https://example.com/live.webp"
            ),
            None
        );
    }

    #[test]
    fn join_art_uses_only_a_well_formed_local_scene_asset() {
        assert_eq!(
            validated_join_scene_asset(Some("assets/ui/generated/scene-place-guardhouse")),
            Some("/assets/ui/generated/scene-place-guardhouse".to_owned())
        );
        assert_eq!(validated_join_scene_asset(None), None);
        assert_eq!(validated_join_scene_asset(Some("")), None);
        assert_eq!(
            validated_join_scene_asset(Some("https://example.com/scene.webp")),
            None
        );
        assert_eq!(validated_join_scene_asset(Some("assets/scene?bad")), None);
    }

    #[test]
    fn retained_private_clue_is_not_a_new_reveal() {
        assert!(!is_new_private_clue(None, "saved clue"));
        assert!(is_new_private_clue(Some(""), "new clue"));
        assert!(!is_new_private_clue(Some("saved clue"), "saved clue"));
        assert!(!is_new_private_clue(Some(""), ""));
    }

    #[test]
    fn pending_request_disables_each_offered_choice() {
        assert_eq!(
            choice_disabled_reason(true, false),
            Some("Waiting for the game server…")
        );
        assert_eq!(
            choice_disabled_reason(false, true),
            Some("Confirm your last choice first.")
        );
        assert_eq!(choice_disabled_reason(false, false), None);
    }
}

/// The join screen before a player has a seat: the same lobby surface in its invitation stage.
pub(super) fn render_join(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, view, code, join_scene_asset, has_request, retries) = {
        let mut state = client.borrow_mut();
        clear_callbacks(&mut state);
        (
            state.document.clone(),
            state
                .root
                .query_selector(".view")?
                .ok_or_else(|| JsValue::from_str("view missing"))?,
            state.room_code.clone(),
            state.root.get_attribute("data-join-scene-asset"),
            state.join_request.is_some(),
            state.join_retries,
        )
    };
    let retired = {
        let mut state = client.borrow_mut();
        let exploration = state.live.exploration.take();
        let tv = state.live.tv.take();
        (exploration, tv)
    };
    if let Some(exploration) = retired.0 {
        exploration.dispose()?;
    }
    if let Some(tv) = retired.1 {
        tv.dispose()?;
    }
    prepare_mount(client, &view)?;
    let model = LobbyModel {
        stage: JoinStage::Invitation,
        title: "Join the table".to_owned(),
        narration: "Connect to the shared room and create your own hero.".to_owned(),
        scene_asset: validated_join_scene_asset(join_scene_asset.as_deref()),
        code,
        join_url: String::new(),
        join_label: if retries >= crate::reconnect::JOIN_RETRIES {
            "Try again"
        } else if has_request {
            "Confirm the same join"
        } else {
            "Join this room"
        },
        panel_heading: "Join your companions",
        panel_description: "Enter the room code from the TV.",
        members: Vec::new(),
        offer: None,
    };
    LobbyScreen::present(client, &view, &model)?;
    let dock = ensure_dock(&document, &view)?;
    dock.set_text_content(None);
    recovery_details(client, &dock)
}
