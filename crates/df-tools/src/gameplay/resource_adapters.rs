//! The shared seam under the voice, video, text and transcription adapters.
//!
//! [`resources`](super::resources) owns the registry, the failure memory, the attempt limit, the
//! timeout, the freshness check, the spend ledger and the `RESOURCE` audit lines. This module is
//! what the four non-picture adapters add on top of it, once, so that none of them keeps a cache
//! index, an in-flight map, an atomic-write routine or an audit format of its own:
//!
//! * [`Kit::produce`]: one generation of a hub kind with the coalescing of identical concurrent
//!   asks (the hub documents that two concurrent `render` calls both run, and that the provider
//!   layer coalesces; this is that layer), the ready-entry fast path and the payload hand-back.
//! * [`Flights`]: that coalescing, also used inline by the text adapter.
//! * [`CallKind`]: the versioned key and the `RESOURCE` line of the two kinds the hub does not
//!   list yet (`text`, `transcription`), in the hub's own line format.
//! * [`versioned_digest`], [`write_atomic`], [`slug`]: the one key and the one store-write helper.
//!
//! # What waits for the hub
//!
//! Everything marked `TODO(hub)` is a shim for something `resources.rs` should own; the comment
//! says what the hub needs to provide so the shim can be deleted:
//!
//! * `ResourceKind::Text` and `ResourceKind::Transcription` (so [`CallKind`] becomes two specs);
//! * an eraser per kind, called by `invalidate` and `regenerate`, so the stored bytes of an
//!   entry go with it (today [`Kit::invalidate`] does it);
//! * single-flight inside `Hub::render` (today [`Flights`] does it).

use std::{
    collections::HashMap,
    future::Future,
    io,
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::resources::{
    Generated, Generation, Hub, Invalidate, Limits, ResourceFailure, ResourceKey, ResourceKind,
    ResourceSpec, Unit, Validator,
};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lowercase hexadecimal text of `bytes`.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The SHA-256 of `label|version|canonical`, the text [`ResourceKey::of`] hashes. The hub's key
/// is the first 16 bytes of it, so a store file named by the whole digest and the hub entry that
/// points at it are the same identity (`resource_key_is_the_prefix_of_the_versioned_digest`).
pub(super) fn versioned_digest(label: &str, version: u32, canonical: &str) -> [u8; 32] {
    Sha256::digest(format!("{label}|{version}|{canonical}").as_bytes()).into()
}

/// [`versioned_digest`] of a hub spec.
pub(super) fn spec_digest<S: ResourceSpec>(spec: &S) -> [u8; 32] {
    versioned_digest(S::KIND.label(), S::CONTRACT_VERSION, &spec.canonical())
}

/// `text` as a subject token for an audit line: lowercase ASCII letters and digits, with every
/// other run replaced by one `-`, at most `limit` characters. Audit lines are `key=value` words,
/// so a subject may not hold a space.
pub(super) fn slug(text: &str, limit: usize) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            gap = true;
        }
        if slug.len() >= limit {
            break;
        }
    }
    slug
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `directory/name` so that a reader never sees a partial file: a uniquely named
/// staging file beside the target, then a rename over it. The directory is created. Blocking I/O,
/// so an async caller runs it off the async workers.
pub(super) fn write_atomic(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let staging = directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&staging, bytes)
        .and_then(|()| std::fs::rename(&staging, directory.join(name)));
    if written.is_err() {
        // Best effort: a leftover staging file is the only residue of a failed write.
        let _removed = std::fs::remove_file(&staging);
    }
    written
}

/// A kind the hub does not list yet (TODO(hub): `ResourceKind::Text` and
/// `ResourceKind::Transcription`). It gives the kind the hub's versioned key and writes its
/// `RESOURCE` lines in exactly the hub's format, so the audit log reads the same for every kind.
#[derive(Clone, Copy, Debug)]
pub(super) struct CallKind {
    pub(super) label: &'static str,
    pub(super) version: u32,
}

/// Cached generated text (the storyteller). The text itself stays in the storyteller's cache.
pub(super) const TEXT: CallKind = CallKind {
    label: "text",
    version: 1,
};

/// Speech-to-text: a call with no store, so only the audit lines and the spend remain.
pub(super) const TRANSCRIPTION: CallKind = CallKind {
    label: "transcription",
    version: 1,
};

impl CallKind {
    /// The versioned digest of a canonical description of one request.
    pub(super) fn digest(self, canonical: &str) -> [u8; 32] {
        versioned_digest(self.label, self.version, canonical)
    }

