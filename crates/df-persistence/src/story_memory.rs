//! The PostgreSQL side of the story memory (STORY-MEM): an append-only event table in the run's
//! own database, its full-text and pgvector indexes, the recall record and the entity dossier
//! cache. This module stores and searches; it decides nothing about meaning, ranking policy or
//! who may hear what beyond filtering rows by the audience it is handed. The engine in the
//! gameplay crate owns capture, embedding, recall, consistency and the dev surface.
//!
//! Every read takes a [`Scope`] whose audience filter is part of the SQL, applied before any
//! ranking or nearest-neighbour selection, so a private row can never be ranked into a public
//! result. The engine rechecks the audience on the rows it gets back.
//!
//! The schema is `df_story` (migrations 0003 and 0004). 0003 is core and always applies; 0004
//! needs the pgvector extension and its failure only disables the semantic half.
use std::{fmt, time::Duration};

use tokio::{sync::Mutex, task::JoinHandle, time::timeout};
use tokio_postgres::{Client, Config, NoTls, Row, types::ToSql};

/// The width of every stored embedding (OpenAI `text-embedding-3-small`).
pub const EMBEDDING_DIMENSIONS: usize = 1536;
/// The longest stored event text, in characters.
pub const MAXIMUM_BODY_CHARACTERS: usize = 2000;
/// The most rows any one read returns (the policy's per-query event bound).
pub const MAXIMUM_QUERY_EVENTS: usize = 64;
/// The most entity ids of one kind an event carries.
pub const MAXIMUM_ENTITIES_PER_KIND: usize = 16;

const OPERATION_DEADLINE: Duration = Duration::from_secs(8);
const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
const CORE_MIGRATION: &str = include_str!("../migrations/0003_story_memory.sql");
const VECTOR_MIGRATION: &str = include_str!("../migrations/0004_story_memory_vector.sql");
/// Placeholder text of an erased event: its words are gone, its place in the record is not.
pub const ERASED_TEXT: &str = "[erased]";

/// What kind of happening an event records.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EventKind {
    HeroSpeech,
    NpcSpeech,
    Narration,
    ActionRuling,
    CheckRoll,
    CombatOutcome,
    SceneChange,
    CanonFact,
    GrievanceAttitude,
    PromiseThread,
    Transaction,
}

impl EventKind {
    pub const ALL: [Self; 11] = [
        Self::HeroSpeech,
        Self::NpcSpeech,
        Self::Narration,
        Self::ActionRuling,
        Self::CheckRoll,
        Self::CombatOutcome,
        Self::SceneChange,
        Self::CanonFact,
        Self::GrievanceAttitude,
        Self::PromiseThread,
        Self::Transaction,
    ];

    /// The closed label stored in the table (and typed at the dev CLI).
    pub const fn label(self) -> &'static str {
        match self {
            Self::HeroSpeech => "hero_speech",
            Self::NpcSpeech => "npc_speech",
            Self::Narration => "narration",
            Self::ActionRuling => "action_ruling",
            Self::CheckRoll => "check_roll",
            Self::CombatOutcome => "combat_outcome",
            Self::SceneChange => "scene_change",
            Self::CanonFact => "canon_fact",
            Self::GrievanceAttitude => "grievance_attitude",
            Self::PromiseThread => "promise_thread",
            Self::Transaction => "transaction",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }
}

/// Who may be told an event.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Visibility {
    /// Said or done at the table; any line may repeat it.
    Public,
    /// Known to the heroes as a party, not shown on the public table.
    Party,
    /// A clue for one hero ([`NewEvent::private_to`]); never recalled into a public line.
    Private,
}

impl Visibility {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Party => "party",
            Self::Private => "private",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "public" => Some(Self::Public),
            "party" => Some(Self::Party),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Status {
    Active,
    Superseded,
}

impl Status {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
        }
    }
}

/// The audience a recall serves. It is part of every query.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Audience {
    /// A line the whole table sees: public events only.
    Public,
    /// The heroes together: public and party events.
    Party,
    /// One hero's private view: public, party and that hero's own private clues.
    Member(String),
}

impl Audience {
    /// Whether an event with this visibility (and private recipient) may be told to this audience.
    pub fn allows(&self, visibility: Visibility, private_to: Option<&str>) -> bool {
        match (self, visibility) {
            (_, Visibility::Public) => true,
            (Self::Public, _) => false,
            (_, Visibility::Party) => true,
            (Self::Party, Visibility::Private) => false,
            (Self::Member(hero), Visibility::Private) => private_to == Some(hero.as_str()),
        }
    }

    fn sql_parts(&self) -> (bool, Option<String>) {
        match self {
            Self::Public => (false, None),
            Self::Party => (true, None),
            Self::Member(hero) => (true, Some(hero.clone())),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Public => "public".to_owned(),
            Self::Party => "party".to_owned(),
            Self::Member(hero) => format!("member:{hero}"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "public" => Some(Self::Public),
            "party" => Some(Self::Party),
            other => other
                .strip_prefix("member:")
                .filter(|hero| !hero.is_empty())
                .map(|hero| Self::Member(hero.to_owned())),
        }
    }

    /// The audience a cached dossier is kept for: private clues never enter a shared summary.
    pub fn dossier_tier(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Party | Self::Member(_) => "party",
        }
    }
}

/// The entity ids an event involves, by kind. The ids are the game's own stable ids (the NPC id,
/// the hero's name, the place key, the item id); there is no frozen EntityId binding yet.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Entities {
    pub npc: Vec<String>,
    pub hero: Vec<String>,
    pub place: Vec<String>,
    pub item: Vec<String>,
}

impl Entities {
    /// Every id, kinds together.
    pub fn all(&self) -> Vec<&str> {
        self.npc
            .iter()
            .chain(&self.hero)
            .chain(&self.place)
            .chain(&self.item)
            .map(String::as_str)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.npc.is_empty() && self.hero.is_empty() && self.place.is_empty() && self.item.is_empty()
    }

