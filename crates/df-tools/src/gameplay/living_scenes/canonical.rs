//! Accepted identities feed the existing run-owned image worker. Results are selected only
//! when their complete scope and revision still match the accepted source.
use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneModel, ScenePerson},
    hero_identity::{AcceptedVisualDossier, ValidatedIdentity},
    imagery::{self, HeroVisualIdentity, Lookup},
};
use df_model::checkpoint::{Checkpoint, EntityId};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Target {
    Hero(EntityId),
    Npc(String),
    Room { place: String, revision: String },
}

#[derive(Clone, Debug)]
enum Source {
    Hero {
        identity: ValidatedIdentity,
        visual_dossier: Option<&'static AcceptedVisualDossier>,
        species: String,
        gear: Vec<&'static str>,
    },
    Npc {
        person: ScenePerson,
        identity_photo: Option<String>,
    },
    Room {
        model: SceneModel,
        place: String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Work {
    target: Target,
    scope: String,
    binding: String,
    source: Source,
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
}

#[derive(Default)]
pub(super) struct References {
    scope: String,
    entries: BTreeMap<Target, Entry>,
    seats: BTreeMap<u8, EntityId>,
    selected_rooms: BTreeMap<String, Target>,
    current_room: Option<Target>,
    current_npcs: BTreeSet<String>,
}

pub(super) struct WorkerLifetime(pub(super) Arc<Inner>);

impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        for entry in locked(&self.0.state).canonical.entries.values_mut() {
            if matches!(entry.status, Status::Queued | Status::Generating) {
                diagnostic(&entry.work, "cancelled", None);
                entry.status = Status::Failed(ImageFailure::Interrupted);
            }
        }
    }
}

