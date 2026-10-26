//! Rebind cross-sheet navigation after the page renderer clears its click callbacks.
use super::*;

const ACTION_PLACEHOLDER: &str = "Type or speak what you do, e.g. I ask Finn about the boot";

pub(super) fn bind(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let root = client.borrow().root.clone();
    // Exploration has no draft offer; its generic df-ui draft is superseded by the one
    // authenticated dialogue editor. Remove the unused controls instead of hiding duplicates.
    let unused = root.query_selector_all(
        ".df-exploration .exploration-submit, .df-exploration .exploration-actions .df-ui-field",
    )?;
    for index in 0..unused.length() {
        if let Some(node) = unused.item(index) {
            node.unchecked_into::<Element>().remove();
        }
    }
    simplify_composer(&root)?;
    label_inspection_submit(&root)?;
    let tabs = root.query_selector_all(".live-tab")?;
    for index in 0..tabs.length() {
        let Some(tab) = tabs.item(index) else {
            continue;
        };
        let tab = tab.dyn_into::<Element>()?;
        let label = tab.text_content().unwrap_or_default();
        tab.set_attribute("aria-label", &label)?;
        let root = root.clone();
        let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if let Ok(sheets) = root.query_selector_all(".live-sheet") {
                for index in 0..sheets.length() {
                    let Some(sheet) = sheets.item(index) else {
                        continue;
                    };
                    let sheet = sheet.unchecked_into::<Element>();
                    let keep = match label.as_str() {
                        "Notes" => sheet.class_list().contains("live-notebook-sheet"),
                        "Menu" => !sheet.class_list().contains("live-notebook-sheet"),
                        _ => false,
                    };
                    if !keep {
                        let _ = sheet.remove_attribute("data-open");
                    }
                }
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        tab.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        client.borrow_mut().callbacks.push((tab, callback));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_action_composer_has_one_entry_and_a_send_button() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let dock = node(&document, &root, "div", "live-dock", "").unwrap();
        let details = node(&document, &dock, "details", "say-more", "").unwrap();
        node(&document, &details, "summary", "", "Say something else…").unwrap();
        let panel = node(&document, &dock, "section", "dialogue-recovery", "").unwrap();
        node(&document, &panel, "textarea", "dialogue-draft", "").unwrap();
        let send = node(&document, &panel, "button", "", "Say it").unwrap();
        send.set_attribute("data-dialogue", "submit").unwrap();
        let actions = node(&document, &dock, "div", "scene-actions", "").unwrap();
        let inspection = node(&document, &actions, "div", "offered-action", "").unwrap();
        let inspect = node(&document, &inspection, "button", "offered", "Look around").unwrap();
        inspect
            .set_attribute(
                "data-action-kind",
                &(rpc::GameplayActionKind::ExamineHarborSeal as i32).to_string(),
            )
            .unwrap();

        simplify_composer(&root).unwrap();
        label_inspection_submit(&root).unwrap();

        assert!(
            dock.query_selector("details.say-more > summary")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
        assert_eq!(
            panel
                .query_selector(".dialogue-draft")
                .unwrap()
                .unwrap()
                .get_attribute("placeholder")
                .as_deref(),
            Some(ACTION_PLACEHOLDER)
        );
        assert_eq!(send.text_content().as_deref(), Some("Send"));
        assert_eq!(inspect.text_content().as_deref(), Some("Do this"));
    }
}

fn simplify_composer(root: &Element) -> Result<(), JsValue> {
    if let Some(summary) = root.query_selector(".live-dock details.say-more > summary")? {
        summary.set_attribute("hidden", "")?;
    }
    if let Some(editor) = root.query_selector(".dialogue-recovery .dialogue-draft")? {
        editor.set_attribute("placeholder", ACTION_PLACEHOLDER)?;
    }
    if let Some(send) = root.query_selector(".dialogue-recovery [data-dialogue=submit]")? {
        send.set_text_content(Some("Send"));
    }
    Ok(())
}

fn label_inspection_submit(root: &Element) -> Result<(), JsValue> {
    let buttons = root.query_selector_all(".offered-action > button.offered[data-action-kind]")?;
    let inspection_kind = (rpc::GameplayActionKind::ExamineHarborSeal as i32).to_string();
    for index in 0..buttons.length() {
        let Some(button) = buttons
            .item(index)
            .and_then(|item| item.dyn_into::<Element>().ok())
        else {
            continue;
        };
        if button.get_attribute("data-action-kind").as_deref() == Some(inspection_kind.as_str()) {
            button.set_text_content(Some("Do this"));
        }
    }
    Ok(())
}
