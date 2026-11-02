//! The files the page reads from the host that are not the controls or the canvas: the theme, the
//! list of scenarios and the scenario chosen.

use super::sound::sync_audio;
use super::state::{
    FIXTURE_DIRECTORY_ADDRESS, FIXTURE_LIST_ADDRESS, FixtureProblem, PageError, REVISION_ADDRESS,
    THEME_ADDRESS, THEME_ID, log_error, set_problem, with_app, with_app_mut,
};
use crate::decisions::scene_changed;
use crate::dom::{document, js_message, with_continuity};
use crate::draw_plan::transition_for;
use crate::fixture::Fixture;
use crate::net::fetch_text;
use wasm_bindgen::JsValue;

fn set_theme(css: &str) -> Result<(), JsValue> {
    let document = document()?;
    let style = document
        .get_element_by_id(THEME_ID)
        .ok_or_else(|| JsValue::from_str("the theme element is missing"))?;
    style.set_text_content(Some(css));
    Ok(())
}

/// Reads the theme and the fixture list, and remembers the revision they came from.
pub async fn refresh_assets() {
    match fetch_text(THEME_ADDRESS).await {
        Ok(css) => {
            if let Err(error) = set_theme(&css) {
                log_error(&format!("could not set the theme: {}", js_message(&error)));
            }
        }
        Err(error) => set_problem(error),
    }
    match fetch_text(FIXTURE_LIST_ADDRESS).await {
        Ok(text) => match serde_json::from_str::<Vec<String>>(&text) {
            Ok(names) => with_app_mut(|app| {
                app.connection_problem = None;
                app.fixture_names = names;
            }),
            Err(error) => set_problem(PageError::NotRead {
                address: FIXTURE_LIST_ADDRESS,
                wanted: "a list",
                reason: error.to_string(),
            }),
        },
        Err(error) => set_problem(error),
    }
    match fetch_text(REVISION_ADDRESS).await {
        Ok(revision) => with_app_mut(|app| app.asset_revision = revision),
        Err(error) => log_error(&format!("could not read the revision: {error}")),
    }
}

/// Reads the fixture the controls name, draws nothing, and sets the transition for the next draw
/// when the scene has changed.
pub async fn load_selected_fixture() {
    let Some(name) = with_app(|app| app.controls.fixture.clone()) else {
        with_app_mut(|app| {
            app.fixture = None;
            app.loaded_name = None;
            app.reshow();
        });
        return;
    };
    let address = format!("{FIXTURE_DIRECTORY_ADDRESS}{name}");
    let loaded = match fetch_text(&address).await {
        Ok(text) => Fixture::parse(&text).map_err(|error| FixtureProblem::Invalid {
            name: name.clone(),
            error,
        }),
        Err(error) => Err(FixtureProblem::Fetch(error)),
    };
    with_app_mut(|app| {
        let transition = transition_for(app.loaded_name.as_deref(), &name, app.controls.transition);
        if scene_changed(app.loaded_name.as_deref(), &name) {
            with_continuity(|continuity| continuity.forget());
        }
        if transition.is_some() {
            app.pending_transition = transition;
        }
        app.fixture = Some(loaded);
        app.loaded_name = Some(name);
        app.reshow();
    });
    // A scene brings its own music, so the soundtrack changes with it.
    sync_audio();
}