    /// Trimmed, lowercase, de-duplicated, bounded ids.
    #[must_use]
    pub fn normalised(self) -> Self {
        fn clean(ids: Vec<String>) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for id in ids {
                let id: String = id.trim().to_lowercase().chars().take(128).collect();
                if !id.is_empty() && !out.contains(&id) && out.len() < MAXIMUM_ENTITIES_PER_KIND {
                    out.push(id);
                }
            }
            out
        }
        Self {
            npc: clean(self.npc),
            hero: clean(self.hero),
            place: clean(self.place),
            item: clean(self.item),
        }
    }
}

/// One event to append.
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub run_id: String,
    /// The source event's identity. A second write with the same key (a replay, a restart) is
    /// recognised and adds nothing.
    pub natural_key: String,
    /// Which capture path made it (`table_log`, `canon`, `social`, `journal`, ...).
    pub source: String,
    /// The source's own lifetime marker and position, when it has one (a table log sequence).
    pub source_epoch: Option<String>,
    pub source_seq: Option<i64>,
    /// The committed game revision the event belongs to, when the source knows it.
    pub game_revision: Option<i64>,
    pub kind: EventKind,
    pub visibility: Visibility,
    /// Required exactly when the visibility is private.
    pub private_to: Option<String>,
    pub text: String,
    pub scene: Option<String>,
    pub entities: Entities,
    /// A canonical source can write this event again.
    pub rebuildable: bool,
    /// The older event this one replaces (belief-level events only; the engine decides).
    pub supersedes: Option<i64>,
}

/// What an insert found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Inserted {
    pub event_id: i64,
    /// False when the natural key was already stored.
    pub created: bool,
}

/// One stored event as read back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEvent {
    pub event_id: i64,
    pub run_id: String,
    pub natural_key: String,
    pub source: String,
    pub game_revision: Option<i64>,
    pub created_ms: i64,
    pub kind: EventKind,
    pub visibility: Visibility,
    pub private_to: Option<String>,
    pub text: String,
    pub scene: Option<String>,
    pub entities: Entities,
    pub rebuildable: bool,
    pub status: Status,
    pub supersedes_id: Option<i64>,
    pub superseded_by: Option<i64>,
    pub retired_reason: Option<String>,
    pub embedding_model: Option<String>,
    pub embedded: bool,
}

impl StoredEvent {
    pub fn erased(&self) -> bool {
        self.retired_reason.as_deref() == Some("erased")
    }
}

/// What a read may see.
#[derive(Clone, Debug)]
pub struct Scope {
    pub run_id: String,
    pub audience: Audience,
    pub include_superseded: bool,
    /// Only these kinds; empty means every kind.
    pub kinds: Vec<EventKind>,
}

impl Scope {
    pub fn active(run_id: &str, audience: Audience) -> Self {
        Self {
            run_id: run_id.to_owned(),
            audience,
            include_superseded: false,
            kinds: Vec::new(),
        }
    }
}

/// Narrows a selection to what bears on the named things. An empty relevance selects everything.
#[derive(Clone, Debug, Default)]
pub struct Relevance {
    pub entities: Vec<String>,
    pub scene: Option<String>,
    /// Also select events that name no entity at all (a party-wide promise).
    pub unattached: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scored {
    pub event_id: i64,
    pub score: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEmbedding {
    pub event_id: i64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    /// `(kind label, active, superseded)`.
    pub by_kind: Vec<(String, i64, i64)>,
    pub total: i64,
    pub rebuildable: i64,
    pub embedded: i64,
    /// Events with no vector for the current model.
    pub backlog: i64,
    /// `(model, events)`.
    pub models: Vec<(String, i64)>,
    pub recalls: i64,
    pub dossiers: i64,
}

/// One recall, as recorded for replay.
#[derive(Clone, Debug, PartialEq)]
pub struct RecallRecord {
    pub run_id: String,
    pub purpose: String,
    pub scene: Option<String>,
    pub speaker: Option<String>,
    pub addressee: Option<String>,
    pub audience: String,
    pub query_text: String,
    pub mode: String,
    pub embedding_model: Option<String>,
    pub query_vector_digest: Option<String>,
    pub pinned_ids: Vec<i64>,
    pub related_ids: Vec<i64>,
    pub scores: Vec<f64>,
    pub pack_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DossierRow {
    pub entity_id: String,
    pub audience: String,
    pub event_set_hash: String,
    pub covered_ids: Vec<i64>,
    pub summary: String,
    pub writer: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Removed {
    pub events: u64,
    pub recalls: u64,
    pub dossiers: u64,
}

#[derive(Debug)]
pub enum StoryMemoryError {
    Connect(String),
    Timeout,
    Invalid(&'static str),
    Database(String),
}

impl fmt::Display for StoryMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(reason) => {
                write!(formatter, "story memory database unavailable: {reason}")
            }
            Self::Timeout => formatter.write_str("story memory database deadline expired"),
            Self::Invalid(reason) => write!(formatter, "story memory input refused: {reason}"),
            Self::Database(reason) => write!(formatter, "story memory database error: {reason}"),
        }
    }
}

impl std::error::Error for StoryMemoryError {}

fn database(error: tokio_postgres::Error) -> StoryMemoryError {
    StoryMemoryError::Database(
        error
            .as_db_error()
            .map_or_else(|| error.to_string(), |db| db.message().to_owned()),
    )
}

/// Whether the semantic half of the schema exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VectorState {
    Ready,
    /// pgvector is not usable here; the reason is the database's own message.
    Unavailable(String),
}

/// Applies migrations 0003 and 0004 (idempotent, serialised by an advisory lock). 0003 failing
/// is an error; 0004 failing is reported as [`VectorState::Unavailable`].
pub async fn ensure_story_memory(client: &mut Client) -> Result<VectorState, StoryMemoryError> {
    {
        let transaction = client.transaction().await.map_err(database)?;
        transaction
            .batch_execute("SELECT pg_advisory_xact_lock(7283410001)")
            .await
            .map_err(database)?;
        transaction
            .batch_execute(CORE_MIGRATION)
            .await
            .map_err(database)?;
        transaction.commit().await.map_err(database)?;
    }
    let attempt = async {
        let transaction = client.transaction().await?;
        transaction
            .batch_execute("SELECT pg_advisory_xact_lock(7283410001)")
            .await?;
        transaction.batch_execute(VECTOR_MIGRATION).await?;
        transaction.commit().await
    }
    .await;
    Ok(match attempt {
        Ok(()) => VectorState::Ready,
        Err(error) => VectorState::Unavailable(
            error
                .as_db_error()
                .map_or_else(|| error.to_string(), |db| db.message().to_owned()),
        ),
    })
}

/// pgvector's text form of a vector.
pub fn vector_literal(vector: &[f32]) -> Result<String, StoryMemoryError> {
    if vector.len() != EMBEDDING_DIMENSIONS || vector.iter().any(|value| !value.is_finite()) {
        return Err(StoryMemoryError::Invalid("embedding width or value"));
    }
    let mut text = String::with_capacity(vector.len() * 10 + 2);
    text.push('[');
    for (index, value) in vector.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&value.to_string());
    }
    text.push(']');
    Ok(text)
}

