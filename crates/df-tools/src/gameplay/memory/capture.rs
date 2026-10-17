//! Capture: every way an event reaches the story memory.
//!
//! * The **table tap** ([`attach`]) copies each public table line as the table log appends it.
//! * The **capture API** (the free functions below) is what the structured sources call, one line
//!   each: canon, grievances, scene changes, rulings, narration, private clues, promises and
//!   combat outcomes. Each does nothing when no memory is installed.
//! * [`backfill`](super::backfill) writes the same events from read-only accessors at start, with
//!   the same builders, so a live capture and a backfill of one source event share a natural key
//!   and add one row.
//!
//! Nothing here waits: an event is a `try_send` into the bounded queue.

use std::sync::Arc;

use df_persistence::story_memory::{
    Entities, EventKind, MAXIMUM_BODY_CHARACTERS, NewEvent, Visibility,
};
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};

use super::{Job, StoryMemory, current, locked};
use crate::gameplay::table_log::{self, LineSink, TableLog};

/// The longest text a capture keeps, in characters.
const KEPT_CHARACTERS: usize = 1200;

/// A short stable hash of text parts, for natural keys.
pub(in crate::gameplay) fn short_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0x1f]);
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `text` as an id token: lowercase letters and digits, with every other run made one `-`.
pub(in crate::gameplay) fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.extend(character.to_lowercase());
        } else {
            gap = true;
        }
    }
    slug.chars().take(96).collect()
}

/// Control characters become spaces, runs of space become one, the ends are trimmed.
pub(in crate::gameplay) fn tidy(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(KEPT_CHARACTERS.min(MAXIMUM_BODY_CHARACTERS))
        .collect()
}

/// One event, before it is given a run and a natural key.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct Draft {
    kind: EventKind,
    source: &'static str,
    text: String,
    visibility: Visibility,
    private_to: Option<String>,
    scene: Option<String>,
    entities: Entities,
    revision: Option<i64>,
    key: Option<String>,
    rebuildable: bool,
}

impl Draft {
    pub(in crate::gameplay) fn new(kind: EventKind, source: &'static str, text: &str) -> Self {
        Self {
            kind,
            source,
            text: tidy(text),
            visibility: Visibility::Public,
            private_to: None,
            scene: None,
            entities: Entities::default(),
            revision: None,
            key: None,
            rebuildable: false,
        }
    }

