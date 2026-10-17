//! An in-process stand-in for the PostgreSQL store, for tests only. It keeps the same contract:
//! the audience filter is applied before ranking, a natural key is stored once, rows are never
//! rewritten except for status, links and vectors, and the vector and text searches rank the
//! way the real indexes do (cosine similarity; shared words weighted by rarity).

use std::sync::Mutex;

use df_persistence::story_memory::{
    Counts, DossierRow, ERASED_TEXT, Entities, Inserted, NewEvent, PendingEmbedding, RecallRecord,
    Relevance, Removed, Scope, Scored, Status, StoredEvent, StoryMemoryError,
};

use super::store::StoreResult;

struct Row {
    event: StoredEvent,
    source_epoch: Option<String>,
    source_seq: Option<i64>,
    vector: Option<(String, Vec<f32>)>,
}

#[derive(Default)]
struct Inner {
    rows: Vec<Row>,
    next_id: i64,
    recalls: Vec<(i64, RecallRecord)>,
    dossiers: Vec<(String, DossierRow)>,
}

pub(in crate::gameplay) struct TestStore {
    inner: Mutex<Inner>,
    vector: bool,
}

fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.chars().count() >= 2 && !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

fn visible(scope: &Scope, row: &Row) -> bool {
    let event = &row.event;
    event.run_id == scope.run_id
        && scope
            .audience
            .allows(event.visibility, event.private_to.as_deref())
        && (scope.include_superseded || event.status == Status::Active)
        && (scope.kinds.is_empty() || scope.kinds.contains(&event.kind))
}

impl TestStore {
    pub(in crate::gameplay) fn new(vector: bool) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            vector,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(in crate::gameplay) fn vector_enabled(&self) -> bool {
        self.vector
    }

    /// Every stored event of a run, oldest first, whatever its audience (tests only).
    pub(in crate::gameplay) fn all(&self, run: &str) -> Vec<StoredEvent> {
        self.lock()
            .rows
            .iter()
            .filter(|row| row.event.run_id == run)
            .map(|row| row.event.clone())
            .collect()
    }

    pub(in crate::gameplay) async fn insert_events(
        &self,
        events: &[NewEvent],
    ) -> StoreResult<Vec<Inserted>> {
        let mut inner = self.lock();
        let mut out = Vec::new();
        for new in events {
            if new.text.trim().is_empty()
                || new.text.chars().count() > df_persistence::story_memory::MAXIMUM_BODY_CHARACTERS
                || (new.visibility == df_persistence::story_memory::Visibility::Private)
                    != new.private_to.is_some()
            {
                return Err(StoryMemoryError::Invalid("event fields"));
            }
            if let Some(row) = inner.rows.iter().find(|row| {
                row.event.run_id == new.run_id && row.event.natural_key == new.natural_key
            }) {
                out.push(Inserted {
                    event_id: row.event.event_id,
                    created: false,
                });
                continue;
            }
            inner.next_id += 1;
            let event_id = inner.next_id;
            inner.rows.push(Row {
                event: StoredEvent {
                    event_id,
                    run_id: new.run_id.clone(),
                    natural_key: new.natural_key.clone(),
                    source: new.source.clone(),
                    game_revision: new.game_revision,
                    created_ms: i64::try_from(event_id).unwrap_or(0),
                    kind: new.kind,
                    visibility: new.visibility,
                    private_to: new.private_to.clone(),
                    text: new.text.clone(),
                    scene: new.scene.clone(),
                    entities: new.entities.clone().normalised(),
                    rebuildable: new.rebuildable,
                    status: Status::Active,
                    supersedes_id: new.supersedes,
                    superseded_by: None,
                    retired_reason: None,
                    embedding_model: None,
                    embedded: false,
                },
                source_epoch: new.source_epoch.clone(),
                source_seq: new.source_seq,
                vector: None,
            });
            if let Some(old) = new.supersedes {
                if let Some(row) = inner.rows.iter_mut().find(|row| {
                    row.event.run_id == new.run_id
                        && row.event.event_id == old
                        && row.event.status == Status::Active
                }) {
                    row.event.status = Status::Superseded;
                    row.event.superseded_by = Some(event_id);
                }
            }
            out.push(Inserted {
                event_id,
                created: true,
            });
        }
        Ok(out)
    }

