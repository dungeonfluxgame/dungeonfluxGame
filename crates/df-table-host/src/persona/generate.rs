//! What the text service does for personas: writes the persona of a character of the shown scene
//! and the shared helpers of every persona question; `line` words one line.
//!
//! Every question goes through a [`TextRoute`] (the persona model, or the one a request names;
//! `TextRuntime::persona_route`) and `TextRoute::ask`, under the scope [`PERSONA_SCOPE`]. The
//! operation key is the kind of question and a process-wide counter: each model call is a
//! deliberately fresh operation, a creative request and not an idempotent command, so a retry
//! pays again and never replays an earlier answer (owner decision 2026-11-06). The persona
//! question is `df_content::persona::generation`; the line question is in `line`, and is a dev
//! tool:
//! the game feeds the same briefs into the expression contract, and the production engine, not the
//! model, owns whether a secret is revealed. An audit line names the character and the operation,
//! never the hint, the situation, a persona or a line. `backfill` holds the questions about what a
//! persona lacks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_ai::text::{TextProfile, TextPurpose, TextRequest};
use df_ai::text_attempts::OperationKey;
use df_content::persona::backfill::NarrativeGuidance;
use df_content::persona::generation::{
    self, GenerationError, GenerationRequest, INSTRUCTION, OUTPUT_NAME, SceneContext,
};
use df_content::persona::privacy::{LEAK_REPORT_INSTRUCTION, split_leak_report, with_leak_report};
use df_content::persona::voice::{
    VOICE_INSTRUCTION, VOICE_OUTPUT_NAME, read_voice, voice_facts, voice_schema,
};
use df_content::persona::{Persona, PersonaEdit, PersonaId, PersonaState};
use futures_util::future::join_all;
use serde_json::Value;
use tokio::time::Instant;

use super::handles::Handles;
use super::request::{CastRow, Generation, PersonaError, summary_of};
use super::storage::{persona_directory, persona_id, read_persona, store_generated};
use crate::blocking::run_blocking;
use crate::shown_scene::{ShownCast, ShownSceneError, named, shown_cast};
use crate::state::AppState;
use crate::text::{TEXT_DEADLINE, TextRoute, TextRuntime};

/// The text-service scope of every persona question.
pub const PERSONA_SCOPE: &str = "persona";

/// The note every prompt that carries world facts holds: prices and stock of goods offered for
/// sale are public, so nothing hides them from a line (an entry marked `audience: private` in
/// the world bible is the designer's exception).
pub(super) const PUBLIC_NOTE: &str = concat!(
    "Prices and stock of goods offered for sale are public: anyone in the scene may know and ",
    "state them. person-1, person-2 and so on, and someone, stand for people whose name the ",
    "character does not know: they are no names, so never say one aloud or write one into a ",
    "gap, a gist or a detail; speak of such a person by what can be seen of them."
);

/// Gives each question its own operation.
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(0);

/// The operation key of one question of `kind`: new on every call. It holds the kind and the
/// counter only, so it is always inside the 128-byte label rule whatever the character is called.
pub(crate) fn operation_key(kind: &str) -> Result<OperationKey, PersonaError> {
    // Relaxed: the counter only has to hand out distinct numbers.
    let ordinal = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    TextRuntime::key(PERSONA_SCOPE, &format!("{kind}:{ordinal}"))
        .ok_or(PersonaError::InvalidOperationKey)
}

