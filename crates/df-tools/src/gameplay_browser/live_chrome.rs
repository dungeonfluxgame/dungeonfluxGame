//! The quiet page chrome: a problem toast on every screen and, for the player, a bottom tab
//! bar with a menu sheet. Nothing here is shown while the connection is healthy; the toast
//! appears only when a recovery action is needed, and the rarely used controls (reconnect,
//! confirm a saved choice, leave this device, credits) live behind the menu.
use super::*;
use web_sys::Event;

const HERO_SHEET_ID: &str = "hero-sheet";
const CREDITS_PATH: &str = "/gameplay/credits";

type Listener = (Element, Closure<dyn FnMut(Event)>);

struct Mounted {
    sheet: Element,
    body: Element,
    hero: Element,
    listeners: Vec<Listener>,
}

/// The player's persistent tab bar and menu sheet.
#[derive(Default)]
pub(super) struct Chrome {
    mounted: Option<Mounted>,
}

impl Chrome {
    pub(super) fn dispose(&mut self) {
        if let Some(mounted) = self.mounted.take() {
            for (element, callback) in mounted.listeners {
                let _ = element.remove_event_listener_with_callback(
                    "click",
                    callback.as_ref().unchecked_ref(),
                );
            }
        }
    }
}

fn listen(
    listeners: &mut Vec<Listener>,
    element: &Element,
    mut activate: impl FnMut(Event) + 'static,
) -> Result<(), JsValue> {
    let callback =
        Closure::wrap(Box::new(move |event: Event| activate(event)) as Box<dyn FnMut(Event)>);
    element.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
    listeners.push((element.clone(), callback));
    Ok(())
}

fn tab(document: &Document, parent: &Element, icon: &str, label: &str) -> Result<Element, JsValue> {
    let tab = node(document, parent, "button", "live-tab", "")?;
    tab.set_attribute("type", "button")?;
    let image = node(document, &tab, "img", "live-tab-icon", "")?;
    image.set_attribute("src", &format!("{}/{icon}", live_tv::GENERATED))?;
    image.set_attribute("alt", "")?;
    node(document, &tab, "span", "", label)?;
    Ok(tab)
}

/// Mounts the player's tab bar and menu sheet once beside the view. Other roles have none.
pub(super) fn mount(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, root, player) = {
        let state = client.borrow();
        (
            state.document.clone(),
            state.root.clone(),
            state.role == "player",
        )
    };
    toast(client)?;
    if !player || client.borrow().chrome.mounted.is_some() {
        return Ok(());
    }
    let nav = node(&document, &root, "nav", "live-tabs", "")?;
    nav.set_attribute("aria-label", "Game")?;
    let play = tab(&document, &nav, "icon-talk", "Play")?;
    play.set_attribute("aria-current", "page")?;
    let hero = tab(&document, &nav, "icon-character", "Hero")?;
    hero.set_attribute("hidden", "")?;
    let menu = tab(&document, &nav, "icon-menu", "Menu")?;
    let sheet = node(&document, &root, "div", "live-sheet", "")?;
    let panel = node(&document, &sheet, "div", "live-sheet-panel", "")?;
    node(&document, &panel, "h2", "", "Menu")?;
    let body = node(&document, &panel, "div", "live-sheet-body", "")?;
    let close = node(&document, &panel, "button", "live-sheet-close", "Close")?;
    close.set_attribute("type", "button")?;
    let mut listeners = Vec::new();
    let closing = sheet.clone();
    listen(&mut listeners, &play, {
        let sheet = sheet.clone();
        move |_| {
            let _ = sheet.remove_attribute("data-open");
            if let Some(window) = web_sys::window() {
                window.scroll_to_with_x_and_y(0.0, 0.0);
            }
        }
    })?;
    listen(&mut listeners, &hero, {
        let (sheet, document) = (sheet.clone(), document.clone());
        move |_| {
            let _ = sheet.remove_attribute("data-open");
            if let Some(target) = document.get_element_by_id(HERO_SHEET_ID) {
                target.scroll_into_view();
            }
        }
    })?;
    listen(&mut listeners, &menu, {
        let sheet = sheet.clone();
        move |_| {
            if sheet.has_attribute("data-open") {
                let _ = sheet.remove_attribute("data-open");
            } else {
                let _ = sheet.set_attribute("data-open", "");
            }
        }
    })?;
    listen(&mut listeners, &close, {
        let sheet = sheet.clone();
        move |_| {
            let _ = sheet.remove_attribute("data-open");
        }
    })?;
    // A tap on the dimmed backdrop, outside the panel, closes the sheet.
    listen(&mut listeners, &sheet, move |event| {
        let outside = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .is_some_and(|target| target.is_same_node(Some(closing.as_ref())));
        if outside {
            let _ = closing.remove_attribute("data-open");
        }
    })?;
    client.borrow_mut().chrome.mounted = Some(Mounted {
        sheet,
        body,
        hero,
        listeners,
    });
    Ok(())
}

fn close_menu(client: &Rc<RefCell<Client>>) {
    if let Some(mounted) = client.borrow().chrome.mounted.as_ref() {
        let _ = mounted.sheet.remove_attribute("data-open");
    }
}

