//! What the text service does when a character is asked something their persona does not cover:
//! judge whether the persona holds enough (coverage), fill what it lacks by adding detail
//! (backfill), and record every committed backfill. A line now reports its own gaps, so the
//! backfill normally runs after the line on a task of its own (`PERSONA backfill_done id=..
//! record=..
//! edits=.. duration_ms=..`); `coverage: true` keeps the old separate coverage call before the
//! line.
//!
//! The questions are `df_content::persona::backfill`. A backfill can only add, and the stored
//! persona is changed by applying its edits to the persona as it is when they are committed, with
//! a record of what was invented (`<id>.backfills.json`). The audit lines are
//! `PERSONA covered id=.. enough=.. gaps=..`, `PERSONA backfilled id=.. gaps=.. edits=..
//! duration_ms=..`, `PERSONA backfill_retried id=..`, `PERSONA backfill_failed id=..
//! error=<key>` and `PERSONA guided id=.. status=<already_given|held|backfilled>`; they never hold
//! persona, question, guidance or edit text.
//!
//! A backfill answer the reader refuses is asked once more with the refusal's reason in the facts
//! as `previous_refusal`; a second refusal is the backfill's failure, reported, and the line is
//! still answered. The world facts of the active place (`world`) are in the coverage and backfill
//! facts: a price, a quantity or a place comes from them and a backfill never invents one.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use df_content::persona::backfill::{
    BACKFILL_INSTRUCTION, BACKFILL_OUTPUT_NAME, COVERAGE_INSTRUCTION, COVERAGE_OUTPUT_NAME,
    Coverage, FILLABLE, Gap, Latitude, NarrativeGuidance, backfill_facts, backfill_schema,
    coverage_facts, coverage_schema, read_backfill, read_coverage,
};
use df_content::persona::privacy::{
    PRIVACY_INSTRUCTION, PRIVACY_OUTPUT_NAME, parse_privacy, privacy_facts, privacy_schema,
    private_side, public_additions,
};
use df_content::persona::{Persona, PersonaEdit, PersonaField};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::generate::{
    PUBLIC_NOTE, ShownScene, compact, load_persona, operation_key, other_personas, text_request,
};
use super::handles::Handles;
use super::line::{Occasion, say_line};
use super::request::{
    Answered, AskRequest, BackfillOutcome, BackfillRecord, Backfilled, GivenGuidance, GuideRequest,
    Guided, PersonaError, SaidLine, Spoken,
};
use super::state::load_state;
use super::storage::{
    commit_backfill, keep_given_guidance, persona_directory, persona_id, read_backfills,
    read_given_guidance,
};
use crate::blocking::run_blocking;
use crate::state::AppState;
use crate::text::TextRoute;

/// That every gap names a field a backfill can fill, and that there is at least one.
pub(super) fn check_gaps(gaps: &[Gap]) -> Result<(), PersonaError> {
    if gaps.is_empty() {
        return Err(PersonaError::NoGaps);
    }
    match gaps.iter().find(|gap| !FILLABLE.contains(&gap.field)) {
        Some(gap) => Err(PersonaError::NotFillable { field: gap.field }),
        None => Ok(()),
    }
}

/// Whether `own` holds enough to answer `question`, and what it lacks if not.
pub(super) async fn coverage_of(
    app: &AppState,
    route: &TextRoute,
    own: &Persona,
    question: &str,
    guidance: Option<&NarrativeGuidance>,
) -> Result<Coverage, PersonaError> {
    let character = own.id().as_str();
    let mut facts = coverage_facts(own, question, guidance);
    facts["world"] = crate::world::context_for_viewer(app, own.voice_packs(), character).facts;
    facts["public_note"] = json!(PUBLIC_NOTE);
    let request = text_request(
        route.profile(),
        COVERAGE_INSTRUCTION,
        compact(facts),
        COVERAGE_OUTPUT_NAME,
        coverage_schema(),
    );
    let key = operation_key("coverage")?;
    let coverage = route.ask(&key, &request, read_coverage).await?.value;
    app.record_line(format!(
        "PERSONA covered id={character} enough={} gaps={}",
        coverage.enough,
        coverage.gaps.len()
    ));
    Ok(coverage)
}

