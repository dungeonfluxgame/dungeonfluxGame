//! `dev memory`: what the story memory holds, as JSON for the development surface.
//!
//! Reads: `status`, `search` (the same recall path the generators use), `entity` and `event`.
//! Maintenance writes: `supersede`, `reindex`, `delete-run-index` and `erase-hero`. Nothing here
//! shows an embedding vector or a key. The operator's `event` view shows an event whatever its
//! audience; `search` and `entity` take an explicit audience (party by default).

use std::sync::Arc;

use df_persistence::story_memory::Status;
use serde_json::{Value, json};

use super::{
    Audience, EventKind, SearchMode, Stats, StoredEvent, StoryMemory,
    dossier::DossierOutcome,
    recall::{Purpose, RecallRequest, RecalledItem},
};
use crate::gameplay::resources::Unit;

fn entities_json(event: &StoredEvent) -> Value {
    json!({
        "npc": event.entities.npc,
        "hero": event.entities.hero,
        "place": event.entities.place,
        "item": event.entities.item,
    })
}

pub(in crate::gameplay) fn event_json(event: &StoredEvent) -> Value {
    json!({
        "id": event.event_id,
        "kind": event.kind.label(),
        "visibility": event.visibility.label(),
        "private_to": event.private_to,
        "status": event.status.label(),
        "source": event.source,
        "natural_key": event.natural_key,
        "game_revision": event.game_revision,
        "created_ms": event.created_ms,
        "scene": event.scene,
        "entities": entities_json(event),
        "text": event.text,
        "rebuildable": event.rebuildable,
        "supersedes": event.supersedes_id,
        "superseded_by": event.superseded_by,
        "retired_reason": event.retired_reason,
        "embedding_model": event.embedding_model,
        "embedded": event.embedded,
    })
}

fn item_json(item: &RecalledItem) -> Value {
    let mut value = event_json(&item.event);
    value["score"] = json!((item.score * 10_000.0).round() / 10_000.0);
    value
}

impl StoryMemory {
    pub(in crate::gameplay) async fn status_json(&self) -> Value {
        let model = self.model();
        let counts = self
            .backend
            .counts(&self.run, model.as_deref().unwrap_or(""))
            .await;
        let spend = self.ledger.snapshot(Unit::EmbeddingToken);
        let (search, reason) = match self.mode() {
            SearchMode::Hybrid => ("hybrid", None),
            SearchMode::TextOnly(reason) => ("text_only", Some(reason)),
        };
        let mut value = json!({
            "status": "on",
            "run": self.run,
            "backend": self.backend.name(),
            "search": search,
            "text_only_reason": reason,
            "vector_extension": self.backend.vector_enabled(),
            "vector_reason": self.backend.vector_reason(),
            "embedding_model": model,
            "spend": {
                "unit": Unit::EmbeddingToken.label(),
                "committed": spend.committed,
                "reserved": spend.reserved,
                "cap": "none (recorded and audited, never refused)",
            },
            "queue": {
                "capacity": self.config.queue_capacity,
                "stats": self.stats.snapshot(),
            },
            "dossiers": {
                "enabled_for_storyteller": self.config.dossiers,
                "writer": super::locked(&self.writer).as_ref().map(|w| w.name().to_owned()),
            },
            "last_error": self.last_error(),
            "recent_audit": self.recent_audit(8),
            "kinds": EventKind::ALL.iter().map(|k| k.label()).collect::<Vec<_>>(),
        });
        match counts {
            Ok(counts) => {
                value["events"] = json!({
                    "total": counts.total,
                    "rebuildable": counts.rebuildable,
                    "by_kind": counts.by_kind.iter().map(|(kind, active, superseded)| json!({
                        "kind": kind, "active": active, "superseded": superseded,
                    })).collect::<Vec<_>>(),
                });
                value["embedding"] = json!({
                    "embedded": counts.embedded,
                    "backlog": counts.backlog,
                    "models": counts.models.iter().map(|(model, n)| json!({"model": model, "events": n})).collect::<Vec<_>>(),
                });
                value["recalls_recorded"] = json!(counts.recalls);
                value["dossiers_cached"] = json!(counts.dossiers);
            }
            Err(error) => value["events_error"] = json!(error.to_string()),
        }
        value
    }