/// A failed reference remains terminal for this binding; new authoring creates a new binding.
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
    fn select(&mut self, target: Target, source: Source, revision: &str) {
        let binding = digest(
            format!(
                "canonical-reference-v1|{}|{target:?}|{revision}",
                self.scope
            )
            .as_bytes(),
        );
        if self
            .entries
            .get(&target)
            .is_some_and(|entry| entry.work.binding == binding)
        {
            return;
        }
        let work = Work {
            target: target.clone(),
            scope: self.scope.clone(),
            binding,
            source,
        };
        diagnostic(&work, "queued", None);
        self.entries.insert(
            target,
            Entry {
                work,
                status: Status::Queued,
            },
        );
    }

    #[cfg(test)]
    pub(super) fn take_jobs(&mut self) -> Vec<Job> {
        self.take_jobs_up_to(imagery::MAXIMUM_IN_FLIGHT)
    }

    pub(super) fn take_jobs_up_to(&mut self, available: usize) -> Vec<Job> {
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
                diagnostic(&entry.work, "requested", None);
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

    pub(super) fn hero(&self, entity: EntityId) -> Option<ImageKey> {
        self.ready(&Target::Hero(entity))
    }

    pub(super) fn hero_at_seat(&self, seat: u8) -> Option<(EntityId, ReferenceStatus)> {
        let entity = *self.seats.get(&seat)?;
        Some((entity, self.status(&Target::Hero(entity))))
    }

    fn status(&self, target: &Target) -> ReferenceStatus {
        match self.entries.get(target).map(|entry| &entry.status) {
            None => ReferenceStatus::Missing,
            Some(Status::Queued | Status::Generating) => ReferenceStatus::Pending,
            Some(Status::Ready(_)) => ReferenceStatus::Ready,
            Some(Status::Failed(failure)) => ReferenceStatus::Failed(*failure),
        }
    }

    pub(super) fn room(&self, place: &str) -> Option<ImageKey> {
        self.ready(self.selected_rooms.get(place)?)
    }

    fn room_at(&self, place: &str, revision: &str) -> Option<ImageKey> {
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
mod tests {
    use super::*;
    use crate::gameplay::{
        hero_identity::Identity, imagery::tests::DirectionFixture,
        startup_configuration::ImageryOff,
    };
    use std::path::PathBuf;

    fn model() -> SceneModel {
        SceneModel {
            inventory_version: 1,
            time_of_day: "morning".to_owned(),
            weather: "clear".to_owned(),
            people: Vec::new(),
            things: Vec::new(),
            ways_out: Vec::new(),
            offers: Vec::new(),
        }
    }

    fn references(scope: &str) -> References {
        References {
            scope: scope.to_owned(),
            ..Default::default()
        }
    }

    fn room(references: &mut References, revision: &str) {
        let mut authored = model();
        authored.weather = revision.to_owned();
        references.admit_room("same-place", &authored, &[], true);
    }

    fn selected_room_binding(references: &References) -> &str {
        let target = references.selected_rooms.get("same-place").unwrap();
        &references.entries.get(target).unwrap().work.binding
    }

    fn inner(imagery: Imagery, canonical: References) -> Inner {
        let timing = Timing::from_interval(Duration::from_secs(30));
        let mut state = State::new(timing);
        state.canonical = canonical;
        Inner {
            imagery,
            references: ReferenceStore::photographic(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
            ),
            social: SocialLedger::none(),
            rating: ContentRating::Restrained,
            timing,
            state: Mutex::new(state),
            wake: watch::channel(0).0,
            origin: Instant::now(),
        }
    }

    #[test]
    fn run_pack_layout_and_entity_identity_are_distinct_and_unrelated_observations_dedupe() {
        let mut first = references("session-a/run-a/pack-a");
        room(&mut first, "layout-a");
        let first_binding = selected_room_binding(&first).to_owned();
        assert_eq!(first.take_jobs().len(), 1);
        room(&mut first, "layout-a");
        assert!(first.take_jobs().is_empty());
        room(&mut first, "layout-b");
        assert_ne!(selected_room_binding(&first), first_binding);
        let mut second = references("session-a/run-b/pack-a");
        room(&mut second, "layout-a");
        assert_ne!(selected_room_binding(&second), first_binding);
        second.scope = "session-a/run-a/pack-b".to_owned();
        room(&mut second, "layout-a");
        assert_ne!(selected_room_binding(&second), first_binding);
        let source = Source::Hero {
            identity: Identity::from_menus(&[]).unwrap(),
            visual_dossier: None,
            species: "orc".to_owned(),
            gear: vec!["a greatsword"],
        };
        first.select(
            Target::Hero(EntityId::from_bytes(&[1; 16]).unwrap()),
            source.clone(),
            "same-name/same-appearance",
        );
        first.select(
            Target::Hero(EntityId::from_bytes(&[2; 16]).unwrap()),
            source,
            "same-name/same-appearance",
        );
        let bindings = first
            .entries
            .iter()
            .filter(|(target, _)| matches!(target, Target::Hero(_)))
            .map(|(_, entry)| entry.work.binding.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            bindings.len(),
            2,
            "same display name must never merge durable entities"
        );
    }

    #[test]
    fn stale_run_completion_cannot_replace_current_room_and_failure_is_terminal_for_the_binding() {
        let mut canonical = references("run-a");
        room(&mut canonical, "layout-a");
        let Job::Canonical(old) = canonical.take_jobs().remove(0) else {
            panic!("canonical job");
        };
        let mut replacement = references("run-b");
        room(&mut replacement, "layout-b");
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), replacement);
        inner.finish_canonical(old, Ok(ImageKey::parse(&"a".repeat(64)).unwrap()));
        assert_eq!(
            inner.canonical_room_status("same-place"),
            ReferenceStatus::Pending
        );
        assert!(inner.canonical_room_key("same-place").is_none());
        let Job::Canonical(current) = locked(&inner.state).canonical.take_jobs().remove(0) else {
            panic!("canonical job");
        };
        inner.finish_canonical(current, Err(ImageFailure::Provider));
        assert_eq!(
            inner.canonical_room_status("same-place"),
            ReferenceStatus::Failed(ImageFailure::Provider)
        );
        room(&mut locked(&inner.state).canonical, "layout-b");
        assert!(
            locked(&inner.state).canonical.take_jobs().is_empty(),
            "ordinary updates cannot retry paid failures"
        );
    }

    #[tokio::test]
    async fn owned_completion_exposes_runtime_bytes_and_identical_recovery_uses_the_cache() {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/canonical-reference-tests/runtime-{}",
            std::process::id()
        ));
        let fixture = DirectionFixture::new(&cache);
        let mut canonical = references("runtime-test-run");
        room(&mut canonical, "accepted-layout");
        let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
            panic!("canonical job");
        };
        let inner = Arc::new(inner(fixture.imagery.clone(), canonical));
        let mut tasks = JoinSet::new();
        inner.spawn(&mut tasks, Job::Canonical(work.clone()));
        inner.finish(tasks.join_next().await.unwrap().unwrap());
        let key = inner.canonical_room_key("same-place").unwrap();
        for _ in 0..3 {
            room(&mut locked(&inner.state).canonical, "accepted-layout");
            assert!(
                locked(&inner.state).canonical.take_jobs().is_empty(),
                "reobserving accepted authoring preserves ready work"
            );
            assert_eq!(inner.canonical_room_key("same-place"), Some(key.clone()));
        }
        let loaded = inner.canonical_room_reference("same-place").await.unwrap();
        let Lookup::Image { bytes, format } = fixture.imagery.image(key.as_str()).await else {
            panic!("runtime bytes");
        };
        assert_eq!(loaded.sha256, ReferenceImage::new(format, bytes).sha256);
        assert_eq!(inner.render_canonical(&work).await.unwrap(), key);
        assert_eq!(fixture.calls(), 1, "cache hit must not pay twice");
    }

    #[test]
    fn same_place_authored_layouts_coexist_and_select_the_exact_current_room() {
        let mut canonical = references("same-run-pack");
        let morning = model();
        let mut evening = morning.clone();
        evening.time_of_day = "evening".to_owned();
        let morning_revision = room_revision(&morning).unwrap();
        let evening_revision = room_revision(&evening).unwrap();
        canonical.admit_room("shared-place", &morning, &[], true);
        canonical.admit_room("shared-place", &evening, &[], true);
        assert_eq!(canonical.entries.len(), 2);
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
        let work = locked(&inner.state).canonical.take_jobs();
        assert_eq!(work.len(), imagery::MAXIMUM_IN_FLIGHT);
        for (index, job) in work.into_iter().enumerate() {
            let Job::Canonical(work) = job else {
                panic!("canonical work");
            };
            inner.finish_canonical(
                work,
                Ok(ImageKey::parse(&format!("{:064x}", index + 1)).unwrap()),
            );
        }
        let state = locked(&inner.state);
        let first = state
            .canonical
            .room_at("shared-place", &morning_revision)
            .unwrap();
        let second = state
            .canonical
            .room_at("shared-place", &evening_revision)
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(state.canonical.room("shared-place"), Some(second));
        drop(state);
        locked(&inner.state)
            .canonical
            .admit_room("shared-place", &morning, &[], true);
        assert_eq!(inner.canonical_room_key("shared-place"), Some(first));
        assert!(
            locked(&inner.state).canonical.take_jobs().is_empty(),
            "returning to accepted room reuses its pack"
        );
    }

    #[test]
    fn room_reference_revision_ignores_cast_absent_from_the_room_prompt() {
        let original = model();
        let mut changed_cast = original.clone();
        changed_cast.people.push(ScenePerson {
            id: "merchant".to_owned(),
            name: "Marek".to_owned(),
            role: "cloth merchant".to_owned(),
            description: "Marek folds blue linen behind the counter.".to_owned(),
            relation_to_speaker: "shopkeeper".to_owned(),
            position: "behind the wooden counter".to_owned(),
            aliases: vec!["Marek".to_owned()],
            is_background: false,
            present_in: vec!["room".to_owned()],
            attitude: crate::gameplay::adventure::scene_model::StartingAttitude::Friendly,
            attitudes: Default::default(),
            knowledge: Vec::new(),
            wants: "Sell suitable clothing at a fair price.".to_owned(),
            coin_cp: 0,
        });

        assert_eq!(
            room_revision(&original),
            room_revision(&changed_cast),
            "cast changes belong to character references; the canonical room is empty"
        );
    }

    #[test]
    fn actual_room_admission_preserves_terminal_provider_outcomes_on_reobservation() {
        for failure in [
            ImageFailure::Provider,
            ImageFailure::Refused,
            ImageFailure::BudgetExhausted,
            ImageFailure::Interrupted,
        ] {
            let mut canonical = references("terminal-admission-run");
            room(&mut canonical, "unchanged-authored-layout");
            let binding = selected_room_binding(&canonical).to_owned();
            let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
                panic!("canonical work");
            };
            let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
            inner.finish_canonical(work, Err(failure));
            for _ in 0..3 {
                room(
                    &mut locked(&inner.state).canonical,
                    "unchanged-authored-layout",
                );
                assert_eq!(
                    inner.canonical_room_status("same-place"),
                    ReferenceStatus::Failed(failure)
                );
                assert_eq!(
                    selected_room_binding(&locked(&inner.state).canonical),
                    binding
                );
                assert!(
                    locked(&inner.state).canonical.take_jobs().is_empty(),
                    "accepted unchanged authoring cannot redispatch a terminal paid job"
                );
            }
        }
    }

    #[test]
    fn canonical_dispatch_obeys_the_existing_media_capacity_and_cancellation_is_terminal() {
        let mut canonical = references("capacity-run");
        for place in ["one", "two", "three"] {
            canonical.admit_room(place, &model(), &[], true);
        }
        assert_eq!(canonical.take_jobs_up_to(1).len(), 1);
        assert!(canonical.take_jobs_up_to(0).is_empty());
        let inner = Arc::new(inner(
            Imagery::disabled(ImageryOff::Qualification),
            canonical,
        ));
        drop(WorkerLifetime(inner.clone()));
        assert_eq!(
            inner.canonical_room_status("one"),
            ReferenceStatus::Failed(ImageFailure::Interrupted)
        );
        assert!(locked(&inner.state).canonical.take_jobs().is_empty());
    }
}