    /// One `RESOURCE <state> kind=<label> key=<key> ...` line. `key` is shortened to the hub's 16
    /// bytes so it can be compared with a hub key.
    pub(super) fn line(
        self,
        state: &str,
        key: &str,
        subject: &str,
        took: Duration,
        handle: &str,
        error: &str,
    ) {
        let key: String = key.chars().take(32).collect();
        let line = format!(
            "RESOURCE {state} kind={} key={key} subject={subject} ms={} handle={handle} error={error}",
            self.label,
            took.as_millis()
        );
        super::actor::record_audit_line("gameplay.resource", line);
    }
}

/// Whether the caller started the work or waited for one already running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Role {
    Leader,
    Joined,
}

/// Removes a flight from the map when its leader ends, however it ends (finished, panicked or
/// dropped), so no later caller waits on a flight nobody runs.
struct Lead<P> {
    flights: Arc<Mutex<HashMap<String, watch::Receiver<Option<P>>>>>,
    key: String,
}

impl<P> Drop for Lead<P> {
    fn drop(&mut self) {
        locked(&self.flights).remove(&self.key);
    }
}

enum Claim<P> {
    Join(watch::Receiver<Option<P>>),
    Lead {
        sender: watch::Sender<Option<P>>,
        lead: Lead<P>,
    },
}

/// Identical concurrent asks share one run. The first caller for a key leads; callers that arrive
/// while it runs wait for its value. TODO(hub): `Hub::render` should join an in-flight key and
/// hand every waiter the same result, which makes this type unnecessary.
pub(super) struct Flights<P> {
    running: Arc<Mutex<HashMap<String, watch::Receiver<Option<P>>>>>,
}

impl<P> Clone for Flights<P> {
    fn clone(&self) -> Self {
        Self {
            running: Arc::clone(&self.running),
        }
    }
}

impl<P> Default for Flights<P> {
    fn default() -> Self {
        Self {
            running: Arc::default(),
        }
    }
}

impl<P: Clone + Send + Sync + 'static> Flights<P> {
    fn claim(&self, key: &str) -> Claim<P> {
        let mut running = locked(&self.running);
        if let Some(receiver) = running.get(key) {
            return Claim::Join(receiver.clone());
        }
        let (sender, receiver) = watch::channel(None);
        running.insert(key.to_owned(), receiver);
        Claim::Lead {
            sender,
            lead: Lead {
                flights: Arc::clone(&self.running),
                key: key.to_owned(),
            },
        }
    }

    /// Runs `work` in the caller's task when nobody else is running `key`; otherwise waits for
    /// that run. A leader that is dropped before it finishes hands the key to the next waiter, who
    /// runs its own `work`.
    pub(super) async fn run_inline<W>(&self, key: &str, work: W) -> (P, Role)
    where
        W: Future<Output = P>,
    {
        let mut work = Some(work);
        loop {
            match self.claim(key) {
                Claim::Join(receiver) => {
                    // No value means the leader went away: claim the key again.
                    if let Some(value) = wait_for_value(receiver).await {
                        return (value, Role::Joined);
                    }
                }
                Claim::Lead { sender, lead } => {
                    let Some(work) = work.take() else {
                        continue;
                    };
                    let value = work.await;
                    drop(lead);
                    sender.send_replace(Some(value.clone()));
                    return (value, Role::Leader);
                }
            }
        }
    }

    /// Like [`Self::run_inline`], but the leader runs in its own task, so a caller that goes away
    /// does not cancel work that is already paid for. `None` when that task ended without a value.
    pub(super) async fn run_spawned<W>(&self, key: &str, work: W) -> (Option<P>, Role)
    where
        W: Future<Output = P> + Send + 'static,
    {
        match self.claim(key) {
            Claim::Join(receiver) => (wait_for_value(receiver).await, Role::Joined),
            Claim::Lead { sender, lead } => {
                let receiver = sender.subscribe();
                // The task owns the work to its end: the hub's timeout bounds it and the
                // result is published to every waiter and recorded in the hub.
                tokio::spawn(async move {
                    let value = work.await;
                    drop(lead);
                    sender.send_replace(Some(value));
                });
                (wait_for_value(receiver).await, Role::Leader)
            }
        }
    }
}

async fn wait_for_value<P: Clone>(mut receiver: watch::Receiver<Option<P>>) -> Option<P> {
    receiver
        .wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|finished| finished.clone())
}

/// What a finished generation hands back to the adapter that asked for it.
#[derive(Clone, Debug)]
pub(super) struct Produced<P> {
    pub(super) generated: Generated,
    /// The fresh output, when this run made it or found it in the store; `None` when the hub
    /// answered from its registry and the adapter reads its store by `generated.handle`.
    pub(super) payload: Option<P>,
    /// The output came from the kind's store, not from the provider.
    pub(super) from_store: bool,
}

