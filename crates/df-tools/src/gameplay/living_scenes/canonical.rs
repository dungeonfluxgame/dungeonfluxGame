//! Accepted identities feed the existing run-owned image worker. Results are selected only
//! when their complete scope and revision still match the accepted source.
use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneModel, ScenePerson},
    hero_identity::ValidatedIdentity,
    imagery::{self, HeroVisualIdentity, Lookup},
};
use df_model::checkpoint::{Checkpoint, EntityId};
use df_render::art_contract;
use sha2::{Digest, Sha256};

type RoomResources<'a> = (
    Option<&'a str>,
    Option<&'a str>,
    Option<imagery::RoomArchitectureContext>,
);

#[path = "mural.rs"]
mod mural;
// Test builds already load this file as crate::live_art::mural_view; load it once per build.
#[cfg(not(test))]
#[path = "mural_view.rs"]
mod mural_view;
#[cfg(test)]
use crate::live_art::mural_view;

const ROOM_RETRY_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Target {
    Mural,
    Hero(EntityId),
    Npc(String),
    Room { place: String, revision: String },
}

#[derive(Clone, Debug)]
enum Source {
    Mural(Vec<mural::MuralHero>),
    Hero {
        name: String,
        identity: Box<ValidatedIdentity>,
        species: String,
        gear: Vec<&'static str>,
        public_design: String,
        portrait: Option<df_model::checkpoint::AssetReference>,
        locator: String,
    },
    Npc {
        person: ScenePerson,
        gender: Option<pack::Gender>,
        age: Option<pack::Age>,
        identity_photo: Option<String>,
        identity_figure_photo: Option<String>,
        canon: Vec<serde_json::Value>,
    },
    Room {
        model: SceneModel,
        place: String,
        context: Option<imagery::RoomArchitectureContext>,
        canon: Vec<serde_json::Value>,
    },
    ReviewedRoom {
        asset: String,
    },
    /// A cast member with no authored description and no reviewed photograph (a creature, a
    /// patron or a foe the adventure names), made on demand by the art contract's rule.
    Cast {
        member: Member,
        canon: Vec<serde_json::Value>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Work {
    target: Target,
    scope: String,
    binding: String,
    source: Source,
    regenerate: bool,
}

#[derive(Clone, Debug)]
enum Status {
    Queued,
    Generating,
    Ready(ImageKey),
    Failed(ImageFailure),
}

#[derive(Clone, Debug)]
struct Entry {
    work: Work,
    status: Status,
    /// Dispatch time while generating; failure completion time during the readmission cooldown.
    started: Option<Instant>,
}

#[derive(Default)]
pub(super) struct References {
    lore: Option<Arc<crate::gameplay::npc_reply::local_lore::LocalLore>>,
    room_revisions: BTreeMap<(String, String), String>,
    hero_checkpoint: Option<(Checkpoint, crate::gameplay::pack::CacheIdentity)>,
    mural_enabled: bool,
    mural_expected_count: usize,
    scope: String,
    entries: BTreeMap<Target, Entry>,
    seats: BTreeMap<u8, EntityId>,
    selected_rooms: BTreeMap<String, Target>,
    authored_rooms: BTreeMap<Target, String>,
    current_room: Option<Target>,
    current_npcs: BTreeSet<String>,
    /// How many times each person's picture was cleared for regeneration; part of the binding, so
    /// a cleared picture is a new request and a new image.
    regen: BTreeMap<String, u32>,
    /// The `ART resolve` lines already written, so a view that is rebuilt often logs once.
    art_logged: BTreeSet<String>,
}

pub(super) struct WorkerLifetime(pub(super) Arc<Inner>);

impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        for entry in locked(&self.0.state).canonical.entries.values_mut() {
            if matches!(entry.status, Status::Queued | Status::Generating) {
                diagnostic(&entry.work, "cancelled", None, None);
                entry.status = Status::Failed(ImageFailure::Interrupted);
            }
        }
    }
}

