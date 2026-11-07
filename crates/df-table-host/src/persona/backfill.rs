//! What the text service does when a character is asked something their persona does not cover:
//! judge whether the persona holds enough (coverage), fill what it lacks by adding detail
//! (backfill), and record every committed backfill.
//!
//! The questions are `df_content::persona::backfill`. A backfill can only add, and the stored
//! persona is changed by applying its edits to the persona as it is when they are committed, with
//! a record of what was invented (`<id>.backfills.json`). The audit lines are
//! `PERSONA covered id=.. enough=.. gaps=..`, `PERSONA backfilled id=.. gaps=.. edits=..
//! duration_ms=..` and `PERSONA backfill_failed id=.. error=<key>`; they never hold persona,
//! question, guidance or edit text.

use df_content::persona::Persona;
use df_content::persona::backfill::{
    BACKFILL_INSTRUCTION, BACKFILL_OUTPUT_NAME, COVERAGE_INSTRUCTION, COVERAGE_OUTPUT_NAME,
    Coverage, FILLABLE, Gap, Latitude, NarrativeGuidance, backfill_facts, backfill_schema,
    coverage_facts, coverage_schema, read_backfill, read_coverage,
};
use tokio::time::Instant;

use crate::blocking::run_blocking;

use super::generate::{
    Occasion, ShownScene, load_persona, operation_key, other_personas, say_line, text_request,
};
use super::request::{
    Answered, AskRequest, BackfillOutcome, BackfillRecord, Backfilled, PersonaError, SaidLine,
    Spoken,
};
use super::storage::{commit_backfill, persona_directory, persona_id};
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
    let request = text_request(
        route.profile(),
        COVERAGE_INSTRUCTION,
        coverage_facts(own, question, guidance),
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

async fn backfill(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    own: &Persona,
    job: BackfillJob<'_>,
) -> Result<Committed, PersonaError> {
    let character = own.id().as_str();
    let others: Vec<_> = other_personas(app, scene, character)
        .await?
        .iter()
        .map(Persona::public_view)
        .collect();
    // Owner decision 2026-11-06: no guidance means open latitude (a generic story backfill);
    // guidance without a latitude of its own means guided.
    let latitude = job
        .guidance
        .map_or(Latitude::Open, |guidance| guidance.latitude);
    let scene_text = format!("{}: {}", scene.title(), scene.description());
    let request = text_request(
        route.profile(),
        BACKFILL_INSTRUCTION,
        backfill_facts(own, &others, job.gaps, job.guidance, &scene_text),
        BACKFILL_OUTPUT_NAME,
        backfill_schema(),
    );
    let key = operation_key("backfill")?;
    let started = Instant::now();
    let edits = route
        .ask(&key, &request, |answer| {
            read_backfill(answer, own, latitude)
        })
        .await?
        .value;
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
        "PERSONA backfilled id={character} gaps={} edits={} duration_ms={}",
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

/// Judges the coverage of `persona` for the question of `request` and, when it has gaps and the
/// request asks, fills them. A coverage or backfill that fails is reported in the outcome and
/// `persona` is left as it was: it never blocks the answer.
async fn fill_gaps(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    persona: Persona,
    request: &AskRequest,
    at_ms: u64,
) -> Filled {
    let guidance = request.guidance.as_ref();
    if !request.coverage {
        return Filled {
            covered: false,
            gaps: Vec::new(),
            outcome: BackfillOutcome::CoverageSkipped,
            persona,
        };
    }
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

/// Puts `request.question` to its character: coverage, then a backfill of any gaps when asked,
/// then the line, from the persona as it is after.
pub(super) async fn consider(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    request: &AskRequest,
    at_ms: u64,
) -> Result<Considered, PersonaError> {
    let persona = load_persona(app, &request.character).await?;
    let filled = fill_gaps(app, route, scene, persona, request, at_ms).await;
    let occasion = Occasion {
        situation: request.situation.as_deref().unwrap_or(""),
        question: Some(&request.question),
        guidance: request.guidance.as_ref(),
    };
    let said = say_line(app, route, scene, &filled.persona, occasion).await?;
    Ok(Considered {
        covered: filled.covered,
        gaps: filled.gaps,
        backfill: filled.outcome,
        profile: route.profile().as_str().to_owned(),
        said,
    })
}
