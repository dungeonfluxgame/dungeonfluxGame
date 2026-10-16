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

const LIMITS: CampaignLimits = CampaignLimits {
    max_members: 8,
    max_objectives: 4,
    max_text_bytes: 2048,
};
const STAMP_GENERATION: u64 = 1;

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
}

fn initial(name: &str) -> String {
    name.chars()
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
            connection: "Local browser room · two players · the server owns the game",
            notice: "Players join here",
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
            detail: if loopback { "" } else { &model.join_url },
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
    use super::{LobbyModel, is_loopback_join_url, with_view};
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
    fn network_join_urls_remain_shareable() {
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
        let (detail, fallback) = with_view(&model, 1, |view| {
            (
                view.room.as_ref().map(|room| room.detail.to_owned()),
                view.pairing_fallback.to_owned(),
            )
        });
        assert_eq!(
            detail.as_deref(),
            Some("http://192.168.1.20:63419/gameplay/player?room=ABCD")
        );
        assert_eq!(
            fallback,
            "Share the room code, or open the join link on a second phone."
        );
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
            return fix_fields(&screen.surface);
        }
        let document = client.borrow().document.clone();
        let surface = with_view(model, 1, |view| {
            JoinPhaseSurface::create(&document, "live-join", view, &model.code, "", LIMITS)
        })
        .map_err(|error| render_error(client, &error))?;
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