struct Slot {
    client: Option<Client>,
    driver: Option<JoinHandle<()>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.abort();
        }
    }
}

/// The story memory's connection to the run's database. One connection, used one operation at a
/// time; it reconnects when the server dropped it.
pub struct StoryMemoryDb {
    config: Config,
    slot: Mutex<Slot>,
    vector: bool,
    vector_reason: Option<String>,
}

#[derive(Default)]
struct Params {
    values: Vec<Box<dyn ToSql + Sync + Send>>,
}

impl Params {
    fn add<T: ToSql + Sync + Send + 'static>(&mut self, value: T) -> String {
        self.values.push(Box::new(value));
        format!("${}", self.values.len())
    }

    fn refs(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values
            .iter()
            .map(|value| &**value as &(dyn ToSql + Sync))
            .collect()
    }
}

fn event_columns(vector: bool) -> String {
    let embedding = if vector {
        "embedding_model, (embedding IS NOT NULL) AS embedded"
    } else {
        "NULL::text AS embedding_model, false AS embedded"
    };
    format!(
        "event_id, run_id, natural_key, source, game_revision, \
         (extract(epoch FROM created_at) * 1000)::bigint AS created_ms, kind, visibility, \
         private_to, body, scene, npc_ids, hero_ids, place_ids, item_ids, rebuildable, status, \
         supersedes_id, superseded_by, retired_reason, {embedding}"
    )
}

fn event_from_row(row: &Row) -> Result<StoredEvent, StoryMemoryError> {
    let invalid_stored_row = |_| StoryMemoryError::Invalid("stored event shape");
    let kind: String = row.try_get("kind").map_err(invalid_stored_row)?;
    let visibility: String = row.try_get("visibility").map_err(invalid_stored_row)?;
    let status: String = row.try_get("status").map_err(invalid_stored_row)?;
    Ok(StoredEvent {
        event_id: row.try_get("event_id").map_err(invalid_stored_row)?,
        run_id: row.try_get("run_id").map_err(invalid_stored_row)?,
        natural_key: row.try_get("natural_key").map_err(invalid_stored_row)?,
        source: row.try_get("source").map_err(invalid_stored_row)?,
        game_revision: row.try_get("game_revision").map_err(invalid_stored_row)?,
        created_ms: row.try_get("created_ms").map_err(invalid_stored_row)?,
        kind: EventKind::parse(&kind).ok_or(StoryMemoryError::Invalid("stored kind"))?,
        visibility: Visibility::parse(&visibility)
            .ok_or(StoryMemoryError::Invalid("stored visibility"))?,
        private_to: row.try_get("private_to").map_err(invalid_stored_row)?,
        text: row.try_get("body").map_err(invalid_stored_row)?,
        scene: row.try_get("scene").map_err(invalid_stored_row)?,
        entities: Entities {
            npc: row.try_get("npc_ids").map_err(invalid_stored_row)?,
            hero: row.try_get("hero_ids").map_err(invalid_stored_row)?,
            place: row.try_get("place_ids").map_err(invalid_stored_row)?,
            item: row.try_get("item_ids").map_err(invalid_stored_row)?,
        },
        rebuildable: row.try_get("rebuildable").map_err(invalid_stored_row)?,
        status: if status == "active" {
            Status::Active
        } else {
            Status::Superseded
        },
        supersedes_id: row.try_get("supersedes_id").map_err(invalid_stored_row)?,
        superseded_by: row.try_get("superseded_by").map_err(invalid_stored_row)?,
        retired_reason: row.try_get("retired_reason").map_err(invalid_stored_row)?,
        embedding_model: row.try_get("embedding_model").map_err(invalid_stored_row)?,
        embedded: row.try_get("embedded").map_err(invalid_stored_row)?,
    })
}

/// The WHERE clause every audience-bound read shares: the run, the audience, the status, the kinds.
fn scope_clause(scope: &Scope, params: &mut Params) -> String {
    let (party, member) = scope.audience.sql_parts();
    let run_id_parameter = params.add(scope.run_id.clone());
    let party_visible_parameter = params.add(party);
    let member_id_parameter = params.add(member);
    let include_superseded_parameter = params.add(scope.include_superseded);
    let mut clause = format!(
        "run_id = {run_id_parameter} AND (visibility = 'public' OR (visibility = 'party' AND {party_visible_parameter}::boolean) \
         OR (visibility = 'private' AND private_to = {member_id_parameter}::text)) \
         AND ({include_superseded_parameter}::boolean OR status = 'active')"
    );
    if !scope.kinds.is_empty() {
        let kinds: Vec<String> = scope
            .kinds
            .iter()
            .map(|kind| kind.label().to_owned())
            .collect();
        clause.push_str(&format!(" AND kind = ANY({}::text[])", params.add(kinds)));
    }
    clause
}

fn relevance_clause(relevance: &Relevance, params: &mut Params) -> Option<String> {
    let mut parts = Vec::new();
    if !relevance.entities.is_empty() {
        parts.push(format!(
            "entity_ids && {}::text[]",
            params.add(relevance.entities.clone())
        ));
    }
    if let Some(scene) = &relevance.scene {
        parts.push(format!("scene = {}::text", params.add(scene.clone())));
    }
    if relevance.unattached {
        parts.push("cardinality(entity_ids) = 0".to_owned());
    }
    (!parts.is_empty()).then(|| format!("({})", parts.join(" OR ")))
}