/// The toast shared by every screen. It holds the one `.feedback` line and the recovery
/// prompt, and shows only while either has content.
fn toast(client: &Rc<RefCell<Client>>) -> Result<Element, JsValue> {
    let (document, root) = {
        let state = client.borrow();
        (state.document.clone(), state.root.clone())
    };
    if let Some(existing) = root.query_selector(".live-toast")? {
        return Ok(existing);
    }
    let toast = document.create_element("div")?;
    toast.set_class_name("live-toast");
    toast.set_attribute("role", "status")?;
    node(&document, &toast, "p", "feedback", "")?;
    root.insert_before(&toast, root.first_child().as_ref())?;
    Ok(toast)
}

/// Shows a recovery control only while its condition holds. A dialogue confirmation of
/// unknown outcome has its own retry button inside the dialogue panel.
pub(super) fn prompt(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, needs, dialogue_retry) = {
        let state = client.borrow();
        (
            state.document.clone(),
            recovery_needs(&state),
            state.dialogue.has_confirmation(),
        )
    };
    let toast = toast(client)?;
    if let Some(previous) = toast.query_selector(".recovery-needed")? {
        previous.remove();
    }
    let confirm_here = needs.unknown_choice && !dialogue_retry;
    if !needs.lost_connection && !confirm_here {
        return Ok(());
    }
    let prompt = node(&document, &toast, "div", "recovery-needed", "")?;
    node(
        &document,
        &prompt,
        "p",
        "",
        if needs.lost_connection {
            "The connection to the game was lost."
        } else {
            "Your last choice may not have been saved. Confirm it to check; it will not be applied twice."
        },
    )?;
    let tools = node(&document, &prompt, "div", "tools", "")?;
    if needs.lost_connection {
        button(client, &tools, "Reconnect", connect)?;
    }
    if confirm_here && client.borrow().last_request.is_some() {
        confirm_saved_choice(client, &tools)?;
    }
    if let Some(feedback) = toast.query_selector(".feedback")? {
        toast.insert_before(&prompt, Some(&feedback))?;
    }
    Ok(())
}

/// Whether the table has hidden the choice chips, as the current view says. A view without a
/// journey (none is shown then) keeps the chips shown.
fn choice_hints_hidden(state: &Client) -> bool {
    let current = state.views.as_ref().and_then(|views| views.current());
    let journey = match current.and_then(|view| view.audience.as_ref()) {
        Some(rpc::view_message::Audience::Player(player)) => player.journey.as_ref(),
        Some(rpc::view_message::Audience::Display(display)) => display.journey.as_ref(),
        None => None,
    };
    journey.is_some_and(|journey| journey.choice_hints_hidden)
}

/// Rebuilds the player's menu items for the current state; the display has no menu.
pub(super) fn fill_menu(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, needs, dialogue_retry, last_request, first_attack, body, hero, hidden) = {
        let state = client.borrow();
        let Some(mounted) = state.chrome.mounted.as_ref() else {
            return Ok(());
        };
        (
            state.document.clone(),
            recovery_needs(&state),
            state.dialogue.has_confirmation(),
            state.last_request.is_some(),
            state.first_attack.is_some(),
            mounted.body.clone(),
            mounted.hero.clone(),
            choice_hints_hidden(&state),
        )
    };
    body.set_text_content(None);
    button(client, &body, "Reconnect", |client| {
        close_menu(&client);
        connect(client);
    })?;
    if last_request && !(needs.unknown_choice && !dialogue_retry) {
        button(client, &body, "Confirm saved choice", |client| {
            close_menu(&client);
            replay(client, false);
        })?;
    }
    if first_attack {
        button(client, &body, "Confirm original strike", |client| {
            close_menu(&client);
            replay(client, true);
        })?;
    }
    button(client, &body, LEAVE_LABEL, |client| {
        close_menu(&client);
        leave_saved_game(client);
    })?;
    // The table's settings: any seated player may change them, from the lobby or during play.
    node(&document, &body, "h3", "", "Table settings")?;
    // The label names the state the table is in; pressing it asks for the other state.
    let shown = !hidden;
    let hidden_after_press = !hidden;
    let toggle = button(
        client,
        &body,
        &format!(
            "Show choices on the TV: {}",
            if shown { "on" } else { "off" }
        ),
        move |client| {
            close_menu(&client);
            match table_setting(&client, hidden_after_press) {
                Ok(request) => send(client, request, false),
                Err(error) => feedback(&client.borrow(), &error),
            }
        },
    )?;
    toggle.set_attribute("aria-pressed", if shown { "true" } else { "false" })?;
    let credits = node(&document, &body, "a", "live-credits", "Credits")?;
    credits.set_attribute("href", CREDITS_PATH)?;
    // The hero tab appears only when another screen provides the character sheet section.
    if document.get_element_by_id(HERO_SHEET_ID).is_some() {
        hero.remove_attribute("hidden")?;
    } else {
        hero.set_attribute("hidden", "")?;
    }
    Ok(())
}
