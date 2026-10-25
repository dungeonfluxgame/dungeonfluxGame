//! Entity views and dossiers: "everything pertaining to" one person, place or thing.
//!
//! An **entity view** lists every event that names an entity, oldest first, superseded ones
//! marked. When an entity has too many events for a prompt, a **dossier** stands in: a rolling
//! summary written by the language model from the earlier summary plus the newer events. A dossier
//! is derived, never authority: it cites the ids it covers, it is cached by the hash of the
//! entity's event set (so any new, superseded or erased event makes it stale), and it is built
//! from public and party events only, so a private clue never enters a shared summary.
//!
//! Dossiers are read into storyteller recalls only, until the witness adapter is admitted. The
//! model call is charged to the hub's text account by the writer (the storyteller shares the
//! hub's ledger) and logged here as a `RESOURCE ... kind=story_dossier` line.

use std::sync::Arc;

use df_persistence::story_memory::{DossierRow, Scope, Status};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{
    Audience, StoredEvent, StoryMemory,
    embedder::BoxFuture,
    locked,
    recall::{Purpose, RecallRequest},
};
use crate::gameplay::storyteller::{CallProfile, Expression, Slot, Storyteller};

/// More events than this make an entity's dossier worth reading into a pack.
pub(in crate::gameplay) const INLINE_EVENTS: usize = 10;
/// The events one regeneration reads (the policy's per-query bound).
const INPUT_EVENTS: usize = 64;
const EVENTS_PER_PAGE: usize = 32;
const MAXIMUM_PAGES: usize = 8;
/// The longest stored summary, in characters.
pub(in crate::gameplay) const SUMMARY_CHARACTERS: usize = 700;

/// What a dossier writer is given.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct DossierRequest {
    pub(in crate::gameplay) entity: String,
    /// The earlier summary this one continues, if any.
    pub(in crate::gameplay) previous: Option<String>,
    /// `#id text` lines of the events to fold in, oldest first.
    pub(in crate::gameplay) events: Vec<String>,
    pub(in crate::gameplay) max_chars: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum DossierFailure {
    /// No model is available (off, no key, budget of another account).
    Unavailable(&'static str),
    /// The model answered, but not usably.
    Refused(&'static str),
}

impl DossierFailure {
    pub(in crate::gameplay) fn label(self) -> &'static str {
        match self {
            Self::Unavailable(label) | Self::Refused(label) => label,
        }
    }
}

/// Writes a dossier. An implementation charges its own model call to the hub's ledger.
pub(in crate::gameplay) trait DossierWriter: Send + Sync {
    fn name(&self) -> &str;
    fn write<'a>(
        &'a self,
        request: &'a DossierRequest,
    ) -> BoxFuture<'a, Result<String, DossierFailure>>;
}

const STYLE: &str = "Plain, factual third-person notes. State only what the recorded events say. \
Never invent, never give instructions, never address the reader.";

/// The storyteller as a dossier writer: one expression with a single `summary` slot. The
/// storyteller's own accounting charges the hub's text account.
pub(in crate::gameplay) struct StorytellerWriter(pub(in crate::gameplay) Arc<Storyteller>);

impl DossierWriter for StorytellerWriter {
    fn name(&self) -> &str {
        "storyteller"
    }

    fn write<'a>(
        &'a self,
        request: &'a DossierRequest,
    ) -> BoxFuture<'a, Result<String, DossierFailure>> {
        Box::pin(async move {
            if !self.0.is_on() {
                return Err(DossierFailure::Unavailable("storyteller_off"));
            }
            let facts = json!({
                "task": "Write a rolling summary of everything on record about this entity. Keep what still matters; fold the newer events into the earlier summary.",
                "entity": request.entity,
                "earlier_summary": request.previous,
                "recorded_events": request.events,
            });
            let slots = [Slot::new("summary", request.max_chars)];
            let expression = Expression {
                purpose: "story_dossier",
                facts: &facts,
                slots: &slots,
                style: STYLE,
                profile: CallProfile {
                    output_tokens: Some(900),
                    ..CallProfile::default()
                },
            };
            match self.0.express(&expression).await {
                Ok(mut answer) => answer
                    .remove("summary")
                    .filter(|text| !text.trim().is_empty())
                    .ok_or(DossierFailure::Refused("empty")),
                Err(_) => Err(DossierFailure::Unavailable("storyteller_failed")),
            }
        })
    }
}