fn limit_of(limit: usize) -> i64 {
    i64::try_from(limit.clamp(1, MAXIMUM_QUERY_EVENTS)).unwrap_or(64)
}

/// The words of a query as an OR search (`websearch_to_tsquery` syntax). Only letters and digits
/// survive, so a quote, a dash or an operator in player text cannot change the query's shape.
fn or_query(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.chars().count() >= 2 && !words.contains(&word) && words.len() < 48 {
            words.push(word);
        }
    }
    words.join(" or ")
}

impl StoryMemoryDb {
    /// Connects, applies the migrations and reports whether the semantic half is available.
    pub async fn connect(config: &Config) -> Result<Self, StoryMemoryError> {
        let mut slot = Self::open(config).await?;
        let state = ensure_story_memory(
            slot.client
                .as_mut()
                .ok_or(StoryMemoryError::Connect("no client".to_owned()))?,
        )
        .await?;
        let (vector, vector_reason) = match state {
            VectorState::Ready => (true, None),
            VectorState::Unavailable(reason) => (false, Some(reason)),
        };
        Ok(Self {
            config: config.clone(),
            slot: Mutex::new(slot),
            vector,
            vector_reason,
        })
    }

    async fn open(config: &Config) -> Result<Slot, StoryMemoryError> {
        let (client, connection) = timeout(CONNECT_DEADLINE, config.connect(NoTls))
            .await
            .map_err(|_| StoryMemoryError::Timeout)?
            .map_err(|error| StoryMemoryError::Connect(error.to_string()))?;
        let driver = tokio::spawn(async move {
            let _ended = connection.await;
        });
        client
            .batch_execute("SET statement_timeout = '6s'")
            .await
            .map_err(database)?;
        Ok(Slot {
            client: Some(client),
            driver: Some(driver),
        })
    }

    pub fn vector_enabled(&self) -> bool {
        self.vector
    }

    /// Why the semantic half is off, when it is.
    pub fn vector_reason(&self) -> Option<&str> {
        self.vector_reason.as_deref()
    }

