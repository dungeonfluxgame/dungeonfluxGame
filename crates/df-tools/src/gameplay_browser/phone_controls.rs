//! Rebind cross-sheet navigation after the page renderer clears its click callbacks.
use super::*;

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