pub(super) type Outcome<P> = Result<Produced<P>, ResourceFailure>;

/// Erases the stored bytes of an entry the dev tools drop. TODO(hub): `invalidate` and
/// `regenerate` should call one per kind.
pub(super) type Eraser = Arc<dyn Fn(&Generated) + Send + Sync>;

/// The hub plus what the non-picture adapters add to it. Cheap to clone; every adapter of one
/// server holds a clone of the same kit, whose hub is the imagery service's hub.
#[derive(Clone, Default)]
pub(super) struct Kit {
    hub: Hub,
    erasers: Arc<Mutex<HashMap<ResourceKind, Eraser>>>,
}

impl Kit {
    /// A kit over `hub`.
    pub(super) fn over(hub: Hub) -> Self {
        Self {
            hub,
            erasers: Arc::default(),
        }
    }

    pub(super) fn hub(&self) -> &Hub {
        &self.hub
    }

    /// Tunes one kind: how long a generation may run (at least the provider's own deadline plus
    /// any wait for a free provider slot, or the hub fires first), how many attempts a transient
    /// failure gets, how a ready entry is checked for its stored output, and how that output is
    /// erased.
    pub(super) fn tune(
        &self,
        kind: ResourceKind,
        limits: Limits,
        validator: Validator,
        eraser: Eraser,
    ) {
        self.hub.set_limits(kind, limits);
        self.hub.set_validator(kind, validator);
        locked(&self.erasers).insert(kind, eraser);
    }

    /// One generation of a hub kind.
    ///
    /// A ready entry whose output exists is returned at once. Otherwise identical concurrent asks
    /// share one run, and that run goes through [`Hub::render`], which serves a remembered
    /// permanent failure, or a transient one past its attempts, without running `work` at all.
    /// `work` is only started for the leader; it must keep its own output (the fresh bytes) in
    /// [`Produced::payload`], because the hub keeps only the handle.
    pub(super) async fn produce<S, P, W>(
        &self,
        flights: &Flights<Outcome<P>>,
        spec: S,
        work: W,
    ) -> (Outcome<P>, Role)
    where
        S: ResourceSpec + Send + Sync + 'static,
        P: Clone + Send + Sync + 'static,
        W: Future<Output = Outcome<P>> + Send + 'static,
    {
        let key = ResourceKey::of(&spec);
        if let Some(generated) = self.hub.ready(&key) {
            let produced = Produced {
                generated,
                payload: None,
                from_store: true,
            };
            return (Ok(produced), Role::Leader);
        }
        let hub = self.hub.clone();
        let run = async move {
            let carried: Arc<Mutex<Option<Produced<P>>>> = Arc::new(Mutex::new(None));
            let slot = Arc::clone(&carried);
            let generation: Generation = Box::pin(async move {
                let produced = work.await?;
                let generated = produced.generated.clone();
                *locked(&slot) = Some(produced);
                Ok(generated)
            });
            match hub.render(&spec, generation).await {
                Ok(generated) => {
                    let fresh = locked(&carried).take();
                    Ok(fresh.unwrap_or(Produced {
                        generated,
                        payload: None,
                        from_store: true,
                    }))
                }
                Err(failure) => Err(failure),
            }
        };
        let (outcome, role) = flights.run_spawned(key.as_str(), run).await;
        (outcome.unwrap_or(Err(ResourceFailure::Interrupted)), role)
    }

    /// Records `amount` of `unit` as spent on the hub's ledger, at the moment a paid call is about
    /// to be sent. There is no cap, so this never refuses: the ledger keeps the count and writes
    /// the `SPEND` lines. The charge is kept whatever the call's outcome, since a call of unknown
    /// outcome may have been billed.
    pub(super) fn charge(&self, unit: Unit, amount: u64) {
        self.hub.ledger().reserve(unit, amount).commit(amount);
    }