    pub(in crate::gameplay) async fn pending_embeddings(
        &self,
        run: &str,
        model: &str,
        limit: usize,
    ) -> StoreResult<Vec<PendingEmbedding>> {
        if !self.vector {
            return Ok(Vec::new());
        }
        Ok(self
            .lock()
            .rows
            .iter()
            .filter(|row| {
                row.event.run_id == run
                    && !row.event.erased()
                    && row.vector.as_ref().is_none_or(|(have, _)| have != model)
            })
            .take(limit.clamp(1, 256))
            .map(|row| PendingEmbedding {
                event_id: row.event.event_id,
                text: row.event.text.clone(),
            })
            .collect())
    }

    pub(in crate::gameplay) async fn store_embeddings(
        &self,
        run: &str,
        model: &str,
        items: &[(i64, Vec<f32>)],
    ) -> StoreResult<u64> {
        let mut inner = self.lock();
        let mut stored = 0;
        for (id, vector) in items {
            if let Some(row) = inner.rows.iter_mut().find(|row| {
                row.event.run_id == run && row.event.event_id == *id && !row.event.erased()
            }) {
                row.vector = Some((model.to_owned(), vector.clone()));
                row.event.embedding_model = Some(model.to_owned());
                row.event.embedded = true;
                stored += 1;
            }
        }
        Ok(stored)
    }