    #[must_use]
    pub(in crate::gameplay) fn scene(mut self, scene: &str) -> Self {
        let scene = tidy(scene);
        self.scene = (!scene.is_empty()).then(|| scene.chars().take(128).collect());
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn npc(mut self, id: &str) -> Self {
        self.entities.npc.push(id.to_owned());
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn hero(mut self, name: &str) -> Self {
        self.entities.hero.push(slug(name));
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn place(mut self, id: &str) -> Self {
        self.entities.place.push(id.to_owned());
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn item(mut self, id: &str) -> Self {
        self.entities.item.push(id.to_owned());
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn revision(mut self, revision: Option<i64>) -> Self {
        self.revision = revision;
        self
    }

    /// The source's own identity for this event (a journal position, a canon key); without one
    /// the key is a hash of the event's content.
    #[must_use]
    pub(in crate::gameplay) fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_owned());
        self
    }

    /// A canonical source can write this event again.
    #[must_use]
    pub(in crate::gameplay) fn rebuildable(mut self) -> Self {
        self.rebuildable = true;
        self
    }

    #[must_use]
    pub(in crate::gameplay) fn party_only(mut self) -> Self {
        self.visibility = Visibility::Party;
        self
    }

    /// A clue for one hero: never recalled into a public line.
    #[must_use]
    pub(in crate::gameplay) fn private_to(mut self, hero: &str) -> Self {
        self.visibility = Visibility::Private;
        let hero = slug(hero);
        self.entities.hero.push(hero.clone());
        self.private_to = Some(hero);
        self
    }

    pub(in crate::gameplay) fn event_kind(&self) -> EventKind {
        self.kind
    }
}

/// The canon builder shared by the live call and the backfill.
pub(in crate::gameplay) fn canon_fact_draft(
    scene: &str,
    kind: &str,
    name: &str,
    content: &str,
    established_by: &str,
) -> Draft {
    let mut draft = Draft::new(EventKind::CanonFact, "canon", &format!("{name}: {content}"))
        .scene(scene)
        .rebuildable()
        .key(&format!(
            "lore:{scene}:{kind}:{}:{}",
            slug(name),
            short_hash(&[content])
        ));
    draft = match kind {
        "place" => draft.place(&slug(name)),
        "person" => draft.npc(&slug(name)),
        _ => draft,
    };
    if !established_by.trim().is_empty() && !established_by.eq_ignore_ascii_case("dev control") {
        draft = draft.npc(&slug(established_by));
    }
    draft
}

/// The grievance builder shared by the live call and the backfill. `recollection` is the line the
/// social ledger gives the NPC to recall.
pub(in crate::gameplay) fn grievance_draft(
    hero: &str,
    npc_id: &str,
    npc_name: &str,
    recollection: &str,
) -> Draft {
    Draft::new(
        EventKind::GrievanceAttitude,
        "social",
        &format!("{npc_name} holds a grievance against {hero}: {recollection}"),
    )
    .npc(npc_id)
    .hero(hero)
    .rebuildable()
    .key(&format!(
        "grievance:{}:{npc_id}:{}",
        slug(hero),
        short_hash(&[recollection])
    ))
}

/// The scene-change builder shared by the live call and the backfill. `tag` is the change's own
/// closed kind label (`purchased`, `stolen`, `opened`, ...); `involved` are the other ids its kind
/// object names (holder, seller, buyer).
pub(in crate::gameplay) fn scene_change_draft(
    position: &str,
    revision: Option<i64>,
    place: &str,
    hero: &str,
    entity_id: &str,
    description: &str,
    tag: &str,
    involved: &[String],
) -> Draft {
    let kind = match tag {
        "purchased" | "npc_purchased" | "sold" | "taken" | "stolen" | "steal_attempt" | "given"
        | "returned" => EventKind::Transaction,
        "attitude_changed" => EventKind::GrievanceAttitude,
        _ => EventKind::SceneChange,
    };
    let mut draft = Draft::new(
        kind,
        "journal",
        &format!("{hero} at {place}: {description}"),
    )
    .scene(place)
    .place(&slug(place))
    .hero(hero)
    .item(entity_id)
    .revision(revision)
    .rebuildable()
    .key(&format!(
        "journal:{position}:{}",
        short_hash(&[entity_id, tag, description])
    ));
    for id in involved {
        if slug(id) != slug(hero) && !id.is_empty() {
            draft = draft.npc(id);
        }
    }
    draft
}

impl StoryMemory {
    /// Builds the stored event for a draft: this run, a natural key, a tidy bounded text.
    pub(in crate::gameplay) fn event_from(&self, draft: Draft) -> Option<NewEvent> {
        if draft.text.is_empty() {
            return None;
        }
        let entities = draft.entities.normalised();
        let scene = draft.scene.clone().or_else(|| locked(&self.scene).clone());
        let natural_key = match &draft.key {
            Some(key) => format!("{key}"),
            None => format!(
                "{}:{}",
                draft.source,
                short_hash(&[
                    draft.kind.label(),
                    scene.as_deref().unwrap_or(""),
                    draft.visibility.label(),
                    draft.private_to.as_deref().unwrap_or(""),
                    &draft.text,
                    &entities.all().join(","),
                ])
            ),
        };
        Some(NewEvent {
            run_id: self.run.clone(),
            natural_key: natural_key.chars().take(250).collect(),
            source: draft.source.to_owned(),
            source_epoch: None,
            source_seq: None,
            game_revision: draft.revision,
            kind: draft.kind,
            visibility: draft.visibility,
            private_to: draft.private_to,
            text: draft.text,
            scene,
            entities,
            rebuildable: draft.rebuildable,
            supersedes: None,
        })
    }

    /// Queues a draft; returns whether it was accepted into the queue.
    pub(in crate::gameplay) fn record(&self, draft: Draft) -> bool {
        match self.event_from(draft) {
            Some(event) => {
                self.submit(Job::Event(Box::new(event)));
                true
            }
            None => false,
        }
    }

    /// The scene the next un-staged events belong to (a table line does not carry one).
    pub(in crate::gameplay) fn set_scene(&self, scene: &str) {
        let scene = tidy(scene);
        *locked(&self.scene) = (!scene.is_empty()).then(|| scene.chars().take(128).collect());
    }

    /// The stored event for one public table line, or none for a line that is not story (the
    /// server's own `table` notes, an empty line).
    pub(in crate::gameplay) fn table_event(&self, line: &rpc::TableLine) -> Option<NewEvent> {
        let speaker = tidy(&line.speaker);
        let text = tidy(&line.text);
        if text.is_empty() || speaker == table_log::SYSTEM_SPEAKER {
            return None;
        }
        let role = rpc::TableRole::try_from(line.role).unwrap_or(rpc::TableRole::Unspecified);
        let kind = rpc::TableKind::try_from(line.kind).unwrap_or(rpc::TableKind::Unspecified);
        let mut entities = Entities::default();
        let (event_kind, body) = match (role, kind) {
            (rpc::TableRole::Hero, _) => {
                locked(&self.roster).insert(line.hero_index, speaker.clone());
                entities.hero.push(slug(&speaker));
                if kind == rpc::TableKind::Speech {
                    (EventKind::HeroSpeech, format!("{speaker}: {text}"))
                } else {
                    (EventKind::ActionRuling, format!("{speaker} {text}"))
                }
            }
            (rpc::TableRole::Npc, _) => {
                entities.npc.push(if line.actor_id.trim().is_empty() {
                    slug(&speaker)
                } else {
                    line.actor_id.clone()
                });
                if kind == rpc::TableKind::Speech {
                    (EventKind::NpcSpeech, format!("{speaker}: {text}"))
                } else {
                    (EventKind::ActionRuling, format!("{speaker} {text}"))
                }
            }
            (rpc::TableRole::Narrator, _) if speaker == table_log::CHECK_SPEAKER => {
                if let Some(name) = locked(&self.roster).get(&line.hero_index) {
                    entities.hero.push(slug(name));
                }
                (EventKind::CheckRoll, text)
            }
            _ => (EventKind::Narration, text),
        };
        let epoch = self.boot.clone();
        let sequence = i64::try_from(line.sequence).ok()?;
        Some(NewEvent {
            run_id: self.run.clone(),
            natural_key: format!("table:{epoch}:{sequence}:{}", short_hash(&[&body])),
            source: "table_log".to_owned(),
            source_epoch: Some(epoch),
            source_seq: Some(sequence),
            game_revision: None,
            kind: event_kind,
            visibility: Visibility::Public,
            private_to: None,
            text: body,
            scene: locked(&self.scene).clone(),
            entities: entities.normalised(),
            rebuildable: false,
            supersedes: None,
        })
    }
}

/// The table log's tap into the memory.
struct TableTap(Arc<StoryMemory>);

impl LineSink for TableTap {
    fn line(&self, line: &rpc::TableLine) {
        if let Some(event) = self.0.table_event(line) {
            self.0.submit(Job::Event(Box::new(event)));
        }
    }

    fn rewound(&self, floor: u64) {
        self.0.submit(Job::RetireTableSince {
            from_seq: i64::try_from(floor).unwrap_or(i64::MAX),
        });
    }
}

/// Installs the tap: from now on every public line the log appends is captured.
pub(in crate::gameplay) fn attach(memory: &Arc<StoryMemory>, log: &TableLog) {
    log.set_sink(Arc::new(TableTap(Arc::clone(memory))));
}

fn with_memory(work: impl FnOnce(&StoryMemory)) {
    if let Some(memory) = current() {
        work(&memory);
    }
}

// ---- The capture API: one line at each structured source. ----

/// A fact an NPC established as local canon (`kind` is the fact's own kind label).
pub(in crate::gameplay) fn canon_fact(
    scene: &str,
    kind: &str,
    name: &str,
    content: &str,
    established_by: &str,
) {
    with_memory(|memory| {
        memory.record(canon_fact_draft(scene, kind, name, content, established_by));
    });
}

/// A wrong a hero did an NPC, as the line the NPC recalls it in.
pub(in crate::gameplay) fn grievance(hero: &str, npc_id: &str, npc_name: &str, recollection: &str) {
    with_memory(|memory| {
        memory.record(grievance_draft(hero, npc_id, npc_name, recollection));
    });
}

/// One committed scene change.
pub(in crate::gameplay) fn scene_change(
    position: &str,
    revision: Option<i64>,
    place: &str,
    hero: &str,
    entity_id: &str,
    description: &str,
    tag: &str,
    involved: &[String],
) {
    with_memory(|memory| {
        memory.record(scene_change_draft(
            position,
            revision,
            place,
            hero,
            entity_id,
            description,
            tag,
            involved,
        ));
    });
}

/// A freeform action's ruling (what the hero tried and what the table decided).
pub(in crate::gameplay) fn ruling(hero: &str, scene: &str, text: &str, revision: Option<i64>) {
    with_memory(|memory| {
        memory.record(
            Draft::new(EventKind::ActionRuling, "freeform", text)
                .hero(hero)
                .scene(scene)
                .revision(revision),
        );
    });
}

/// Storyteller narration of a scene.
pub(in crate::gameplay) fn narration(scene: &str, text: &str, revision: Option<i64>) {
    with_memory(|memory| {
        memory.record(
            Draft::new(EventKind::Narration, "storyteller", text)
                .scene(scene)
                .revision(revision),
        );
    });
}

/// A clue only `hero` may be told.
pub(in crate::gameplay) fn private_clue(
    hero: &str,
    scene: &str,
    text: &str,
    revision: Option<i64>,
) {
    with_memory(|memory| {
        memory.record(
            Draft::new(EventKind::CanonFact, "private_clue", text)
                .scene(scene)
                .revision(revision)
                .private_to(hero),
        );
    });
}

/// A promise made or an open thread raised (`by` is the NPC id, `to` the hero it concerns).
pub(in crate::gameplay) fn promise(
    by: &str,
    to: &str,
    scene: &str,
    text: &str,
    revision: Option<i64>,
) {
    with_memory(|memory| {
        let mut draft = Draft::new(EventKind::PromiseThread, "promise", text)
            .scene(scene)
            .revision(revision);
        if !by.is_empty() {
            draft = draft.npc(by);
        }
        if !to.is_empty() {
            draft = draft.hero(to);
        }
        memory.record(draft);
    });
}

/// A combat outcome worth remembering (who fell, who fled).
pub(in crate::gameplay) fn combat_outcome(scene: &str, text: &str, revision: Option<i64>) {
    with_memory(|memory| {
        memory.record(
            Draft::new(EventKind::CombatOutcome, "combat", text)
                .scene(scene)
                .revision(revision),
        );
    });
}

/// The scene now being played, so lines that carry none are stamped with it.
pub(in crate::gameplay) fn scene_entered(scene: &str) {
    with_memory(|memory| memory.set_scene(scene));
}

/// A development rewind abandoned every revision after `revision`.
pub(in crate::gameplay) fn rewound_to_revision(revision: i64) {
    with_memory(|memory| memory.submit(Job::RetireAfterRevision(revision)));
}
