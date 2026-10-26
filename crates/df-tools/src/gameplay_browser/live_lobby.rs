//! The player's join and lobby screen: `df_ui::JoinPhaseSurface` mounted once and updated
//! in place from the accepted room view. Join and lobby intents reuse the existing join and
//! action paths; the surface has no network, rules or authority.
use super::*;
use df_ui::{
    CampaignLimits, CampaignView, ConceptScene, JoinFeedback, JoinPhaseIntent, JoinPhaseSurface,
    JoinPhaseView, JoinRoom, JoinStage, JoinStamp, JoinUpdate, LobbyOffer, LobbyParticipant,
};
use std::cell::Cell;
use std::net::{IpAddr, SocketAddr};
use wasm_bindgen::closure::Closure;

const LIMITS: CampaignLimits = CampaignLimits {
    max_members: 8,
    max_objectives: 4,
    max_text_bytes: 2048,
};
const STAMP_GENERATION: u64 = 1;
const SHARE_INVITE_LABEL: &str = "Share invite";
const COPIED_LABEL: &str = "Copied";

fn watch_promise(
    promise: &js_sys::Promise,
    success: &js_sys::Function,
    failure: &js_sys::Function,
) -> bool {
    js_sys::Reflect::get(promise, &JsValue::from_str("then"))
        .ok()
        .and_then(|then| then.dyn_into::<js_sys::Function>().ok())
        .is_some_and(|then| then.call2(promise, success, failure).is_ok())
}

pub(super) struct LobbyMember {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) readiness: &'static str,
}

pub(super) struct LobbyOfferModel {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) enabled: bool,
    pub(super) pending: bool,
}

/// The wire-derived text of one lobby presentation, owned so the borrowed df-ui view can be
/// built at the call.
pub(super) struct LobbyModel {
    pub(super) stage: JoinStage,
    pub(super) title: String,
    pub(super) narration: String,
    pub(super) scene_asset: Option<String>,
    pub(super) code: String,
    pub(super) join_url: String,
    pub(super) join_label: &'static str,
    pub(super) panel_heading: &'static str,
    pub(super) panel_description: &'static str,
    pub(super) members: Vec<LobbyMember>,
    pub(super) away_heroes: Vec<rpc::AwayHero>,
    pub(super) offer: Option<LobbyOfferModel>,
}

pub(super) struct LobbyScreen {
    surface: JoinPhaseSurface,
    stamp: Cell<u64>,
    scene_asset: RefCell<Option<live_screens::ServerImage>>,
    share_invite: Option<ShareInvite>,
}

struct ShareInvite {
    _root: Element,
    button: Element,
    address: Element,
    status: Element,
    url: Rc<RefCell<String>>,
    _listener: Closure<dyn FnMut(web_sys::Event)>,
    _shared: Closure<dyn FnMut(JsValue)>,
    _copied: Closure<dyn FnMut(JsValue)>,
    _failed: Closure<dyn FnMut(JsValue)>,
}