/// What a backfill is about.
#[derive(Clone, Copy)]
pub(super) struct BackfillJob<'a> {
    /// The question that led to it, if any.
    pub(super) question: Option<&'a str>,
    pub(super) gaps: &'a [Gap],
    pub(super) guidance: Option<&'a NarrativeGuidance>,
    /// When the backfill is committed, in milliseconds since the Unix epoch.
    pub(super) at_ms: u64,
}

/// A committed backfill and the persona it produced.
pub(super) struct Committed {
    pub(super) persona: Persona,
    pub(super) done: Backfilled,
}

/// Fills `job`'s gaps of `own`, a checked character of `scene`, and commits the edits with their
/// record. A failure is recorded as `PERSONA backfill_failed`.
pub(super) async fn backfill_persona(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    own: &Persona,
    job: BackfillJob<'_>,
) -> Result<Committed, PersonaError> {
    let result = backfill(app, route, scene, own, job).await;
    if let Err(error) = &result {
        app.record_line(format!(
            "PERSONA backfill_failed id={} error={}",
            own.id(),
            error.key()
        ));
    }
    result
}

/// Asks the model whether the public additions of `edits` give away the private side of `own`
/// (itself as the edits would leave it), and refuses the backfill when it says so. A judge call
/// that fails is a refusal too, never an allowance. A backfill with no public addition has
/// nothing to judge and makes no call.
async fn refuse_private_detail(
    app: &AppState,
    route: &TextRoute,
    own: &Persona,
    edits: &[PersonaEdit],
) -> Result<(), PersonaError> {
    let public = public_additions(edits);
    if public.is_empty() {
        return Ok(());
    }
    let after = own
        .edit(edits)
        .map_err(|cause| PersonaError::InvalidEdit { cause })?;
    let request = text_request(
        route.profile(),
        PRIVACY_INSTRUCTION,
        compact(privacy_facts(
            &public,
            &private_side(&after),
            &crate::world::context_for_viewer(app, after.voice_packs(), after.id().as_str()).facts,
        )),
        PRIVACY_OUTPUT_NAME,
        privacy_schema(),
    );
    let key = operation_key("privacy")?;
    let verdict = route.ask(&key, &request, parse_privacy).await?.value;
    app.record_line(format!(
        "PERSONA privacy_judged id={} leaks={}",
        own.id(),
        verdict.leaks
    ));
    if verdict.leaks {
        return Err(PersonaError::PrivateDetailInPublicField);
    }
    Ok(())
}

/// Asks the backfill question with `facts` and reads the edits of the answer.
async fn ask_backfill(
    route: &TextRoute,
    own: &Persona,
    latitude: Latitude,
    facts: Value,
) -> Result<Vec<PersonaEdit>, PersonaError> {
    let request = text_request(
        route.profile(),
        BACKFILL_INSTRUCTION,
        compact(facts),
        BACKFILL_OUTPUT_NAME,
        backfill_schema(),
    );
    let key = operation_key("backfill")?;
    Ok(route
        .ask(&key, &request, |answer| {
            read_backfill(answer, own, latitude)
        })
        .await?
        .value)
}

