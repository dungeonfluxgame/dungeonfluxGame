//! Designed, server-fed screens for combat, the encounter aftermath, the short rest and the
//! character sheet, mounted from the df-ui component library instead of hand-built DOM.
//!
//! The server stays authoritative: each module here only maps the received `ViewMessage`
//! onto borrowed df-ui props, and every input goes through the existing `prepare`/`send`
//! path. Surfaces are created once per phase, updated in place for every accepted view
//! and disposed when the phase changes.

mod aftermath;
mod combat;
pub(super) mod sheet;
mod style;

use super::{Client, dialogue_recovery, node, recovery_details, recovery_prompt};
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

/// Client state owned by the live screens: the mounted surface, its host element, local
/// attack choices, the open character sheet and the local presentation ordering.
#[derive(Default)]
pub(super) struct Live {
    host: Option<Element>,
    mounted: Option<Mounted>,
    sheet: Option<sheet::SheetOverlay>,
    choices: BTreeMap<String, bool>,
    shown: Option<(u64, u64)>,
    redraws: u16,
}
impl Live {
    fn choice(&self, group: &str) -> bool {
        self.choices.get(group).copied().unwrap_or(false)
    }
    fn toggle_choice(&mut self, group: &str) {
        let selected = self.choice(group);
        self.choices.insert(group.to_owned(), !selected);
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
    let live = &mut state.live;
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

/// The host sits before the plain scene view, which stays hidden while a live screen shows.
/// It is full-viewport only for a display that is the page's single client.
fn ensure_host(
    client: &Rc<RefCell<Client>>,
    scene_view: &Element,
    screen: Screen,
) -> Result<Element, JsValue> {
    let (document, role, existing) = {
        let state = client.borrow();
        (state.document.clone(), state.role, state.live.host.clone())
    };
    ensure_style(&document)?;
    let host = match existing {
        Some(host) => host,
        None => {
            let host = document.create_element("div")?;
            host.set_class_name("live-screen");
            scene_view.insert_adjacent_element("beforebegin", &host)?;
            client.borrow_mut().live.host = Some(host.clone());
            host
        }
    };
    let name = screen.name(role == "player");
    if host.get_attribute("data-screen").as_deref() != Some(name) {
        // A different screen starts from a clean host.
        let previous = client.borrow_mut().live.mounted.take();
        if let Some(previous) = previous {
            previous.dispose()?;
        }
        host.set_text_content(None);
        host.set_attribute("data-screen", name)?;
    }
    host.set_attribute("data-role", role)?;
    host.set_attribute(
        "data-fit",
        if role == "display" && document.get_element_by_id("player").is_none() {
            "viewport"
        } else {
            "card"
        },
    )?;
    scene_view.set_attribute("hidden", "")?;
    Ok(host)
}

/// Recovery, free-text dialogue and the sheet button stay reachable below (or over) the screen.
fn mount_extras(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    let (document, player) = {
        let state = client.borrow();
        (state.document.clone(), state.role == "player")
    };
    let extras = match host.query_selector(":scope > .live-extras")? {
        Some(extras) => extras,
        None => node(&document, host, "div", "live-extras", "")?,
    };
    for stale in [".live-sheet-open", ".recovery-needed", ".trouble"] {
        if let Some(previous) = extras.query_selector(stale)? {
            previous.remove();
        }
    }
    if player && let Some(own) = view.journey.own_character.as_ref() {
        sheet::summary_button(client, &extras, own)?;
    }
    dialogue_recovery::mount(client, &extras, view.offers)?;
    recovery_prompt(client, &extras)?;
    // The feedback node is kept (and moved last) so its message survives redraws.
    match extras.query_selector(".feedback")? {
        Some(feedback) => {
            extras.append_child(&feedback)?;
        }
        None => {
            node(&document, &extras, "p", "feedback", "")?;
        }
    }
    recovery_details(client, &extras)?;
    Ok(())
}

/// Presents combat or the aftermath when the accepted view is one. Returns false for every
/// other view, which the plain scene renderer shows.
pub(super) fn present(client: &Rc<RefCell<Client>>, view: &LiveView<'_>) -> Result<bool, JsValue> {
    let Some(screen) = classify(view) else {
        return Ok(false);
    };
    if let Some(scene) = &client.borrow().scene_delivery {
        scene
            .retire()
            .map_err(|_| JsValue::from_str("retired scene cleanup failed"))?;
    }
    let host = ensure_host(client, view.surface, screen)?;
    let player = client.borrow().role == "player";
    match (screen, player) {
        (Screen::Combat, _) => combat::present(client, &host, view)?,
        (Screen::Aftermath, false) => aftermath::present_display(client, &host, view)?,
        (Screen::Aftermath, true) => aftermath::present_rest(client, &host, view)?,
    }
    mount_extras(client, &host, view)?;
    Ok(true)
}

/// Returns to the plain scene view after a live screen, or does nothing if none is shown.
pub(super) fn release(client: &Rc<RefCell<Client>>, scene_view: &Element) -> Result<(), JsValue> {
    let (mounted, host) = {
        let mut state = client.borrow_mut();
        (state.live.mounted.take(), state.live.host.take())
    };
    let cleanup = mounted.map(|mounted| mounted.dispose()).transpose();
    if let Some(host) = host {
        host.remove();
        scene_view.remove_attribute("hidden")?;
    }
    cleanup.map(|_| ())
}

/// Disposes everything this module owns. Used when the view scope is replaced and on drop.
pub(super) fn dispose(client: &mut Client) -> Result<(), JsValue> {
    let mounted = client.live.mounted.take();
    let overlay = client.live.sheet.take();
    let host = client.live.host.take();
    client.live.shown = None;
    client.live.redraws = 0;
    let cleanup = mounted.map(|mounted| mounted.dispose()).transpose();
    drop(overlay);
    if let Some(host) = host {
        host.remove();
        if let Some(scene_view) = client.root.query_selector(".view")? {
            scene_view.remove_attribute("hidden")?;
        }
    }
    cleanup.map(|_| ())
}