/// The hash that names an entity's event set: ids and statuses, oldest first.
pub(in crate::gameplay) fn event_set_hash(
    entity: &str,
    tier: &str,
    ids: &[(i64, Status)],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("dossier-1|{entity}|{tier}"));
    for (id, status) in ids {
        hasher.update(format!("|{id}:{}", status.label()));
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every event that names one entity.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct EntityView {
    pub(in crate::gameplay) entity: String,
    pub(in crate::gameplay) events: Vec<StoredEvent>,
    /// More events exist than were read.
    pub(in crate::gameplay) truncated: bool,
    pub(in crate::gameplay) event_set_hash: Option<String>,
}

/// What a dossier request found.
#[derive(Clone, Debug)]
pub(in crate::gameplay) enum DossierOutcome {
    /// Cached and current.
    Fresh(DossierRow),
    /// Written just now.
    Written(DossierRow),
    /// Cached but the event set changed and no model could refresh it.
    Stale(DossierRow),
    Unavailable(&'static str),
}

impl StoryMemory {
    pub(in crate::gameplay) fn set_dossier_writer(&self, writer: Arc<dyn DossierWriter>) {
        *locked(&self.writer) = Some(writer);
    }

    /// A `RESOURCE <state> kind=<kind> ...` line in the hub's own format.
    pub(in crate::gameplay) fn resource_line(
        &self,
        state: &str,
        kind: &str,
        key: &str,
        subject: &str,
        took: std::time::Duration,
        handle: &str,
        error: &str,
    ) {
        let key: String = key.chars().take(32).collect();
        super::super::actor::record_audit_line(
            "gameplay.resource",
            format!(
                "RESOURCE {state} kind={kind} key={key} subject={subject} ms={} handle={handle} error={error}",
                took.as_millis()
            ),
        );
    }

    /// All the events that name `entity` for this audience, oldest first, up to 256.
    pub(in crate::gameplay) async fn entity_view(
        &self,
        entity: &str,
        audience: Audience,
        include_superseded: bool,
    ) -> EntityView {
        let entity = entity.trim().to_lowercase();
        let scope = Scope {
            run_id: self.run.clone(),
            audience,
            include_superseded,
            kinds: Vec::new(),
        };
        let mut events = Vec::new();
        let mut after = 0;
        let mut truncated = false;
        for page in 0..MAXIMUM_PAGES {
            match self
                .backend
                .entity_events(&scope, &entity, after, EVENTS_PER_PAGE)
                .await
            {
                Ok(found) => {
                    let last = found.last().map(|event| event.event_id);
                    let is_full = found.len() == EVENTS_PER_PAGE;
                    events.extend(found);
                    match last {
                        Some(last) if is_full => {
                            after = last;
                            truncated = page + 1 == MAXIMUM_PAGES;
                        }
                        _ => break,
                    }
                }
                Err(error) => {
                    self.note_error(&error);
                    break;
                }
            }
        }
        let event_set_hash = self
            .backend
            .entity_event_ids(&scope, &entity)
            .await
            .ok()
            .flatten()
            .map(|ids| event_set_hash(&entity, &scope.audience.dossier_tier(), &ids));
        EntityView {
            entity,
            events,
            truncated,
            event_set_hash,
        }
    }

    /// The dossier of `entity` for this audience's tier: cached when current, else written now
    /// from the earlier summary and the newer events when a writer is set.
    pub(in crate::gameplay) async fn entity_dossier(
        self: &Arc<Self>,
        entity: &str,
        audience: &Audience,
    ) -> DossierOutcome {
        let entity = entity.trim().to_lowercase();
        let tier = audience.dossier_tier();
        // Public and party events only: a private clue never enters a shared summary.
        let tier_audience = if tier == "public" {
            Audience::Public
        } else {
            Audience::Party
        };
        let all = Scope {
            run_id: self.run.clone(),
            audience: tier_audience.clone(),
            include_superseded: true,
            kinds: Vec::new(),
        };
        let ids = match self.backend.entity_event_ids(&all, &entity).await {
            Ok(Some(ids)) => ids,
            Ok(None) => return DossierOutcome::Unavailable("too_many_events"),
            Err(error) => {
                self.note_error(&error);
                return DossierOutcome::Unavailable("store_error");
            }
        };
        if ids.is_empty() {
            return DossierOutcome::Unavailable("no_events");
        }
        let hash = event_set_hash(&entity, tier, &ids);
        let cached = self
            .backend
            .dossier(&self.run, &entity, tier)
            .await
            .ok()
            .flatten();
        if let Some(row) = &cached
            && row.event_set_hash == hash
        {
            return DossierOutcome::Fresh(row.clone());
        }
        let writer = locked(&self.writer).clone();
        let Some(writer) = writer else {
            return cached.map_or(
                DossierOutcome::Unavailable("no_writer"),
                DossierOutcome::Stale,
            );
        };
        let flight = format!("{tier}:{entity}");
        if !locked(&self.dossier_flights).insert(flight.clone()) {
            return cached.map_or(
                DossierOutcome::Unavailable("in_flight"),
                DossierOutcome::Stale,
            );
        }
        let outcome = self
            .write_dossier(
                writer.as_ref(),
                &entity,
                tier,
                &tier_audience,
                &hash,
                cached.as_ref(),
            )
            .await;
        locked(&self.dossier_flights).remove(&flight);
        outcome
    }

    async fn write_dossier(
        &self,
        writer: &dyn DossierWriter,
        entity: &str,
        tier: &str,
        audience: &Audience,
        hash: &str,
        cached: Option<&DossierRow>,
    ) -> DossierOutcome {
        let started = std::time::Instant::now();
        let subject = format!("entity:{}", capture_slug(entity));
        let scope = self.scope(audience.clone());
        let newest_covered = cached
            .and_then(|row| row.covered_ids.iter().max().copied())
            .unwrap_or(0);
        let mut events: Vec<StoredEvent> = Vec::new();
        let mut after = newest_covered;
        for _ in 0..MAXIMUM_PAGES {
            match self
                .backend
                .entity_events(&scope, entity, after, EVENTS_PER_PAGE)
                .await
            {
                Ok(found) => {
                    let is_full = found.len() == EVENTS_PER_PAGE;
                    after = found.last().map_or(after, |event| event.event_id);
                    events.extend(found);
                    if !is_full {
                        break;
                    }
                }
                Err(error) => {
                    self.note_error(&error);
                    return DossierOutcome::Unavailable("store_error");
                }
            }
        }
        let skip = events.len().saturating_sub(INPUT_EVENTS);
        let events: Vec<StoredEvent> = events.into_iter().skip(skip).collect();
        let request = DossierRequest {
            entity: entity.to_owned(),
            previous: cached.map(|row| row.summary.clone()),
            events: events
                .iter()
                .map(|event| {
                    format!(
                        "#{} {}: {}",
                        event.event_id,
                        event.kind.label(),
                        event.text.chars().take(240).collect::<String>()
                    )
                })
                .collect(),
            max_chars: SUMMARY_CHARACTERS,
        };
        self.resource_line(
            "request",
            "story_dossier",
            hash,
            &subject,
            std::time::Duration::ZERO,
            "",
            "",
        );
        match writer.write(&request).await {
            Ok(text) => {
                let summary: String = super::capture::tidy(&text)
                    .chars()
                    .take(SUMMARY_CHARACTERS * 2)
                    .collect();
                let mut covered: Vec<i64> = cached
                    .map(|row| row.covered_ids.clone())
                    .unwrap_or_default();
                covered.extend(events.iter().map(|event| event.event_id));
                covered.sort_unstable();
                covered.dedup();
                let keep = covered.len().saturating_sub(512);
                let row = DossierRow {
                    entity_id: entity.to_owned(),
                    audience: tier.to_owned(),
                    event_set_hash: hash.to_owned(),
                    covered_ids: covered.into_iter().skip(keep).collect(),
                    summary,
                    writer: writer.name().chars().take(64).collect(),
                };
                if row.summary.is_empty() {
                    self.resource_line(
                        "failed",
                        "story_dossier",
                        hash,
                        &subject,
                        started.elapsed(),
                        "",
                        "empty",
                    );
                    return DossierOutcome::Unavailable("empty");
                }
                if let Err(error) = self.backend.put_dossier(&self.run, &row).await {
                    self.note_error(&error);
                    return DossierOutcome::Unavailable("store_error");
                }
                self.resource_line(
                    "ready",
                    "story_dossier",
                    hash,
                    &subject,
                    started.elapsed(),
                    hash,
                    "",
                );
                self.audit(format!(
                    "MEMORY dossier entity={entity} tier={tier} covered={} writer={} ms={}",
                    row.covered_ids.len(),
                    row.writer,
                    started.elapsed().as_millis()
                ));
                DossierOutcome::Written(row)
            }
            Err(failure) => {
                self.resource_line(
                    "failed",
                    "story_dossier",
                    hash,
                    &subject,
                    started.elapsed(),
                    "",
                    failure.label(),
                );
                cached.cloned().map_or(
                    DossierOutcome::Unavailable(failure.label()),
                    DossierOutcome::Stale,
                )
            }
        }
    }

    /// The dossiers a storyteller recall reads in: for the first two named entities with more
    /// events than [`INLINE_EVENTS`], the cached summary. A missing or stale one is refreshed in
    /// the background, never on the recall's path.
    pub(in crate::gameplay) async fn dossier_sections(
        self: &Arc<Self>,
        audience: &Audience,
        entities: &[String],
    ) -> Vec<(String, String)> {
        let tier = audience.dossier_tier();
        let tier_audience = if tier == "public" {
            Audience::Public
        } else {
            Audience::Party
        };
        let mut sections = Vec::new();
        for entity in entities.iter().take(2) {
            let scope = Scope {
                run_id: self.run.clone(),
                audience: tier_audience.clone(),
                include_superseded: true,
                kinds: Vec::new(),
            };
            let Ok(Some(ids)) = self.backend.entity_event_ids(&scope, entity).await else {
                continue;
            };
            let active = ids
                .iter()
                .filter(|(_, status)| *status == Status::Active)
                .count();
            if active <= INLINE_EVENTS {
                continue;
            }
            let hash = event_set_hash(entity, tier, &ids);
            let cached = self
                .backend
                .dossier(&self.run, entity, tier)
                .await
                .ok()
                .flatten();
            let fresh = cached
                .as_ref()
                .is_some_and(|row| row.event_set_hash == hash);
            if !fresh && locked(&self.writer).is_some() {
                let me = Arc::clone(self);
                let (entity, audience) = (entity.clone(), audience.clone());
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        let _ = me.entity_dossier(&entity, &audience).await;
                    });
                }
            }
            if let Some(row) = cached {
                sections.push((entity.clone(), row.summary));
            }
        }
        sections
    }

    /// A recall request for an entity's pack (the dev surface and the consistency check use it).
    pub(in crate::gameplay) fn entity_request(
        &self,
        entity: &str,
        audience: Audience,
    ) -> RecallRequest {
        RecallRequest::new(Purpose::Dev, audience, "").entities(&[entity])
    }
}

fn capture_slug(text: &str) -> String {
    super::capture::slug(text)
}