/// Failures remain visible until explicit room readmission or a changed binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay::living_scenes) enum ReferenceStatus {
    Missing,
    Pending,
    Ready,
    Failed(ImageFailure),
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl References {
    pub(super) fn new(lore: Arc<crate::gameplay::npc_reply::local_lore::LocalLore>) -> Self {
        Self {
            lore: Some(lore),
            ..Self::default()
        }
    }

    fn select(&mut self, target: Target, source: Source, revision: &str) -> bool {
        let binding = digest(
            format!(
                "canonical-reference-v1|{}|{target:?}|{revision}",
                self.scope
            )
            .as_bytes(),
        );
        if let Some(entry) = self.entries.get_mut(&target)
            && entry.work.binding == binding
        {
            // Re-admission after cooldown is an explicit hub invalidation, not a side retry
            // loop. Each generation batch still uses the hub/imagery attempt bounds.
            if (matches!(target, Target::Room { .. })
                && matches!(entry.status, Status::Failed(ImageFailure::BadOutput))
                || matches!(target, Target::Mural)
                    && matches!(entry.status, Status::Failed(ImageFailure::Interrupted)))
                && entry
                    .started
                    .is_some_and(|finished| finished.elapsed() >= ROOM_RETRY_COOLDOWN)
            {
                entry.work.regenerate = true;
                entry.status = Status::Queued;
                diagnostic(&entry.work, "queued", None, None);
            }
            return false;
        }
        let work = Work {
            target: target.clone(),
            scope: self.scope.clone(),
            binding,
            source,
            regenerate: false,
        };
        diagnostic(&work, "queued", None, None);
        self.entries.insert(
            target,
            Entry {
                work,
                status: Status::Queued,
                started: None,
            },
        );
        true
    }

    #[cfg(test)]
    pub(super) fn take_jobs(&mut self) -> Vec<Job> {
        self.take_jobs_up_to(imagery::MAXIMUM_IN_FLIGHT)
    }

    pub(super) fn take_jobs_up_to(&mut self, available: usize) -> Vec<Job> {
        self.refresh_mural();
        let mut wanted = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.status, Status::Queued))
            .map(|(target, _)| {
                let priority = if self.current_room.as_ref() == Some(target) {
                    0
                } else {
                    match target {
                        Target::Hero(_) => 1,
                        Target::Npc(id) if self.current_npcs.contains(id) => 2,
                        Target::Room { .. } => 3,
                        Target::Npc(_) => 4,
                        Target::Mural => 5,
                    }
                };
                (priority, target.clone())
            })
            .collect::<Vec<_>>();
        wanted.sort();
        wanted
            .into_iter()
            .take(available)
            .filter_map(|(_, target)| {
                let entry = self.entries.get_mut(&target)?;
                entry.status = Status::Generating;
                entry.started = Some(Instant::now());
                diagnostic(&entry.work, "requested", None, None);
                Some(Job::Canonical(entry.work.clone()))
            })
            .collect()
    }

    fn ready(&self, target: &Target) -> Option<ImageKey> {
        match &self.entries.get(target)?.status {
            Status::Ready(key) => Some(key.clone()),
            _ => None,
        }
    }

    /// The art contract's on-demand request for a cast member whose picture is first needed in a
    /// view. Idempotent per subject: a person already admitted (from a scene model, with an
    /// authored description) is left alone; a cast source is refreshed only when its accepted
    /// presentation or committed appearance canon changes. Server only.
    pub(super) fn admit_cast(&mut self, member: &Member) {
        let target = Target::Npc(member.id.clone());
        if self.scope.is_empty()
            || self
                .entries
                .get(&target)
                .is_some_and(|entry| !matches!(entry.work.source, Source::Cast { .. }))
        {
            return;
        }
        let canon = self.committed_canon(None, Some(&member.id));
        let mut revision_content = serde_json::json!({
            "id": member.id,
            "name": member.name,
            "role": format!("{:?}", member.role),
            "life_stage": format!("{:?}", member.life_stage),
            "regen": self.regen.get(&member.id),
        });
        if let Some(gender) = member.gender {
            revision_content["gender"] = serde_json::json!(gender);
        }
        if let Some(age) = member.age {
            revision_content["age"] = serde_json::json!(age);
        }
        if !canon.is_empty() {
            revision_content["canon"] = serde_json::json!(canon);
        }
        let revision = digest(revision_content.to_string().as_bytes());
        let selected = self.select(
            target,
            Source::Cast {
                member: member.clone(),
                canon,
            },
            &revision,
        );
        if selected
            && (!matches!(
                member.gender,
                Some(pack::Gender::He | pack::Gender::She | pack::Gender::They)
            ) || member.age.is_none())
        {
            super::super::actor::record_audit_line(
                "gameplay.imagery",
                format!("IMAGERY presentation_missing person={}", member.id),
            );
        }
    }

    /// Writes one `ART resolve` line per subject and source: what a view first found for it.
    pub(super) fn note_resolved(&mut self, subject: &art_contract::Subject, source: &str) {
        if self
            .art_logged
            .insert(format!("{}|{source}", subject.key()))
        {
            super::super::actor::record_audit_line(
                "gameplay.art",
                format!(
                    "ART resolve subject={} role={:?} source={source}",
                    subject.key(),
                    art_contract::Role::CharacterToken
                ),
            );
        }
    }

    /// Drops a finished picture (ready or failed) so the next view needing it asks again, and
    /// makes the next request a new binding so the image is made anew rather than read back from
    /// the cache. A picture still being made is left alone. Returns whether one was dropped.
    pub(super) fn clear_npc(&mut self, id: &str) -> bool {
        let target = Target::Npc(id.to_owned());
        let finished = self
            .entries
            .get(&target)
            .is_some_and(|entry| matches!(entry.status, Status::Ready(_) | Status::Failed(_)));
        if finished {
            self.entries.remove(&target);
            *self.regen.entry(id.to_owned()).or_default() += 1;
            self.art_logged
                .retain(|line| !line.starts_with(&format!("character:{id}|")));
        }
        finished
    }

    pub(super) fn hero(&self, entity: EntityId) -> Option<ImageKey> {
        self.ready(&Target::Hero(entity))
    }

    #[cfg(test)]
    pub(super) fn fixture_hero(&mut self, seat: u8, name: &str, key: ImageKey) {
        let entity = EntityId::from_bytes(&[seat; 16]).unwrap();
        if self.scope.is_empty() {
            self.scope = "health-fixture".to_owned();
        }
        self.select(
            Target::Hero(entity),
            Source::Hero {
                name: name.to_owned(),
                identity: Box::new(
                    crate::gameplay::hero_identity::Identity::from_menus(&[]).unwrap(),
                ),
                species: "dwarf".to_owned(),
                gear: Vec::new(),
                public_design: name.to_owned(),
                portrait: None,
                locator: name.to_owned(),
            },
            name,
        );
        self.seats.insert(seat, entity);
        self.entries.get_mut(&Target::Hero(entity)).unwrap().status = Status::Ready(key);
    }

    pub(super) fn hero_path(&self, entity: EntityId) -> Option<String> {
        if let Some(key) = self.hero(entity) {
            return Some(key.path());
        }
        match &self.entries.get(&Target::Hero(entity))?.work.source {
            Source::Hero {
                portrait: Some(asset),
                ..
            } => Some(format!("/gameplay/creator-portrait/{}", asset.key.as_str())),
            _ => None,
        }
    }

    pub(super) fn hero_at_seat(&self, seat: u8) -> Option<(EntityId, ReferenceStatus)> {
        let entity = *self.seats.get(&seat)?;
        Some((entity, self.status(&Target::Hero(entity))))
    }

    pub(super) fn bound_room_revision<'a>(&'a self, place: &str, revision: &'a str) -> &'a str {
        self.room_revisions
            .get(&(place.to_owned(), revision.to_owned()))
            .map_or(revision, String::as_str)
    }

    /// Only committed harmless journal facts with an exact owner reach imagery. Journal
    /// reasons, questions, answers and protected engine context never enter this projection.
    pub(super) fn committed_canon(
        &self,
        place: Option<&str>,
        person: Option<&str>,
    ) -> Vec<serde_json::Value> {
        use crate::gameplay::npc_reply::{
            fact_backfill::{Decision, Reason},
            local_lore::FactKind,
        };
        let Some(lore) = &self.lore else {
            return Vec::new();
        };
        let secret_terms = pack::with_content(|content| content.secret_terms.clone());
        lore.development_decisions()
            .into_iter()
            .filter(|decision| {
                decision.decision == Decision::Backfill
                    && decision.reason == Reason::HarmlessMissingDetail
            })
            .filter(|decision| match (place, person) {
                (Some(place), _) => decision.place == place,
                (_, Some(person)) => decision.npc_id == person,
                _ => false,
            })
            .filter_map(|decision| {
                let fact = decision.fact?;
                // The journal has no separate visual-applicability field. Person is the only
                // typed appearance source; history/rumours are not portrait instructions.
                if person.is_some() && fact.kind != FactKind::Person {
                    return None;
                }
                if secret_terms.iter().any(|term| {
                    crate::gameplay::adventure::dossier::mentions_term(&fact.name, term)
                        || crate::gameplay::adventure::dossier::mentions_term(&fact.content, term)
                }) {
                    return None;
                }
                Some(serde_json::json!({
                    "kind": fact.kind.label(), "name": fact.name, "content": fact.content,
                    "person_id": decision.npc_id, "place_id": decision.place,
                }))
            })
            .collect()
    }

    fn status(&self, target: &Target) -> ReferenceStatus {
        match self.entries.get(target).map(|entry| &entry.status) {
            None => ReferenceStatus::Missing,
            Some(Status::Queued | Status::Generating) => ReferenceStatus::Pending,
            Some(Status::Ready(_)) => ReferenceStatus::Ready,
            Some(Status::Failed(failure)) => ReferenceStatus::Failed(*failure),
        }
    }

    /// True only while work for the selected room can actually produce its photograph.
    pub(super) fn room_pending(&self, place: &str, imagery_on: bool) -> bool {
        self.selected_rooms
            .get(place)
            .and_then(|target| self.entries.get(target))
            .is_some_and(|entry| {
                matches!(entry.status, Status::Queued | Status::Generating)
                    && (imagery_on || matches!(entry.work.source, Source::ReviewedRoom { .. }))
            })
    }

    pub(super) fn room(&self, place: &str) -> Option<ImageKey> {
        self.ready(self.selected_rooms.get(place)?)
    }

    fn room_at(&self, place: &str, revision: &str) -> Option<ImageKey> {
        let revision = self.bound_room_revision(place, revision);
        self.ready(&Target::Room {
            place: place.to_owned(),
            revision: revision.to_owned(),
        })
    }

    pub(super) fn npc(&self, id: &str) -> Option<ImageKey> {
        self.ready(&Target::Npc(id.to_owned()))
    }
}