impl ShareInvite {
    fn mount(document: &Document, parent: &Element, url: &str) -> Result<Self, JsValue> {
        let root = document.create_element("div")?;
        root.set_class_name("live-lobby-share");
        let button = node(document, &root, "button", "", SHARE_INVITE_LABEL)?;
        button.set_attribute("type", "button")?;
        let address = node(document, &root, "p", "live-lobby-share-address", url)?;
        let status = node(document, &root, "span", "live-lobby-share-status", "")?;
        status.set_attribute("role", "status")?;
        let url = Rc::new(RefCell::new(url.to_owned()));
        let shared_button = button.clone();
        let shared_status = status.clone();
        let shared = Closure::wrap(Box::new(move |_: JsValue| {
            shared_button.set_text_content(Some(SHARE_INVITE_LABEL));
            shared_status.set_text_content(Some("Invite shared."));
        }) as Box<dyn FnMut(JsValue)>);
        let copied_button = button.clone();
        let copied_status = status.clone();
        let copied = Closure::wrap(Box::new(move |_: JsValue| {
            copied_button.set_text_content(Some(COPIED_LABEL));
            copied_status.set_text_content(Some(COPIED_LABEL));
        }) as Box<dyn FnMut(JsValue)>);
        let failed_button = button.clone();
        let failed_status = status.clone();
        let failed = Closure::wrap(Box::new(move |_: JsValue| {
            failed_button.set_text_content(Some(SHARE_INVITE_LABEL));
            failed_status.set_text_content(Some("Invite not shared."));
        }) as Box<dyn FnMut(JsValue)>);
        let active_url = url.clone();
        let active_button = button.clone();
        let active_status = status.clone();
        let active_document = document.clone();
        let shared_callback = shared.as_ref().unchecked_ref::<js_sys::Function>().clone();
        let copied_callback = copied.as_ref().unchecked_ref::<js_sys::Function>().clone();
        let failed_callback = failed.as_ref().unchecked_ref::<js_sys::Function>().clone();
        let listener = Closure::wrap(Box::new(move |_: web_sys::Event| {
            let Some(window) = web_sys::window() else {
                active_status.set_text_content(Some("Could not share this invite."));
                return;
            };
            let navigator = window.navigator();
            let href = active_url.borrow().clone();
            let share = js_sys::Reflect::get(&navigator, &JsValue::from_str("share"));
            if let Ok(share) = share
                && let Some(share) = share.dyn_ref::<js_sys::Function>()
            {
                let data = js_sys::Object::new();
                let _ = js_sys::Reflect::set(
                    &data,
                    &JsValue::from_str("title"),
                    &JsValue::from_str("Join our game"),
                );
                let _ = js_sys::Reflect::set(
                    &data,
                    &JsValue::from_str("url"),
                    &JsValue::from_str(&href),
                );
                if let Ok(result) = share.call1(&navigator, &data)
                    && let Ok(promise) = result.dyn_into::<js_sys::Promise>()
                {
                    if !watch_promise(&promise, &shared_callback, &failed_callback) {
                        active_status.set_text_content(Some("Could not share this invite."));
                    }
                    return;
                }
            }
            let clipboard = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard"));
            let write = clipboard.ok().and_then(|clipboard| {
                js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText"))
                    .ok()
                    .map(|write| (clipboard, write))
            });
            if let Some((clipboard, write)) = write
                && let Some(write) = write.dyn_ref::<js_sys::Function>()
                && let Ok(result) = write.call1(&clipboard, &JsValue::from_str(&href))
                && let Ok(promise) = result.dyn_into::<js_sys::Promise>()
            {
                active_button.set_text_content(Some("Copying…"));
                if !watch_promise(&promise, &copied_callback, &failed_callback) {
                    active_status.set_text_content(Some("Could not copy this invite."));
                }
            } else if copy_with_selection(&active_document, &href) {
                active_button.set_text_content(Some(COPIED_LABEL));
                active_status.set_text_content(Some(COPIED_LABEL));
            } else {
                active_status.set_text_content(Some("Copy is unavailable in this browser."));
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        button.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref())?;
        parent.append_child(&root)?;
        Ok(Self {
            _root: root,
            button,
            address,
            status,
            url,
            _listener: listener,
            _shared: shared,
            _copied: copied,
            _failed: failed,
        })
    }

    fn update(&self, url: &str) {
        if self.url.borrow().as_str() == url {
            return;
        }
        *self.url.borrow_mut() = url.to_owned();
        self.address.set_text_content(Some(url));
        self.status.set_text_content(None);
        self.button.set_text_content(Some(SHARE_INVITE_LABEL));
    }
}

fn copy_with_selection(document: &Document, text: &str) -> bool {
    let Ok(textarea) = document.create_element("textarea") else {
        return false;
    };
    if textarea.set_attribute("readonly", "").is_err()
        || textarea
            .set_attribute("style", "position:fixed;left:-10000px;top:0")
            .is_err()
    {
        return false;
    }
    textarea.set_text_content(Some(text));
    let Some(body) = document.body() else {
        return false;
    };
    if body.append_child(&textarea).is_err() {
        return false;
    }
    let copied = js_sys::Reflect::get(&textarea, &JsValue::from_str("select"))
        .ok()
        .and_then(|select| select.dyn_into::<js_sys::Function>().ok())
        .is_some_and(|select| select.call0(&textarea).is_ok())
        && js_sys::Reflect::get(document, &JsValue::from_str("execCommand"))
            .ok()
            .and_then(|command| command.dyn_into::<js_sys::Function>().ok())
            .is_some_and(|command| {
                command
                    .call1(document, &JsValue::from_str("copy"))
                    .is_ok_and(|result| result.as_bool() == Some(true))
            });
    textarea.remove();
    copied
}

fn initial(member_name: &str) -> String {
    member_name
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

fn is_loopback_join_url(join_url: &str) -> bool {
    // The phone is handed the address as shown, without its scheme.
    let address = join_url
        .split_once("://")
        .map_or(join_url, |(_, address)| address);
    let host = address.split('/').next().unwrap_or_default();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host
        .parse::<SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback())
    {
        return true;
    }
    if host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return true;
    }
    host.split_once(':')
        .is_some_and(|(hostname, _)| hostname.eq_ignore_ascii_case("localhost"))
}

fn with_view<R>(
    model: &LobbyModel,
    sequence: u64,
    present: impl FnOnce(&JoinPhaseView<'_>) -> R,
) -> R {
    let location = format!("Room {}", model.code);
    let party_identity = format!("Your party · {location}");
    let sigils: Vec<String> = model
        .members
        .iter()
        .map(|member| initial(&member.name))
        .collect();
    let participants: Vec<LobbyParticipant<'_>> = model
        .members
        .iter()
        .zip(&sigils)
        .map(|(member, sigil)| LobbyParticipant {
            key: &member.key,
            name: &member.name,
            sigil,
            readiness: member.readiness,
            presence: "In the room",
        })
        .collect();
    let away_names: Vec<_> = model
        .away_heroes
        .iter()
        .map(|hero| format!("{} (away)", hero.name))
        .collect();
    let away_keys: Vec<_> = model
        .away_heroes
        .iter()
        .map(|hero| {
            hero.seat_token
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .collect();
    let away_heroes: Vec<_> = away_names
        .iter()
        .zip(&away_keys)
        .map(|(name, key)| df_ui::AwayHero {
            key,
            name,
            reclaim_label: "Take this hero back",
        })
        .collect();
    let loopback = is_loopback_join_url(&model.join_url);
    let view = JoinPhaseView {
        stamp: JoinStamp {
            generation: STAMP_GENERATION,
            sequence,
        },
        stage: model.stage,
        campaign: CampaignView {
            scene: ConceptScene::Campfire,
            chapter: "An adventure awaits",
            title: &model.title,
            description: &model.narration,
            location: &location,
            scene_label: "Lantern Wharf",
            narration: "Create your own hero on your phone, then step into the story together.",
            connection: "Local browser room · players · the server owns the game",
            notice: &party_identity,
            members: &[],
            objectives: &[],
        },
        panel_heading: model.panel_heading,
        panel_description: model.panel_description,
        invitation_label: "Room code",
        name_label: "Player name",
        join_label: model.join_label,
        join_enabled: true,
        feedback: JoinFeedback::None,
        room: (model.stage == JoinStage::Lobby).then(|| JoinRoom {
            label: "Room code",
            code: &model.code,
            detail: "",
        }),
        roster_heading: "Your party",
        empty_roster: "No heroes have joined yet.",
        participants: &participants,
        away_heroes: &away_heroes,
        lobby_offer: model.offer.as_ref().map(|offer| LobbyOffer {
            key: &offer.key,
            label: &offer.label,
            enabled: offer.enabled,
            pending: offer.pending,
        }),
        pairing_fallback: if loopback {
            "Phones cannot join yet: the game was started for this computer only."
        } else {
            "Share the room code, or open the join link on a second phone."
        },
    };
    present(&view)
}

fn render_error(client: &Rc<RefCell<Client>>, error: &dyn std::fmt::Display) -> JsValue {
    let _ = client
        .borrow()
        .root
        .set_attribute("data-render-error", "lobby");
    JsValue::from_str(&format!("The lobby could not be displayed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{COPIED_LABEL, LobbyModel, SHARE_INVITE_LABEL, is_loopback_join_url, with_view};
    use df_ui::JoinStage;

    #[test]
    fn loopback_join_urls_are_not_presented_as_shareable_links() {
        for join_url in [
            "http://127.0.0.1:63419/gameplay/player?room=ABCD",
            "http://127.42.0.9:63419/gameplay/player?room=ABCD",
            "http://[::1]:63419/gameplay/player?room=ABCD",
            "http://localhost:63419/gameplay/player?room=ABCD",
            "127.0.0.1:63419/gameplay/player?room=ABCD",
            "localhost:63419/gameplay/player?room=ABCD",
        ] {
            assert!(is_loopback_join_url(join_url));
            let model = LobbyModel {
                stage: JoinStage::Lobby,
                title: String::new(),
                narration: String::new(),
                scene_asset: None,
                code: "ABCD".to_owned(),
                join_url: join_url.to_owned(),
                join_label: "Join this room",
                panel_heading: "Your party gathers",
                panel_description: "Waiting for heroes",
                members: Vec::new(),
                away_heroes: Vec::new(),
                offer: None,
            };
            let (detail, fallback) = with_view(&model, 1, |view| {
                (
                    view.room.as_ref().map(|room| room.detail.to_owned()),
                    view.pairing_fallback.to_owned(),
                )
            });
            assert_eq!(detail.as_deref(), Some(""));
            assert_eq!(
                fallback,
                "Phones cannot join yet: the game was started for this computer only."
            );
        }
    }

    #[test]
    fn network_join_url_is_shared_separately_from_the_room_code() {
        let model = LobbyModel {
            stage: JoinStage::Lobby,
            title: String::new(),
            narration: String::new(),
            scene_asset: None,
            code: "ABCD".to_owned(),
            join_url: "http://192.168.1.20:63419/gameplay/player?room=ABCD".to_owned(),
            join_label: "Join this room",
            panel_heading: "Your party gathers",
            panel_description: "Waiting for heroes",
            members: Vec::new(),
            away_heroes: Vec::new(),
            offer: None,
        };
        let (detail, fallback, notice) = with_view(&model, 1, |view| {
            (
                view.room.as_ref().map(|room| room.detail.to_owned()),
                view.pairing_fallback.to_owned(),
                view.campaign.notice.to_owned(),
            )
        });
        assert_eq!(detail.as_deref(), Some(""));
        assert_eq!(
            model.join_url,
            "http://192.168.1.20:63419/gameplay/player?room=ABCD"
        );
        assert_eq!(notice, "Your party · Room ABCD");
        assert_eq!(
            fallback,
            "Share the room code, or open the join link on a second phone."
        );
    }

    #[test]
    fn invite_control_uses_one_share_action_and_a_copied_confirmation() {
        assert_eq!(SHARE_INVITE_LABEL, "Share invite");
        assert_eq!(COPIED_LABEL, "Copied");
        let style = include_str!("live_player.css");
        assert!(style.contains(".live-lobby-share-address"));
        assert!(style.contains(".live-lobby-share-status"));
    }

    #[test]
    fn ready_lobby_action_stays_above_navigation_with_reserved_page_space() {
        let style = include_str!("live_player.css");
        assert!(style.contains("@media(max-width:700px)"));
        assert!(style.contains(".join-lobby>button{"));
        assert!(style.contains("position:fixed;"));
        assert!(style.contains("padding-bottom:calc(var(--lobby-action-height)"));
    }
}

/// The player's name is chosen during character creation, so the join form offers no name
/// field. The room code is filled in from the page or the `?room=` link and can be corrected
/// by typing the one shown on the display.
fn fix_fields(surface: &JoinPhaseSurface) -> Result<(), JsValue> {
    surface.player_name().root().set_attribute("hidden", "")?;
    surface.invitation().input().set_read_only(false);
    Ok(())
}

impl LobbyScreen {
    pub(super) fn invitation_value(&self) -> String {
        self.surface.invitation().input().value()
    }

    fn set_scene_asset(&self, scene_asset: Option<&str>) -> Result<(), JsValue> {
        self.scene_asset.borrow_mut().take();
        let Some(source) = scene_asset else {
            return Ok(());
        };
        let Some(image) = self.surface.root().query_selector(".scene-art")? else {
            return Ok(());
        };
        *self.scene_asset.borrow_mut() =
            Some(live_screens::ServerImage::install(image, source, None)?);
        Ok(())
    }

    /// Presents `model` in place, mounting the surface under `parent` on first use.
    pub(super) fn present(
        client: &Rc<RefCell<Client>>,
        parent: &Element,
        model: &LobbyModel,
    ) -> Result<(), JsValue> {
        let existing = client.borrow().live.lobby.is_some();
        if existing {
            let state = client.borrow();
            let Some(screen) = state.live.lobby.as_ref() else {
                return Ok(());
            };
            let sequence = screen.stamp.get() + 1;
            screen.stamp.set(sequence);
            let applied = with_view(model, sequence, |view| screen.surface.update(view))
                .map_err(|error| render_error(client, &error))?;
            if applied != JoinUpdate::Applied {
                return Err(render_error(client, &"stale lobby presentation"));
            }
            screen.set_scene_asset(model.scene_asset.as_deref())?;
            if let Some(share_invite) = &screen.share_invite {
                share_invite.update(&model.join_url);
            }
            return fix_fields(&screen.surface);
        }
        let document = client.borrow().document.clone();
        let surface = with_view(model, 1, |view| {
            JoinPhaseSurface::create(&document, "live-join", view, &model.code, "", LIMITS)
        })
        .map_err(|error| render_error(client, &error))?;
        let share_invite = if is_loopback_join_url(&model.join_url) {
            None
        } else {
            let panel = surface
                .root()
                .query_selector(".join-panel")?
                .ok_or_else(|| JsValue::from_str("lobby invitation panel is missing"))?;
            Some(ShareInvite::mount(&document, &panel, &model.join_url)?)
        };
        let weak = Rc::downgrade(client);
        surface
            .on_intent(move |intent| {
                let Some(client) = weak.upgrade() else {
                    return;
                };
                match intent {
                    JoinPhaseIntent::Join { invitation, .. } => {
                        if use_room_code(&client, &invitation) {
                            join_room(client);
                        }
                    }
                    JoinPhaseIntent::LobbyAction { offer_key, .. } => {
                        live_screens::submit_offer(&client, &offer_key);
                    }
                }
            })
            .map_err(|error| render_error(client, &error))?;
        fix_fields(&surface)?;
        let screen = LobbyScreen {
            surface,
            stamp: Cell::new(1),
            scene_asset: RefCell::new(None),
            share_invite,
        };
        screen.set_scene_asset(model.scene_asset.as_deref())?;
        parent.append_child(screen.surface.root())?;
        client.borrow_mut().live.lobby = Some(screen);
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.surface
            .dispose()
            .map_err(|error| JsValue::from_str(&format!("lobby retirement failed: {error}")))
    }
}