    /// Corrects the spend of `unit` by `delta` once the supplier reports what a call really used
    /// (or refuses it outright, a negative delta).
    pub(super) fn adjust(&self, unit: Unit, delta: i64) {
        self.hub.ledger().reconcile(unit, delta);
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the dev CLI is the caller; it lands with that task"
        )
    )]
    /// Drops the entries that match `target` and erases their stored output, so the next need
    /// generates again instead of finding the old bytes. Returns how many entries were dropped.
    /// This and [`Self::regenerate`] are the dev-tool hooks for the kinds that keep bytes; the
    /// read hooks are [`Hub::status`], [`Hub::list`] and [`Hub::for_subject`].
    pub(super) fn invalidate(&self, target: Invalidate<'_>) -> usize {
        let erasers = locked(&self.erasers).clone();
        for row in self.hub.list(None) {
            let named = match &target {
                Invalidate::Key(wanted) => **wanted == row.key,
                Invalidate::Subject(subject) => row.subject == *subject,
            };
            if !named || row.handle.is_empty() {
                continue;
            }
            if let Some(erase) = erasers.get(&row.kind) {
                erase(&Generated::new(row.handle));
            }
        }
        self.hub.invalidate(target)
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the dev CLI is the caller; it lands with that task"
        )
    )]
    /// Forgets `key` (failure and attempts included) and erases its output; the consumer asks
    /// again on its next need. The entries of these kinds are rendered inline, so the hub's own
    /// `regenerate` (which re-runs a queued entry's maker) has nothing to run. Returns whether the
    /// key was known.
    pub(super) fn regenerate(&self, key: &ResourceKey) -> bool {
        self.invalidate(Invalidate::Key(key)) > 0
    }
}

/// The kit one adapter reads on every use. A new adapter starts on a private kit so that it works
/// alone (tests, the disabled mode); the server then attaches the shared one, once, before
/// anything is asked.
pub(super) struct KitSlot {
    kit: Mutex<Kit>,
}

impl KitSlot {
    pub(super) fn private() -> Self {
        Self {
            kit: Mutex::new(Kit::default()),
        }
    }

    pub(super) fn get(&self) -> Kit {
        locked(&self.kit).clone()
    }