#[cfg(test)]
#[path = "plates/hero_identity_fixture.rs"]
pub(super) mod hero_identity_fixture;

/// The art contract's subject of a canonical target: what the `IMAGERY` audit lines name.
fn subject(target: &Target) -> art_contract::Subject {
    match target {
        Target::Mural => art_contract::Subject::place("party-mural"),
        Target::Hero(entity) => {
            art_contract::Subject::character(format!("hero:{}", digest(entity.as_bytes())))
        }
        Target::Npc(id) => art_contract::Subject::character(id.clone()),
        Target::Room { place, .. } => art_contract::Subject::place(place.clone()),
    }
}

fn diagnostic(work: &Work, status: &str, key: Option<&ImageKey>, elapsed: Option<Duration>) {
    let target = match &work.target {
        Target::Mural => "party-mural".to_owned(),
        Target::Hero(entity) => format!("hero:{}", digest(entity.as_bytes())),
        Target::Npc(id) => format!("npc:{id}"),
        Target::Room { place, revision } => format!("room:{place}:{revision}"),
    };
    let key = key.map_or("", ImageKey::as_str);
    let source = if matches!(work.source, Source::ReviewedRoom { .. }) {
        "reviewed"
    } else {
        "live"
    };
    // The on-demand generation is observable in the same IMAGERY audit lines as every other
    // image: one line per state change of the subject, and a failure names the neutral fallback
    // the display keeps.
    let kind = match &work.target {
        Target::Mural => "party_mural",
        Target::Room { .. } => ImageKind::CanonicalRoom.label(),
        Target::Hero(_) | Target::Npc(_) => ImageKind::CanonicalCharacter.label(),
    };
    let fallback = match status {
        "queued" | "requested" | "ready" | "stale_discarded" => "",
        _ => " fallback=neutral",
    };
    // The art contract's audit line: subject, role, source and timing of each step.
    let (event, role) = (
        match status {
            "requested" => Some("request"),
            "ready" => Some("ready"),
            "queued" | "stale_discarded" => None,
            _ => Some("failed"),
        },
        match &work.target {
            Target::Hero(_) => art_contract::Role::HeroPortrait,
            Target::Npc(_) => art_contract::Role::CharacterToken,
            Target::Room { .. } | Target::Mural => art_contract::Role::StagePlace,
        },
    );
    if let Some(event) = event {
        super::super::actor::record_audit_line(
            "gameplay.art",
            format!(
                "ART {event} subject={} role={role:?} source={source} ms={} status={status} id={key}",
                subject(&work.target).key(),
                elapsed.map_or(0, |elapsed| elapsed.as_millis()),
            ),
        );
    }
    super::super::actor::record_audit_line(
        "gameplay.imagery_demand",
        format!(
            "IMAGERY demand kind={kind} subject={} status={status} id={key} binding={}{fallback}",
            subject(&work.target).key(),
            work.binding
        ),
    );
    super::super::actor::record_audit_diagnostic(
        "gameplay.canonical_reference",
        format!(
            "CANONICAL_REFERENCE target={target} binding={} status={status} key={key}",
            work.binding
        ),
        serde_json::Map::from_iter([
            ("target".to_owned(), serde_json::json!(target)),
            ("binding".to_owned(), serde_json::json!(work.binding)),
            ("run_scope".to_owned(), serde_json::json!(work.scope)),
            ("status".to_owned(), serde_json::json!(status)),
            ("image_key".to_owned(), serde_json::json!(key)),
        ]),
        &[
            ("reference.binding", work.binding.clone()),
            ("reference.scope", work.scope.clone()),
            ("reference.status", status.to_owned()),
        ],
    );
}

