//! Designed, server-fed screens for combat, the encounter aftermath, the short rest and the
//! character sheet, mounted from the df-ui component library instead of hand-built DOM.
//!
//! The server stays authoritative: each module here only maps the received `ViewMessage`
//! onto borrowed df-ui props, and every input goes through the existing `prepare`/`send`
//! path. Surfaces are created once per phase, updated in place for every accepted view and
//! disposed when the phase changes. The shared display mounts inside its persistent
//! `.tv-stage`; a player's screen is mounted in the player's view.

mod aftermath;
mod combat;
pub(super) use combat::describe;
pub(super) mod sheet;
mod style;

use super::{
    Client, button, dialogue_recovery, live_battlefield, live_rules, live_screens, live_stage,
    live_tv, live_tv::TvStage, node, recovery_needs, recovery_prompt, replay,
};
use df_protocol::common as rpc;
use df_types::{RecoveryEpoch, SessionRevision};
use df_ui::{CampfireSurface, CombatPhaseSurface, EncounterAftermathSurface};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

const STYLE_ID: &str = "df-live-style";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Combat,
    Aftermath,
}
impl Screen {
    const fn name(self, player: bool) -> &'static str {
        match (self, player) {
            (Self::Combat, _) => "combat",
            (Self::Aftermath, true) => "rest",
            (Self::Aftermath, false) => "aftermath",
        }
    }
}

enum Mounted {
    Combat(Rc<CombatPhaseSurface>),
    Aftermath(Rc<EncounterAftermathSurface>),
    Rest(Rc<CampfireSurface>),
}
impl Mounted {
    fn dispose(&self) -> Result<(), JsValue> {
        let failed = match self {
            Self::Combat(surface) => surface.dispose().is_err(),
            Self::Aftermath(surface) => surface.dispose().is_err(),
            Self::Rest(surface) => surface.dispose().is_err(),
        };
        if failed {
            return Err(JsValue::from_str("live screen cleanup failed"));
        }
        Ok(())
    }
}

/// Client state owned by these screens: the mounted surface, its host element, local attack
/// choices, the open character sheet and the local presentation ordering.
#[derive(Default)]
pub(super) struct Live {
    host: Option<Element>,
    mounted: Option<Mounted>,
    sheet: Option<sheet::SheetOverlay>,
    choices: BTreeMap<String, bool>,
    shown: Option<(u64, u64)>,
    redraws: u16,
    scene_image: Option<live_screens::ServerImage>,
}
impl Live {
    /// An attack switch starts on: Savage Attacker and Graze cost nothing to use, so a player
    /// turns one off on purpose rather than losing it by not noticing the switch.
    fn choice(&self, group: &str) -> bool {
        self.choices.get(group).copied().unwrap_or(true)
    }
    fn toggle_choice(&mut self, group: &str) {
        let selected = self.choice(group);
        self.choices.insert(group.to_owned(), !selected);
    }
}