/// The route a persona question goes through: refused when the text service is not set up or the
/// model name is not usable.
pub(super) fn persona_route(
    text: &TextRuntime,
    model: Option<&str>,
) -> Result<Arc<TextRoute>, PersonaError> {
    if let Some(missing) = text.missing() {
        return Err(PersonaError::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    text.persona_route(model)
        .map_err(|cause| PersonaError::InvalidModel { cause })
}

/// The shown scene as a persona question is told it: its characters and what it is.
pub(super) struct ShownScene {
    cast: ShownCast,
}

impl ShownScene {
    /// The scene the page shows, or why it has no named characters to read.
    pub(super) async fn read(app: &AppState) -> Result<Self, ShownSceneError> {
        Ok(Self {
            cast: shown_cast(app).await?,
        })
    }

    /// The named characters, left to right.
    pub(super) fn names(&self) -> Vec<String> {
        self.cast
            .layout
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// That `character` is a named character of the scene, or the refusal listing the ones that
    /// are.
    pub(super) fn check(&self, character: &str) -> Result<(), ShownSceneError> {
        named(&self.cast.layout, &self.cast.picture, character)?;
        Ok(())
    }
}

/// The stored persona of `character`, or `UnknownPersona`.
pub(super) async fn load_persona(app: &AppState, character: &str) -> Result<Persona, PersonaError> {
    let id = persona_id(character)?;
    let directory = persona_directory(app);
    run_blocking(
        move || {
            read_persona(&directory, &id)?
                .ok_or_else(|| PersonaError::UnknownPersona { id: id.to_string() })
        },
        PersonaError::Stopped,
    )
    .await
}

/// The stored personas of the cast other than `character`, in cast order.
pub(super) async fn other_personas(
    app: &AppState,
    scene: &ShownScene,
    character: &str,
) -> Result<Vec<Persona>, PersonaError> {
    let others: Vec<String> = scene
        .names()
        .into_iter()
        .filter(|name| name != character)
        .collect();
    let directory = persona_directory(app);
    run_blocking(
        move || {
            let mut personas = Vec::new();
            for name in &others {
                // A cast name is a layout label, which has the persona id grammar (a test keeps
                // the two in step).
                let Ok(other) = PersonaId::new(name) else {
                    continue;
                };
                personas.extend(read_persona(&directory, &other)?);
            }
            Ok(personas)
        },
        PersonaError::Stopped,
    )
    .await
}

/// The text request of a question under `profile`.
pub(crate) fn text_request(
    profile: &TextProfile,
    instructions: &str,
    facts: Value,
    output_name: &str,
    output_schema: Value,
) -> TextRequest {
    TextRequest {
        purpose: TextPurpose::Storyteller,
        profile: profile.clone(),
        instructions: instructions.to_owned(),
        facts,
        output_name: output_name.to_owned(),
        output_schema,
        deadline: TEXT_DEADLINE,
        images: Vec::new(),
    }
}

/// `facts` without the fields that say nothing: a null, an empty text, an empty list or an empty
/// object, at every depth. A number or a flag, `0` and `false` included, always stays: it is a
/// value. Fewer tokens, the same information.
pub(super) fn compact(facts: Value) -> Value {
    match facts {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, compact(value)))
                .filter(|(_, value)| !says_nothing(value))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(compact)
                .filter(|value| !says_nothing(value))
                .collect(),
        ),
        other => other,
    }
}

fn says_nothing(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

/// How a persona is written.
#[derive(Clone, Copy)]
pub(super) struct Writing<'a> {
    pub(super) hint: Option<&'a str>,
    pub(super) guidance: Option<&'a NarrativeGuidance>,
    pub(super) model: Option<&'a str>,
    pub(super) overwrite: bool,
}

/// Writes the persona of the checked character `character` of `scene`, unless one is stored and
/// `overwrite` is not asked. A failure is recorded as `PERSONA generate_failed`.
pub(super) async fn generate_character(
    app: &AppState,
    text: &TextRuntime,
    scene: &ShownScene,
    character: &str,
    writing: Writing<'_>,
) -> Result<Generation, PersonaError> {
    let result = generate(app, text, scene, character, writing).await;
    if let Err(error) = &result {
        app.record_line(format!(
            "PERSONA generate_failed id={character} error={}",
            error.key()
        ));
    }
    result
}

/// The world's active place as the scene a new character is written into: its name and
/// description, both public; empty when no place is active.
fn place_context(app: &AppState) -> SceneContext {
    let facts = crate::world::context_for(app, &[]).facts;
    let field = |key: &str| facts["place"][key].as_str().unwrap_or_default().to_owned();
    SceneContext {
        title: field("name"),
        description: field("description"),
    }
}

/// The others of `scene` as a new character, who knows no one yet, is told of them: handles.
fn stranger_cast(id: &PersonaId, scene: &ShownScene) -> Vec<String> {
    let names = scene.names();
    let fresh = PersonaState {
        id: id.to_string(),
        ..PersonaState::default()
    };
    let handles = Handles::new(&fresh, names.iter().map(String::as_str));
    names
        .iter()
        .filter(|name| name.as_str() != id.as_str())
        .map(|name| handles.shown(name))
        .collect()
}