impl Inner {
    pub(super) fn observe_checkpoint(&self, checkpoint: &Checkpoint) {
        let basis = checkpoint.basis();
        let scope = format!(
            "session={:?};run={:?};pack={:?}",
            basis.session,
            basis.run,
            checkpoint.pins().content
        );
        let mut state = locked(&self.state);
        if state.canonical.scope != scope {
            // A new scope starts fresh references but keeps the run's lore source, so committed
            // facts keep reaching room prompts and plate invalidation (HS71 finding).
            state.canonical = References {
                scope,
                lore: state.canonical.lore.clone(),
                ..Default::default()
            };
        }
        let source = crate::gameplay::pack::cache_identity();
        let expected = super::super::journey::participants(checkpoint)
            .iter()
            .filter(|member| member.character.is_some())
            .count();
        if state
            .canonical
            .hero_checkpoint
            .as_ref()
            .is_some_and(|(known, pack)| known == checkpoint && *pack == source)
            && state.canonical.seats.len() == expected
            && state
                .canonical
                .seats
                .values()
                .all(|entity| state.canonical.entries.contains_key(&Target::Hero(*entity)))
        {
            // Mural cooldowns still advance even when no hero source changed.
            state.canonical.refresh_mural();
            return;
        }
        state.canonical.hero_checkpoint = Some((checkpoint.clone(), source));
        state.canonical.mural_expected_count = super::super::journey::participants(checkpoint)
            .iter()
            .filter(|member| member.character.is_some())
            .count();
        state.canonical.seats.clear();
        let mut accepted = BTreeSet::new();
        for (index, member) in super::super::journey::participants(checkpoint)
            .into_iter()
            .enumerate()
        {
            let Some(entity) = member.character else {
                continue;
            };
            let Some(character) = checkpoint
                .state()
                .characters
                .iter()
                .find(|character| character.entity == entity)
            else {
                continue;
            };
            let choices = character
                .choices
                .iter()
                .map(|choice| {
                    (
                        choice.offer.as_str().to_owned(),
                        choice.selected.as_str().to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            let identity = ValidatedIdentity::from_canonical_appearance_choices(&choices);
            let selected = |field: &str| {
                choices
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, value)| value.as_str())
            };
            let species = selected("species").unwrap_or("");
            let Ok(identity) = identity else {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            };
            let Ok((appearance, _)) = crate::gameplay::creator_catalog::portrait_context(
                &identity,
                species,
                selected("class").unwrap_or(""),
                selected("background").unwrap_or(""),
            ) else {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            };
            let actual_gear = checkpoint.state().inventory.iter()
                .filter(|item| item.owner == entity && item.quantity > 0)
                .map(|item| serde_json::json!({"item": item.origin.entry.as_str(), "quantity": item.quantity}))
                .collect::<Vec<_>>();
            let Ok(name) = crate::gameplay::journey::name(checkpoint.state(), entity) else {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            };
            let public_design = serde_json::json!({
                "name": name,
                "public_appearance": appearance,
                "actual_gear": actual_gear,
            })
            .to_string();
            let portrait =
                crate::gameplay::hero_identity::approved_portrait_asset(checkpoint.state(), entity)
                    .cloned();
            let revision = format!(
                "{:?}|{name}|{appearance}|{portrait:?}",
                checkpoint
                    .state()
                    .entities
                    .iter()
                    .find(|world| world.id == entity)
                    .map(|world| &world.identity_revision)
            );
            let public_look = &identity.identity().appearance.public_description;
            let locator_look = if public_look.is_empty() {
                appearance.as_str()
            } else {
                public_look.as_str()
            };
            let locator = format!("{name}, {locator_look}")
                .chars()
                .take(300)
                .collect();
            let target = Target::Hero(entity);
            accepted.insert(target.clone());
            state.canonical.select(
                target,
                Source::Hero {
                    name,
                    identity: Box::new(identity),
                    species: species.to_owned(),
                    gear: Vec::new(),
                    public_design,
                    portrait,
                    locator,
                },
                &revision,
            );
            if let Ok(seat) = u8::try_from(index + 1) {
                state.canonical.seats.insert(seat, entity);
            }
        }
        state
            .canonical
            .entries
            .retain(|target, _| !matches!(target, Target::Hero(_)) || accepted.contains(target));
        state.canonical.refresh_mural();
        drop(state);
        if !crate::gameplay::adventure::free_roam::is_active() {
            return;
        }
        // Checkpoint-owned lookahead also runs without a display subscription. Canonical room
        // and person requests share the normal bounded worker and its current-scene priority.
        let scene = match crate::gameplay::beat_script_runtime::scene(checkpoint.state()) {
            Ok(scene) => scene,
            Err(error) => {
                record(
                    "gameplay.living_scene.plate_plan_failed",
                    &[("failure", format!("{error:?}"))],
                );
                return;
            }
        };
        let Some(setting) = &scene.setting else {
            return;
        };
        for ready in crate::gameplay::adventure::free_roam::prepared(checkpoint)
            .into_iter()
            .filter(|ready| ready.from == setting.place_id)
            .take(2)
        {
            match crate::gameplay::place_move::destination_scene(checkpoint, &ready.place, &scene) {
                Ok(destination) => {
                    if let Some(model) = destination
                        .setting
                        .as_ref()
                        .and_then(|setting| setting.model.as_ref())
                    {
                        self.admit_canonical_room(
                            &ready.place.place_id,
                            model,
                            &destination.cast,
                            Some(&destination.asset),
                            None,
                        );
                    }
                }
                Err(error) => record(
                    "gameplay.living_scene.plate_plan_failed",
                    &[("failure", format!("{error:?}"))],
                ),
            }
        }
    }

    pub(super) async fn render_canonical(&self, work: &Work) -> Result<ImageKey, ImageFailure> {
        if let Source::Mural(heroes) = &work.source {
            return self.render_mural(work, heroes).await;
        }
        // Only real photographs are sent; with none the request is generated from text alone.
        let mut references = Vec::new();
        match &work.source {
            Source::Hero {
                portrait: Some(asset),
                ..
            } => {
                references
                    .push(crate::gameplay::creator_portraits::approved_reference(asset).await?);
            }
            Source::Npc {
                identity_photo,
                identity_figure_photo,
                ..
            } => {
                if let Some(photo) = identity_photo {
                    references.push(self.load(photo.clone()).await?);
                }
                if let Some(photo) = identity_figure_photo {
                    references.push(self.load(photo.clone()).await?);
                }
            }
            _ => {}
        }
        if !locked(&self.state)
            .canonical
            .entries
            .get(&work.target)
            .is_some_and(|entry| entry.work.binding == work.binding)
        {
            return Err(ImageFailure::Interrupted);
        }
        let (kind, prompt, output) = match &work.source {
            Source::Mural(_) => return Err(ImageFailure::InvalidRequest),
            Source::Hero {
                identity,
                species,
                gear,
                public_design,
                portrait,
                ..
            } => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_character(
                    self.rating,
                    HeroVisualIdentity {
                        identity,
                        species,
                        held_gear: gear,
                    },
                    None,
                    public_design,
                    portrait.is_some(),
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
            Source::Npc {
                person,
                gender,
                age,
                identity_photo,
                identity_figure_photo,
                canon,
            } => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_npc(
                    self.rating,
                    person,
                    (*gender, *age, canon.as_slice()).into(),
                    identity_photo.is_some(),
                    identity_figure_photo.is_some(),
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
            Source::ReviewedRoom { asset } => {
                let reference = self.load(asset.clone()).await?;
                return self
                    .imagery
                    .register_reviewed(&subject(&work.target).key(), reference)
                    .await;
            }
            Source::Room {
                model,
                place,
                context,
                canon,
            } => (
                ImageKind::CanonicalRoom,
                imagery::canonical_room_with_context(
                    self.rating,
                    model,
                    place,
                    context.as_ref(),
                    canon,
                    &work.binding,
                )?,
                OutputSpec::BACKDROP,
            ),
            Source::Cast { member, canon } => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_cast_member(
                    self.rating,
                    &member.id,
                    &member.name,
                    member.role,
                    member.life_stage,
                    (member.gender, member.age, canon.as_slice()).into(),
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
        };
        let request = ImageRequest {
            kind,
            references,
            prompt,
            output,
        };
        let subject_key = subject(&work.target).key();
        if work.regenerate {
            use crate::gameplay::resources::{
                DepictionSpec, Invalidate, ResourceKey, ResourceStatus,
            };
            let image_key = self.imagery.key_for(&request);
            let spec = DepictionSpec::new(
                &subject_key,
                image_key.as_str(),
                art_contract::Neutral::Nothing,
            );
            let key = ResourceKey::of(&spec);
            if matches!(
                self.imagery.hub().status(&key),
                ResourceStatus::Failed(_, _)
            ) {
                self.imagery.hub().invalidate(Invalidate::Key(&key));
            }
        }
        self.imagery.generate(&subject_key, request).await
    }

    pub(super) fn finish_canonical(&self, work: Work, result: Result<ImageKey, ImageFailure>) {
        let mut state = locked(&self.state);
        let Some(entry) = state
            .canonical
            .entries
            .get_mut(&work.target)
            .filter(|entry| entry.work.binding == work.binding)
        else {
            diagnostic(&work, "stale_discarded", result.as_ref().ok(), None);
            return;
        };
        diagnostic(
            &work,
            result
                .as_ref()
                .map_or_else(|failure| failure.label(), |_| "ready"),
            result.as_ref().ok(),
            entry.started.map(|started| started.elapsed()),
        );
        let ready = result.is_ok();
        entry.status = match result {
            Ok(key) => Status::Ready(key),
            Err(failure) => Status::Failed(failure),
        };
        // Cooldown starts at failure completion, so a slow provider cannot consume it.
        entry.started = Some(Instant::now());
        // The next watcher projection recomputes the graph from the exact ready identity.
        drop(state);
        if ready && let Target::Room { place, .. } = &work.target {
            super::super::fight_foresight::request_prepared_world(place);
        }
        self.wake.send_modify(|count| *count += 1);
    }

    pub(super) fn canonical_room_key(&self, place: &str) -> Option<ImageKey> {
        locked(&self.state).canonical.room(place)
    }

    pub(super) fn canonical_room_status(&self, place: &str) -> ReferenceStatus {
        let state = locked(&self.state);
        state
            .canonical
            .selected_rooms
            .get(place)
            .map_or(ReferenceStatus::Missing, |target| {
                state.canonical.status(target)
            })
    }

    pub(super) fn admit_canonical_room(
        &self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        asset: Option<&str>,
        known_revision: Option<&str>,
    ) {
        locked(&self.state).canonical.admit_room_with_asset(
            place,
            model,
            cast,
            false,
            asset,
            known_revision,
        );
    }

    pub(super) fn canonical_room_status_at(&self, place: &str, revision: &str) -> ReferenceStatus {
        let state = locked(&self.state);
        state.canonical.status(&Target::Room {
            place: place.to_owned(),
            revision: state
                .canonical
                .bound_room_revision(place, revision)
                .to_owned(),
        })
    }

    pub(super) async fn canonical_room_reference_at(
        &self,
        place: &str,
        revision: &str,
    ) -> Option<ReferenceImage> {
        let key = locked(&self.state).canonical.room_at(place, revision)?;
        let reference = self.runtime_reference(key.clone()).await?;
        (locked(&self.state)
            .canonical
            .room_at(place, revision)
            .as_ref()
            == Some(&key))
        .then_some(reference)
    }

    pub(super) fn canonical_npc_status(&self, id: &str) -> ReferenceStatus {
        locked(&self.state)
            .canonical
            .status(&Target::Npc(id.to_owned()))
    }

    pub(super) fn canonical_hero_status(&self, seat: u8) -> ReferenceStatus {
        locked(&self.state)
            .canonical
            .hero_at_seat(seat)
            .map_or(ReferenceStatus::Missing, |(_, status)| status)
    }

    pub(super) fn canonical_hero_name(&self, seat: u8) -> Option<String> {
        let state = locked(&self.state);
        let (entity, _) = state.canonical.hero_at_seat(seat)?;
        match &state
            .canonical
            .entries
            .get(&Target::Hero(entity))?
            .work
            .source
        {
            Source::Hero { name, .. } => Some(name.clone()),
            _ => None,
        }
    }

    pub(super) fn canonical_hero_locator(&self, seat: u8) -> Option<String> {
        let state = locked(&self.state);
        let (entity, _) = state.canonical.hero_at_seat(seat)?;
        match &state
            .canonical
            .entries
            .get(&Target::Hero(entity))?
            .work
            .source
        {
            Source::Hero { locator, .. } => Some(locator.clone()),
            _ => None,
        }
    }

    pub(super) fn canonical_cast_binding<'a>(
        &self,
        heroes: &[u8],
        npcs: impl Iterator<Item = &'a str>,
    ) -> Option<String> {
        let state = locked(&self.state);
        let mut bindings = Vec::new();
        for seat in heroes {
            if let Some(entity) = state.canonical.seats.get(seat)
                && let Some(entry) = state.canonical.entries.get(&Target::Hero(*entity))
            {
                bindings.push(entry.work.binding.as_str());
            }
        }
        for id in npcs {
            if let Some(entry) = state.canonical.entries.get(&Target::Npc(id.to_owned())) {
                bindings.push(entry.work.binding.as_str());
            }
        }
        (!bindings.is_empty()).then(|| digest(bindings.join("|").as_bytes()))
    }

    async fn runtime_reference(&self, key: ImageKey) -> Option<ReferenceImage> {
        match self.imagery.image(key.as_str()).await {
            Lookup::Image { bytes, format } => Some(ReferenceImage::new(format, bytes)),
            Lookup::Unregistered | Lookup::NotReady => None,
        }
    }

    #[cfg(test)]
    pub(super) async fn canonical_room_reference(&self, place: &str) -> Option<ReferenceImage> {
        let target = locked(&self.state)
            .canonical
            .selected_rooms
            .get(place)
            .cloned()?;
        let Target::Room { revision, .. } = &target else {
            return None;
        };
        let reference = self.canonical_room_reference_at(place, revision).await?;
        (locked(&self.state).canonical.selected_rooms.get(place) == Some(&target))
            .then_some(reference)
    }

    pub(super) async fn canonical_hero_reference(
        &self,
        seat: u8,
    ) -> Option<(EntityId, ReferenceImage)> {
        let (entity, key) = {
            let state = locked(&self.state);
            let (entity, _) = state.canonical.hero_at_seat(seat)?;
            (entity, state.canonical.hero(entity)?)
        };
        let reference = self.runtime_reference(key.clone()).await?;
        (locked(&self.state).canonical.hero(entity).as_ref() == Some(&key))
            .then_some((entity, reference))
    }

    pub(super) async fn canonical_npc_reference(&self, id: &str) -> Option<ReferenceImage> {
        let key = locked(&self.state)
            .canonical
            .ready(&Target::Npc(id.to_owned()))?;
        let reference = self.runtime_reference(key.clone()).await?;
        (locked(&self.state).canonical.npc(id).as_ref() == Some(&key)).then_some(reference)
    }
}

fn room_architecture_context(place: &str) -> Option<imagery::RoomArchitectureContext> {
    if let Some(world) = crate::gameplay::adventure::free_roam::world() {
        return Some(imagery::RoomArchitectureContext {
            name: world.city.name,
            character: world.city.character,
        });
    }
    pack::with_content(|content| {
        content
            .scenes
            .iter()
            .filter_map(|scene| scene.setting.as_ref())
            .find(|setting| setting.place_id == place)
            .map(|setting| imagery::RoomArchitectureContext {
                name: setting.name.clone(),
                character: setting.short.clone(),
            })
    })
}

pub(super) fn room_revision(model: &SceneModel) -> Option<String> {
    let context = if crate::gameplay::adventure::free_roam::is_active() {
        room_architecture_context("")
    } else {
        pack::with_content(|content| {
            content
                .scenes
                .iter()
                .filter_map(|scene| scene.setting.as_ref())
                .find(|setting| setting.model.as_ref() == Some(model))
                .map(|setting| imagery::RoomArchitectureContext {
                    name: setting.name.clone(),
                    character: setting.short.clone(),
                })
        })
    };
    room_revision_with_context(model, context.as_ref())
}

fn room_revision_with_context(
    model: &SceneModel,
    context: Option<&imagery::RoomArchitectureContext>,
) -> Option<String> {
    let mut physical;
    let model = if crate::gameplay::adventure::free_roam::is_active() {
        physical = model.clone();
        crate::gameplay::place::physical_way_names(&mut physical);
        &physical
    } else {
        model
    };
    let (layout, goods, context) = imagery::canonical_room_content(model, context).ok()?;
    Some(digest(
        serde_json::json!({"layout": layout, "goods": goods, "context": context})
            .to_string()
            .as_bytes(),
    ))
}

/// A reviewed scene photograph is an authored place, never a generic style carrier.
pub(super) fn authored_place_photo(asset: &str) -> Option<&str> {
    let stem = art_contract::stem_of(asset);
    (stem.starts_with("scene-") && art_contract::REVIEWED_PHOTOS.contains(&stem)).then_some(asset)
}

fn documentary_photo_stem(asset: &str) -> Option<String> {
    let filename = asset.rsplit('/').next()?;
    let stem = filename
        .strip_suffix(".webp")
        .or_else(|| filename.strip_suffix(".png"))
        .unwrap_or(filename);
    Some(stem.strip_suffix("-documentary-photo")?.to_owned())
}

impl References {
    pub(super) fn observe_room(&mut self, scene: &pack::Scene) {
        let Some(setting) = scene.setting.as_ref() else {
            return;
        };
        let Some(model) = setting.model.as_ref() else {
            return;
        };
        self.admit_room_with_asset(
            &setting.place_id,
            model,
            &scene.cast,
            true,
            Some(&scene.asset),
            None,
        );
    }

    #[cfg(test)]
    fn admit_room(
        &mut self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        select: bool,
    ) {
        self.admit_room_with_asset(place, model, cast, select, None, None);
    }

    fn admit_room_with_asset(
        &mut self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        select: bool,
        asset: Option<&str>,
        known_revision: Option<&str>,
    ) {
        self.admit_room_with_asset_and_context(
            place,
            model,
            cast,
            select,
            (asset, known_revision, room_architecture_context(place)),
        );
    }

    fn admit_room_with_asset_and_context(
        &mut self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        select: bool,
        resources: RoomResources<'_>,
    ) {
        let (asset, known_revision, context) = resources;
        if self.scope.is_empty() {
            return;
        }
        let canon = self.committed_canon(Some(place), None);
        let Some(base_revision) = known_revision
            .map(str::to_owned)
            .or_else(|| room_revision_with_context(model, context.as_ref()))
        else {
            return;
        };
        let revision = if canon.is_empty() {
            base_revision.clone()
        } else {
            digest(
                serde_json::json!({"layout_revision": base_revision, "canon": canon})
                    .to_string()
                    .as_bytes(),
            )
        };
        let previous = self
            .room_revisions
            .insert((place.to_owned(), base_revision), revision.clone());
        let target = Target::Room {
            place: place.to_owned(),
            revision: revision.clone(),
        };
        if previous.as_ref().is_some_and(|previous| {
            self.selected_rooms.get(place)
                == Some(&Target::Room {
                    place: place.to_owned(),
                    revision: previous.clone(),
                })
        }) {
            if self.current_room == self.selected_rooms.get(place).cloned() {
                self.current_room = Some(target.clone());
            }
            self.selected_rooms.insert(place.to_owned(), target.clone());
        }
        // A prepared destination must be discoverable by the photographic world source before
        // it becomes current. Lookahead cannot replace an already selected room revision.
        self.selected_rooms
            .entry(place.to_owned())
            .or_insert_with(|| target.clone());
        if select {
            self.selected_rooms.insert(place.to_owned(), target.clone());
            self.current_room = Some(target.clone());
            self.current_npcs = model
                .people
                .iter()
                .filter(|person| person.role != "hero")
                .map(|person| person.id.clone())
                .collect();
        }
        if let Some(asset) = asset
            .and_then(authored_place_photo)
            .filter(|_| canon.is_empty())
        {
            // The owned worker loads reviewed bytes locally. Existing key-based consumers
            // receive the same photograph, and superseded generated completions are fenced.
            self.authored_rooms.insert(target.clone(), asset.to_owned());
            self.select(
                target,
                Source::ReviewedRoom {
                    asset: asset.to_owned(),
                },
                &format!("reviewed|{asset}|{revision}"),
            );
        } else if !self.authored_rooms.contains_key(&target) {
            let mut physical = model.clone();
            if crate::gameplay::adventure::free_roam::is_active() {
                crate::gameplay::place::physical_way_names(&mut physical);
            }
            self.select(
                target,
                Source::Room {
                    model: physical,
                    place: place.to_owned(),
                    context,
                    canon,
                },
                &revision,
            );
        }
        for person in model.people.iter().filter(|person| person.role != "hero") {
            // Lookahead may prepare a new person but cannot redefine an identity already
            // selected by the current accepted room merely because that person is doing
            // something different in a successor's authored description.
            if !select && self.entries.contains_key(&Target::Npc(person.id.clone())) {
                continue;
            }
            let matching_cast = cast
                .iter()
                .filter(|member| member.id == person.id)
                .collect::<Vec<_>>();
            let (identity_photo, identity_figure_photo) = match matching_cast.as_slice() {
                [member] => (
                    documentary_photo_stem(&member.portrait),
                    documentary_photo_stem(&member.sprite),
                ),
                _ => (None, None),
            };
            // One reviewed photograph that serves as both portrait and figure is a single
            // reference, not two copies of the same image.
            let identity_photo =
                identity_photo.filter(|photo| Some(photo) != identity_figure_photo.as_ref());
            let (gender, age) = match matching_cast.as_slice() {
                [member] => (member.gender, member.age),
                _ => (None, None),
            };
            let canon = self.committed_canon(None, Some(&person.id));
            let mut revision_content = serde_json::json!({
                "id": person.id,
                "name": person.name,
                "role": person.role,
                "description": person.description,
                "regen": self.regen.get(&person.id),
                "identity_photo": identity_photo,
                "identity_figure_photo": identity_figure_photo,
            });
            if !canon.is_empty() {
                revision_content["canon"] = serde_json::json!(canon);
            }
            if let Some(gender) = gender {
                revision_content["gender"] = serde_json::json!(gender);
            }
            if let Some(age) = age {
                revision_content["age"] = serde_json::json!(age);
            }
            let revision = digest(revision_content.to_string().as_bytes());
            let selected = self.select(
                Target::Npc(person.id.clone()),
                Source::Npc {
                    person: person.clone(),
                    gender,
                    age,
                    identity_photo,
                    identity_figure_photo,
                    canon,
                },
                &revision,
            );
            if selected
                && (!matches!(
                    gender,
                    Some(pack::Gender::He | pack::Gender::She | pack::Gender::They)
                ) || age.is_none())
            {
                super::super::actor::record_audit_line(
                    "gameplay.imagery",
                    format!("IMAGERY presentation_missing person={}", person.id),
                );
            }
        }
    }
}

impl LivingScenes {
    pub(in crate::gameplay) fn canonical_room_ready(&self, place: &str) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.canonical_room_status(place) == ReferenceStatus::Ready)
    }

    /// Admit the speculative destination through the same canonical worker as a visited room.
    /// This prepares visual resources only; no checkpoint or party whereabouts are changed.
    pub(in crate::gameplay) fn prepare_arena_photograph(
        &self,
        destination: &crate::gameplay::place::PlaceRecord,
    ) -> Result<(), crate::gameplay::place::PlaceRefusal> {
        let Some(inner) = &self.inner else {
            return Ok(());
        };
        let expanded = destination.expand()?;
        inner.admit_canonical_room(
            &destination.place_id,
            &expanded.model,
            &expanded.cast,
            None,
            None,
        );
        if inner.canonical_room_status(&destination.place_id) == ReferenceStatus::Ready {
            super::super::fight_foresight::request_prepared_world(&destination.place_id);
        }
        inner.wake.send_modify(|count| *count += 1);
        Ok(())
    }
}

#[cfg(test)]
#[path = "canonical_tests.rs"]
mod tests;

#[cfg(test)]
mod presentation_diagnostics_tests {
    use super::*;

