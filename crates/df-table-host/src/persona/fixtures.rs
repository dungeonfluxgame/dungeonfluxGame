//! Helpers the persona tests share: a host that shows a scene with one named character, a persona
//! router over a scripted text backend, and the stored files to compare.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use df_ai::text::TextOutcome;
use df_ai::text_fixture::ScriptedBackend;
use df_content::persona::Persona;
use serde_json::{Value, json};

use super::routes::router_with;
use crate::test_support::{Running, STARTER_FILES, layout_of, start};
use crate::text::{BackendChoice, TextRuntime};

/// The character the scene names.
pub(super) const CHARACTER: &str = "innkeeper";

/// A host showing a scene whose backdrop names [`CHARACTER`], and a persona router over a text
/// backend that answers with a script.
pub(super) struct ScriptedHost {
    pub(super) running: Running,
    /// The port of the persona router.
    pub(super) port: u16,
    /// How many questions the text backend was asked.
    pub(super) calls: Arc<AtomicUsize>,
}

impl ScriptedHost {
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn directory(&self) -> PathBuf {
        self.running.playground_path("assets/personas")
    }

    /// The text of a file of the `personas` folder, or none when it is not there.
    pub(super) fn file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.directory().join(name)).ok()
    }

    /// Every `PERSONA` audit line so far.
    pub(super) fn persona_lines(&self) -> Vec<String> {
        self.lines_starting_with("PERSONA")
    }

    pub(super) fn lines_starting_with(&self, start: &str) -> Vec<String> {
        self.running
            .state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with(start))
            .collect()
    }
}

/// The persona the tests store for [`CHARACTER`]: public backstory and personality, private fears
/// and backstory.
pub(super) fn innkeeper() -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": CHARACTER, "display_name": "Marta Venn", "job": "Keeper of the inn",
        "personality": ["warm but shrewd"],
        "fears": ["the guild collector"],
        "backstory": "Took over the Ember Roads inn from her father.",
        "private_backstory": "Her brother Tomas vanished going north."
    }))
    .unwrap()
}

/// A host whose text backend answers with `answers`, in order.
pub(super) async fn scripted_host(answers: Vec<Value>) -> ScriptedHost {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    crate::locate::write_layout(
        &running.playground_path("assets/layouts"),
        "plate.png",
        &layout_of("plate.png", CHARACTER),
    )
    .unwrap();
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "Roadside inn", "description": "A fire, a table, four travellers.", "#,
            r#""tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let script = ScriptedBackend::new(
        answers.into_iter().map(TextOutcome::Answered),
        Arc::clone(&calls),
    );
    let recorder = Arc::clone(&running.state);
    let text = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    );
    let app = router_with(Arc::clone(&running.state), Arc::new(text));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    ScriptedHost {
        running,
        port,
        calls,
    }
}

/// The answer to a coverage question with `gaps` (field, topic).
pub(super) fn coverage_answer(gaps: &[(&str, &str)]) -> Value {
    json!({
        "enough": gaps.is_empty(),
        "gaps": gaps.iter().map(|(field, topic)| json!({"field": field, "topic": topic}))
            .collect::<Vec<_>>(),
    })
}

/// The answer to a line question.
pub(super) fn line_answer(line: &str) -> Value {
    json!({
        "line": line, "behavior": null, "fired_triggers": [], "revealed_secrets": []
    })
}

/// The answer to a backfill question that appends `text` to the backstory.
pub(super) fn append_backstory_answer(text: &str) -> Value {
    json!({"edits": [{
        "op": "append_text", "field": "backstory", "text": text, "id": null,
        "sensitivity": null, "reveal_conditions": null, "known_by": null,
        "speech_style": null, "condition": null, "event": null, "behavior": null
    }]})
}
