//! The phone's pages in the browser: the clickable demo (`?phone_kit=demo`) and the live phone's
//! tab bar. Both keep one `PhoneNav` here, take their taps as action strings through the page's
//! one click routing (`app::events`), and redraw the page in place. Nothing here asks the host:
//! the live phone's other pages are its seat's live pages (`app::pages`) where they are built, and
//! the gallery's sample pages elsewhere.

use std::cell::RefCell;

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::dom::{append, clear, element, js_message, set_action};
use crate::phone_kit::draw::{body, ensure_style, screen, shell};
use crate::phone_kit::nav::{CLOSE_MORE_ACTION, NavAction, PhoneNav, page_action};
use crate::phone_kit::shell::{EmptyState, MoreLink, TabBar, TabKey};
use crate::phone_kit::{AutoNext, PhonePanels, PhoneScreen};
use crate::timeline::ErrorSource;

/// The page the demo and the live phone open on.
const FIRST_PAGE: &str = "story";
/// The class of the demo's page, which `on_action` looks for to know it redraws the demo.
const DEMO_ROOT_CLASS: &str = "ph-demo-root";

thread_local! {
    static NAV: RefCell<PhoneNav> = RefCell::new(PhoneNav::new(FIRST_PAGE));
}

/// Whose bar it is: the live phone's or the clickable demo's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bar {
    Live,
    Demo,
}

/// The pages the "more" sheet reaches beyond the panels. The live phone's are its live pages: a
/// close look at what is in sight and the paintings. The demo's are its sample pages: the
/// creator, the look studio and a fight.
fn more_links(bar: Bar) -> Vec<MoreLink> {
    let links: &[(&str, &str)] = match bar {
        Bar::Live if crate::app::pages::has_paintings() => &[
            ("Look closer", crate::pages::screens::LOOK_PAGE),
            ("Paintings", crate::pages::screens::GALLERY_PAGE),
        ],
        Bar::Live => &[("Look closer", crate::pages::screens::LOOK_PAGE)],
        Bar::Demo => &[
            ("Make a hero", "create"),
            ("Look studio", "studio"),
            ("Battle mode", "battle"),
        ],
    };
    links
        .iter()
        .map(|(label, page)| MoreLink {
            label: (*label).to_owned(),
            action: page_action(page),
        })
        .collect()
}

fn current() -> PhoneNav {
    NAV.with(|nav| nav.borrow().clone())
}

/// The sample page named `key`: the gallery's, in a development build.
#[cfg(debug_assertions)]
fn sample_screen(key: &str) -> Option<PhoneScreen> {
    crate::phone_kit::gallery::PhoneGallery::read()
        .ok()?
        .boards
        .into_iter()
        .flat_map(|board| board.screens)
        .find(|sample| sample.key.as_deref() == Some(key))
}

/// A release build has no sample pages.
#[cfg(not(debug_assertions))]
fn sample_screen(_key: &str) -> Option<PhoneScreen> {
    None
}

/// The panel of the bar a page belongs to.
fn tab_of(page: &str) -> Option<TabKey> {
    TabKey::ALL
        .into_iter()
        .find(|tab| tab.page_key() == page)
        .or_else(|| crate::pages::screens::tab_of(page))
        .or_else(|| sample_screen(page).and_then(|sample| sample.tab))
}

fn tabs_for(nav: &PhoneNav, bar: Bar) -> TabBar {
    TabBar {
        active: tab_of(&nav.page),
        tabs: TabKey::ALL.to_vec(),
        // The live phone marks the tabs where the table waits on the player.
        badges: match bar {
            Bar::Live => crate::app::pages::table_waits().badges(),
            Bar::Demo => Vec::new(),
        },
        more_open: nav.more_open,
        working: Vec::new(),
        links: more_links(bar),
    }
}

/// The live phone's bar, lit where the phone is.
pub fn live_tabs() -> TabBar {
    tabs_for(&current(), Bar::Live)
}

/// The scrim under the open "more" sheet: a tap on it closes the sheet.
fn scrim(document: &Document) -> Result<Element, JsValue> {
    let scrim = element(document, "div", "ph-more-scrim")?;
    set_action(&scrim, CLOSE_MORE_ACTION)?;
    Ok(scrim)
}

fn schedule(auto: AutoNext, from: String) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let wait = i32::try_from(auto.after_ms).unwrap_or(i32::MAX);
    let next = Closure::once_into_js(move || {
        if current().page == from {
            on_action(&page_action(&auto.page));
        }
    });
    if let Err(error) =
        window.set_timeout_with_callback_and_timeout_and_arguments_0(next.unchecked_ref(), wait)
    {
        note_failure(&error);
    }
}

fn demo_screen(document: &Document, nav: &PhoneNav) -> Result<Element, JsValue> {
    let sample = sample_screen(&nav.page)
        .or_else(|| sample_screen(FIRST_PAGE))
        .ok_or_else(|| JsValue::from_str("the demo has no sample pages"))?;
    let mut panels: PhonePanels = sample.panels;
    panels.tabs = Some(tabs_for(nav, Bar::Demo));
    let drawn = screen(document, &panels)?;
    if nav.more_open {
        append(&drawn, &scrim(document)?)?;
    }
    if let Some(auto) = sample.auto {
        schedule(auto, nav.page.clone());
    }
    Ok(drawn)
}