    /// The same recall a generator makes, for one query. `entity` and `kind` narrow it.
    pub(in crate::gameplay) async fn search_json(
        self: &Arc<Self>,
        query: &str,
        entity: Option<&str>,
        kind: Option<EventKind>,
        audience: Audience,
        limit: usize,
    ) -> Value {
        let mut request = RecallRequest::new(Purpose::Dev, audience.clone(), query);
        if let Some(entity) = entity {
            request = request.entities(&[entity]);
        }
        if let Some(kind) = kind {
            request.kinds = vec![kind];
        }
        request.related_limit = limit;
        request.budget_bytes = super::recall::DEFAULT_BUDGET;
        let pack = self.recall(&request).await;
        json!({
            "query": request.text,
            "audience": audience.label(),
            "entity": entity,
            "kind": kind.map(EventKind::label),
            "recall_id": pack.recall_id,
            "mode": pack.coverage.mode.label(),
            "query_embedded": pack.coverage.query_embedded,
            "pinned": pack.pinned.iter().map(item_json).collect::<Vec<_>>(),
            "related": pack.related.iter().map(item_json).collect::<Vec<_>>(),
            "omitted": pack.coverage.omitted,
            "store_errors": pack.coverage.store_errors,
            "rendered": pack.rendered,
            "rendered_bytes": pack.rendered.len(),
        })
    }

    pub(in crate::gameplay) async fn entity_json(
        self: &Arc<Self>,
        entity: &str,
        audience: Audience,
        include_superseded: bool,
        refresh_dossier: bool,
    ) -> Value {
        let view = self
            .entity_view(entity, audience.clone(), include_superseded)
            .await;
        let dossier = if refresh_dossier {
            self.entity_dossier(entity, &audience).await
        } else {
            let tier = audience.dossier_tier();
            match self.backend.dossier(&self.run, &view.entity, tier).await {
                Ok(Some(row)) if Some(&row.event_set_hash) == view.event_set_hash.as_ref() => {
                    DossierOutcome::Fresh(row)
                }
                Ok(Some(row)) => DossierOutcome::Stale(row),
                _ => DossierOutcome::Unavailable("not_written"),
            }
        };
        let dossier = match dossier {
            DossierOutcome::Fresh(row) => {
                json!({"state": "fresh", "summary": row.summary, "covers": row.covered_ids.len(), "writer": row.writer})
            }
            DossierOutcome::Written(row) => {
                json!({"state": "written_now", "summary": row.summary, "covers": row.covered_ids.len(), "writer": row.writer})
            }
            DossierOutcome::Stale(row) => {
                json!({"state": "stale", "summary": row.summary, "covers": row.covered_ids.len(), "writer": row.writer})
            }
            DossierOutcome::Unavailable(reason) => {
                json!({"state": "unavailable", "reason": reason})
            }
        };
        json!({
            "entity": view.entity,
            "audience": audience.label(),
            "count": view.events.len(),
            "active": view.events.iter().filter(|e| e.status == Status::Active).count(),
            "truncated": view.truncated,
            "event_set_hash": view.event_set_hash,
            "dossier": dossier,
            "events": view.events.iter().map(event_json).collect::<Vec<_>>(),
        })
    }