fn diagnostic(work: &Work, status: &str, key: Option<&ImageKey>) {
    let target = match &work.target {
        Target::Hero(entity) => format!("hero:{}", digest(entity.as_bytes())),
        Target::Npc(id) => format!("npc:{id}"),
        Target::Room { place, revision } => format!("room:{place}:{revision}"),
    };
    let key = key.map_or("", ImageKey::as_str);
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
            state.canonical = References {
                scope,
                ..Default::default()
            };
        }
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
            let visual_dossier = ValidatedIdentity::accepted_visual_dossier_from_choices(&choices);
            let species = choices
                .iter()
                .find(|(field, _)| field == "species")
                .map(|(_, value)| value.clone());
            let archer = choices
                .iter()
                .any(|(field, value)| field == "build-package" && value == "archer");
            let weapon = if archer {
                "longbow".to_owned()
            } else {
                pack::hero_weapon_item()
            };
            let gear = match weapon.as_str() {
                "greatsword" => Some(vec!["a greatsword"]),
                "longbow" => Some(vec!["a longbow and quiver of arrows"]),
                "flail" => Some(vec!["a flail"]),
                "javelin" => Some(vec!["a javelin"]),
                _ => None,
            };
            let (Ok(identity), Ok(visual_dossier), Some(species), Some(gear)) =
                (identity, visual_dossier, species, gear)
            else {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            };
            let portrait = choices
                .iter()
                .find(|(field, _)| field == "portrait")
                .map(|(_, value)| value.as_str());
            if visual_dossier.is_some_and(|dossier| {
                !portrait.is_some_and(|portrait| dossier.agrees_with_portrait(portrait))
                    || !dossier.agrees_with_identity(&identity)
            }) {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            }
            let revision = format!(
                "{:?}|{:?}|{species}|{gear:?}",
                checkpoint
                    .state()
                    .entities
                    .iter()
                    .find(|world| world.id == entity)
                    .map(|world| &world.identity_revision),
                choices
            );
            let target = Target::Hero(entity);
            accepted.insert(target.clone());
            state.canonical.select(
                target,
                Source::Hero {
                    identity,
                    visual_dossier,
                    species,
                    gear,
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
    }

    pub(super) async fn render_canonical(&self, work: &Work) -> Result<ImageKey, ImageFailure> {
        let mut references = vec![self.neutral_photographic_reference().await?];
        match &work.source {
            Source::Hero {
                visual_dossier: Some(dossier),
                ..
            } => references.push(
                self.load_photographic(dossier.figure_reference.to_owned())
                    .await?,
            ),
            Source::Npc {
                identity_photo: Some(photo),
                ..
            } => references.push(self.load_photographic(photo.clone()).await?),
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
            Source::Hero {
                identity,
                visual_dossier,
                species,
                gear,
            } => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_character(
                    self.rating,
                    HeroVisualIdentity {
                        identity,
                        species,
                        held_gear: gear,
                    },
                    visual_dossier.map(|dossier| dossier.description),
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
            Source::Npc {
                person,
                identity_photo,
            } => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_npc(
                    self.rating,
                    person,
                    identity_photo.is_some(),
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
            Source::Room { model, place } => (
                ImageKind::CanonicalRoom,
                imagery::canonical_room(self.rating, model, place, &work.binding)?,
                OutputSpec::BACKDROP,
            ),
        };
        self.imagery
            .render(ImageRequest {
                kind,
                references,
                prompt,
                output,
            })
            .await
    }

    pub(super) fn finish_canonical(&self, work: Work, result: Result<ImageKey, ImageFailure>) {
        let mut state = locked(&self.state);
        let Some(entry) = state
            .canonical
            .entries
            .get_mut(&work.target)
            .filter(|entry| entry.work.binding == work.binding)
        else {
            diagnostic(&work, "stale_discarded", result.as_ref().ok());
            return;
        };
        diagnostic(
            &work,
            result
                .as_ref()
                .map_or_else(|failure| failure.label(), |_| "ready"),
            result.as_ref().ok(),
        );
        entry.status = match result {
            Ok(key) => Status::Ready(key),
            Err(failure) => Status::Failed(failure),
        };
        // The next watcher projection recomputes the graph from the exact ready identity.
        drop(state);
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
    ) {
        locked(&self.state)
            .canonical
            .admit_room(place, model, cast, false);
    }

    pub(super) fn canonical_room_status_at(&self, place: &str, revision: &str) -> ReferenceStatus {
        locked(&self.state).canonical.status(&Target::Room {
            place: place.to_owned(),
            revision: revision.to_owned(),
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

    pub(super) async fn canonical_room_reference(&self, place: &str) -> Option<ReferenceImage> {
        let key = self.canonical_room_key(place)?;
        let reference = self.runtime_reference(key.clone()).await?;
        (self.canonical_room_key(place).as_ref() == Some(&key)).then_some(reference)
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

pub(super) fn room_revision(model: &SceneModel) -> Option<String> {
    let (layout, goods) = imagery::canonical_room_content(model).ok()?;
    Some(digest(
        serde_json::json!({"layout": layout, "goods": goods})
            .to_string()
            .as_bytes(),
    ))
}

impl References {
    pub(super) fn observe_room(&mut self, scene: &pack::Scene) {
        let Some(setting) = scene.setting.as_ref() else {
            return;
        };
        let Some(model) = setting.model.as_ref() else {
            return;
        };
        self.admit_room(&setting.place_id, model, &scene.cast, true);
    }

    fn admit_room(
        &mut self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        select: bool,
    ) {
        if self.scope.is_empty() {
            return;
        }
        let Some(revision) = room_revision(model) else {
            return;
        };
        let target = Target::Room {
            place: place.to_owned(),
            revision: revision.clone(),
        };
        if select {
            self.selected_rooms.insert(place.to_owned(), target.clone());
            self.current_room = Some(target.clone());
            self.current_npcs = model
                .people
                .iter()
                .map(|person| person.id.clone())
                .collect();
        }
        self.select(
            target,
            Source::Room {
                model: model.clone(),
                place: place.to_owned(),
            },
            &revision,
        );
        for person in &model.people {
            // Lookahead may prepare a new person but cannot redefine an identity already
            // selected by the current accepted room merely because that person is doing
            // something different in a successor's authored description.
            if !select && self.entries.contains_key(&Target::Npc(person.id.clone())) {
                continue;
            }
            let identity_photo = cast
                .iter()
                .filter(|member| member.id == person.id)
                .map(|member| member.portrait.as_str())
                .collect::<Vec<_>>();
            let identity_photo = match identity_photo.as_slice() {
                [portrait] => portrait
                    .strip_suffix("-documentary-photo")
                    .map(str::to_owned),
                _ => None,
            };
            let revision = digest(
                serde_json::json!({
                    "id": person.id,
                    "name": person.name,
                    "role": person.role,
                    "description": person.description,
                    "identity_photo": identity_photo,
                })
                .to_string()
                .as_bytes(),
            );
            self.select(
                Target::Npc(person.id.clone()),
                Source::Npc {
                    person: person.clone(),
                    identity_photo,
                },
                &revision,
            );
        }
    }
}
