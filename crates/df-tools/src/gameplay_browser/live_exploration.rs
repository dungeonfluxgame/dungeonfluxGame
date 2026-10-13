//! The player's exploration and dialogue screen: `df_ui::ExplorationPhase` mounted once and
//! updated in place from the accepted view. A chosen offer reuses the existing prepare/send
//! path with the server's own offer, so the screen proposes input and decides nothing.
use super::*;
use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
use df_ui::{
    CampaignLimits, CampaignMember, CampaignView, ConceptScene, ExplorationChoice,
    ExplorationLimits, ExplorationNpc, ExplorationPhase, ExplorationPortrait, ExplorationView,
};
use std::cell::Cell;

const LIMITS: ExplorationLimits = ExplorationLimits {
    campaign: CampaignLimits {
        max_members: 8,
        max_objectives: 4,
        max_text_bytes: 2048,
    },
    max_choices: 4,
    max_text_bytes: 2048,
    max_identifier_bytes: 128,
};
/// The surface orders presentations by revision. The view has no independent presentation
/// sequence and redraws on local busy changes, so a fixed epoch carries a local counter.
const STAMP_EPOCH: u64 = 1;

pub(super) struct ExplorationMember {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) role: String,
    pub(super) sigil: String,
}

pub(super) struct ExplorationChoiceModel {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) icon: &'static str,
    pub(super) disabled_reason: Option<&'static str>,
}

pub(super) struct ExplorationSpeaker {
    pub(super) name: String,
    pub(super) context: String,
    pub(super) line: String,
    pub(super) portrait: Option<ExplorationPortrait>,
}

pub(super) struct ExplorationModel {
    /// PLAY-V09: where the speaker's head is across the scene plate, in permille, when the scene
    /// picture is a plate; the phone's narrow crop is centred there.
    pub(super) plate_focus: Option<u32>,
    pub(super) scene_asset: Option<String>,
    pub(super) portrait_asset: Option<String>,
    pub(super) has_whisper: bool,
    /// The scene's rules wording, shown on its own line under the narration.
    pub(super) rules_note: String,
    pub(super) binding: ClientBindingId,
    pub(super) scene: ConceptScene,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) whisper: String,
    pub(super) location: String,
    pub(super) scene_label: &'static str,
    pub(super) members: Vec<ExplorationMember>,
    pub(super) speaker: Option<ExplorationSpeaker>,
    pub(super) heading: &'static str,
    pub(super) choices: Vec<ExplorationChoiceModel>,
    pub(super) pending: Option<&'static str>,
}

pub(super) struct ExplorationScreen {
    phase: ExplorationPhase,
    stamp: Cell<u64>,
    image_fallbacks: RefCell<Vec<live_screens::ServerImage>>,
}

fn server_image(
    screen: &ExplorationScreen,
    selector: &str,
    source: &Option<String>,
    fallback: Option<String>,
) -> Result<(), JsValue> {
    let Some(source) = source else {
        return Ok(());
    };
    let Some(image) = screen.phase.root().query_selector(selector)? else {
        return Ok(());
    };
    if selector == ".exploration-portrait" {
        image.set_class_name("exploration-portrait");
    }
    let owner = live_screens::ServerImage::install(image, source, fallback)?;
    screen.image_fallbacks.borrow_mut().push(owner);
    Ok(())
}