async fn backfill(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    own: &Persona,
    job: BackfillJob<'_>,
) -> Result<Committed, PersonaError> {
    let character = own.id().as_str();
    // The backfill writes into the persona, so it is told of the others as the character knows
    // them: a stranger under a handle and without a name, or the name would be kept for good.
    let met = other_personas(app, scene, character).await?;
    let state = load_state(app, character);
    // The scene's cast first, in its order, as the line's prompt has it: a handle means the same
    // person in a line and in the backfill of its gaps.
    let cast = scene.names();
    let handles = Handles::new(
        &state,
        cast.iter()
            .map(String::as_str)
            .chain(met.iter().map(|persona| persona.id().as_str())),
    );
    let others: Vec<_> = met
        .iter()
        .map(|persona| handles.public_view(persona.public_view()))
        .collect();
    // Owner decision 2026-11-06: no guidance means open latitude (a generic story backfill);
    // guidance without a latitude of its own means guided.
    let latitude = job
        .guidance
        .map_or(Latitude::Open, |guidance| guidance.latitude);
    let started = Instant::now();
    // What the line already filled in the moment is recorded as it said it; only the rest is
    // asked of a backfill, and no model is asked when the line filled every gap.
    let mut edits: Vec<PersonaEdit> = job.gaps.iter().filter_map(Gap::spoken_edit).collect();
    let spoken = edits.len();
    let asked: Vec<Gap> = job
        .gaps
        .iter()
        .filter(|gap| gap.spoken_edit().is_none())
        .cloned()
        .collect();
    if !asked.is_empty() {
        // No fixture words, which may name people (see `line`): the world facts say where they
        // are, with every keeper shown as the character knows them.
        let mut facts = backfill_facts(own, &others, &asked, job.guidance);
        facts["world"] = handles
            .world_facts(crate::world::context_for_viewer(app, own.voice_packs(), character).facts);
        facts["public_note"] = json!(PUBLIC_NOTE);
        // A refused answer is asked again with every reason so far until one reads, as the
        // storyteller's judges are (2026-11-08: one story apply stopped on a second refusal).
        let mut refusals: Vec<String> = Vec::new();
        edits.extend(loop {
            match ask_backfill(route, own, latitude, facts.clone()).await {
                Err(PersonaError::AnswerRefused { cause, .. }) => {
                    app.record_line(format!("PERSONA backfill_retried id={character}"));
                    refusals.push(cause.to_string());
                    facts["previous_refusal"] = json!(refusals.join("; "));
                }
                answered => break answered?,
            }
        });
    }
    refuse_private_detail(app, route, own, &edits).await?;
    let profile = route.profile().as_str().to_owned();
    let record = BackfillRecord {
        sequence: 0,
        at_ms: job.at_ms,
        question: job.question.map(str::to_owned),
        gaps: job.gaps.to_vec(),
        edits: edits.clone(),
        guidance: job.guidance.cloned(),
        profile: profile.clone(),
    };
    let (directory, id) = (persona_directory(app), persona_id(character)?);
    let (persona, sequence) = run_blocking(
        move || commit_backfill(&directory, &id, record),
        PersonaError::Stopped,
    )
    .await?;
    app.record_line(format!(
        "PERSONA backfilled id={character} gaps={} edits={} spoken={spoken} duration_ms={}",
        job.gaps.len(),
        edits.len(),
        started.elapsed().as_millis()
    ));
    Ok(Committed {
        persona,
        done: Backfilled {
            gaps: job.gaps.to_vec(),
            edits,
            sequence,
            profile,
        },
    })
}

/// Gives `own`, a checked character of `scene`, the story guidance of `request` once: the coverage
/// judge weighs it, every gap it names is filled (the topic's when it names none) and the guidance
/// is kept as given, at `at_ms`. The same guidance again is not judged again: the judge is not
/// stable (dungeon run 25: one unchanged persona judged covered, then six gaps, then three) and
/// each gap it found added to the backstory. Guidance that changed (a new plot) is judged anew.
pub(super) async fn give_guidance(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    own: &Persona,
    request: &GuideRequest,
    at_ms: u64,
) -> Result<Guided, PersonaError> {
    let (directory, id) = (persona_directory(app), own.id().clone());
    let given = run_blocking(
        move || read_given_guidance(&directory, &id),
        PersonaError::Stopped,
    )
    .await?;
    if given.is_some_and(|given| given.guidance == request.guidance) {
        app.record_line(format!(
            "PERSONA guided id={} status=already_given",
            own.id()
        ));
        return Ok(Guided::AlreadyGiven);
    }
    let question = format!("Tell me {}.", request.topic);
    let coverage = coverage_of(app, route, own, &question, Some(&request.guidance)).await?;
    let guided = if coverage.enough {
        Guided::Held
    } else {
        let mut gaps = coverage.gaps;
        if gaps.is_empty() {
            gaps.push(Gap {
                field: PersonaField::PrivateBackstory,
                topic: request.topic.clone(),
                filled: None,
            });
        }
        check_gaps(&gaps)?;
        let job = BackfillJob {
            question: None,
            gaps: &gaps,
            guidance: Some(&request.guidance),
            at_ms,
        };
        let done = backfill_persona(app, route, scene, own, job).await?.done;
        Guided::Backfilled {
            edits: done.edits.len(),
            sequence: done.sequence,
        }
    };
    let (directory, id) = (persona_directory(app), own.id().clone());
    let given = GivenGuidance {
        guidance: request.guidance.clone(),
        at_ms,
    };
    run_blocking(
        move || keep_given_guidance(&directory, &id, &given),
        PersonaError::Stopped,
    )
    .await?;
    let status = match guided {
        Guided::AlreadyGiven => "already_given",
        Guided::Held => "held",
        Guided::Backfilled { .. } => "backfilled",
    };
    app.record_line(format!("PERSONA guided id={} status={status}", own.id()));
    Ok(guided)
}