/// The stored persona of `id` as a kept outcome, recorded as `PERSONA kept`.
fn kept(app: &AppState, persona: &Persona) -> Generation {
    app.record_line(format!("PERSONA kept id={}", persona.id()));
    Generation::KeptExisting {
        persona: summary_of(persona),
    }
}

async fn generate(
    app: &AppState,
    text: &TextRuntime,
    scene: &ShownScene,
    character: &str,
    writing: Writing<'_>,
) -> Result<Generation, PersonaError> {
    let id = persona_id(character)?;
    let directory = persona_directory(app);
    if !writing.overwrite {
        let (kept_in, kept_id) = (directory.clone(), id.clone());
        let stored = run_blocking(
            move || read_persona(&kept_in, &kept_id),
            PersonaError::Stopped,
        )
        .await?;
        if let Some(persona) = stored {
            return Ok(kept(app, &persona));
        }
    }
    let route = persona_route(text, writing.model)?;
    let asked = GenerationRequest {
        character: id.clone(),
        hint: writing.hint.map(str::to_owned),
        // Where they are is the world's active place, never the fixture's words (an author note
        // that may name people); the others are strangers to a new character, so they are
        // handles, as every prompt shows them (`handles`).
        scene: place_context(app),
        cast: stranger_cast(&id, scene),
        guidance: writing.guidance.cloned(),
        available_packs: crate::world::available_packs(app),
    };
    let request = text_request(
        route.profile(),
        &format!("{INSTRUCTION} {LEAK_REPORT_INSTRUCTION}"),
        generation::facts(&asked),
        OUTPUT_NAME,
        with_leak_report(generation::output_schema()),
    );
    let key = operation_key("generate")?;
    app.record_line(format!(
        "PERSONA generate_started id={id} operation={}",
        key.operation()
    ));
    let started = Instant::now();
    let (persona, leak_suspect) = route
        .ask(&key, &request, |answer| {
            let (persona_answer, report) =
                split_leak_report(answer).map_err(|error| GenerationError::Malformed {
                    detail: error.to_string(),
                })?;
            Ok::<_, GenerationError>((
                generation::read_answer(&persona_answer, &asked)?,
                report.leaks,
            ))
        })
        .await?
        .value;
    let overwrite = writing.overwrite;
    // The check for a persona that appeared meanwhile is made again at the write, under the lock.
    let existing = run_blocking(
        move || {
            let existing = store_generated(&directory, &persona, overwrite)?;
            Ok((existing, summary_of(&persona)))
        },
        PersonaError::Stopped,
    )
    .await?;
    match existing {
        (Some(existing), _) => Ok(kept(app, &existing)),
        (None, persona) => {
            app.record_line(format!(
                "PERSONA generated id={id} duration_ms={} leak_suspect={leak_suspect}",
                started.elapsed().as_millis()
            ));
            Ok(Generation::Generated {
                persona,
                leak_suspect,
            })
        }
    }
}

/// Writes the persona of every named character of `scene` at once. One character's failure never
/// stops the others.
pub(super) async fn generate_cast(
    app: &AppState,
    text: &TextRuntime,
    scene: &ShownScene,
    writing: Writing<'_>,
) -> Vec<CastRow> {
    let names = scene.names();
    join_all(names.iter().map(|character| async move {
        let result = generate_character(app, text, scene, character, writing).await;
        CastRow {
            character: character.clone(),
            outcome: result.into(),
        }
    }))
    .await
}

/// Asks the model for only the voice, the expertise and the voice packs of `own`, and gives the
/// edit that sets them; nothing else of the persona changes.
pub(super) async fn voice_edit(
    app: &AppState,
    route: &TextRoute,
    own: &Persona,
) -> Result<PersonaEdit, PersonaError> {
    let packs = crate::world::available_packs(app);
    let request = text_request(
        route.profile(),
        VOICE_INSTRUCTION,
        voice_facts(own, &packs),
        VOICE_OUTPUT_NAME,
        voice_schema(),
    );
    let key = operation_key("voice")?;
    let started = Instant::now();
    let edit = route
        .ask(&key, &request, |answer| read_voice(answer, own, &packs))
        .await?
        .value;
    app.record_line(format!(
        "PERSONA voice_generated id={} duration_ms={}",
        own.id(),
        started.elapsed().as_millis()
    ));
    Ok(edit)
}