fn with_view<R>(
    model: &ExplorationModel,
    sequence: u64,
    present: impl FnOnce(&ExplorationView<'_>) -> Result<R, String>,
) -> Result<R, String> {
    let epoch = RecoveryEpoch::new(STAMP_EPOCH).map_err(|error| format!("{error:?}"))?;
    let members: Vec<CampaignMember<'_>> = model
        .members
        .iter()
        .map(|member| CampaignMember {
            key: &member.key,
            name: &member.name,
            role: &member.role,
            sigil: &member.sigil,
        })
        .collect();
    let choices: Vec<ExplorationChoice<'_>> = model
        .choices
        .iter()
        .map(|choice| ExplorationChoice {
            id: &choice.id,
            label: &choice.label,
            // The compact player layout hides this line; the label is the visible text.
            detail: &choice.label,
            disabled_reason: choice.disabled_reason,
        })
        .collect();
    let view = ExplorationView {
        binding: model.binding,
        revision: SessionRevision::new(epoch, sequence),
        campaign: CampaignView {
            scene: model.scene,
            chapter: "Your story · your choices",
            title: &model.title,
            description: &model.description,
            location: &model.location,
            scene_label: model.scene_label,
            narration: &model.whisper,
            connection: "Local browser room · two players · the server owns the game",
            // The title is the screen's heading; the top bar does not repeat it.
            notice: "",
            members: &members,
            objectives: &[],
        },
        npc: model.speaker.as_ref().map(|speaker| ExplorationNpc {
            name: &speaker.name,
            context: &speaker.context,
            dialogue: &speaker.line,
            portrait: speaker.portrait,
        }),
        heading: model.heading,
        choices: &choices,
        draft_label: "Your words",
        submit_label: "Say it",
        draft_offer_id: None,
        pending: model.pending,
        rejection: None,
    };
    present(&view)
}

fn render_error(client: &Rc<RefCell<Client>>, error: &dyn std::fmt::Display) -> JsValue {
    let _ = client
        .borrow()
        .root
        .set_attribute("data-render-error", "exploration");
    JsValue::from_str(&format!("The scene could not be displayed: {error}"))
}

/// Marks the mounted surface for the player theme: whether the narration box carries a private
/// whisper, and which icon leads each choice. Both are presentation only.
fn decorate(screen: &ExplorationScreen, model: &ExplorationModel) -> Result<(), JsValue> {
    let root = screen.phase.root();
    screen.image_fallbacks.borrow_mut().clear();
    server_image(
        screen,
        ".scene-art",
        &model.scene_asset,
        Some(format!("/{}", model.scene.asset_path())),
    )?;
    let portrait_fallback = model
        .speaker
        .as_ref()
        .and_then(|s| s.portrait)
        .map(|p| format!("/{}", p.asset_path()));
    server_image(
        screen,
        ".exploration-portrait",
        &model.portrait_asset,
        portrait_fallback,
    )?;
    if let Some(art) = root.query_selector(".scene-art")? {
        match model.plate_focus {
            Some(focus) => {
                art.set_attribute("style", &format!("object-position:{}% top", focus / 10))?
            }
            None => art.remove_attribute("style")?,
        }
    }
    if model.has_whisper {
        root.set_attribute("data-whisper", "")?;
    } else {
        root.remove_attribute("data-whisper")?;
    }
    if let Some(document) = root.owner_document() {
        live_rules::mount(&document, root, &model.rules_note)?;
    }
    for choice in &model.choices {
        if choice.id.contains(['"', '\\']) {
            continue;
        }
        if let Some(button) =
            root.query_selector(&format!("[data-exploration-offer=\"{}\"]", choice.id))?
        {
            button.set_attribute("data-icon", choice.icon)?;
        }
    }
    Ok(())
}

impl ExplorationScreen {
    /// Presents `model` in place, mounting the surface under `parent` on first use.
    pub(super) fn present(
        client: &Rc<RefCell<Client>>,
        parent: &Element,
        model: &ExplorationModel,
    ) -> Result<(), JsValue> {
        let existing = client.borrow().live.exploration.is_some();
        if existing {
            let state = client.borrow();
            let Some(screen) = state.live.exploration.as_ref() else {
                return Ok(());
            };
            let sequence = screen.stamp.get() + 1;
            screen.stamp.set(sequence);
            with_view(model, sequence, |view| {
                screen.phase.update(view).map_err(|error| error.to_string())
            })
            .map_err(|error| render_error(client, &error))?;
            return decorate(screen, model);
        }
        let document = client.borrow().document.clone();
        let phase = with_view(model, 1, |view| {
            ExplorationPhase::create(&document, "live-explore-draft", view, LIMITS)
                .map_err(|error| error.to_string())
        })
        .map_err(|error| render_error(client, &error))?;
        let weak = Rc::downgrade(client);
        phase
            .on_input(move |input| {
                let Some(client) = weak.upgrade() else {
                    return;
                };
                // Free text is not offered here; typed dialogue has its own confirmed path.
                if let df_ui::ExplorationInput::Choice { id, .. } = input {
                    live_screens::submit_offer(&client, &id);
                }
            })
            .map_err(|error| render_error(client, &error))?;
        let screen = ExplorationScreen {
            phase,
            stamp: Cell::new(1),
            image_fallbacks: RefCell::new(Vec::new()),
        };
        decorate(&screen, model)?;
        parent.append_child(screen.phase.root())?;
        client.borrow_mut().live.exploration = Some(screen);
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.phase
            .dispose()
            .map_err(|error| JsValue::from_str(&format!("scene retirement failed: {error}")))
    }
}