    pub(in crate::gameplay) async fn event_view_json(&self, id: i64) -> Value {
        match self.backend.operator_event(&self.run, id).await {
            Ok(Some(event)) => {
                let mut value = event_json(&event);
                for (field, link) in [
                    ("supersedes_event", event.supersedes_id),
                    ("superseded_by_event", event.superseded_by),
                ] {
                    if let Some(link) = link
                        && let Ok(Some(other)) = self.backend.operator_event(&self.run, link).await
                    {
                        value[field] = event_json(&other);
                    }
                }
                json!({"found": true, "event": value})
            }
            Ok(None) => json!({"found": false, "id": id}),
            Err(error) => json!({"found": false, "id": id, "error": error.to_string()}),
        }
    }

    /// Rebuilds the derived rows from the canonical sources and fills in vectors: backfill,
    /// then wait for the writes and the embeddings.
    pub(in crate::gameplay) async fn reindex_json(&self) -> Value {
        let before = Stats::get(&self.stats.written);
        let offered = self.backfill();
        self.settle().await;
        let counts = self
            .backend
            .counts(&self.run, self.model().as_deref().unwrap_or(""))
            .await
            .unwrap_or_default();
        self.audit(format!(
            "MEMORY reindex offered={} written={} backlog={}",
            offered.canon + offered.journal + offered.grievances,
            Stats::get(&self.stats.written) - before,
            counts.backlog
        ));
        json!({
            "status": "reindexed",
            "offered": {"canon": offered.canon, "journal": offered.journal, "grievances": offered.grievances, "unreadable": offered.unreadable},
            "newly_written": Stats::get(&self.stats.written) - before,
            "events": counts.total,
            "embedding_backlog": counts.backlog,
            "note": "table lines, rolls, narration and private clues are not journaled elsewhere and are not rebuilt",
        })
    }

    /// Drops the run's rebuildable events and the dossiers built from them; with `all`, every
    /// event and the recall records too (a per-run erasure).
    pub(in crate::gameplay) async fn delete_run_index_json(&self, all: bool) -> Value {
        match self.backend.delete_run(&self.run, !all).await {
            Ok(removed) => {
                super::locked(&self.query_cache).clear();
                self.audit(format!(
                    "MEMORY delete_run_index all={all} events={} dossiers={} recalls={}",
                    removed.events, removed.dossiers, removed.recalls
                ));
                json!({
                    "status": "deleted",
                    "scope": if all { "every event of this run, its recall records and dossiers" } else { "rebuildable events and the dossiers built from them" },
                    "events": removed.events,
                    "dossiers": removed.dossiers,
                    "recall_records": removed.recalls,
                    "next": if all { "the run's recorded story is gone; nothing can rebuild table lines or rolls" } else { "run `dev memory reindex` to write them again from the canonical sources" },
                })
            }
            Err(error) => json!({"status": "failed", "error": error.to_string()}),
        }
    }

    /// Erases one hero's own words (see [`StoryMemory::erase_hero`]).
    pub(in crate::gameplay) async fn erase_hero_json(&self, hero: &str) -> Value {
        match self.erase_hero(hero).await {
            Ok(removed) => json!({
                "status": "erased",
                "hero": hero,
                "events_redacted": removed.events,
                "dossiers_dropped": removed.dossiers,
                "recall_records_dropped": removed.recalls,
            }),
            Err(error) => json!({"status": "failed", "error": error}),
        }
    }

    /// Erases a hero's own words: the player's speech, the grievances recorded from it, and every
    /// private clue for them. Rows stay as redacted places; the text, vectors and full-text
    /// entries go, as do the dossiers and recall records derived from them.
    pub(in crate::gameplay) async fn erase_hero(
        &self,
        hero: &str,
    ) -> Result<df_persistence::story_memory::Removed, String> {
        let hero = super::capture::slug(hero);
        match self.backend.erase_hero(&self.run, &hero).await {
            Ok(removed) => {
                super::locked(&self.query_cache).clear();
                self.audit(format!(
                    "MEMORY erase hero={hero} events={} dossiers={} recalls={}",
                    removed.events, removed.dossiers, removed.recalls
                ));
                Ok(removed)
            }
            Err(error) => Err(error.to_string()),
        }
    }
}