    async fn bounded<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, StoryMemoryError>>,
    ) -> Result<T, StoryMemoryError> {
        timeout(OPERATION_DEADLINE, work)
            .await
            .unwrap_or(Err(StoryMemoryError::Timeout))
    }

    async fn lock(&self) -> Result<tokio::sync::MutexGuard<'_, Slot>, StoryMemoryError> {
        let mut slot = self.slot.lock().await;
        if slot.client.as_ref().is_none_or(Client::is_closed) {
            *slot = Self::open(&self.config).await?;
        }
        Ok(slot)
    }

    /// Appends events in one transaction. A natural key already stored adds nothing and reports
    /// the stored event's id.
    pub async fn insert_events(
        &self,
        events: &[NewEvent],
    ) -> Result<Vec<Inserted>, StoryMemoryError> {
        for event in events {
            let is_private = event.visibility == Visibility::Private;
            if event.text.trim().is_empty()
                || event.text.chars().count() > MAXIMUM_BODY_CHARACTERS
                || event.run_id.is_empty()
                || event.natural_key.is_empty()
                || is_private != event.private_to.is_some()
            {
                return Err(StoryMemoryError::Invalid("event fields"));
            }
        }
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            let insert = transaction
                .prepare(
                    "INSERT INTO df_story.story_memory \
                     (run_id, natural_key, source, source_epoch, source_seq, game_revision, kind, \
                      visibility, private_to, body, body_digest, scene, npc_ids, hero_ids, \
                      place_ids, item_ids, rebuildable, supersedes_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                             sha256(convert_to($10, 'UTF8')), $11, $12, $13, $14, $15, $16, $17) \
                     ON CONFLICT (run_id, natural_key) DO NOTHING RETURNING event_id",
                )
                .await
                .map_err(database)?;
            let existing = transaction
                .prepare(
                    "SELECT event_id FROM df_story.story_memory \
                     WHERE run_id = $1 AND natural_key = $2",
                )
                .await
                .map_err(database)?;
            let mut out = Vec::with_capacity(events.len());
            for event in events {
                let entities = event.entities.clone().normalised();
                let row = transaction
                    .query_opt(
                        &insert,
                        &[
                            &event.run_id,
                            &event.natural_key,
                            &event.source,
                            &event.source_epoch,
                            &event.source_seq,
                            &event.game_revision,
                            &event.kind.label(),
                            &event.visibility.label(),
                            &event.private_to,
                            &event.text,
                            &event.scene,
                            &entities.npc,
                            &entities.hero,
                            &entities.place,
                            &entities.item,
                            &event.rebuildable,
                            &event.supersedes,
                        ],
                    )
                    .await
                    .map_err(database)?;
                match row {
                    Some(row) => {
                        let event_id: i64 = row.try_get(0).map_err(database)?;
                        if let Some(old) = event.supersedes {
                            transaction
                                .execute(
                                    "UPDATE df_story.story_memory SET status = 'superseded', \
                                     superseded_by = $3 WHERE run_id = $1 AND event_id = $2 \
                                     AND status = 'active'",
                                    &[&event.run_id, &old, &event_id],
                                )
                                .await
                                .map_err(database)?;
                        }
                        out.push(Inserted {
                            event_id,
                            created: true,
                        });
                    }
                    None => {
                        let row = transaction
                            .query_one(&existing, &[&event.run_id, &event.natural_key])
                            .await
                            .map_err(database)?;
                        out.push(Inserted {
                            event_id: row.try_get(0).map_err(database)?,
                            created: false,
                        });
                    }
                }
            }
            transaction.commit().await.map_err(database)?;
            Ok(out)
        })
        .await
    }

    /// The oldest events with no vector for `model` (none yet, or one from another model).
    pub async fn pending_embeddings(
        &self,
        run_id: &str,
        model: &str,
        limit: usize,
    ) -> Result<Vec<PendingEmbedding>, StoryMemoryError> {
        if !self.vector {
            return Ok(Vec::new());
        }
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let rows = client
                .query(
                    "SELECT event_id, body FROM df_story.story_memory \
                     WHERE run_id = $1 AND retired_reason IS DISTINCT FROM 'erased' \
                     AND (embedding IS NULL OR embedding_model IS DISTINCT FROM $2) \
                     ORDER BY event_id LIMIT $3",
                    &[
                        &run_id,
                        &model,
                        &i64::try_from(limit.clamp(1, 256)).unwrap_or(64),
                    ],
                )
                .await
                .map_err(database)?;
            rows.iter()
                .map(|row| {
                    Ok(PendingEmbedding {
                        event_id: row.try_get(0).map_err(database)?,
                        text: row.try_get(1).map_err(database)?,
                    })
                })
                .collect()
        })
        .await
    }

    /// Stores vectors made by `model`. Returns how many rows took one (an erased row does not).
    pub async fn store_embeddings(
        &self,
        run_id: &str,
        model: &str,
        items: &[(i64, Vec<f32>)],
    ) -> Result<u64, StoryMemoryError> {
        if !self.vector {
            return Err(StoryMemoryError::Invalid("vector extension unavailable"));
        }
        let mut literals = Vec::with_capacity(items.len());
        for (id, vector) in items {
            literals.push((*id, vector_literal(vector)?));
        }
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            let update = transaction
                .prepare(
                    "UPDATE df_story.story_memory SET embedding = $3::text::vector, \
                     embedding_model = $4, embedded_at = clock_timestamp() \
                     WHERE run_id = $1 AND event_id = $2 AND retired_reason IS DISTINCT FROM 'erased'",
                )
                .await
                .map_err(database)?;
            let mut stored_rows = 0;
            for (id, literal) in &literals {
                stored_rows += transaction
                    .execute(&update, &[&run_id, id, literal, &model])
                    .await
                    .map_err(database)?;
            }
            transaction.commit().await.map_err(database)?;
            Ok(stored_rows)
        })
        .await
    }

    /// The events nearest `query` by cosine similarity among those embedded by `model`, inside
    /// the scope. The audience filter is part of the SQL; `hnsw.iterative_scan` keeps a filtered
    /// search from running dry when the index is on.
    pub async fn vector_search(
        &self,
        scope: &Scope,
        model: &str,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<Scored>, StoryMemoryError> {
        if !self.vector {
            return Ok(Vec::new());
        }
        let literal = vector_literal(query)?;
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            transaction
                .batch_execute("SET LOCAL hnsw.ef_search = 100")
                .await
                .map_err(database)?;
            // Newer pgvector only; an older one refuses the setting and the search goes on without.
            let iterative = async {
                transaction.batch_execute("SAVEPOINT iterative").await?;
                if transaction
                    .batch_execute("SET LOCAL hnsw.iterative_scan = 'relaxed_order'")
                    .await
                    .is_err()
                {
                    transaction
                        .batch_execute("ROLLBACK TO SAVEPOINT iterative")
                        .await?;
                }
                Ok::<(), tokio_postgres::Error>(())
            }
            .await;
            iterative.map_err(database)?;
            let mut params = Params::default();
            let clause = scope_clause(scope, &mut params);
            let vector = params.add(literal);
            let model = params.add(model.to_owned());
            let sql = format!(
                "SELECT event_id, score FROM (\
                    SELECT event_id, (1 - (embedding <=> {vector}::text::vector))::float8 AS score \
                    FROM df_story.story_memory \
                    WHERE {clause} AND embedding IS NOT NULL AND embedding_model = {model}::text \
                    ORDER BY embedding <=> {vector}::text::vector LIMIT {limit}\
                 ) nearest ORDER BY score DESC, event_id DESC",
                limit = limit_of(limit)
            );
            let rows = transaction
                .query(&sql, &params.refs())
                .await
                .map_err(database)?;
            transaction.commit().await.map_err(database)?;
            scored(&rows)
        })
        .await
    }

    /// Candidate generation by full text: the events sharing words with `query`, best first. It
    /// proposes candidates for the engine to fuse with the vector ranking; it never decides what
    /// a candidate means.
    pub async fn text_search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Scored>, StoryMemoryError> {
        let words = or_query(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut params = Params::default();
            let clause = scope_clause(scope, &mut params);
            let words = params.add(words);
            let sql = format!(
                "SELECT event_id, ts_rank_cd(body_tsv, q.query)::float8 AS score \
                 FROM df_story.story_memory, (SELECT websearch_to_tsquery('english', {words}::text) AS query) q \
                 WHERE {clause} AND body_tsv @@ q.query \
                 ORDER BY score DESC, event_id DESC LIMIT {limit}",
                limit = limit_of(limit)
            );
            let rows = client
                .query(&sql, &params.refs())
                .await
                .map_err(database)?;
            scored(&rows)
        })
        .await
    }

    /// Events inside the scope that bear on `relevance`, newest first.
    pub async fn select_events(
        &self,
        scope: &Scope,
        relevance: &Relevance,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut params = Params::default();
            let mut clause = scope_clause(scope, &mut params);
            if let Some(extra) = relevance_clause(relevance, &mut params) {
                clause.push_str(&format!(" AND {extra}"));
            }
            let sql = format!(
                "SELECT {} FROM df_story.story_memory WHERE {clause} \
                 ORDER BY event_id DESC LIMIT {}",
                event_columns(self.vector),
                limit_of(limit)
            );
            let rows = client.query(&sql, &params.refs()).await.map_err(database)?;
            rows.iter().map(event_from_row).collect()
        })
        .await
    }

    /// The listed events that the scope may see, in the order asked.
    pub async fn events_by_ids(
        &self,
        scope: &Scope,
        ids: &[i64],
    ) -> Result<Vec<StoredEvent>, StoryMemoryError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = ids.iter().copied().take(MAXIMUM_QUERY_EVENTS).collect();
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut params = Params::default();
            let clause = scope_clause(scope, &mut params);
            let wanted = params.add(ids.clone());
            let sql = format!(
                "SELECT {} FROM df_story.story_memory WHERE {clause} AND event_id = ANY({wanted}::bigint[])",
                event_columns(self.vector)
            );
            let rows = client
                .query(&sql, &params.refs())
                .await
                .map_err(database)?;
            let mut found: Vec<StoredEvent> =
                rows.iter().map(event_from_row).collect::<Result<_, _>>()?;
            found.sort_by_key(|event| ids.iter().position(|id| *id == event.event_id));
            Ok(found)
        })
        .await
    }

    /// One event of the run whatever its audience: the operator's view (the dev surface only).
    pub async fn operator_event(
        &self,
        run_id: &str,
        event_id: i64,
    ) -> Result<Option<StoredEvent>, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let sql = format!(
                "SELECT {} FROM df_story.story_memory WHERE run_id = $1 AND event_id = $2",
                event_columns(self.vector)
            );
            let row = client
                .query_opt(&sql, &[&run_id, &event_id])
                .await
                .map_err(database)?;
            row.as_ref().map(event_from_row).transpose()
        })
        .await
    }

    /// The id and status of every event that names `entity` inside the scope, oldest first (the
    /// input of an event-set hash). Bounded; a set larger than the bound reports `None`.
    pub async fn entity_event_ids(
        &self,
        scope: &Scope,
        entity: &str,
    ) -> Result<Option<Vec<(i64, Status)>>, StoryMemoryError> {
        const BOUND: i64 = 5000;
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut params = Params::default();
            let clause = scope_clause(scope, &mut params);
            let entity = params.add(entity.to_owned());
            let sql = format!(
                "SELECT event_id, status FROM df_story.story_memory \
                 WHERE {clause} AND {entity}::text = ANY(entity_ids) ORDER BY event_id LIMIT {}",
                BOUND + 1
            );
            let rows = client.query(&sql, &params.refs()).await.map_err(database)?;
            if rows.len() as i64 > BOUND {
                return Ok(None);
            }
            rows.iter()
                .map(|row| {
                    let status: String = row.try_get(1).map_err(database)?;
                    Ok((
                        row.try_get(0).map_err(database)?,
                        if status == "active" {
                            Status::Active
                        } else {
                            Status::Superseded
                        },
                    ))
                })
                .collect::<Result<Vec<_>, StoryMemoryError>>()
                .map(Some)
        })
        .await
    }

    /// A page of the events that name `entity`, oldest first, after `after_id`.
    pub async fn entity_events(
        &self,
        scope: &Scope,
        entity: &str,
        after_id: i64,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut params = Params::default();
            let clause = scope_clause(scope, &mut params);
            let entity = params.add(entity.to_owned());
            let after = params.add(after_id);
            let sql = format!(
                "SELECT {} FROM df_story.story_memory \
                 WHERE {clause} AND {entity}::text = ANY(entity_ids) AND event_id > {after}::bigint \
                 ORDER BY event_id LIMIT {}",
                event_columns(self.vector),
                limit_of(limit)
            );
            let rows = client
                .query(&sql, &params.refs())
                .await
                .map_err(database)?;
            rows.iter().map(event_from_row).collect()
        })
        .await
    }

    /// Marks `old` superseded by `new` and links them. Returns whether the old event changed.
    pub async fn supersede(
        &self,
        run_id: &str,
        old: i64,
        new: i64,
    ) -> Result<bool, StoryMemoryError> {
        if old == new {
            return Err(StoryMemoryError::Invalid(
                "an event cannot supersede itself",
            ));
        }
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            let both: i64 = transaction
                .query_one(
                    "SELECT count(*) FROM df_story.story_memory \
                     WHERE run_id = $1 AND event_id IN ($2, $3)",
                    &[&run_id, &old, &new],
                )
                .await
                .map_err(database)?
                .try_get(0)
                .map_err(database)?;
            if both != 2 {
                return Err(StoryMemoryError::Invalid("unknown event"));
            }
            let changed = transaction
                .execute(
                    "UPDATE df_story.story_memory SET status = 'superseded', superseded_by = $3 \
                     WHERE run_id = $1 AND event_id = $2 AND status = 'active'",
                    &[&run_id, &old, &new],
                )
                .await
                .map_err(database)?;
            transaction
                .execute(
                    "UPDATE df_story.story_memory SET supersedes_id = $2 \
                     WHERE run_id = $1 AND event_id = $3 AND supersedes_id IS NULL",
                    &[&run_id, &old, &new],
                )
                .await
                .map_err(database)?;
            transaction.commit().await.map_err(database)?;
            Ok(changed == 1)
        })
        .await
    }

    /// Retires the events a source recorded in one lifetime from sequence `from_seq` on (a
    /// development rewind abandons that future). They stay stored, no longer active.
    pub async fn retire_source_since(
        &self,
        run_id: &str,
        source: &str,
        epoch: &str,
        from_seq: i64,
        reason: &str,
    ) -> Result<u64, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            client
                .execute(
                    "UPDATE df_story.story_memory SET status = 'superseded', retired_reason = $5 \
                     WHERE run_id = $1 AND source = $2 AND source_epoch = $3 AND source_seq >= $4 \
                     AND status = 'active'",
                    &[&run_id, &source, &epoch, &from_seq, &reason],
                )
                .await
                .map_err(database)
        })
        .await
    }

    /// Retires the events committed after `revision` (a development rewind).
    pub async fn retire_revisions_after(
        &self,
        run_id: &str,
        revision: i64,
        reason: &str,
    ) -> Result<u64, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot
                .client
                .as_ref()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            client
                .execute(
                    "UPDATE df_story.story_memory SET status = 'superseded', retired_reason = $3 \
                     WHERE run_id = $1 AND game_revision > $2 AND status = 'active'",
                    &[&run_id, &revision, &reason],
                )
                .await
                .map_err(database)
        })
        .await
    }

    /// The counts the status view and the start-up line report.
    pub async fn counts(&self, run_id: &str, model: &str) -> Result<Counts, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let mut counts = Counts::default();
            let rows = client
                .query(
                    "SELECT kind, count(*) FILTER (WHERE status = 'active'), \
                            count(*) FILTER (WHERE status = 'superseded') \
                     FROM df_story.story_memory WHERE run_id = $1 GROUP BY kind ORDER BY kind",
                    &[&run_id],
                )
                .await
                .map_err(database)?;
            for row in &rows {
                let (active, superseded): (i64, i64) =
                    (row.try_get(1).map_err(database)?, row.try_get(2).map_err(database)?);
                counts.total += active + superseded;
                counts
                    .by_kind
                    .push((row.try_get(0).map_err(database)?, active, superseded));
            }
            counts.rebuildable = client
                .query_one(
                    "SELECT count(*) FROM df_story.story_memory WHERE run_id = $1 AND rebuildable",
                    &[&run_id],
                )
                .await
                .map_err(database)?
                .try_get(0)
                .map_err(database)?;
            if self.vector {
                let row = client
                    .query_one(
                        "SELECT count(*) FILTER (WHERE embedding IS NOT NULL), \
                                count(*) FILTER (WHERE retired_reason IS DISTINCT FROM 'erased' \
                                    AND (embedding IS NULL OR embedding_model IS DISTINCT FROM $2)) \
                         FROM df_story.story_memory WHERE run_id = $1",
                        &[&run_id, &model],
                    )
                    .await
                    .map_err(database)?;
                counts.embedded = row.try_get(0).map_err(database)?;
                counts.backlog = row.try_get(1).map_err(database)?;
                let rows = client
                    .query(
                        "SELECT embedding_model, count(*) FROM df_story.story_memory \
                         WHERE run_id = $1 AND embedding IS NOT NULL GROUP BY 1 ORDER BY 1",
                        &[&run_id],
                    )
                    .await
                    .map_err(database)?;
                for row in &rows {
                    counts
                        .models
                        .push((row.try_get(0).map_err(database)?, row.try_get(1).map_err(database)?));
                }
            }
            counts.recalls = client
                .query_one("SELECT count(*) FROM df_story.memory_recalls WHERE run_id = $1", &[&run_id])
                .await
                .map_err(database)?
                .try_get(0)
                .map_err(database)?;
            counts.dossiers = client
                .query_one("SELECT count(*) FROM df_story.entity_dossiers WHERE run_id = $1", &[&run_id])
                .await
                .map_err(database)?
                .try_get(0)
                .map_err(database)?;
            Ok(counts)
        })
        .await
    }

    /// Drops the run's index: the events a canonical source can write again, or all of them, and
    /// with them every dossier built from them (and, for all of them, the recall records). The
    /// only deletion the table allows, done inside one maintenance transaction.
    pub async fn delete_run(
        &self,
        run_id: &str,
        rebuildable_only: bool,
    ) -> Result<Removed, StoryMemoryError> {
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            transaction
                .batch_execute("SELECT set_config('df_story.maintenance', 'on', true)")
                .await
                .map_err(database)?;
            let dossiers = transaction
                .execute("DELETE FROM df_story.entity_dossiers WHERE run_id = $1", &[&run_id])
                .await
                .map_err(database)?;
            let recalls = if !rebuildable_only {
                transaction
                    .execute("DELETE FROM df_story.memory_recalls WHERE run_id = $1", &[&run_id])
                    .await
                    .map_err(database)?
            } else {
                0
            };
            let events = transaction
                .execute(
                    "DELETE FROM df_story.story_memory WHERE run_id = $1 AND (NOT $2 OR rebuildable)",
                    &[&run_id, &rebuildable_only],
                )
                .await
                .map_err(database)?;
            transaction.commit().await.map_err(database)?;
            Ok(Removed {
                events,
                recalls,
                dossiers,
            })
        })
        .await
    }

    /// Erases one hero's own words: their speech, their private clues and the grievances recorded
    /// from what they said. The rows stay as redacted places in the record; the text, the vector
    /// and the full-text entry go, and so do every dossier and recall record derived from them.
    pub async fn erase_hero(&self, run_id: &str, hero: &str) -> Result<Removed, StoryMemoryError> {
        self.bounded(async {
            let mut slot = self.lock().await?;
            let client = slot
                .client
                .as_mut()
                .ok_or(StoryMemoryError::Invalid("no client"))?;
            let transaction = client.transaction().await.map_err(database)?;
            transaction
                .batch_execute("SELECT set_config('df_story.maintenance', 'on', true)")
                .await
                .map_err(database)?;
            let clear_vector = if self.vector {
                ", embedding = NULL, embedding_model = NULL, embedded_at = NULL"
            } else {
                ""
            };
            let sql = format!(
                "UPDATE df_story.story_memory SET body = '{ERASED_TEXT}', \
                 body_digest = sha256(convert_to('{ERASED_TEXT}', 'UTF8')), status = 'superseded', \
                 retired_reason = 'erased'{clear_vector} \
                 WHERE run_id = $1 AND retired_reason IS DISTINCT FROM 'erased' \
                 AND ((kind IN ('hero_speech', 'grievance_attitude') AND $2::text = ANY(hero_ids)) \
                      OR private_to = $2::text) RETURNING event_id"
            );
            let rows = transaction
                .query(&sql, &[&run_id, &hero])
                .await
                .map_err(database)?;
            let ids: Vec<i64> = rows
                .iter()
                .map(|row| row.try_get(0).map_err(database))
                .collect::<Result<_, _>>()?;
            let mut removed = Removed {
                events: ids.len() as u64,
                ..Removed::default()
            };
            removed.dossiers = transaction
                .execute(
                    "DELETE FROM df_story.entity_dossiers WHERE run_id = $1 AND covered_ids && $2::bigint[]",
                    &[&run_id, &ids],
                )
                .await
                .map_err(database)?;
            removed.recalls = transaction
                .execute(
                    "DELETE FROM df_story.memory_recalls WHERE run_id = $1 AND \
                     (pinned_ids && $2::bigint[] OR related_ids && $2::bigint[] \
                      OR speaker = $3::text OR addressee = $3::text OR audience = 'member:' || $3::text)",
                    &[&run_id, &ids, &hero],
                )
                .await
                .map_err(database)?;
            transaction.commit().await.map_err(database)?;
            Ok(removed)
        })
        .await
    }

    /// Records one recall; returns its id.
    pub async fn log_recall(&self, record: &RecallRecord) -> Result<i64, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let query: String = record.query_text.chars().take(600).collect();
            let row = client
                .query_one(
                    "INSERT INTO df_story.memory_recalls \
                     (run_id, purpose, scene, speaker, addressee, audience, query_text, mode, \
                      embedding_model, query_vector_digest, pinned_ids, related_ids, scores, pack_text) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
                     RETURNING recall_id",
                    &[
                        &record.run_id,
                        &record.purpose,
                        &record.scene,
                        &record.speaker,
                        &record.addressee,
                        &record.audience,
                        &query,
                        &record.mode,
                        &record.embedding_model,
                        &record.query_vector_digest,
                        &record.pinned_ids,
                        &record.related_ids,
                        &record.scores,
                        &record.pack_text,
                    ],
                )
                .await
                .map_err(database)?;
            row.try_get(0).map_err(database)
        })
        .await
    }

    /// A recorded recall, for a replay that makes no provider call.
    pub async fn recall(
        &self,
        run_id: &str,
        recall_id: i64,
    ) -> Result<Option<RecallRecord>, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let row = client
                .query_opt(
                    "SELECT purpose, scene, speaker, addressee, audience, query_text, mode, \
                            embedding_model, query_vector_digest, pinned_ids, related_ids, scores, pack_text \
                     FROM df_story.memory_recalls WHERE run_id = $1 AND recall_id = $2",
                    &[&run_id, &recall_id],
                )
                .await
                .map_err(database)?;
            row.map(|row| {
                Ok(RecallRecord {
                    run_id: run_id.to_owned(),
                    purpose: row.try_get(0).map_err(database)?,
                    scene: row.try_get(1).map_err(database)?,
                    speaker: row.try_get(2).map_err(database)?,
                    addressee: row.try_get(3).map_err(database)?,
                    audience: row.try_get(4).map_err(database)?,
                    query_text: row.try_get(5).map_err(database)?,
                    mode: row.try_get(6).map_err(database)?,
                    embedding_model: row.try_get(7).map_err(database)?,
                    query_vector_digest: row.try_get(8).map_err(database)?,
                    pinned_ids: row.try_get(9).map_err(database)?,
                    related_ids: row.try_get(10).map_err(database)?,
                    scores: row.try_get(11).map_err(database)?,
                    pack_text: row.try_get(12).map_err(database)?,
                })
            })
            .transpose()
        })
        .await
    }

    pub async fn dossier(
        &self,
        run_id: &str,
        entity: &str,
        audience: &str,
    ) -> Result<Option<DossierRow>, StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            let row = client
                .query_opt(
                    "SELECT event_set_hash, covered_ids, summary, writer FROM df_story.entity_dossiers \
                     WHERE run_id = $1 AND entity_id = $2 AND audience = $3",
                    &[&run_id, &entity, &audience],
                )
                .await
                .map_err(database)?;
            row.map(|row| {
                Ok(DossierRow {
                    entity_id: entity.to_owned(),
                    audience: audience.to_owned(),
                    event_set_hash: row.try_get(0).map_err(database)?,
                    covered_ids: row.try_get(1).map_err(database)?,
                    summary: row.try_get(2).map_err(database)?,
                    writer: row.try_get(3).map_err(database)?,
                })
            })
            .transpose()
        })
        .await
    }

    pub async fn put_dossier(
        &self,
        run_id: &str,
        dossier: &DossierRow,
    ) -> Result<(), StoryMemoryError> {
        self.bounded(async {
            let slot = self.lock().await?;
            let client = slot.client.as_ref().ok_or(StoryMemoryError::Invalid("no client"))?;
            client
                .execute(
                    "INSERT INTO df_story.entity_dossiers \
                     (run_id, entity_id, audience, event_set_hash, covered_ids, summary, writer) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (run_id, entity_id, audience) DO UPDATE SET \
                     event_set_hash = EXCLUDED.event_set_hash, covered_ids = EXCLUDED.covered_ids, \
                     summary = EXCLUDED.summary, writer = EXCLUDED.writer, created_at = clock_timestamp()",
                    &[
                        &run_id,
                        &dossier.entity_id,
                        &dossier.audience,
                        &dossier.event_set_hash,
                        &dossier.covered_ids,
                        &dossier.summary,
                        &dossier.writer,
                    ],
                )
                .await
                .map_err(database)?;
            Ok(())
        })
        .await
    }
}

