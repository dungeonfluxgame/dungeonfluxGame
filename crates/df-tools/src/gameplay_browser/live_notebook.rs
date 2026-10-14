//! The phone's server-backed notebook. Entries are text-only and controls submit commands to
//! the same dialogue owner path used by the ordinary composer.
use super::*;
use df_ui::{NotebookEntryView, NotebookKind, NotebookView};
use wasm_bindgen::JsValue;

const SECTIONS: [(NotebookKind, &str, &str); 5] = [
    (NotebookKind::Person, "People", "No people recorded yet."),
    (NotebookKind::Place, "Places", "No places recorded yet."),
    (NotebookKind::Task, "Task", "No task recorded yet."),
    (NotebookKind::Clue, "Clues", "No clues recorded yet."),
    (NotebookKind::Note, "Party notes", "No party notes yet."),
];

pub(super) fn view(journey: &rpc::JourneyView) -> Result<NotebookView, JsValue> {
    if journey.notebook.len() > df_ui::MAX_NOTEBOOK_ENTRIES {
        return Err(JsValue::from_str("notebook entry count exceeded"));
    }
    let entries = journey
        .notebook
        .iter()
        .map(|entry| {
            let kind = match rpc::NotebookKind::try_from(entry.kind)
                .map_err(|_| JsValue::from_str("notebook kind invalid"))?
            {
                rpc::NotebookKind::Person => NotebookKind::Person,
                rpc::NotebookKind::Place => NotebookKind::Place,
                rpc::NotebookKind::Task => NotebookKind::Task,
                rpc::NotebookKind::Clue => NotebookKind::Clue,
                rpc::NotebookKind::Note => NotebookKind::Note,
                rpc::NotebookKind::Unspecified => {
                    return Err(JsValue::from_str("notebook kind invalid"));
                }
            };
            Ok(NotebookEntryView {
                kind,
                title: entry.title.clone(),
                text: entry.text.clone(),
                scene: entry.scene.clone(),
                order: entry.order,
                attitude: entry.attitude.clone(),
                owner: entry.owner.clone(),
                author: entry.author.clone(),
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let notebook = NotebookView { entries };
    notebook
        .validate()
        .map_err(|_| JsValue::from_str("notebook view invalid"))?;
    Ok(notebook)
}

fn input_value(element: &Element) -> Result<String, JsValue> {
    js_sys::Reflect::get(element.as_ref(), &JsValue::from_str("value"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("notebook input value unavailable"))
}

fn set_input_value(element: &Element, value: &str) -> Result<(), JsValue> {
    js_sys::Reflect::set(
        element.as_ref(),
        &JsValue::from_str("value"),
        &JsValue::from_str(value),
    )?;
    Ok(())
}

fn add_entry(
    document: &Document,
    parent: &Element,
    entry: &NotebookEntryView,
    own_hero: &str,
    client: &Rc<RefCell<Client>>,
) -> Result<(), JsValue> {
    let card = node(document, parent, "article", "live-notebook-entry", "")?;
    node(document, &card, "h4", "", &entry.title)?;
    node(document, &card, "p", "", &entry.text)?;
    if let Some(attitude) = &entry.attitude {
        node(
            document,
            &card,
            "p",
            "live-notebook-attitude",
            &format!("Attitude toward you: {attitude}."),
        )?;
    }
    if let Some(author) = &entry.author {
        let label = if entry.kind == NotebookKind::Clue {
            format!("Found by {author}.")
        } else {
            format!("Added by {author}.")
        };
        node(document, &card, "p", "live-notebook-author", &label)?;
    }
    node(
        document,
        &card,
        "p",
        "live-notebook-scene",
        &format!("Scene: {}", entry.scene.replace('_', " ")),
    )?;
    if entry.kind == NotebookKind::Clue && entry.owner.as_deref() == Some(own_hero) {
        let share = node(document, &card, "button", "", "Share with the party")?;
        share.set_attribute("type", "button")?;
        let weak = Rc::downgrade(client);
        let order = entry.order;
        live_chrome::listen_notebook(client, &share, move |_| {
            if let Some(client) = weak.upgrade() {
                dialogue_recovery::submit_notebook_command(client, format!("share clue {order}"));
            }
        })?;
    }
    Ok(())
}

/// Rebuilds the notebook from the current accepted player view. Chrome owns callbacks separately
/// from the page callbacks because render clears the latter on every redraw.
pub(super) fn render(
    client: &Rc<RefCell<Client>>,
    notebook: &NotebookView,
    own_hero: &str,
) -> Result<(), JsValue> {
    notebook
        .validate()
        .map_err(|_| JsValue::from_str("notebook view invalid"))?;
    let Some(body) = live_chrome::notebook_body(client) else {
        return Ok(());
    };
    let draft = body
        .query_selector("[data-note-editor]")?
        .map(|input| input_value(&input))
        .transpose()?
        .unwrap_or_default();
    live_chrome::clear_notebook_callbacks(client);
    body.set_text_content(None);
    let document = client.borrow().document.clone();
    for (kind, title, empty) in SECTIONS {
        let section = node(&document, &body, "section", "live-notebook-section", "")?;
        node(&document, &section, "h3", "", title)?;
        let entries: Vec<&NotebookEntryView> = notebook
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .collect();
        if entries.is_empty() {
            node(&document, &section, "p", "live-notebook-empty", empty)?;
        } else {
            for entry in entries {
                add_entry(&document, &section, entry, own_hero, client)?;
            }
        }
    }
    let composer = node(&document, &body, "div", "live-notebook-composer", "")?;
    node(
        &document,
        &composer,
        "label",
        "",
        "Add a note to the party notebook",
    )?;
    let input = node(&document, &composer, "input", "", "")?;
    input.set_attribute("type", "text")?;
    input.set_attribute("maxlength", "4096")?;
    input.set_attribute("autocomplete", "off")?;
    input.set_attribute("data-note-editor", "")?;
    input.set_attribute("aria-label", "Party note")?;
    set_input_value(&input, &draft)?;
    let submit = node(&document, &composer, "button", "", "Note it")?;
    submit.set_attribute("type", "button")?;
    let weak = Rc::downgrade(client);
    let editor = input.clone();
    live_chrome::listen_notebook(client, &submit, move |_| {
        let Ok(text) = input_value(&editor) else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        if let Some(client) = weak.upgrade() {
            // Empty the box first: sending redraws the notebook, and the redraw carries the
            // box's text over as a draft, so clearing afterwards cleared a box already replaced.
            let _clear = set_input_value(&editor, "");
            dialogue_recovery::submit_notebook_command(client, format!("note: {text}"));
        }
    })?;
    if let Some(latest) = notebook.entries.iter().map(|entry| entry.order).max() {
        live_chrome::update_notebook_unread(client, latest);
    } else {
        live_chrome::update_notebook_unread(client, 0);
    }
    live_chrome::set_notebook_room(client, &client.borrow().room_code);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::view;
    use df_protocol::common as rpc;

    // SRD 5.2.1 p. N/A: this checks narrative view mapping, not a game rule.
    #[test]
    fn server_notebook_entries_map_to_the_phone_view() {
        let journey = rpc::JourneyView {
            notebook: vec![rpc::NotebookEntry {
                kind: rpc::NotebookKind::Person as i32,
                title: "Tomas Greaves".to_owned(),
                text: "Mapmaker at Tin Street Tea House.".to_owned(),
                scene: "Jorun's Quiet Warning".to_owned(),
                order: 1,
                attitude: Some("friendly".to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let notebook = view(&journey).unwrap();
        assert_eq!(notebook.entries.len(), 1);
        assert_eq!(notebook.entries[0].title, "Tomas Greaves");
        assert_eq!(notebook.entries[0].scene, "Jorun's Quiet Warning");
        assert_eq!(notebook.entries[0].attitude.as_deref(), Some("friendly"));
    }

    // SRD 5.2.1 p. N/A: this checks bounded narrative fields, not a game rule.
    #[test]
    fn phone_view_refuses_an_unknown_notebook_kind() {
        let journey = rpc::JourneyView {
            notebook: vec![rpc::NotebookEntry {
                kind: 99,
                title: "unknown".to_owned(),
                text: "unknown".to_owned(),
                scene: "opening".to_owned(),
                order: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(view(&journey).is_err());
    }
}
