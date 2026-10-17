//! The story memory: what the Dungeon Master remembers of this run, so the story stays
//! consistent. Design note: `planning/story-memory.md`.
//!
//! * **Capture** ([`capture`]): every public table line is tapped from the table log, the
//!   structured sources (canon, grievances, scene changes, rulings, narration, private clues)
//!   call the capture API, and existing read-only accessors are backfilled at start
//!   ([`backfill`]). A write is a `try_send` into a bounded queue: it never blocks or slows play.
//!   A background writer appends batches; a background embedder fills in vectors afterwards,
//!   never inside any commit.
//! * **Embeddings** ([`embedder`]): `text-embedding-3-small` behind the [`Embedder`] trait, every
//!   call recorded on the hub's spend ledger. No key means events are stored and searched by
//!   full text only.
//! * **Recall** ([`recall`]): one call before a generator writes. It filters by audience first,
//!   fuses vector, full-text and entity ranks (reciprocal rank fusion), pins promises, grievances
//!   and canon about the named entities, and renders about 1.5 KB framed as data. The recalled
//!   ids and the pack are recorded so a replay reads the recording and calls no provider.
//! * **Consistency** ([`consistency`]): Jev says whether a proposed fact contradicts, extends or
//!   is unrelated to the closest recorded events. Nothing here deletes; a belief-level event may be
//!   marked superseded, a rules-committed one never is.
//! * **Dev surface** ([`dev`]): `dev memory` reads and a few maintenance writes.
//!
//! The store is a derived projection, never a second source of truth: a row cites its source
//! (`natural_key`, `source`, `source_epoch`/`source_seq`, `game_revision`). Table lines, rolls and
//! narration are not journaled anywhere else today, so a row that a canonical source cannot write
//! again is marked `rebuildable = false` and survives a [`StoryMemory::delete_run_index`].

#![allow(
    dead_code,
    reason = "the capture, recall, consistency and erasure API waits for its callers; see planning/story-memory.md"
)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use df_persistence::story_memory::{NewEvent, Scope};
use tokio::sync::{Notify, mpsc};

use super::resources::SpendLedger;

pub(in crate::gameplay) mod backfill;
pub(in crate::gameplay) mod capture;
pub(in crate::gameplay) mod consistency;
pub(in crate::gameplay) mod dev;
pub(in crate::gameplay) mod dossier;
pub(in crate::gameplay) mod embedder;
pub(in crate::gameplay) mod recall;
pub(in crate::gameplay) mod startup;
pub(in crate::gameplay) mod store;
#[cfg(test)]
mod test_store;
#[cfg(test)]
mod tests;

pub(in crate::gameplay) use df_persistence::story_memory::{Audience, EventKind, StoredEvent};
pub(in crate::gameplay) use embedder::Embedder;
pub(in crate::gameplay) use store::Backend;

/// How many captured events may wait for the writer. A full queue drops the new event and counts
/// it: play is never made to wait for memory.
pub(in crate::gameplay) const QUEUE_CAPACITY: usize = 4096;
/// The most events one database transaction writes.
const WRITE_BATCH: usize = 64;

/// Work for the background writer.
pub(in crate::gameplay) enum Job {
    Event(Box<NewEvent>),
    /// A development rewind abandoned the table log from this sequence on.
    RetireTableSince {
        from_seq: i64,
    },
    /// A development rewind abandoned every revision after this one.
    RetireAfterRevision(i64),
}

/// Counters for the status view. All start at zero for a process.
#[derive(Default)]
pub(in crate::gameplay) struct Stats {
    pub(in crate::gameplay) submitted: AtomicU64,
    pub(in crate::gameplay) dropped: AtomicU64,
    pub(in crate::gameplay) written: AtomicU64,
    pub(in crate::gameplay) duplicates: AtomicU64,
    pub(in crate::gameplay) write_failures: AtomicU64,
    pub(in crate::gameplay) embedded: AtomicU64,
    pub(in crate::gameplay) embed_failures: AtomicU64,
    pub(in crate::gameplay) recalls: AtomicU64,
    pub(in crate::gameplay) violations: AtomicU64,
    pub(in crate::gameplay) retired: AtomicU64,
}

impl Stats {
    pub(in crate::gameplay) fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    fn processed(&self) -> u64 {
        Self::get(&self.written)
            + Self::get(&self.duplicates)
            + Self::get(&self.write_failures)
            + Self::get(&self.retired)
    }
}