/// An option label for people: ids such as `common-sign` read as "Common Sign". Labels that
/// already carry capitals are left alone. Presentation only; submitted values stay the ids.
pub(super) fn display_label(label: &str) -> String {
    if label.chars().any(char::is_uppercase) {
        return label.to_owned();
    }
    label
        .split(['-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The marker the server appends to narration when the rules are customised.
const RULES_MARKER: &str = "Custom rules:";

/// Narration without a trailing custom-rules label, and whether the label was present. The
/// long label is never shown; a small badge stands in for it.
fn split_rules_marker(text: &str) -> (&str, bool) {
    match text.find(RULES_MARKER) {
        Some(index) => (text[..index].trim_end(), true),
        None => (text, false),
    }
}

/// The accepted view parts a live screen presents.
pub(super) struct LiveView<'a> {
    pub(super) surface: &'a Element,
    pub(super) narration: &'a str,
    pub(super) scene: &'a rpc::GameplayScene,
    pub(super) journey: &'a rpc::JourneyView,
    pub(super) offers: &'a [rpc::GameplayActionOffer],
}

/// df-ui admits a view only when its revision is newer, but the same server revision is
/// redrawn locally (busy, choices). The wire carries no separate presentation sequence, so
/// this local ordering token is `server sequence * 65536 + redraws of that revision`.
fn presentation_revision(
    client: &Rc<RefCell<Client>>,
    revision: SessionRevision,
) -> Result<SessionRevision, JsValue> {
    let mut state = client.borrow_mut();
    let live = &mut state.combat;
    let key = (revision.epoch().get(), revision.sequence());
    if live.shown == Some(key) {
        live.redraws = live
            .redraws
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("redraw count exhausted"))?;
    } else {
        live.shown = Some(key);
        live.redraws = 0;
    }
    let sequence = revision
        .sequence()
        .checked_mul(65_536)
        .and_then(|sequence| sequence.checked_add(u64::from(live.redraws)))
        .ok_or_else(|| JsValue::from_str("presentation sequence exhausted"))?;
    Ok(SessionRevision::new(
        RecoveryEpoch::new(revision.epoch().get())
            .map_err(|_| JsValue::from_str("presentation epoch invalid"))?,
        sequence,
    ))
}

fn classify(view: &LiveView<'_>) -> Option<Screen> {
    let combat = view.journey.combat.as_ref()?;
    match rpc::JourneyPhase::try_from(view.journey.phase).ok()? {
        rpc::JourneyPhase::Combat => Some(Screen::Combat),
        // The inn scene is a later narrative beat that keeps the finished encounter.
        rpc::JourneyPhase::Complete
            if combat.finished
                && view.scene.destination != rpc::HarborDestination::HarborInn as i32 =>
        {
            Some(Screen::Aftermath)
        }
        _ => None,
    }
}

fn ensure_style(document: &Document) -> Result<(), JsValue> {
    if document.get_element_by_id(STYLE_ID).is_some() {
        return Ok(());
    }
    let head = document
        .query_selector("head")?
        .ok_or_else(|| JsValue::from_str("document head absent"))?;
    let element = document.create_element("style")?;
    element.set_id(STYLE_ID);
    element.set_text_content(Some(&style::stylesheet()));
    head.append_child(&element)?;
    Ok(())
}

/// Where the live host is mounted. A player's screen replaces what the player's view held; the
/// display's persistent stage is reused (or created) and switched to its combat mode.
fn host_parent(client: &Rc<RefCell<Client>>, scene_view: &Element) -> Result<Element, JsValue> {
    let (document, player) = {
        let state = client.borrow();
        (state.document.clone(), state.role == "player")
    };
    if let Some(stage) = client.borrow().live.tv.as_ref() {
        return Ok(stage.root().clone());
    }
    live_screens::release(client, scene_view)?;
    if let Some(scene) = &client.borrow().scene_delivery {
        scene
            .retire()
            .map_err(|_| JsValue::from_str("retired scene cleanup failed"))?;
    }
    scene_view.set_text_content(None);
    if player {
        return Ok(scene_view.clone());
    }
    let stage = TvStage::mount(&document, scene_view)?;
    let root = stage.root().clone();
    client.borrow_mut().live.tv = Some(stage);
    Ok(root)
}

fn ensure_host(
    client: &Rc<RefCell<Client>>,
    scene_view: &Element,
    screen: Screen,
) -> Result<Element, JsValue> {
    let (document, role, existing) = {
        let state = client.borrow();
        (
            state.document.clone(),
            state.role,
            state.combat.host.clone(),
        )
    };
    ensure_style(&document)?;
    let player = role == "player";
    let host = match existing.filter(|host| host.is_connected()) {
        Some(host) => host,
        None => {
            let stale = client.borrow_mut().combat.mounted.take();
            if let Some(stale) = stale {
                stale.dispose()?;
            }
            let parent = host_parent(client, scene_view)?;
            let host = document.create_element("div")?;
            host.set_class_name("live-screen");
            parent.append_child(&host)?;
            client.borrow_mut().combat.host = Some(host.clone());
            host
        }
    };
    let name = screen.name(player);
    if host.get_attribute("data-screen").as_deref() != Some(name) {
        // A different screen starts from a clean host.
        let previous = client.borrow_mut().combat.mounted.take();
        if let Some(previous) = previous {
            previous.dispose()?;
        }
        host.set_text_content(None);
        host.set_attribute("data-screen", name)?;
    }
    host.set_attribute("data-role", role)?;
    if let Some(stage) = client.borrow().live.tv.as_ref() {
        stage.root().set_attribute("data-tv-mode", "combat")?;
    }
    Ok(host)
}

fn party_portrait(view: &LiveView<'_>, own: &rpc::CharacterSheet) -> &'static str {
    sheet::portrait_token(&view.journey.party, own)
}

/// Feedback and recovery controls, shown only when their condition holds.
fn recovery(client: &Rc<RefCell<Client>>, parent: &Element) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    for stale in [".recovery-needed", ".live-confirm"] {
        if let Some(previous) = parent.query_selector(stale)? {
            previous.remove();
        }
    }
    recovery_prompt(client, parent)?;
    // The feedback node is kept (and moved last) so its message survives redraws.
    match parent.query_selector(".feedback")? {
        Some(node) => {
            parent.append_child(&node)?;
        }
        None => {
            node(&document, parent, "p", "feedback", "")?;
        }
    }
    let confirm_strike = {
        let state = client.borrow();
        let needs = recovery_needs(&state);
        state.first_attack.is_some() && (needs.lost_connection || needs.unknown_choice)
    };
    if confirm_strike {
        let confirm = button(client, parent, "Confirm original strike", |client| {
            replay(client, true);
        })?;
        confirm.set_class_name("live-confirm");
    }
    Ok(())
}

/// The player's hero header, character sheet, free-text dialogue and recovery controls, or the
/// display's recovery dock.
fn mount_chrome(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    if client.borrow().role != "player" {
        let dock = client
            .borrow()
            .live
            .tv
            .as_ref()
            .map(|stage| stage.dock().clone())
            .ok_or_else(|| JsValue::from_str("display stage absent"))?;
        dock.set_text_content(None);
        return recovery(client, &dock);
    }
    let extras = match host.query_selector(":scope > .live-extras")? {
        Some(extras) => extras,
        None => node(&document, host, "div", "live-extras", "")?,
    };
    host.append_child(&extras)?;
    if let Some(previous) = extras.query_selector(".live-sheet-inline")? {
        previous.remove();
    }
    if let Some(own) = view.journey.own_character.as_ref() {
        let portrait = party_portrait(view, own);
        sheet::mount_head(client, host, own, portrait)?;
        sheet::mount_inline(client, &extras, own, portrait)?;
    }
    dialogue_recovery::mount(client, &extras, view.offers)?;
    recovery(client, &extras)
}

/// A small gold badge for customised rules: top-right on the display, under the hero header on
/// a phone.
fn mount_rules_badge(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    custom: bool,
) -> Result<(), JsValue> {
    if let Some(previous) = host.query_selector(":scope > .live-rules-badge")? {
        previous.remove();
    }
    if !custom {
        return Ok(());
    }
    let document = client.borrow().document.clone();
    let badge = node(&document, host, "div", "live-rules-badge", "Custom rules")?;
    let after = host
        .query_selector(":scope > .live-hero-head")?
        .and_then(|head| head.next_sibling());
    if let Some(after) = after {
        host.insert_before(&badge, Some(&after))?;
    }
    Ok(())
}

/// Presents combat or the aftermath when the accepted view is one. Returns false for every
/// other view, which the other renderers show.
pub(super) fn present(client: &Rc<RefCell<Client>>, view: &LiveView<'_>) -> Result<bool, JsValue> {
    let Some(screen) = classify(view) else {
        // PLAY-F02: any non-combat view ends the 3D battlefield (it finishes its blow first).
        live_battlefield::release();
        return Ok(false);
    };
    let (narration, custom_rules) = split_rules_marker(view.narration);
    let view = &LiveView {
        narration: if narration.is_empty() {
            &view.scene.title
        } else {
            narration
        },
        ..*view
    };
    let host = ensure_host(client, view.surface, screen)?;
    let player = client.borrow().role == "player";
    if !player {
        present_stage(client, view, screen)?;
    }
    let mut assets = String::new();
    if let Some(url) = live_screens::asset_url(&view.scene.scene_asset) {
        assets.push_str(&format!("--asset-scene-courier-road:url({url});"));
    }
    if let Some(url) = live_screens::asset_url(&view.scene.speaker_portrait)
        && screen == Screen::Combat
    {
        assets.push_str(&format!("--asset-portrait-bandit:url({url}),url(/assets/ui/generated/portrait-bandit);--asset-portrait-bandit-full:url({url}),url(/assets/ui/generated/portrait-bandit-full);"));
    }
    host.set_attribute("style", &assets)?;
    match (screen, player) {
        (Screen::Combat, _) => combat::present(client, &host, view)?,
        (Screen::Aftermath, false) => aftermath::present_display(client, &host, view)?,
        (Screen::Aftermath, true) => aftermath::present_rest(client, &host, view)?,
    }
    client.borrow_mut().combat.scene_image = None;
    if screen == Screen::Combat
        && let Some(url) = live_screens::asset_url(&view.scene.scene_asset)
        && let Some(image) = host.query_selector(".combat-art")?
    {
        client.borrow_mut().combat.scene_image = Some(live_screens::ServerImage::install(
            image,
            &url,
            Some(style::COMBAT_ART_URL.into()),
        )?);
    }
    mount_chrome(client, &host, view)?;
    mount_rules_badge(client, &host, custom_rules)?;
    // PLAY-F03: the scene's rules note on its own line under the narration (see live_rules).
    live_rules::mount(&client.borrow().document, &host, &view.scene.rules_note)?;
    // PLAY-F02 dispatch point: on the shared display the 3D battlefield overlays this flat view
    // when it can run; otherwise today's view above is what the table sees.
    if !player {
        if screen == Screen::Combat {
            live_battlefield::present(client, view);
        } else {
            live_battlefield::release();
        }
    }
    Ok(true)
}

/// The display's composited stage behind the combat or aftermath screen: the painting and the
/// characters standing in it. The screen's own elements sit above and carry the text.
fn present_stage(
    client: &Rc<RefCell<Client>>,
    view: &LiveView<'_>,
    screen: Screen,
) -> Result<(), JsValue> {
    let cast = live_screens::current_cast(client);
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    // PLAY-F03: the table's choices row follows the view (empty during a fight).
    stage.set_choices(&view.journey.table_choices)?;
    let art = live_screens::asset_url(&view.scene.scene_asset)
        .unwrap_or_else(|| style::COMBAT_ART_URL.to_owned());
    stage.set_art(&live_tv::image_layers(&art, style::COMBAT_ART_URL))?;
    stage.figures().update(
        &cast,
        &view.scene.title,
        &view.scene.scene_asset,
        crate::presentation::scene_family(&view.scene.scene_asset),
        if screen == Screen::Combat {
            live_stage::StageMode::Combat
        } else {
            live_stage::StageMode::Aftermath
        },
    )
}

/// Removes the live screen so another renderer can show the view; does nothing if none is shown.
pub(super) fn release(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (mounted, host) = {
        let mut state = client.borrow_mut();
        (state.combat.mounted.take(), state.combat.host.take())
    };
    let cleanup = mounted.map(|mounted| mounted.dispose()).transpose();
    if let Some(host) = host {
        host.remove();
    }
    live_battlefield::release();
    cleanup.map(|_| ())
}

/// Disposes everything this module owns. Used when the view scope is replaced and on drop.
pub(super) fn dispose(client: &mut Client) -> Result<(), JsValue> {
    let mounted = client.combat.mounted.take();
    let overlay = client.combat.sheet.take();
    let host = client.combat.host.take();
    client.combat.shown = None;
    client.combat.redraws = 0;
    let cleanup = mounted.map(|mounted| mounted.dispose()).transpose();
    drop(overlay);
    if let Some(host) = host {
        host.remove();
    }
    live_battlefield::release();
    cleanup.map(|_| ())
}
