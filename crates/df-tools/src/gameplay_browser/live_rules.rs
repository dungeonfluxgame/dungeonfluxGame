//! The scene's rules wording, shown on its own line under the story narration of the phone's
//! exploration screen and of the combat and aftermath screens.
//!
//! The server sends the story and the rules note separately (`GameplayScene.rules_note`), so
//! the story is the only text that is narrated and spoken and the rules read as a quieter
//! aside. The screens themselves belong to `df-ui`; this only adds one line at the end of the
//! narration paragraph they already draw and removes it again when the scene has no note.
use super::*;

/// The narration paragraph of each screen this module annotates.
const NARRATION: &str = ".combat-narration, .aftermath-narrative, .hero .description";

/// Puts `note` under the narration inside `host`, or removes the line when `note` is empty.
pub(super) fn mount(document: &Document, host: &Element, note: &str) -> Result<(), JsValue> {
    if let Some(previous) = host.query_selector(".rules-note")? {
        previous.remove();
    }
    if note.is_empty() {
        return Ok(());
    }
    let Some(narration) = host.query_selector(NARRATION)? else {
        return Ok(());
    };
    // Inside the narration paragraph: its container is laid out by the screen and may not be
    // a block, while a line within the paragraph always follows the story text.
    let line = document.create_element("span")?;
    line.set_class_name("rules-note");
    node(document, &line, "span", "rules-note-tag", "Rules")?;
    node(document, &line, "span", "rules-note-text", note)?;
    narration.append_child(&line)?;
    Ok(())
}