    pub(in crate::gameplay) async fn vector_search(
        &self,
        scope: &Scope,
        model: &str,
        query: &[f32],
        limit: usize,
    ) -> StoreResult<Vec<Scored>> {
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let query_norm = norm(query);
        let mut scored: Vec<Scored> = self
            .lock()
            .rows
            .iter()
            .filter(|row| visible(scope, row))
            .filter_map(|row| {
                let (have, vector) = row.vector.as_ref()?;
                if have != model || vector.len() != query.len() {
                    return None;
                }
                let dot: f32 = vector.iter().zip(query).map(|(a, b)| a * b).sum();
                let denominator = norm(vector) * query_norm;
                Some(Scored {
                    event_id: row.event.event_id,
                    score: if denominator > 0.0 {
                        f64::from(dot / denominator)
                    } else {
                        0.0
                    },
                })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.event_id.cmp(&a.event_id))
        });
        scored.truncate(limit.clamp(1, 64));
        Ok(scored)
    }

    pub(in crate::gameplay) async fn text_search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<Scored>> {
        let wanted = words(query);
        let inner = self.lock();
        let documents: Vec<(i64, Vec<String>, bool)> = inner
            .rows
            .iter()
            .filter(|row| row.event.run_id == scope.run_id)
            .map(|row| {
                (
                    row.event.event_id,
                    words(&row.event.text),
                    visible(scope, row),
                )
            })
            .collect();
        let total = documents.len() as f64;
        let mut scored: Vec<Scored> = documents
            .iter()
            .filter(|(_, _, allowed)| *allowed)
            .filter_map(|(id, text_words, _)| {
                let mut score = 0.0;
                for word in &wanted {
                    if text_words.contains(word) {
                        let frequency = documents
                            .iter()
                            .filter(|(_, other, _)| other.contains(word))
                            .count() as f64;
                        score += (1.0 + total / (1.0 + frequency)).ln();
                    }
                }
                (score > 0.0).then_some(Scored {
                    event_id: *id,
                    score,
                })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.event_id.cmp(&a.event_id))
        });
        scored.truncate(limit.clamp(1, 64));
        Ok(scored)
    }

    pub(in crate::gameplay) async fn select_events(
        &self,
        scope: &Scope,
        relevance: &Relevance,
        limit: usize,
    ) -> StoreResult<Vec<StoredEvent>> {
        let mut found: Vec<StoredEvent> = self
            .lock()
            .rows
            .iter()
            .filter(|row| visible(scope, row))
            .filter(|row| {
                let selective = !relevance.entities.is_empty()
                    || relevance.scene.is_some()
                    || relevance.unattached;
                !selective
                    || relevance
                        .entities
                        .iter()
                        .any(|entity| row.event.entities.all().contains(&entity.as_str()))
                    || relevance
                        .scene
                        .as_ref()
                        .is_some_and(|scene| row.event.scene.as_ref() == Some(scene))
                    || (relevance.unattached && row.event.entities.is_empty())
            })
            .map(|row| row.event.clone())
            .collect();
        found.sort_by_key(|event| std::cmp::Reverse(event.event_id));
        found.truncate(limit.clamp(1, 64));
        Ok(found)
    }

    pub(in crate::gameplay) async fn events_by_ids(
        &self,
        scope: &Scope,
        ids: &[i64],
    ) -> StoreResult<Vec<StoredEvent>> {
        let inner = self.lock();
        Ok(ids
            .iter()
            .filter_map(|id| {
                inner
                    .rows
                    .iter()
                    .find(|row| row.event.event_id == *id && visible(scope, row))
                    .map(|row| row.event.clone())
            })
            .collect())
    }

    pub(in crate::gameplay) async fn operator_event(
        &self,
        run: &str,
        id: i64,
    ) -> StoreResult<Option<StoredEvent>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .find(|row| row.event.run_id == run && row.event.event_id == id)
            .map(|row| row.event.clone()))
    }

    pub(in crate::gameplay) async fn entity_event_ids(
        &self,
        scope: &Scope,
        entity: &str,
    ) -> StoreResult<Option<Vec<(i64, Status)>>> {
        Ok(Some(
            self.lock()
                .rows
                .iter()
                .filter(|row| visible(scope, row) && row.event.entities.all().contains(&entity))
                .map(|row| (row.event.event_id, row.event.status))
                .collect(),
        ))
    }

    pub(in crate::gameplay) async fn entity_events(
        &self,
        scope: &Scope,
        entity: &str,
        after_id: i64,
        limit: usize,
    ) -> StoreResult<Vec<StoredEvent>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .filter(|row| {
                visible(scope, row)
                    && row.event.entities.all().contains(&entity)
                    && row.event.event_id > after_id
            })
            .take(limit.clamp(1, 64))
            .map(|row| row.event.clone())
            .collect())
    }

    pub(in crate::gameplay) async fn supersede(
        &self,
        run: &str,
        old: i64,
        new: i64,
    ) -> StoreResult<bool> {
        let mut inner = self.lock();
        let known = |inner: &Inner, id: i64| {
            inner
                .rows
                .iter()
                .any(|row| row.event.run_id == run && row.event.event_id == id)
        };
        if old == new || !known(&inner, old) || !known(&inner, new) {
            return Err(StoryMemoryError::Invalid("unknown event"));
        }
        let mut changed = false;
        for row in inner.rows.iter_mut().filter(|row| row.event.run_id == run) {
            if row.event.event_id == old && row.event.status == Status::Active {
                row.event.status = Status::Superseded;
                row.event.superseded_by = Some(new);
                changed = true;
            }
            if row.event.event_id == new && row.event.supersedes_id.is_none() {
                row.event.supersedes_id = Some(old);
            }
        }
        Ok(changed)
    }

    pub(in crate::gameplay) async fn retire_source_since(
        &self,
        run: &str,
        source: &str,
        epoch: &str,
        from_seq: i64,
        reason: &str,
    ) -> StoreResult<u64> {
        let mut count = 0;
        for row in self.lock().rows.iter_mut() {
            if row.event.run_id == run
                && row.event.source == source
                && row.source_epoch.as_deref() == Some(epoch)
                && row.source_seq.is_some_and(|seq| seq >= from_seq)
                && row.event.status == Status::Active
            {
                row.event.status = Status::Superseded;
                row.event.retired_reason = Some(reason.to_owned());
                count += 1;
            }
        }
        Ok(count)
    }

    pub(in crate::gameplay) async fn retire_revisions_after(
        &self,
        run: &str,
        revision: i64,
        reason: &str,
    ) -> StoreResult<u64> {
        let mut count = 0;
        for row in self.lock().rows.iter_mut() {
            if row.event.run_id == run
                && row.event.game_revision.is_some_and(|r| r > revision)
                && row.event.status == Status::Active
            {
                row.event.status = Status::Superseded;
                row.event.retired_reason = Some(reason.to_owned());
                count += 1;
            }
        }
        Ok(count)
    }

    pub(in crate::gameplay) async fn counts(&self, run: &str, model: &str) -> StoreResult<Counts> {
        let inner = self.lock();
        let mut counts = Counts::default();
        for kind in df_persistence::story_memory::EventKind::ALL {
            let rows: Vec<&Row> = inner
                .rows
                .iter()
                .filter(|row| row.event.run_id == run && row.event.kind == kind)
                .collect();
            if rows.is_empty() {
                continue;
            }
            let active = rows
                .iter()
                .filter(|r| r.event.status == Status::Active)
                .count() as i64;
            counts.total += rows.len() as i64;
            counts
                .by_kind
                .push((kind.label().to_owned(), active, rows.len() as i64 - active));
        }
        for row in inner.rows.iter().filter(|row| row.event.run_id == run) {
            counts.rebuildable += i64::from(row.event.rebuildable);
            if self.vector {
                match &row.vector {
                    Some((have, _)) => {
                        counts.embedded += 1;
                        match counts.models.iter_mut().find(|(model, _)| model == have) {
                            Some(entry) => entry.1 += 1,
                            None => counts.models.push((have.clone(), 1)),
                        }
                        if have != model {
                            counts.backlog += 1;
                        }
                    }
                    None if !row.event.erased() => counts.backlog += 1,
                    None => {}
                }
            }
        }
        counts.recalls = inner
            .recalls
            .iter()
            .filter(|(_, r)| r.run_id == run)
            .count() as i64;
        counts.dossiers = inner.dossiers.iter().filter(|(r, _)| r == run).count() as i64;
        Ok(counts)
    }

    pub(in crate::gameplay) async fn delete_run(
        &self,
        run: &str,
        rebuildable_only: bool,
    ) -> StoreResult<Removed> {
        let mut inner = self.lock();
        let mut removed = Removed::default();
        let dossiers = inner.dossiers.len();
        inner.dossiers.retain(|(r, _)| r != run);
        removed.dossiers = (dossiers - inner.dossiers.len()) as u64;
        if !rebuildable_only {
            let recalls = inner.recalls.len();
            inner.recalls.retain(|(_, r)| r.run_id != run);
            removed.recalls = (recalls - inner.recalls.len()) as u64;
        }
        let rows = inner.rows.len();
        inner
            .rows
            .retain(|row| row.event.run_id != run || (rebuildable_only && !row.event.rebuildable));
        removed.events = (rows - inner.rows.len()) as u64;
        Ok(removed)
    }

    pub(in crate::gameplay) async fn erase_hero(
        &self,
        run: &str,
        hero: &str,
    ) -> StoreResult<Removed> {
        use df_persistence::story_memory::EventKind;
        let mut inner = self.lock();
        let mut ids = Vec::new();
        for row in inner.rows.iter_mut().filter(|row| row.event.run_id == run) {
            let event = &mut row.event;
            let own_words = matches!(
                event.kind,
                EventKind::HeroSpeech | EventKind::GrievanceAttitude
            ) && event.entities.hero.iter().any(|h| h == hero);
            if !event.erased() && (own_words || event.private_to.as_deref() == Some(hero)) {
                event.text = ERASED_TEXT.to_owned();
                event.status = Status::Superseded;
                event.retired_reason = Some("erased".to_owned());
                event.embedded = false;
                event.embedding_model = None;
                row.vector = None;
                ids.push(event.event_id);
            }
        }
        let mut removed = Removed {
            events: ids.len() as u64,
            ..Removed::default()
        };
        let dossiers = inner.dossiers.len();
        inner
            .dossiers
            .retain(|(r, d)| r != run || !d.covered_ids.iter().any(|id| ids.contains(id)));
        removed.dossiers = (dossiers - inner.dossiers.len()) as u64;
        let recalls = inner.recalls.len();
        let member = format!("member:{hero}");
        inner.recalls.retain(|(_, r)| {
            r.run_id != run
                || !(r
                    .pinned_ids
                    .iter()
                    .chain(&r.related_ids)
                    .any(|id| ids.contains(id))
                    || r.speaker.as_deref() == Some(hero)
                    || r.addressee.as_deref() == Some(hero)
                    || r.audience == member)
        });
        removed.recalls = (recalls - inner.recalls.len()) as u64;
        Ok(removed)
    }

    pub(in crate::gameplay) async fn log_recall(&self, record: &RecallRecord) -> StoreResult<i64> {
        let mut inner = self.lock();
        let id = i64::try_from(inner.recalls.len()).unwrap_or(0) + 1;
        inner.recalls.push((id, record.clone()));
        Ok(id)
    }

    pub(in crate::gameplay) async fn recall(
        &self,
        run: &str,
        id: i64,
    ) -> StoreResult<Option<RecallRecord>> {
        Ok(self
            .lock()
            .recalls
            .iter()
            .find(|(recall, record)| *recall == id && record.run_id == run)
            .map(|(_, record)| record.clone()))
    }

    pub(in crate::gameplay) async fn dossier(
        &self,
        run: &str,
        entity: &str,
        audience: &str,
    ) -> StoreResult<Option<DossierRow>> {
        Ok(self
            .lock()
            .dossiers
            .iter()
            .find(|(r, d)| r == run && d.entity_id == entity && d.audience == audience)
            .map(|(_, d)| d.clone()))
    }

    pub(in crate::gameplay) async fn put_dossier(
        &self,
        run: &str,
        dossier: &DossierRow,
    ) -> StoreResult<()> {
        let mut inner = self.lock();
        inner.dossiers.retain(|(r, d)| {
            !(r == run && d.entity_id == dossier.entity_id && d.audience == dossier.audience)
        });
        inner.dossiers.push((run.to_owned(), dossier.clone()));
        Ok(())
    }
}

#[allow(dead_code)]
fn _entities_are_used(_: &Entities) {}
