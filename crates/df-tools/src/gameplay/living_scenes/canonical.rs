//! Accepted identities feed the existing run-owned image worker. Results are selected only
//! when their complete scope and revision still match the accepted source.
use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneModel, ScenePerson},
    hero_identity::ValidatedIdentity,
    imagery::{self, HeroVisualIdentity, Lookup},
};
use df_model::checkpoint::{Checkpoint, EntityId};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Target {
    Hero(EntityId),
    Npc(String),
    Room(String),
}

#[derive(Clone, Debug)]
enum Source {
    Hero {
        identity: ValidatedIdentity,
        species: String,
        gear: Vec<&'static str>,
    },
    Npc(ScenePerson),
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

    pub(super) fn take_jobs(&mut self) -> Vec<Job> {
        self.entries
            .values_mut()
            .filter_map(|entry| {
                if !matches!(entry.status, Status::Queued) {
                    return None;
                }
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
        self.ready(&Target::Room(place.to_owned()))
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
        references.select(
            Target::Room("same-place".to_owned()),
            Source::Room {
                model: model(),
                place: "same-place".to_owned(),
            },
            revision,
        );
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
        let first_binding = first.entries.values().next().unwrap().work.binding.clone();
        assert_eq!(first.take_jobs().len(), 1);
        room(&mut first, "layout-a");
        assert!(first.take_jobs().is_empty());
        room(&mut first, "layout-b");
        assert_ne!(
            first.entries.values().next().unwrap().work.binding,
            first_binding
        );
        let mut second = references("session-a/run-b/pack-a");
        room(&mut second, "layout-a");
        assert_ne!(
            second.entries.values().next().unwrap().work.binding,
            first_binding
        );
        second.scope = "session-a/run-a/pack-b".to_owned();
        room(&mut second, "layout-a");
        assert_ne!(
            second.entries.values().next().unwrap().work.binding,
            first_binding
        );
        let source = Source::Hero {
            identity: Identity::from_menus(&[]).unwrap(),
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
    fn stale_completion_cannot_replace_new_layout_and_failure_is_terminal_for_the_binding() {
        let mut canonical = references("run-a");
        room(&mut canonical, "layout-a");
        let Job::Canonical(old) = canonical.take_jobs().remove(0) else {
            panic!("canonical job");
        };
        room(&mut canonical, "layout-b");
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
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
        let loaded = inner.canonical_room_reference("same-place").await.unwrap();
        let Lookup::Image { bytes, format } = fixture.imagery.image(key.as_str()).await else {
            panic!("runtime bytes");
        };
        assert_eq!(loaded.sha256, ReferenceImage::new(format, bytes).sha256);
        assert_eq!(inner.render_canonical(&work).await.unwrap(), key);
        assert_eq!(fixture.calls(), 1, "cache hit must not pay twice");
    }
}

fn diagnostic(work: &Work, status: &str, key: Option<&ImageKey>) {
    let target = match &work.target {
        Target::Hero(entity) => format!("hero:{}", digest(entity.as_bytes())),
        Target::Npc(id) => format!("npc:{id}"),
        Target::Room(id) => format!("room:{id}"),
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
            let (Ok(identity), Some(species), Some(gear)) = (identity, species, gear) else {
                record(
                    "gameplay.canonical_reference.invalid_identity",
                    &[("reference.entity", digest(entity.as_bytes()))],
                );
                continue;
            };
            let revision = format!(
                "{:?}|{:?}",
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
        if let Some(pack) = pack::active() {
            for scene in &pack.pack.scenes {
                state.canonical.observe_room(scene);
            }
        }
    }

    pub(super) async fn render_canonical(&self, work: &Work) -> Result<ImageKey, ImageFailure> {
        let carrier = self.neutral_photographic_reference().await?;
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
                    &work.binding,
                )?,
                OutputSpec::CUTOUT,
            ),
            Source::Npc(person) => (
                ImageKind::CanonicalCharacter,
                imagery::canonical_npc(self.rating, person, &work.binding)?,
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
                references: vec![carrier],
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
        locked(&self.state)
            .canonical
            .status(&Target::Room(place.to_owned()))
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

impl References {
    pub(super) fn observe_room(&mut self, scene: &pack::Scene) {
        if self.scope.is_empty() {
            return;
        }
        let Some(setting) = scene.setting.as_ref() else {
            return;
        };
        let Some(model) = setting.model.as_ref() else {
            return;
        };
        // This is the accepted authored model, not a mutable presentation snapshot. Dialogue,
        // holder changes and reconnect therefore cannot regenerate the immutable room pack.
        let Ok(layout) = model.picture_state() else {
            return;
        };
        let revision = serde_json::json!({"layout": layout, "goods": model.offers.iter().map(|offer| (&offer.id, &offer.description)).collect::<Vec<_>>()});
        self.select(
            Target::Room(setting.place_id.clone()),
            Source::Room {
                model: model.clone(),
                place: setting.place_id.clone(),
            },
            &digest(revision.to_string().as_bytes()),
        );
        for person in &model.people {
            let revision = digest(
                serde_json::json!({"id": person.id, "description": person.description})
                    .to_string()
                    .as_bytes(),
            );
            self.select(
                Target::Npc(person.id.clone()),
                Source::Npc(person.clone()),
                &revision,
            );
        }
    }
}
