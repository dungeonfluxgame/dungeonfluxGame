//! The player's join and lobby screen: `df_ui::JoinPhaseSurface` mounted once and updated
//! in place from the accepted room view. Join and lobby intents reuse the existing join and
//! action paths; the surface has no network, rules or authority.
use super::*;
use df_ui::{
    CampaignLimits, CampaignView, ConceptScene, JoinFeedback, JoinPhaseIntent, JoinPhaseSurface,
    JoinPhaseView, JoinRoom, JoinStage, JoinStamp, JoinUpdate, LobbyOffer, LobbyParticipant,
};
use std::cell::Cell;

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
    pub(super) code: String,
    pub(super) join_url: String,
    pub(super) join_label: &'static str,
    pub(super) panel_heading: &'static str,
    pub(super) panel_description: &'static str,
    pub(super) members: Vec<LobbyMember>,
    pub(super) offer: Option<LobbyOfferModel>,
}

pub(super) struct LobbyScreen {
    surface: JoinPhaseSurface,
    stamp: Cell<u64>,
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
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
    let view = JoinPhaseView {
        stamp: JoinStamp {
            generation: STAMP_GENERATION,
            sequence,
        },
        stage: model.stage,
        campaign: CampaignView {
            scene: ConceptScene::Harbor,
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
            detail: &model.join_url,
        }),
        roster_heading: "Your party",
        empty_roster: "No heroes have joined yet.",
        participants: &participants,
        lobby_offer: model.offer.as_ref().map(|offer| LobbyOffer {
            key: &offer.key,
            label: &offer.label,
            enabled: offer.enabled,
            pending: offer.pending,
        }),
        pairing_fallback: "Share the room code, or open the join link on a second phone.",
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

/// The room code is fixed by the page and the player's name is chosen during character
/// creation, so the join form shows the code read-only and offers no name field.
fn fix_fields(surface: &JoinPhaseSurface) -> Result<(), JsValue> {
    surface.player_name().root().set_attribute("hidden", "")?;
    surface.invitation().input().set_read_only(true);
    Ok(())
}

impl LobbyScreen {
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
                    JoinPhaseIntent::Join { .. } => join_room(client),
                    JoinPhaseIntent::LobbyAction { offer_key, .. } => {
                        live_screens::submit_offer(&client, &offer_key);
                    }
                }
            })
            .map_err(|error| render_error(client, &error))?;
        fix_fields(&surface)?;
        parent.append_child(surface.root())?;
        client.borrow_mut().live.lobby = Some(LobbyScreen {
            surface,
            stamp: Cell::new(1),
        });
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.surface
            .dispose()
            .map_err(|error| JsValue::from_str(&format!("lobby retirement failed: {error}")))
    }
}
