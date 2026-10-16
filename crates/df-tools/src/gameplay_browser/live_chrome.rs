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
    notebook_body: Element,
    hero: Element,
    notes: Element,
    listeners: Vec<Listener>,
    notebook_listeners: Vec<Listener>,
}

/// The player's persistent tab bar and menu sheet.
#[derive(Default)]
pub(super) struct Chrome {
    mounted: Option<Mounted>,
}

impl Chrome {
    pub(super) fn dispose(&mut self) {
        if let Some(mounted) = self.mounted.take() {
            for (element, callback) in mounted
                .listeners
                .into_iter()
                .chain(mounted.notebook_listeners)
            {
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
    let (document, root, player, room) = {
        let state = client.borrow();
        (
            state.document.clone(),
            state.root.clone(),
            state.role == "player",
            state.room_code.clone(),
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
    let notes = tab(&document, &nav, "icon-journal", "Notes")?;
    notes.set_attribute("data-room", &room)?;
    let menu = tab(&document, &nav, "icon-menu", "Menu")?;
    let sheet = node(&document, &root, "div", "live-sheet", "")?;
    let panel = node(&document, &sheet, "div", "live-sheet-panel", "")?;
    node(&document, &panel, "h2", "", "Menu")?;
    let body = node(&document, &panel, "div", "live-sheet-body", "")?;
    let close = node(&document, &panel, "button", "live-sheet-close", "Close")?;
    close.set_attribute("type", "button")?;
    let notebook_sheet = node(
        &document,
        &root,
        "div",
        "live-sheet live-notebook-sheet",
        "",
    )?;
    let notebook_panel = node(&document, &notebook_sheet, "div", "live-sheet-panel", "")?;
    node(&document, &notebook_panel, "h2", "", "Notebook")?;
    let notebook_body = node(&document, &notebook_panel, "div", "live-notebook-body", "")?;
    node(
        &document,
        &notebook_body,
        "p",
        "live-notebook-empty",
        "Your notes will appear as the story unfolds.",
    )?;
    let notebook_close = node(
        &document,
        &notebook_panel,
        "button",
        "live-sheet-close",
        "Close",
    )?;
    notebook_close.set_attribute("type", "button")?;
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
    listen(&mut listeners, &notes, {
        let notebook_sheet = notebook_sheet.clone();
        let notes = notes.clone();
        move |_| {
            if notebook_sheet.has_attribute("data-open") {
                let _ = notebook_sheet.remove_attribute("data-open");
                return;
            }
            let _ = notebook_sheet.set_attribute("data-open", "");
            mark_notebook_seen(&notes);
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
    listen(&mut listeners, &notebook_close, {
        let notebook_sheet = notebook_sheet.clone();
        move |_| {
            let _ = notebook_sheet.remove_attribute("data-open");
        }
    })?;
    listen(&mut listeners, &notebook_sheet, {
        let closing = notebook_sheet.clone();
        move |event| {
            let outside = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| target.is_same_node(Some(closing.as_ref())));
            if outside {
                let _ = closing.remove_attribute("data-open");
            }
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
        notebook_body,
        hero,
        notes,
        listeners,
        notebook_listeners: Vec::new(),
    });
    Ok(())
}

pub(super) fn update_notebook_unread(client: &Rc<RefCell<Client>>, latest: u64) {
    let Some(notes) = client
        .borrow()
        .chrome
        .mounted
        .as_ref()
        .map(|mounted| mounted.notes.clone())
    else {
        return;
    };
    let _ = notes.set_attribute("data-latest-order", &latest.to_string());
    let seen = notebook_seen(&notes);
    if latest > seen {
        let _ = notes.set_attribute("data-unread", "");
    } else {
        let _ = notes.remove_attribute("data-unread");
    }
}

fn notebook_key(notes: &Element) -> String {
    format!(
        "df.gameplay.{}.notebook.seen",
        notes.get_attribute("data-room").unwrap_or_default()
    )
}

fn notebook_seen(notes: &Element) -> u64 {
    web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .and_then(|storage| storage.get_item(&notebook_key(notes)).ok().flatten())
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

fn mark_notebook_seen(notes: &Element) {
    let latest = notes
        .get_attribute("data-latest-order")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    if let Some(storage) =
        web_sys::window().and_then(|window| window.session_storage().ok().flatten())
    {
        let _ = storage.set_item(&notebook_key(notes), &latest.to_string());
    }
    let _ = notes.remove_attribute("data-unread");
}

pub(super) fn notebook_body(client: &Rc<RefCell<Client>>) -> Option<Element> {
    client
        .borrow()
        .chrome
        .mounted
        .as_ref()
        .map(|mounted| mounted.notebook_body.clone())
}

pub(super) fn clear_notebook_callbacks(client: &Rc<RefCell<Client>>) {
    let listeners = client
        .borrow_mut()
        .chrome
        .mounted
        .as_mut()
        .map(|mounted| std::mem::take(&mut mounted.notebook_listeners))
        .unwrap_or_default();
    for (element, callback) in listeners {
        let _ =
            element.remove_event_listener_with_callback("click", callback.as_ref().unchecked_ref());
    }
}

pub(super) fn listen_notebook(
    client: &Rc<RefCell<Client>>,
    element: &Element,
    mut activate: impl FnMut(Event) + 'static,
) -> Result<(), JsValue> {
    let callback =
        Closure::wrap(Box::new(move |event: Event| activate(event)) as Box<dyn FnMut(Event)>);
    element.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
    if let Some(mounted) = client.borrow_mut().chrome.mounted.as_mut() {
        mounted.notebook_listeners.push((element.clone(), callback));
        Ok(())
    } else {
        let _ =
            element.remove_event_listener_with_callback("click", callback.as_ref().unchecked_ref());
        Err(JsValue::from_str("notebook controls are not mounted"))
    }
}

pub(super) fn set_notebook_room(client: &Rc<RefCell<Client>>, room: &str) {
    if let Some(notes) = client
        .borrow()
        .chrome
        .mounted
        .as_ref()
        .map(|mounted| mounted.notes.clone())
    {
        let _ = notes.set_attribute("data-room", room);
    }
}

fn close_menu(client: &Rc<RefCell<Client>>) {
    if let Some(mounted) = client.borrow().chrome.mounted.as_ref() {
        let _ = mounted.sheet.remove_attribute("data-open");
    }
}

#[cfg(test)]
mod tests {
    use super::super::has_unconfirmed_saved_choice;

    #[test]
    fn menu_reoffers_only_an_unconfirmed_saved_choice() {
        assert!(has_unconfirmed_saved_choice(true, false, false));
        assert!(!has_unconfirmed_saved_choice(false, false, false));
        assert!(!has_unconfirmed_saved_choice(true, true, false));
        assert!(!has_unconfirmed_saved_choice(true, false, true));
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

/// Rebuilds the player's menu items for the current state; the display has no menu.
pub(super) fn fill_menu(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, needs, confirm_strike, body, hero) = {
        let state = client.borrow();
        let Some(mounted) = state.chrome.mounted.as_ref() else {
            return Ok(());
        };
        (
            state.document.clone(),
            recovery_needs(&state),
            state.first_attack.is_some() && {
                let needs = recovery_needs(&state);
                needs.lost_connection || needs.unknown_choice
            },
            mounted.body.clone(),
            mounted.hero.clone(),
        )
    };
    body.set_text_content(None);
    button(client, &body, "Reconnect", |client| {
        close_menu(&client);
        connect(client);
    })?;
    if needs.unknown_choice {
        button(client, &body, "Send my last choice again", |client| {
            close_menu(&client);
            replay(client, false);
        })?;
    }
    if confirm_strike {
        button(
            client,
            &body,
            "Check that your last attack was received",
            |client| {
                close_menu(&client);
                replay(client, true);
            },
        )?;
    }
    button(client, &body, LEAVE_LABEL, |client| {
        close_menu(&client);
        leave_saved_game(client);
    })?;
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