/// How the memory is tuned. There is no spend cap: embeddings are recorded and audited on the
/// hub's ledger and never refused for cost (owner decision).
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct Config {
    pub(in crate::gameplay) run_id: String,
    /// Dossiers (rolling per-entity summaries) are read into storyteller recalls only.
    pub(in crate::gameplay) dossiers: bool,
    pub(in crate::gameplay) queue_capacity: usize,
}

impl Config {
    pub(in crate::gameplay) fn new(run_id: &str) -> Self {
        Self {
            run_id: run_id.chars().take(128).collect(),
            dossiers: true,
            queue_capacity: QUEUE_CAPACITY,
        }
    }
}

/// What the search can use right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SearchMode {
    /// Vector and full text, fused.
    Hybrid,
    /// Full text only, and why.
    TextOnly(&'static str),
}

impl SearchMode {
    pub(in crate::gameplay) fn label(&self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::TextOnly(_) => "text_only",
        }
    }
}

const AUDIT_RING: usize = 200;

pub(in crate::gameplay) struct StoryMemory {
    pub(in crate::gameplay) run: String,
    /// This process's table-log lifetime: table sequences restart at 1 per process, so a
    /// table line's natural key carries this.
    pub(in crate::gameplay) boot: String,
    pub(in crate::gameplay) backend: Backend,
    pub(in crate::gameplay) embedder: Option<Arc<dyn Embedder>>,
    pub(in crate::gameplay) ledger: Arc<SpendLedger>,
    pub(in crate::gameplay) config: Config,
    pub(in crate::gameplay) stats: Stats,
    queue: mpsc::Sender<Job>,
    inbox: tokio::sync::Mutex<Option<mpsc::Receiver<Job>>>,
    wake: Arc<Notify>,
    embedding: tokio::sync::Mutex<()>,
    /// Hero seat index to hero name, learned from the hero lines that pass the tap.
    pub(in crate::gameplay) roster: Mutex<BTreeMap<u32, String>>,
    pub(in crate::gameplay) scene: Mutex<Option<String>>,
    pub(in crate::gameplay) query_cache: Mutex<VecDeque<(String, Vec<f32>)>>,
    pub(in crate::gameplay) writer: Mutex<Option<Arc<dyn dossier::DossierWriter>>>,
    pub(in crate::gameplay) dossier_flights: Mutex<BTreeSet<String>>,
    pub(in crate::gameplay) sources: Mutex<Option<backfill::Sources>>,
    audit_ring: Mutex<VecDeque<String>>,
    last_error: Mutex<String>,
}