/// What answering a question came to, before the line is cued.
pub(super) struct Considered {
    pub(super) covered: bool,
    pub(super) gaps: Vec<Gap>,
    pub(super) backfill: BackfillOutcome,
    pub(super) profile: String,
    pub(super) said: SaidLine,
}

impl Considered {
    /// The answer once the line has been cued as `spoken`.
    pub(super) fn answered(self, spoken: Spoken) -> Answered {
        Answered {
            covered: self.covered,
            gaps: self.gaps,
            backfill: self.backfill,
            profile: self.profile,
            spoken,
        }
    }
}

/// What came of looking for gaps and filling them: the outcome, and the persona to answer from.
struct Filled {
    covered: bool,
    gaps: Vec<Gap>,
    outcome: BackfillOutcome,
    persona: Persona,
}

/// The old way, asked for with `coverage: true`: judges the coverage of `persona` for the question
/// of `request` with a call of its own and, when it has gaps and the request asks, fills them
/// before the line. A coverage or backfill that fails is reported in the outcome and `persona` is
/// left as it was: it never blocks the answer.
async fn fill_gaps(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    persona: Persona,
    request: &AskRequest,
    at_ms: u64,
) -> Filled {
    let guidance = request.guidance.as_ref();
    let coverage = match coverage_of(app, route, &persona, &request.question, guidance).await {
        Ok(coverage) => coverage,
        Err(error) => {
            app.record_line(format!(
                "PERSONA coverage_failed id={} error={}",
                persona.id(),
                error.key()
            ));
            return Filled {
                covered: false,
                gaps: Vec::new(),
                outcome: BackfillOutcome::CoverageFailed { error },
                persona,
            };
        }
    };
    let covered = coverage.enough;
    let job = BackfillJob {
        question: Some(&request.question),
        gaps: &coverage.gaps,
        guidance,
        at_ms,
    };
    if covered || !request.backfill {
        let outcome = if covered {
            BackfillOutcome::NotNeeded
        } else {
            BackfillOutcome::NotAsked
        };
        return Filled {
            covered,
            gaps: coverage.gaps,
            outcome,
            persona,
        };
    }
    let (outcome, persona) = match backfill_persona(app, route, scene, &persona, job).await {
        Ok(committed) => (
            BackfillOutcome::Backfilled {
                edits: committed.done.edits.len(),
                sequence: committed.done.sequence,
            },
            committed.persona,
        ),
        Err(error) => (BackfillOutcome::BackfillFailed { error }, persona),
    };
    Filled {
        covered,
        gaps: coverage.gaps,
        outcome,
        persona,
    }
}

/// The personas whose backfill is running, by personas folder and id, so a persona has one at a
/// time: the task is bounded by the gaps of one line and a second line's gaps wait for the next
/// ask.
static RUNNING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// A claim on the running backfill of one persona, given up when it is dropped.
struct RunningClaim(String);

impl RunningClaim {
    fn take(id: &str) -> Option<Self> {
        let mut running = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
        running.insert(id.to_owned()).then(|| Self(id.to_owned()))
    }
}