/// The demo's page, a single phone screen filling the window, or `None` when the address does
/// not ask for it (`?phone_kit=demo`).
#[cfg(debug_assertions)]
pub fn demo_page(document: &Document) -> Result<Option<Element>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let query = window.location().search()?;
    if crate::path::query_value(&query, crate::phone_kit::gallery::GALLERY_QUERY_KEY)
        != Some("demo")
    {
        return Ok(None);
    }
    ensure_style(document)?;
    let root = element(document, "div", DEMO_ROOT_CLASS)?;
    append(&root, &demo_screen(document, &current())?)?;
    Ok(Some(root))
}

/// A release build has no demo.
#[cfg(not(debug_assertions))]
pub fn demo_page(_document: &Document) -> Result<Option<Element>, JsValue> {
    Ok(None)
}

/// Dresses a live phone's screen for the page the phone is on: which of Story, Act or a sample
/// page shows, the bar and the "more" sheet's scrim.
pub fn paint_live(document: &Document, screen: &Element) -> Result<(), JsValue> {
    let nav = current();
    let page = match nav.page.as_str() {
        "story" => "story",
        "act" => "act",
        _ => "other",
    };
    let was = screen.get_attribute("data-page");
    screen.set_attribute("data-page", page)?;
    // Story opens at its newest line, and follows new ones while the reader stays at the bottom.
    if page == "story" {
        crate::view::phone::follow_story(was.as_deref() != Some("story"))?;
    }
    if let Some(old) = screen.query_selector(".ph-tabs-wrap")? {
        let fresh = shell::tab_bar(document, &tabs_for(&nav, Bar::Live))?;
        old.replace_with_with_node_1(&fresh)?;
    }
    if let Some(old) = screen.query_selector(".ph-more-scrim")? {
        old.remove();
    }
    // A question or a roll the table waits on, named on the pages where it is not answered: a tap
    // goes there. The phone never leaves the page on its own.
    if let Some(old) = screen.query_selector(".ph-wait-banner")? {
        old.remove();
    }
    if let Some(banner) = crate::app::pages::table_waits().banner(&nav.page)
        && let Some(dock) = screen.query_selector(".ph-dock")?
    {
        let line = element(document, "button", "ph-wait-banner")?;
        line.set_attribute("type", "button")?;
        line.set_text_content(Some(&banner.text));
        set_action(&line, &banner.action)?;
        dock.insert_before(&line, dock.first_child().as_ref())?;
    }
    if nav.more_open {
        append(screen, &scrim(document)?)?;
    }
    if let Some(sample) = screen.query_selector(".ph-sample")? {
        clear(&sample);
        if page == "other" {
            // The live phone shows its own pages only, never the demo's sample hero.
            let panels = match crate::app::pages::live_panels(&nav.page) {
                Some(panels) => panels,
                None => PhonePanels {
                    empty: Some(EmptyState {
                        title: "Not open yet".to_owned(),
                        text: "This page is not built into the live phone yet.".to_owned(),
                        glyph: None,
                    }),
                    ..PhonePanels::default()
                },
            };
            let drawn = body(document, &panels)?;
            while let Some(child) = drawn.first_element_child() {
                append(&sample, &child)?;
            }
            // Roll keeps the seat's last roll as the shared screen's full card, above the checks.
            if nav.page == crate::pages::sheet::ROLL_PAGE
                && let Some(roll) = crate::app::pages::last_roll()
            {
                let card = crate::view::roll::phone_card(document, &roll)?;
                sample.insert_before(&card, sample.first_child().as_ref())?;
            }
            sample.remove_attribute("hidden")?;
        } else {
            sample.set_attribute("hidden", "")?;
        }
    }
    crate::app::pages::paint_clue_viewer(
        document,
        screen,
        nav.page == crate::pages::screens::CLUES_PAGE,
    )
}

/// Takes a tap's action when it is navigation, and redraws the page for it. Returns whether it
/// was.
pub fn on_action(action: &str) -> bool {
    let Some(nav_action) = NavAction::parse(action) else {
        return false;
    };
    NAV.with(|nav| nav.borrow_mut().apply(nav_action));
    if let Err(error) = redraw() {
        note_failure(&error);
    }
    true
}

/// Draws the open page again, as it is now: the live pages changed.
pub fn refresh() {
    if let Err(error) = redraw() {
        note_failure(&error);
    }
}

fn redraw() -> Result<(), JsValue> {
    let document = crate::dom::document()?;
    let nav = current();
    if let Some(root) = document.query_selector(&format!(".{DEMO_ROOT_CLASS}"))? {
        clear(&root);
        return append(&root, &demo_screen(&document, &nav)?);
    }
    if let Some(live) = document.query_selector(".ph-live")? {
        return paint_live(&document, &live);
    }
    Ok(())
}

/// Says a page change failed: on the console and in the page's report.
fn note_failure(error: &JsValue) {
    let message = format!("the phone's page could not be drawn: {}", js_message(error));
    web_sys::console::error_1(&JsValue::from_str(&message));
    crate::page_log::note_error(ErrorSource::Draw, &message);
}