pub(in crate::gameplay) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl StoryMemory {
    /// A memory over `backend`. No task runs until [`Self::spawn_workers`]; tests drive the same
    /// work with [`Self::settle`].
    pub(in crate::gameplay) fn new(
        backend: Backend,
        embedder: Option<Arc<dyn Embedder>>,
        ledger: Arc<SpendLedger>,
        config: Config,
    ) -> Arc<Self> {
        let (queue, inbox) = mpsc::channel(config.queue_capacity.max(1));
        let boot = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        Arc::new(Self {
            run: config.run_id.clone(),
            boot: format!("{boot:x}"),
            backend,
            embedder,
            ledger,
            config,
            stats: Stats::default(),
            queue,
            inbox: tokio::sync::Mutex::new(Some(inbox)),
            wake: Arc::new(Notify::new()),
            embedding: tokio::sync::Mutex::new(()),
            roster: Mutex::new(BTreeMap::new()),
            scene: Mutex::new(None),
            query_cache: Mutex::new(VecDeque::new()),
            writer: Mutex::new(None),
            dossier_flights: Mutex::new(BTreeSet::new()),
            sources: Mutex::new(None),
            audit_ring: Mutex::new(VecDeque::new()),
            last_error: Mutex::new(String::new()),
        })
    }

    pub(in crate::gameplay) fn mode(&self) -> SearchMode {
        if self.embedder.is_none() {
            SearchMode::TextOnly("no embedding key")
        } else if !self.backend.vector_enabled() {
            SearchMode::TextOnly("vector extension unavailable")
        } else {
            SearchMode::Hybrid
        }
    }

    pub(in crate::gameplay) fn model(&self) -> Option<String> {
        self.embedder.as_ref().map(|e| e.model().to_owned())
    }

    /// The scope of an ordinary active-events read.
    pub(in crate::gameplay) fn scope(&self, audience: Audience) -> Scope {
        Scope::active(&self.run, audience)
    }

    /// One `MEMORY ...` audit line, also kept in a short ring for the status view.
    pub(in crate::gameplay) fn audit(&self, line: String) {
        {
            let mut ring = locked(&self.audit_ring);
            if ring.len() >= AUDIT_RING {
                ring.pop_front();
            }
            ring.push_back(line.clone());
        }
        super::actor::record_audit_line("gameplay.memory", line);
    }

    /// The newest audit lines, oldest first.
    pub(in crate::gameplay) fn recent_audit(&self, last: usize) -> Vec<String> {
        let ring = locked(&self.audit_ring);
        ring.iter()
            .skip(ring.len().saturating_sub(last))
            .cloned()
            .collect()
    }

    pub(in crate::gameplay) fn note_error(&self, error: impl std::fmt::Display) {
        *locked(&self.last_error) = error.to_string().chars().take(200).collect();
    }

    pub(in crate::gameplay) fn last_error(&self) -> String {
        locked(&self.last_error).clone()
    }

    /// Queues one job without waiting. A full queue drops it and says so.
    pub(in crate::gameplay) fn submit(&self, job: Job) {
        match self.queue.try_send(job) {
            Ok(()) => {
                self.stats.submitted.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                let dropped = self.stats.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped == 1 || dropped % 100 == 0 {
                    self.audit(format!(
                        "MEMORY queue_full dropped={dropped} capacity={}",
                        self.config.queue_capacity
                    ));
                }
            }
        }
    }

    /// Starts the writer and the embedder on `runtime`. Both end when the memory is dropped.
    pub(in crate::gameplay) fn spawn_workers(self: &Arc<Self>, runtime: &tokio::runtime::Handle) {
        let weak = Arc::downgrade(self);
        let writer = weak.clone();
        let inbox = self
            .inbox
            .try_lock()
            .ok()
            .and_then(|mut inbox| inbox.take());
        if let Some(mut inbox) = inbox {
            runtime.spawn(async move {
                while let Some(first) = inbox.recv().await {
                    let mut batch = vec![first];
                    while batch.len() < WRITE_BATCH {
                        match inbox.try_recv() {
                            Ok(job) => batch.push(job),
                            Err(_) => break,
                        }
                    }
                    let Some(memory) = writer.upgrade() else {
                        break;
                    };
                    memory.write_jobs(batch).await;
                }
            });
        }
        let wake = Arc::clone(&self.wake);
        runtime.spawn(async move {
            let mut failures = 0_u32;
            loop {
                // No strong reference is held while waiting, so a dropped memory ends this task.
                if failures == 0 {
                    let _ = tokio::time::timeout(Duration::from_secs(30), wake.notified()).await;
                } else {
                    tokio::time::sleep(Duration::from_secs(1 << failures.min(5))).await;
                }
                let Some(memory) = weak.upgrade() else {
                    break;
                };
                let before = Stats::get(&memory.stats.embed_failures);
                while memory.embed_pending().await > 0 {}
                failures = if Stats::get(&memory.stats.embed_failures) > before {
                    failures + 1
                } else {
                    0
                };
            }
        });
    }

    /// Writes a batch of jobs. A failed transaction is retried a few times with a short pause;
    /// then its events are counted lost and audited (the status view shows the count).
    pub(in crate::gameplay) async fn write_jobs(&self, jobs: Vec<Job>) {
        let mut events: Vec<NewEvent> = Vec::new();
        let mut maintenance = Vec::new();
        for job in jobs {
            match job {
                Job::Event(event) => events.push(*event),
                other => maintenance.push(other),
            }
        }
        if !events.is_empty() {
            let mut attempt = 0_u32;
            loop {
                match self.backend.insert_events(&events).await {
                    Ok(inserted) => {
                        let created = inserted.iter().filter(|row| row.created).count() as u64;
                        self.stats.written.fetch_add(created, Ordering::Relaxed);
                        self.stats
                            .duplicates
                            .fetch_add(inserted.len() as u64 - created, Ordering::Relaxed);
                        if created > 0 {
                            self.audit(format!(
                                "MEMORY write events={} new={created} first_id={} last_id={}",
                                inserted.len(),
                                inserted
                                    .iter()
                                    .filter(|r| r.created)
                                    .map(|r| r.event_id)
                                    .min()
                                    .unwrap_or(0),
                                inserted
                                    .iter()
                                    .filter(|r| r.created)
                                    .map(|r| r.event_id)
                                    .max()
                                    .unwrap_or(0),
                            ));
                            self.wake.notify_one();
                        }
                        break;
                    }
                    Err(error) => {
                        attempt += 1;
                        self.note_error(&error);
                        if attempt >= 3 {
                            self.stats
                                .write_failures
                                .fetch_add(events.len() as u64, Ordering::Relaxed);
                            self.audit(format!(
                                "MEMORY write_failed events={} error={}",
                                events.len(),
                                self.last_error()
                            ));
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                    }
                }
            }
        }
        for job in maintenance {
            let result = match job {
                Job::RetireTableSince { from_seq } => {
                    self.backend
                        .retire_source_since(
                            &self.run,
                            "table_log",
                            &self.boot,
                            from_seq,
                            "rewound",
                        )
                        .await
                }
                Job::RetireAfterRevision(revision) => {
                    self.backend
                        .retire_revisions_after(&self.run, revision, "rewound")
                        .await
                }
                Job::Event(_) => continue,
            };
            match result {
                Ok(count) => self.audit(format!("MEMORY retire rewound events={count}")),
                Err(error) => {
                    self.note_error(&error);
                    self.audit(format!("MEMORY retire_failed error={}", self.last_error()));
                }
            }
            self.stats.retired.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Embeds one batch of events that have no vector for the current model. Returns how many got
    /// one; zero when none are waiting, the search is text-only, or the provider failed.
    pub(in crate::gameplay) async fn embed_pending(&self) -> usize {
        let Some(embedder) = &self.embedder else {
            return 0;
        };
        if !self.backend.vector_enabled() {
            return 0;
        }
        let _one_at_a_time = self.embedding.lock().await;
        let model = embedder.model().to_owned();
        let pending = match self
            .backend
            .pending_embeddings(&self.run, &model, embedder::BATCH)
            .await
        {
            Ok(pending) if !pending.is_empty() => pending,
            Ok(_) => return 0,
            Err(error) => {
                self.note_error(&error);
                return 0;
            }
        };
        let texts: Vec<String> = pending.iter().map(|row| row.text.clone()).collect();
        let started = std::time::Instant::now();
        match embedder::embed_charged(embedder.as_ref(), &self.ledger, &texts).await {
            Ok(answer) => {
                let items: Vec<(i64, Vec<f32>)> = pending
                    .iter()
                    .map(|row| row.event_id)
                    .zip(answer.vectors)
                    .collect();
                match self
                    .backend
                    .store_embeddings(&self.run, &model, &items)
                    .await
                {
                    Ok(stored) => {
                        self.stats.embedded.fetch_add(stored, Ordering::Relaxed);
                        self.audit(format!(
                            "MEMORY embed model={model} events={} tokens={} ms={}",
                            items.len(),
                            answer
                                .tokens
                                .unwrap_or_else(|| embedder::estimate_tokens(&texts)),
                            started.elapsed().as_millis()
                        ));
                        usize::try_from(stored).unwrap_or(0)
                    }
                    Err(error) => {
                        self.stats.embed_failures.fetch_add(1, Ordering::Relaxed);
                        self.note_error(&error);
                        0
                    }
                }
            }
            Err(failure) => {
                self.stats.embed_failures.fetch_add(1, Ordering::Relaxed);
                self.note_error(failure.label());
                self.audit(format!(
                    "MEMORY embed_failed model={model} events={} error={}",
                    texts.len(),
                    failure.label()
                ));
                0
            }
        }
    }

    /// Runs everything queued to completion: writes the queue, then embeds what is waiting. For
    /// tests and for `dev memory reindex`; production play never waits on it.
    pub(in crate::gameplay) async fn settle(&self) {
        let taken = self.inbox.lock().await.is_some();
        if taken {
            loop {
                let mut batch = Vec::new();
                {
                    let mut inbox = self.inbox.lock().await;
                    if let Some(inbox) = inbox.as_mut() {
                        while batch.len() < WRITE_BATCH {
                            match inbox.try_recv() {
                                Ok(job) => batch.push(job),
                                Err(_) => break,
                            }
                        }
                    }
                }
                if batch.is_empty() {
                    break;
                }
                self.write_jobs(batch).await;
            }
        } else {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while Stats::get(&self.stats.submitted) > self.stats.processed()
                && std::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        while self.embed_pending().await > 0 {}
    }
}

/// The memory of the run being played, for the one-line capture calls of the structured
/// sources. Unset in tests and in a run with memory off, where every capture call does nothing.
static CURRENT: Mutex<Option<Arc<StoryMemory>>> = Mutex::new(None);

pub(in crate::gameplay) fn install(memory: Arc<StoryMemory>) {
    *locked(&CURRENT) = Some(memory);
}

pub(in crate::gameplay) fn current() -> Option<Arc<StoryMemory>> {
    locked(&CURRENT).clone()
}

#[cfg(test)]
pub(in crate::gameplay) fn uninstall() {
    *locked(&CURRENT) = None;
}