    pub(super) fn attach(&self, kit: &Kit) {
        *locked(&self.kit) = kit.clone();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use df_render::art_contract::Neutral;

    use super::*;
    use crate::gameplay::resources::Chrome;

    struct Sample {
        text: &'static str,
    }

    impl ResourceSpec for Sample {
        const KIND: ResourceKind = ResourceKind::VoiceLine;
        const CONTRACT_VERSION: u32 = 1;

        fn canonical(&self) -> String {
            serde_json::json!({ "text": self.text }).to_string()
        }

        fn subject(&self) -> String {
            "sample:one".to_owned()
        }

        fn chrome(&self) -> Chrome {
            Neutral::Nothing
        }
    }

    fn counted(
        runs: &Arc<AtomicU32>,
        result: Result<Produced<u32>, ResourceFailure>,
    ) -> impl Future<Output = Outcome<u32>> + Send + 'static {
        let runs = Arc::clone(runs);
        async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            result
        }
    }

    fn made(handle: &str, payload: u32) -> Result<Produced<u32>, ResourceFailure> {
        Ok(Produced {
            generated: Generated::new(handle),
            payload: Some(payload),
            from_store: false,
        })
    }

    #[test]
    fn resource_key_is_the_prefix_of_the_versioned_digest() {
        let spec = Sample { text: "a" };

        let key = ResourceKey::of(&spec);
        let digest = hex(&spec_digest(&spec));

        assert!(digest.starts_with(key.as_str()), "{digest} vs {key}");
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn slug_keeps_audit_subjects_to_one_word() {
        assert_eq!(
            slug("npc-id:bandit|Old Courier", 40),
            "npc-id-bandit-old-courier"
        );
        assert_eq!(slug("  ", 40), "");
        assert_eq!(slug("abcdefghij", 4), "abcd");
    }

    #[test]
    fn write_atomic_replaces_a_file_and_leaves_no_staging_file() {
        let directory = std::env::temp_dir().join(format!("df-adapters-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&directory);

        write_atomic(&directory, "clip", b"one").unwrap();
        write_atomic(&directory, "clip", b"two").unwrap();

        assert_eq!(std::fs::read(directory.join("clip")).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        let _removed = std::fs::remove_dir_all(&directory);
    }

    #[tokio::test]
    async fn identical_concurrent_asks_share_one_generation() {
        let kit = Kit::default();
        let flights = Flights::default();
        let runs = Arc::new(AtomicU32::new(0));

        let asks = (0..4).map(|_| {
            kit.produce(
                &flights,
                Sample { text: "same" },
                counted(&runs, made("h", 7)),
            )
        });
        let results = futures::future::join_all(asks).await;

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        for (outcome, _) in &results {
            assert_eq!(outcome.as_ref().unwrap().generated.handle, "h");
            assert_eq!(outcome.as_ref().unwrap().payload, Some(7));
        }
        assert_eq!(
            results
                .iter()
                .filter(|(_, role)| *role == Role::Joined)
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn a_ready_resource_is_served_without_running_the_generation_again() {
        let kit = Kit::default();
        let flights = Flights::default();
        let runs = Arc::new(AtomicU32::new(0));
        let (_first, _) = kit
            .produce(&flights, Sample { text: "r" }, counted(&runs, made("h", 1)))
            .await;

        let (again, _) = kit
            .produce(&flights, Sample { text: "r" }, counted(&runs, made("x", 2)))
            .await;

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let again = again.unwrap();
        assert_eq!(again.generated.handle, "h");
        assert_eq!(
            again.payload, None,
            "the adapter reads its store by the handle"
        );
    }

    #[tokio::test]
    async fn a_failed_resource_is_not_retried_beyond_its_attempts() {
        let kit = Kit::default();
        let flights = Flights::default();
        kit.hub().set_limits(
            ResourceKind::VoiceLine,
            Limits {
                timeout: Duration::from_secs(5),
                attempts: 2,
            },
        );
        let runs = Arc::new(AtomicU32::new(0));
        let transient = || Err(ResourceFailure::Transient("provider_failed"));

        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let (outcome, _) = kit
                .produce(
                    &flights,
                    Sample { text: "flaky" },
                    counted(&runs, transient()),
                )
                .await;
            outcomes.push(outcome);
        }

        assert_eq!(runs.load(Ordering::SeqCst), 2, "two attempts, then none");
        for outcome in outcomes {
            assert_eq!(
                outcome.unwrap_err(),
                ResourceFailure::Transient("provider_failed")
            );
        }
    }

    #[tokio::test]
    async fn a_permanent_failure_is_remembered_until_the_dev_tools_invalidate_it() {
        let kit = Kit::default();
        let flights = Flights::default();
        let runs = Arc::new(AtomicU32::new(0));
        let refused = || Err(ResourceFailure::Permanent("request_refused"));
        for _ in 0..3 {
            let (_outcome, _) = kit
                .produce(&flights, Sample { text: "no" }, counted(&runs, refused()))
                .await;
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        let key = ResourceKey::of(&Sample { text: "no" });
        assert!(kit.regenerate(&key));
        let (again, _) = kit
            .produce(
                &flights,
                Sample { text: "no" },
                counted(&runs, made("ok", 3)),
            )
            .await;

        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(again.unwrap().generated.handle, "ok");
    }

    #[tokio::test]
    async fn invalidating_a_ready_entry_erases_its_stored_output() {
        let kit = Kit::default();
        let flights = Flights::default();
        let erased = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&erased);
        kit.tune(
            ResourceKind::VoiceLine,
            Limits {
                timeout: Duration::from_secs(5),
                attempts: 2,
            },
            Arc::new(|_| true),
            Arc::new(move |generated| locked(&sink).push(generated.handle.clone())),
        );
        let runs = Arc::new(AtomicU32::new(0));
        let (_made, _) = kit
            .produce(
                &flights,
                Sample { text: "e" },
                counted(&runs, made("file-1", 1)),
            )
            .await;

        let dropped = kit.invalidate(Invalidate::Subject("sample:one"));

        assert_eq!(dropped, 1);
        assert_eq!(*locked(&erased), vec!["file-1".to_owned()]);
    }

    #[tokio::test]
    async fn a_leader_that_is_dropped_hands_its_key_to_the_next_waiter() {
        let flights: Flights<u32> = Flights::default();
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let leader = {
            let flights = flights.clone();
            tokio::spawn(async move {
                flights
                    .run_inline("k", async {
                        let _held = gate.await;
                        1
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        let waiter = {
            let flights = flights.clone();
            tokio::spawn(async move { flights.run_inline("k", async { 2 }).await })
        };
        tokio::task::yield_now().await;

        leader.abort();
        drop(release);
        let (value, role) = waiter.await.unwrap();

        assert_eq!((value, role), (2, Role::Leader));
    }

    #[test]
    fn a_call_kind_writes_the_hub_line_format() {
        let key = hex(&TEXT.digest("{\"probe\":1}"));

        TEXT.line(
            "ready",
            &key,
            "text:probe",
            Duration::from_millis(12),
            "handle-1",
            "",
        );

        let lines = crate::gameplay::actor::audit_lines_containing("subject=text:probe");
        let line = lines.last().unwrap();
        assert!(line.starts_with("RESOURCE ready kind=text key="), "{line}");
        assert!(line.contains(&format!("key={}", &key[..32])), "{line}");
        assert!(line.contains("ms=12 handle=handle-1 error= op="), "{line}");
    }
}