fn scored(rows: &[Row]) -> Result<Vec<Scored>, StoryMemoryError> {
    rows.iter()
        .map(|row| {
            Ok(Scored {
                event_id: row.try_get(0).map_err(database)?,
                score: row.try_get(1).map_err(database)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_keeps_only_words_so_player_text_cannot_shape_the_search() {
        assert_eq!(
            or_query("Marek's toll -- \"harbour\" OR drop;table"),
            "marek or toll or harbour or or or drop or table"
        );
        assert_eq!(or_query("  !! ?? "), "");
        assert_eq!(or_query("a b"), "");
    }

    #[test]
    fn audiences_allow_exactly_what_they_should() {
        use Visibility::*;
        let hero = Audience::Member("brynn".to_owned());
        assert!(Audience::Public.allows(Public, None));
        assert!(!Audience::Public.allows(Party, None));
        assert!(!Audience::Public.allows(Private, Some("brynn")));
        assert!(Audience::Party.allows(Party, None));
        assert!(!Audience::Party.allows(Private, Some("brynn")));
        assert!(hero.allows(Private, Some("brynn")));
        assert!(!hero.allows(Private, Some("tobin")));
        assert!(hero.allows(Party, None));
        assert_eq!(Audience::parse("member:brynn"), Some(hero));
        assert_eq!(Audience::parse("member:"), None);
    }

    #[test]
    fn entities_are_trimmed_lowercased_deduplicated_and_bounded() {
        let entities = Entities {
            npc: vec![" Marek ".to_owned(), "marek".to_owned(), String::new()],
            hero: (0..40).map(|n| format!("h{n}")).collect(),
            ..Entities::default()
        }
        .normalised();
        assert_eq!(entities.npc, vec!["marek".to_owned()]);
        assert_eq!(entities.hero.len(), MAXIMUM_ENTITIES_PER_KIND);
    }

    #[test]
    fn a_vector_literal_needs_the_exact_width_and_finite_values() {
        assert!(vector_literal(&[0.0; 3]).is_err());
        let mut vector = vec![0.5_f32; EMBEDDING_DIMENSIONS];
        assert!(vector_literal(&vector).unwrap().starts_with("[0.5,0.5"));
        vector[3] = f32::NAN;
        assert!(vector_literal(&vector).is_err());
    }
}