impl Drop for RunningClaim {
    fn drop(&mut self) {
        RUNNING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// What a backfill that runs after its line needs: owned, because the task outlives the request.
struct Deferred {
    app: Arc<AppState>,
    route: Arc<TextRoute>,
    character: String,
    question: String,
    gaps: Vec<Gap>,
    guidance: Option<NarrativeGuidance>,
    at_ms: u64,
}

impl Deferred {
    /// Fills the gaps against the persona as it is now and the scene as it is now, and records
    /// `PERSONA backfill_done`.
    async fn run(self) -> Result<Backfilled, PersonaError> {
        let started = Instant::now();
        let result = async {
            let scene = ShownScene::read(&self.app).await?;
            let persona = load_persona(&self.app, &self.character).await?;
            let job = BackfillJob {
                question: Some(&self.question),
                gaps: &self.gaps,
                guidance: self.guidance.as_ref(),
                at_ms: self.at_ms,
            };
            backfill_persona(&self.app, &self.route, &scene, &persona, job)
                .await
                .map(|committed| committed.done)
        }
        .await;
        let outcome = match &result {
            Ok(done) => format!("record={} edits={}", done.sequence, done.edits.len()),
            Err(error) => format!("failed={}", error.key()),
        };
        self.app.record_line_and_echo(format!(
            "PERSONA backfill_done id={} {outcome} duration_ms={}",
            self.character,
            started.elapsed().as_millis()
        ));
        result
    }
}

/// The record number the next backfill of `id` will take.
async fn next_record(app: &AppState, id: &str) -> Result<u64, PersonaError> {
    let (directory, id) = (persona_directory(app), persona_id(id)?);
    let records = run_blocking(
        move || read_backfills(&directory, &id),
        PersonaError::Stopped,
    )
    .await?;
    Ok(records
        .last()
        .map_or(1, |last| last.sequence.saturating_add(1)))
}

/// Fills the gaps the line reported, off the line's critical path: the line is already worded and
/// the backfill runs on a task of its own (it finishes even if the request goes away) and helps
/// the next line. With `backfill_sync` the request waits for it.
async fn fill_after_line(
    app: &Arc<AppState>,
    route: &Arc<TextRoute>,
    request: &AskRequest,
    gaps: &[Gap],
    at_ms: u64,
) -> BackfillOutcome {
    let key = format!("{}/{}", persona_directory(app).display(), request.character);
    let Some(claim) = RunningClaim::take(&key) else {
        return BackfillOutcome::Busy;
    };
    let record = match next_record(app, &request.character).await {
        Ok(record) => record,
        Err(error) => return BackfillOutcome::BackfillFailed { error },
    };
    let deferred = Deferred {
        app: Arc::clone(app),
        route: Arc::clone(route),
        character: request.character.clone(),
        question: request.question.clone(),
        gaps: gaps.to_vec(),
        guidance: request.guidance.clone(),
        at_ms,
    };
    let task = tokio::spawn(async move {
        let _claim = claim;
        deferred.run().await
    });
    if !request.backfill_sync {
        return BackfillOutcome::Running { record };
    }
    match task.await {
        Ok(Ok(done)) => BackfillOutcome::Backfilled {
            edits: done.edits.len(),
            sequence: done.sequence,
        },
        Ok(Err(error)) => BackfillOutcome::BackfillFailed { error },
        Err(_) => BackfillOutcome::BackfillFailed {
            error: PersonaError::Stopped,
        },
    }
}

/// Puts `request.question` to its character: one call words the line and reports the gaps and the
/// appraisal; the gaps are then filled when asked, after the line. With `coverage: true` the old
/// way runs instead: a coverage call, a backfill, then the line.
pub(super) async fn consider(
    app: &Arc<AppState>,
    route: &Arc<TextRoute>,
    scene: &ShownScene,
    request: &AskRequest,
    at_ms: u64,
) -> Result<Considered, PersonaError> {
    let persona = load_persona(app, &request.character).await?;
    let profile = route.profile().as_str().to_owned();
    let mut occasion = Occasion {
        situation: request.situation.as_deref().unwrap_or(""),
        question: Some(&request.question),
        guidance: request.guidance.as_ref(),
        not_established: &[],
        recent: &request.recent,
        asked_by: request.asked_by.as_deref(),
        intent: None,
        strike: false,
        declare: false,
    };
    if request.coverage {
        let filled = fill_gaps(app, route, scene, persona, request, at_ms).await;
        // What the persona lacks and the backfill did not fill (it failed or was not asked) is told
        // to the line as not established, so the line cannot invent a fact the persona does not
        // hold.
        let not_established: Vec<String> = match filled.outcome {
            BackfillOutcome::Backfilled { .. } | BackfillOutcome::NotNeeded => Vec::new(),
            _ => filled.gaps.iter().map(|gap| gap.topic.clone()).collect(),
        };
        occasion.not_established = &not_established;
        let said = say_line(app, route, scene, &filled.persona, occasion).await?;
        return Ok(Considered {
            covered: filled.covered,
            gaps: filled.gaps,
            backfill: filled.outcome,
            profile,
            said,
        });
    }
    let said = say_line(app, route, scene, &persona, occasion).await?;
    let backfill = if said.gaps.is_empty() {
        BackfillOutcome::NotNeeded
    } else if request.backfill {
        fill_after_line(app, route, request, &said.gaps, at_ms).await
    } else {
        BackfillOutcome::NotAsked
    };
    Ok(Considered {
        covered: said.gaps.is_empty(),
        gaps: said.gaps.clone(),
        backfill,
        profile,
        said,
    })
}