    fn warning_count(person: &str) -> usize {
        crate::gameplay::actor::audit_lines_containing(&format!(
            "IMAGERY presentation_missing person={person} "
        ))
        .len()
    }

    #[test]
    fn missing_cast_presentation_is_reported_once_per_selected_revision() {
        let mut references = References {
            scope: "plate-serve-cast".to_owned(),
            ..Default::default()
        };
        let mut member = Member {
            id: "plate-serve-missing-cast".to_owned(),
            role: CharacterRole::Contact,
            life_stage: LifeStage::Adult,
            gender: None,
            age: None,
            name: "Oren".to_owned(),
            sprite: String::new(),
            dossier_id: None,
            opposes: false,
            starting_attitude: None,
        };
        for _ in 0..20 {
            references.admit_cast(&member);
        }
        assert_eq!(warning_count(&member.id), 1);
        member.age = Some(pack::Age::Adult);
        references.admit_cast(&member);
        references.admit_cast(&member);
        assert_eq!(warning_count(&member.id), 2);
        member.gender = Some(pack::Gender::They);
        references.admit_cast(&member);
        assert_eq!(warning_count(&member.id), 2);
        member.age = None;
        references.admit_cast(&member);
        assert_eq!(warning_count(&member.id), 3);
    }

    #[test]
    fn missing_scene_person_presentation_is_reported_once_per_selected_revision() {
        use crate::gameplay::adventure::scene_model::StartingAttitude;

        let mut references = References {
            scope: "plate-serve-room".to_owned(),
            ..Default::default()
        };
        let mut model = SceneModel {
            services: Vec::new(),
            witnessed_thefts: Vec::new(),
            inventory_version: 1,
            time_of_day: "morning".to_owned(),
            weather: "clear".to_owned(),
            people: vec![ScenePerson {
                id: "plate-serve-missing-person".to_owned(),
                name: "Oren".to_owned(),
                role: "merchant".to_owned(),
                description: "A merchant at the counter".to_owned(),
                relation_to_speaker: String::new(),
                position: "at the counter".to_owned(),
                aliases: Vec::new(),
                is_background: false,
                present_in: Vec::new(),
                attitude: StartingAttitude::Friendly,
                attitudes: Default::default(),
                knowledge: Vec::new(),
                wants: String::new(),
                persona: None,
                coin_cp: 0,
                passive_perception: 10,
            }],
            things: Vec::new(),
            ways_out: Vec::new(),
            offers: Vec::new(),
        };
        for _ in 0..20 {
            references.admit_room_with_asset_and_context(
                "plate-serve-place",
                &model,
                &[],
                true,
                (None, Some("layout"), None),
            );
        }
        assert_eq!(warning_count(&model.people[0].id), 1);
        model.people[0].description.push_str(" with a blue cap");
        references.admit_room_with_asset_and_context(
            "plate-serve-place",
            &model,
            &[],
            true,
            (None, Some("layout"), None),
        );
        assert_eq!(warning_count(&model.people[0].id), 2);
    }
}
