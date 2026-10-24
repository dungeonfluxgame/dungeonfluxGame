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

#[path = "mural.rs"]
mod mural;
#[path = "mural_view.rs"]
mod mural_view;

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

    fn select(&mut self, target: Target, source: Source, revision: &str) {
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
            return;
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
        if !matches!(
            member.gender,
            Some(pack::Gender::He | pack::Gender::She | pack::Gender::They)
        ) || member.age.is_none()
        {
            super::super::actor::record_audit_line(
                "gameplay.imagery",
                format!("IMAGERY presentation_missing person={}", member.id),
            );
        }
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
        self.select(
            target,
            Source::Cast {
                member: member.clone(),
                canon,
            },
            &revision,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        hero_identity::Identity, imagery::tests::DirectionFixture,
        startup_configuration::ImageryOff,
    };
    use std::path::PathBuf;

    fn fact_decision(
        place: &str,
        person: &str,
        kind: crate::gameplay::npc_reply::local_lore::FactKind,
        name: &str,
        content: &str,
    ) -> crate::gameplay::npc_reply::fact_backfill::FactDecision {
        use crate::gameplay::npc_reply::{
            fact_backfill::{Decision, FactDecision, Reason},
            local_lore::LocalFact,
        };
        FactDecision {
            scene: "market".to_owned(),
            place: place.to_owned(),
            npc_id: person.to_owned(),
            speaker: "Bram Weller".to_owned(),
            question_hash: "a".repeat(64),
            decision: Decision::Backfill,
            reason: Reason::HarmlessMissingDetail,
            fact: Some(LocalFact {
                kind,
                name: name.to_owned(),
                content: content.to_owned(),
                established_by: "Bram Weller".to_owned(),
            }),
            answer: None,
        }
    }

    #[tokio::test]
    async fn committed_room_canon_reaches_the_fake_and_changes_the_reference_once() {
        use crate::gameplay::npc_reply::{
            fact_backfill::{Decision, Reason},
            local_lore::{FactKind, LocalLore},
        };
        let lore = Arc::new(LocalLore::in_memory());
        let mut canonical = references("hs66-room-canon");
        canonical.lore = Some(lore.clone());
        room(&mut canonical, "layout");
        let original_binding = selected_room_binding(&canonical).to_owned();
        let jobs = canonical.take_jobs();
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS66/fake-room-canon")
            .join(std::process::id().to_string());
        let fixture = DirectionFixture::photographic(&cache);
        let inner = inner(fixture.imagery.clone(), canonical);
        for job in jobs {
            let Job::Canonical(work) = job else {
                panic!("canonical room")
            };
            let result = inner.render_canonical(&work).await;
            assert!(result.is_ok(), "{result:?}");
            inner.finish_canonical(work, result);
        }
        assert!(!fixture.prompt().contains("The Last Stitch"));
        let proposal = fact_decision(
            "same-place",
            "bram",
            FactKind::History,
            "Shop sign",
            "Bram Weller's shop is called The Last Stitch, as its sign says.",
        );
        // A model proposal that has not crossed commit never reaches the picture source.
        {
            let mut state = locked(&inner.state);
            room(&mut state.canonical, "layout");
            assert_eq!(selected_room_binding(&state.canonical), original_binding);
            assert!(state.canonical.take_jobs().is_empty());
        }
        let foreign = fact_decision(
            "other-place",
            "other-person",
            FactKind::History,
            "Other shop",
            "The other shop is called FOREIGN_SHOP.",
        );
        lore.commit_fact_decision(foreign.clone(), foreign.fact.clone())
            .unwrap();
        let mut blank = proposal.clone();
        blank.decision = Decision::LeaveBlank;
        blank.reason = Reason::Secret;
        blank.fact = None;
        lore.commit_fact_decision(blank, None).unwrap();
        {
            let mut state = locked(&inner.state);
            room(&mut state.canonical, "layout");
            assert_eq!(selected_room_binding(&state.canonical), original_binding);
            assert!(state.canonical.take_jobs().is_empty());
        }
        lore.commit_fact_decision(proposal.clone(), proposal.fact.clone())
            .unwrap();
        let base_revision = room_revision(&{
            let mut authored = model();
            authored.weather = "layout".to_owned();
            authored
        })
        .unwrap();
        let jobs = {
            let mut state = locked(&inner.state);
            room(&mut state.canonical, "layout");
            assert_ne!(selected_room_binding(&state.canonical), original_binding);
            let jobs = state.canonical.take_jobs();
            assert_eq!(jobs.len(), 1);
            jobs
        };
        for job in jobs {
            let Job::Canonical(work) = job else {
                panic!("canonical room")
            };
            let result = inner.render_canonical(&work).await;
            assert!(result.is_ok(), "{result:?}");
            inner.finish_canonical(work, result);
        }
        let prompt = fixture.prompt();
        assert!(
            prompt.contains("The Last Stitch, as its sign says."),
            "{prompt}"
        );
        assert!(prompt.contains("visible sign carries the recorded name exactly"));
        assert!(!prompt.contains("No signage and no writing"));
        assert!(!prompt.contains("FOREIGN_SHOP"));
        assert!(!prompt.contains("question_hash"));
        assert!(!prompt.contains("harmless_missing_detail"));
        assert!(!prompt.contains("protected_constraints"));
        assert_eq!(
            inner.canonical_room_status_at("same-place", &base_revision),
            ReferenceStatus::Ready
        );
        assert!(
            inner
                .canonical_room_reference_at("same-place", &base_revision)
                .await
                .is_some()
        );
        {
            let mut state = locked(&inner.state);
            room(&mut state.canonical, "layout");
            assert!(state.canonical.take_jobs().is_empty());
        }
        assert_eq!(fixture.calls(), 2);
    }

    #[test]
    fn person_canon_uses_the_exact_journal_owner_and_excludes_history() {
        use crate::gameplay::npc_reply::local_lore::{FactKind, LocalLore};
        let lore = Arc::new(LocalLore::in_memory());
        let mut canonical = references("hs66-person-canon");
        canonical.lore = Some(lore.clone());
        for decision in [
            fact_decision(
                "same-place",
                "bram",
                FactKind::Person,
                "Appearance",
                "Bram wears a blue wool cap.",
            ),
            fact_decision(
                "same-place",
                "tessa",
                FactKind::Person,
                "Other appearance",
                "Tessa wears a red scarf.",
            ),
            fact_decision(
                "same-place",
                "bram",
                FactKind::History,
                "History",
                "Bram once lived beside the river.",
            ),
        ] {
            lore.commit_fact_decision(decision.clone(), decision.fact.clone())
                .unwrap();
        }
        let canon = canonical.committed_canon(None, Some("bram"));
        assert_eq!(canon.len(), 1);
        assert_eq!(canon[0]["content"], "Bram wears a blue wool cap.");
        assert_eq!(canon[0]["person_id"], "bram");
        assert!(!serde_json::to_string(&canon).unwrap().contains("reason"));
    }

    #[test]
    fn cast_presentation_is_bound_and_missing_values_are_audited() {
        let mut canonical = references("hs66-cast-presentation");
        let mut member = Member {
            id: "hs66-presentation".to_owned(),
            role: CharacterRole::Contact,
            life_stage: LifeStage::Adult,
            gender: Some(pack::Gender::They),
            age: Some(pack::Age::Adult),
            name: "Oren Vale".to_owned(),
            sprite: String::new(),
            dossier_id: None,
            opposes: false,
            starting_attitude: None,
        };
        canonical.admit_cast(&member);
        let target = Target::Npc(member.id.clone());
        let first = canonical.entries[&target].work.binding.clone();
        member.gender = Some(pack::Gender::She);
        canonical.admit_cast(&member);
        assert_ne!(canonical.entries[&target].work.binding, first);
        let second = canonical.entries[&target].work.binding.clone();
        member.age = Some(pack::Age::Elder);
        canonical.admit_cast(&member);
        assert_ne!(canonical.entries[&target].work.binding, second);
        for (gender, age) in [
            (None, Some(pack::Age::Adult)),
            (Some(pack::Gender::They), None),
        ] {
            member.gender = gender;
            member.age = age;
            canonical.admit_cast(&member);
            assert!(
                !crate::gameplay::actor::audit_lines_containing(
                    "IMAGERY presentation_missing person=hs66-presentation"
                )
                .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn custom_seats_use_full_public_design_and_only_their_approved_references() {
        use hero_identity_fixture::{LOOKS, checkpoint, portrait_asset, portrait_bytes};
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS5/canonical-custom-{}",
            std::process::id()
        ));
        let fixture = DirectionFixture::photographic(&cache);
        let inner = inner(fixture.imagery.clone(), References::default());
        inner.observe_checkpoint(&checkpoint(true));
        let jobs = locked(&inner.state).canonical.take_jobs();
        assert_eq!(jobs.len(), 2, "one canonical image per accepted seat");
        let portrait_root = std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("artifacts/tmp/play/imagery-cache"));
        std::fs::create_dir_all(&portrait_root).unwrap();
        std::fs::write(
            portrait_root.join(format!("{}.jpg", portrait_asset().key.as_str())),
            portrait_bytes(),
        )
        .unwrap();
        for (index, job) in jobs.into_iter().enumerate() {
            let Job::Canonical(work) = job else {
                panic!("canonical work")
            };
            let Source::Hero {
                portrait,
                public_design,
                locator,
                ..
            } = &work.source
            else {
                panic!("hero source")
            };
            assert_eq!(portrait.is_some(), index == 0);
            assert!(public_design.contains(if index == 0 { "dragonborn" } else { "tiefling" }));
            assert!(locator.contains(if index == 0 { "Brakka" } else { "Sigrun" }));
            let key = inner.render_canonical(&work).await.unwrap();
            assert_eq!(fixture.image_count(), usize::from(index == 0));
            let prompt = fixture.prompt();
            let design: serde_json::Value = serde_json::from_str(public_design).unwrap();
            let appearance = design["public_appearance"]
                .as_str()
                .unwrap()
                .lines()
                .next()
                .unwrap();
            let appearance: serde_json::Value = serde_json::from_str(appearance).unwrap();
            assert_eq!(appearance["public_visual_details"], LOOKS[index]);
            assert!(prompt.contains(public_design));
            assert!(prompt.contains("actual_gear"));
            assert!(prompt.contains(if index == 0 { "greatsword" } else { "longbow" }));
            assert!(!prompt.contains("SEALED_STORY_NEVER_IMAGE"));
            assert!(!prompt.contains("PRIVATE_ORIGIN_NEVER_IMAGE"));
            assert!(!prompt.contains("SCENERY_NEVER_CANONICAL"));
            assert!(!prompt.contains("long braided beard with plain iron rings"));
            inner.finish_canonical(work, Ok(key));
        }
        assert_eq!(fixture.calls(), 2);
        inner.observe_checkpoint(&checkpoint(true));
        assert!(locked(&inner.state).canonical.take_jobs().is_empty());
        // Mutable inventory is not a new identity or permission to buy another canonical image.
        let accepted = checkpoint(true);
        let mut changed = accepted.state().clone();
        changed.inventory[0].quantity += 1;
        inner.observe_checkpoint(&hero_identity_fixture::recheckpoint(
            accepted.basis(),
            changed,
        ));
        assert!(locked(&inner.state).canonical.take_jobs().is_empty());
    }

    #[tokio::test]
    async fn legacy_quickstart_uses_only_its_written_look_without_a_stock_reference() {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS5/canonical-legacy-{}",
            std::process::id()
        ));
        let fixture = DirectionFixture::photographic(&cache);
        let inner = inner(fixture.imagery.clone(), References::default());
        let checkpoint = hero_identity_fixture::checkpoint(false);
        let mut saved = checkpoint.state().clone();
        for character in &mut saved.characters {
            character
                .choices
                .retain(|choice| !choice.offer.as_str().starts_with("identity-"));
        }
        let saved = crate::gameplay::model::checkpoint(checkpoint.basis(), saved).unwrap();
        inner.observe_checkpoint(&saved);
        let Job::Canonical(work) = locked(&inner.state).canonical.take_jobs().remove(0) else {
            panic!("canonical hero")
        };
        inner.render_canonical(&work).await.unwrap();
        assert_eq!(fixture.image_count(), 0);
        assert!(
            fixture
                .prompt()
                .contains("long braided beard with plain iron rings")
        );
    }

    #[test]
    fn accepted_personal_art_reaches_party_sheet_combat_and_live_faces_without_stock_fallbacks() {
        let inner = inner(
            Imagery::disabled(ImageryOff::Qualification),
            References::default(),
        );
        let committed = hero_identity_fixture::checkpoint(true);
        inner.observe_checkpoint(&committed);
        let party = committed
            .state()
            .characters
            .iter()
            .enumerate()
            .map(|(index, hero)| rpc::PartyMemberView {
                name: ["Brakka", "Sigrun"][index].to_owned(),
                participant_id: hero.entity.as_bytes().to_vec(),
                character_ready: true,
                side: 1,
                portrait: "portrait-hero-1".to_owned(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut journey = rpc::JourneyView {
            party: party.clone(),
            own_character: Some(rpc::CharacterSheet {
                name: "Sigrun".to_owned(),
                ..Default::default()
            }),
            combat: Some(rpc::CombatView {
                participants: party,
                ..Default::default()
            }),
            ..Default::default()
        };
        {
            let state = locked(&inner.state);
            apply_hero_faces(&state, Some(&mut journey));
        }
        assert_eq!(
            journey.party[0].portrait,
            format!("/gameplay/creator-portrait/{}", "4".repeat(64))
        );
        assert!(
            journey.party[1].portrait.is_empty(),
            "no photo without an approved or ready personal image"
        );
        let jobs = locked(&inner.state).canonical.take_jobs();
        for (index, job) in jobs.into_iter().enumerate() {
            let Job::Canonical(work) = job else {
                panic!("hero");
            };
            inner.finish_canonical(
                work,
                Ok(ImageKey::parse(&["a", "b"][index].repeat(64)).unwrap()),
            );
        }
        apply_hero_faces(&locked(&inner.state), Some(&mut journey));
        for (index, member) in journey.party.iter().enumerate() {
            // An approved creator portrait remains the hero's accepted depiction after
            // a generated reference becomes ready. Other heroes use their ready reference.
            let path = if index == 0 {
                format!("/gameplay/creator-portrait/{}", "4".repeat(64))
            } else {
                format!("/gameplay/imagery/{}", ["a", "b"][index].repeat(64))
            };
            assert_eq!(member.portrait, path);
            assert_eq!(journey.live_faces[index].sprite, path);
            assert_eq!(journey.live_faces[index].battle_sprite, path);
            assert_eq!(
                journey.combat.as_ref().unwrap().participants[index].portrait,
                path
            );
            assert_eq!(
                df_render::art_contract::player_hero_image(&path),
                Some(path.as_str())
            );
        }
        assert_eq!(
            journey.own_character.unwrap().portrait_uri,
            journey.party[1].portrait
        );
        for stock in [
            "portrait-hero-1",
            "figure-hero-2",
            "sprite-hero-3",
            "assets/ui/generated/figure-hero-1-documentary-photo",
        ] {
            assert!(
                df_render::art_contract::fetch_target(stock).is_none(),
                "{stock}"
            );
        }
    }

    fn model() -> SceneModel {
        SceneModel {
            services: Vec::new(),
            witnessed_thefts: Vec::new(),
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

    #[tokio::test]
    async fn authored_marek_identity_uses_approved_portrait_and_full_body_photos() {
        use crate::gameplay::{
            adventure::scene_model::StartingAttitude,
            pack::{Age, CastRole, Gender, SceneCastMember},
        };

        let mut authored = model();
        authored.people.push(ScenePerson {
            passive_perception: 10,
            persona: None,
            id: "merchant".to_owned(),
            name: "Marek".to_owned(),
            role: "cloth merchant".to_owned(),
            description: "Marek, a short, round middle-aged man with a neatly trimmed beard, wears a rich velvet cap and a measuring ribbon around his neck.".to_owned(),
            relation_to_speaker: "shopkeeper".to_owned(),
            position: "behind the wooden counter".to_owned(),
            aliases: vec!["Marek".to_owned()],
            is_background: false,
            present_in: vec!["opening".to_owned()],
            attitude: StartingAttitude::Friendly,
            attitudes: Default::default(),
            knowledge: Vec::new(),
            wants: String::new(),
            coin_cp: 0,
        });
        let cast = [SceneCastMember {
            id: "merchant".to_owned(),
            name: "Marek".to_owned(),
            role: CastRole::Patron,
            sprite: "assets/ui/generated/sprite-patron-merchant-documentary-photo".to_owned(),
            portrait: "assets/ui/generated/portrait-patron-merchant-documentary-photo".to_owned(),
            voice: "warm".to_owned(),
            gender: Some(Gender::He),
            age: Some(Age::Adult),
            stat_block: None,
        }];

        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS37/fixture-cache/canonical-reference-tests/marek-{}",
            std::process::id()
        ));
        let fixture = DirectionFixture::photographic(&cache);
        let mut canonical = references("shop-run");
        canonical.admit_room("mareks-cloth-shop", &authored, &cast, true);
        let work = canonical
            .take_jobs()
            .into_iter()
            .find_map(|job| match job {
                Job::Canonical(work) if work.target == Target::Npc("merchant".to_owned()) => {
                    Some(work)
                }
                _ => None,
            })
            .expect("Marek identity job");
        let Source::Npc {
            person,
            identity_photo,
            identity_figure_photo,
            ..
        } = &work.source
        else {
            panic!("Marek NPC identity source");
        };

        assert!(person.description.contains("short, round middle-aged man"));
        assert_eq!(identity_photo.as_deref(), Some("portrait-patron-merchant"));
        assert_eq!(
            identity_figure_photo.as_deref(),
            Some("sprite-patron-merchant")
        );
        let store = ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        );
        assert!(store.load(identity_photo.as_deref().unwrap()).is_ok());
        assert!(
            store
                .load(identity_figure_photo.as_deref().unwrap())
                .is_ok()
        );

        let inner = inner(fixture.imagery.clone(), canonical);
        inner.render_canonical(&work).await.unwrap();
        assert_eq!(fixture.calls(), 1);
        // Only the two real photographs are sent; there is no blank carrier before them.
        assert_eq!(fixture.image_count(), 2);
        let prompt = fixture.prompt();
        assert!(prompt.contains("Image 1 is the approved documentary face portrait"));
        assert!(prompt.contains("Image 2 is the approved full-body photograph"));
    }

    fn inner(imagery: Imagery, canonical: References) -> Inner {
        let timing = Timing::from_interval(Duration::from_secs(30));
        let mut state = State::new(timing);
        state.canonical = canonical;
        Inner {
            imagery,
            references: ReferenceStore::new(
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
            name: "Test hero".to_owned(),
            identity: Box::new(Identity::from_menus(&[]).unwrap()),
            species: "orc".to_owned(),
            gear: vec!["a greatsword"],
            public_design: String::new(),
            portrait: None,
            locator: "Test hero".to_owned(),
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
            "../../artifacts/tmp/lanes/HS37/fixture-cache/canonical-reference-tests/runtime-{}",
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

    #[tokio::test]
    async fn combat_creature_missing_pending_and_late_ready_share_real_cached_art() {
        // The cache is written during the test, so it is a scratch copy of the committed art.
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/gameplay/fixtures/creature-art");
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS37/fixture-cache/creature-art-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&cache).unwrap();
        for entry in std::fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), cache.join(entry.file_name())).unwrap();
        }
        let fixture = DirectionFixture::new(&cache);
        let canonical = references("combat-creature-run");
        let inner = Arc::new(inner(fixture.imagery.clone(), canonical));
        let mut journey = rpc::JourneyView {
            combat: Some(rpc::CombatView {
                participants: vec![
                    rpc::PartyMemberView {
                        name: "Skeleton 1".to_owned(),
                        creature_id: "skeleton".to_owned(),
                        side: 2,
                        ..Default::default()
                    },
                    rpc::PartyMemberView {
                        name: "Skeleton 2".to_owned(),
                        creature_id: "skeleton".to_owned(),
                        side: 2,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let jobs = {
            let mut state = locked(&inner.state);
            apply_creature_faces(&mut state.canonical, Some(&mut journey));
            assert!(
                journey.live_faces.is_empty(),
                "pending art cannot fabricate a URL"
            );
            let jobs = state.canonical.take_jobs();
            assert_eq!(jobs.len(), 1, "one request per creature identity");
            apply_creature_faces(&mut state.canonical, Some(&mut journey));
            assert!(state.canonical.take_jobs().is_empty());
            jobs
        };
        let mut tasks = JoinSet::new();
        for job in jobs {
            inner.spawn(&mut tasks, job);
        }
        inner.finish(tasks.join_next().await.unwrap().unwrap());
        apply_creature_faces(&mut locked(&inner.state).canonical, Some(&mut journey));
        assert_eq!(journey.live_faces.len(), 2);
        let key = locked(&inner.state)
            .canonical
            .npc("combat-creature:skeleton")
            .unwrap();
        assert!(matches!(
            fixture.imagery.image(key.as_str()).await,
            Lookup::Image { .. }
        ));
        assert!(
            journey
                .live_faces
                .iter()
                .all(|face| face.sprite == key.path() && face.battle_sprite == key.path())
        );
    }

    #[test]
    fn same_place_authored_layouts_coexist_and_select_the_exact_current_room() {
        let mut canonical = references("same-run-pack");
        let morning = model();
        let mut evening = morning.clone();
        evening.time_of_day = "evening".to_owned();
        let morning_revision = room_revision_with_context(&morning, None).unwrap();
        let evening_revision = room_revision_with_context(&evening, None).unwrap();
        canonical.admit_room_with_asset_and_context(
            "shared-place",
            &morning,
            &[],
            true,
            None,
            None,
            None,
        );
        canonical.admit_room_with_asset_and_context(
            "shared-place",
            &evening,
            &[],
            true,
            None,
            None,
            None,
        );
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
            .admit_room_with_asset_and_context(
                "shared-place",
                &morning,
                &[],
                true,
                None,
                None,
                None,
            );
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
            passive_perception: 10,
            persona: None,
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
            room_revision_with_context(&original, None),
            room_revision_with_context(&changed_cast, None),
            "cast changes belong to character references; the canonical room is empty"
        );
        let brightwater = imagery::RoomArchitectureContext {
            name: "Brightwater".to_owned(),
            character: "pale stone houses and striped awnings".to_owned(),
        };
        let eastmere = imagery::RoomArchitectureContext {
            name: "Eastmere".to_owned(),
            character: "red brick houses and timber awnings".to_owned(),
        };
        assert_ne!(
            room_revision_with_context(&original, Some(&brightwater)),
            room_revision_with_context(&original, Some(&eastmere)),
            "a changed city architecture context regenerates the room"
        );
    }

    #[test]
    fn actual_room_admission_preserves_terminal_provider_outcomes_on_reobservation() {
        for failure in [
            ImageFailure::Provider,
            ImageFailure::Refused,
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

    #[test]
    fn a_created_place_is_admitted_like_the_shop_with_a_room_photo_and_each_person_by_this_path() {
        use crate::gameplay::{
            place,
            place_creation::{self, tests as made},
        };
        let record = place_creation::build(
            &made::request(),
            &made::place_slot().to_string(),
            &made::people_slot().to_string(),
            &made::things_slot().to_string(),
        )
        .unwrap();
        let scene = place::scene_of(&record, &pack::scene(pack::SceneKey::Opening)).unwrap();
        let mut canonical = references("run-with-a-created-place");
        canonical.observe_room(&scene);
        // The room is the place's own, the current one, and each person has an identity to make.
        let room = canonical
            .selected_rooms
            .get("place-market-street")
            .cloned()
            .expect("the street's room is admitted");
        assert_eq!(canonical.current_room, Some(room));
        let people: BTreeSet<String> = canonical
            .entries
            .keys()
            .filter_map(|target| match target {
                Target::Npc(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            people,
            [
                "market-street-hobb",
                "market-street-wenna",
                "market-street-yara"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(canonical.current_npcs, people);
        // What the photograph is asked for is the street's own layout, never the shop's.
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let context = room_architecture_context("place-market-street");
        let prompt = imagery::canonical_room_with_context(
            ContentRating::Restrained,
            model,
            "place-market-street",
            context.as_ref(),
            &[],
            "binding",
        )
        .unwrap();
        assert!(prompt.as_str().contains("spice stall"));
        assert!(!prompt.as_str().contains("wooden counter"));
        let hobb = model
            .people
            .iter()
            .find(|person| person.name == "Hobb")
            .unwrap();
        let person = imagery::canonical_npc(
            ContentRating::Restrained,
            hobb,
            (None, None, &[] as &[_]).into(),
            false,
            false,
            "b",
        )
        .unwrap();
        assert!(person.as_str().contains("broad carter"));
    }
    #[test]
    fn rejected_room_can_be_readmitted_after_cooldown_once_without_replacing_its_binding() {
        let mut canonical = references("room-retry-run");
        room(&mut canonical, "stable-layout");
        let binding = selected_room_binding(&canonical).to_owned();
        let Job::Canonical(work) = canonical.take_jobs().remove(0) else {
            panic!("room job");
        };
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
        inner.finish_canonical(work, Err(ImageFailure::BadOutput));
        let mut state = locked(&inner.state);
        room(&mut state.canonical, "stable-layout");
        assert!(
            state.canonical.take_jobs().is_empty(),
            "no immediate paid retry loop"
        );
        let target = state.canonical.selected_rooms["same-place"].clone();
        state.canonical.entries.get_mut(&target).unwrap().started =
            Some(Instant::now() - ROOM_RETRY_COOLDOWN);
        room(&mut state.canonical, "stable-layout");
        room(&mut state.canonical, "stable-layout");
        let jobs = state.canonical.take_jobs();
        assert_eq!(jobs.len(), 1);
        let Job::Canonical(work) = &jobs[0] else {
            panic!("room job");
        };
        assert!(work.regenerate);
        assert_eq!(work.binding, binding);
    }

    #[tokio::test]
    async fn authored_place_admission_skips_room_generation_but_keeps_people_and_direct_references()
    {
        let mut scene = pack::scene(pack::SceneKey::Opening);
        scene.asset = format!("/assets/ui/generated/{}.webp", art_contract::HARBOR_PHOTO);
        let model: SceneModel = serde_json::from_slice(include_bytes!(
            "../../../../../content/merchant-shop/scene-model.json"
        ))
        .unwrap();
        scene.setting = Some(crate::gameplay::adventure::setting::SceneSetting {
            place_id: "HS37-authored-quay".into(),
            name: "Working quay".into(),
            short: "Weathered boards beside the water".into(),
            kind: crate::gameplay::adventure::setting::SettingKind::Quay,
            exterior: true,
            art_key: scene.asset.clone(),
            model: Some(model.clone()),
        });
        let place = scene.setting.as_ref().unwrap().place_id.clone();
        let context = room_architecture_context(&place);
        let revision = room_revision_with_context(&model, context.as_ref()).unwrap();
        let mut canonical = references("HS37-authored-admission");
        canonical.observe_room(&scene);
        // An ahead admission without another asset cannot resurrect room generation.
        canonical.admit_room(&place, &model, &scene.cast, false);
        let jobs = canonical.take_jobs_up_to(model.people.len() + 1);
        assert_eq!(
            jobs.len(),
            model.people.len() + 1,
            "NPC work plus one local photo load"
        );
        assert!(
            !canonical
                .entries
                .values()
                .any(|entry| matches!(entry.work.source, Source::Room { .. }))
        );
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS37/authored-registration")
            .join(std::process::id().to_string());
        let fixture = DirectionFixture::photographic(&cache);
        let inner = inner(fixture.imagery.clone(), canonical);
        assert_eq!(
            inner.canonical_room_status(&place),
            ReferenceStatus::Pending
        );
        for job in jobs {
            let Job::Canonical(work) = job else {
                panic!("canonical work")
            };
            if matches!(work.source, Source::ReviewedRoom { .. }) {
                let key = inner.render_canonical(&work).await.unwrap();
                inner.finish_canonical(work, Ok(key));
            } else {
                assert!(matches!(work.source, Source::Npc { .. }));
            }
        }
        assert_eq!(
            fixture.calls(),
            0,
            "authored room registration never invokes a provider"
        );
        let registered = inner.canonical_room_key(&place).unwrap();
        assert!(matches!(
            fixture.imagery.image(registered.as_str()).await,
            Lookup::Image { .. }
        ));
        assert_eq!(inner.canonical_room_status(&place), ReferenceStatus::Ready);
        assert_eq!(
            inner.canonical_room_status_at(&place, &revision),
            ReferenceStatus::Ready
        );
        let expected = inner.load(scene.asset).await.unwrap();
        assert_eq!(
            inner.canonical_room_reference(&place).await.unwrap().sha256,
            expected.sha256
        );
        assert_eq!(
            inner
                .canonical_room_reference_at(&place, &revision)
                .await
                .unwrap()
                .sha256,
            expected.sha256
        );
    }

    #[test]
    fn authored_room_retires_a_queued_generation_and_generic_reference_still_generates() {
        let mut canonical = references("HS37-authored-replacement");
        canonical.admit_room_with_asset(
            "place",
            &model(),
            &[],
            true,
            Some(art_contract::ROOM_PHOTO),
            None,
        );
        assert!(
            canonical
                .entries
                .values()
                .any(|entry| matches!(entry.work.source, Source::Room { .. }))
        );
        canonical.admit_room_with_asset(
            "place",
            &model(),
            &[],
            true,
            Some(art_contract::HARBOR_PHOTO),
            None,
        );
        let jobs = canonical.take_jobs();
        assert_eq!(jobs.len(), 1);
        assert!(
            matches!(&jobs[0], Job::Canonical(work) if matches!(work.source, Source::ReviewedRoom { .. }))
        );
        assert_eq!(canonical.authored_rooms.len(), 1);
    }

    #[test]
    fn prepared_room_is_resolvable_without_changing_current_room_or_duplicating_work() {
        let mut canonical = references("prepared-room-run");
        canonical.admit_room("shop", &model(), &[], true);
        let current = canonical.current_room.clone();
        canonical.admit_room("street", &model(), &[], false);
        canonical.admit_room("street", &model(), &[], false);
        assert_eq!(canonical.current_room, current);
        assert_eq!(canonical.entries.len(), 2);
        assert!(canonical.selected_rooms.contains_key("street"));
        let inner = inner(Imagery::disabled(ImageryOff::Qualification), canonical);
        let jobs = locked(&inner.state).canonical.take_jobs();
        for job in jobs {
            let Job::Canonical(work) = job else {
                panic!("room job");
            };
            if matches!(&work.target, Target::Room { place, .. } if place == "street") {
                // Drop the test's state lock before reporting a completion.
                let key = ImageKey::parse(&"b".repeat(64)).unwrap();
                let mut state = locked(&inner.state);
                state
                    .canonical
                    .entries
                    .get_mut(&work.target)
                    .unwrap()
                    .status = Status::Ready(key.clone());
                assert_eq!(state.canonical.room("street"), Some(key));
            }
        }
    }
}

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
            asset,
            known_revision,
            room_architecture_context(place),
        );
    }

    fn admit_room_with_asset_and_context(
        &mut self,
        place: &str,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
        select: bool,
        asset: Option<&str>,
        known_revision: Option<&str>,
        context: Option<imagery::RoomArchitectureContext>,
    ) {
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
            if !matches!(
                gender,
                Some(pack::Gender::He | pack::Gender::She | pack::Gender::They)
            ) || age.is_none()
            {
                super::super::actor::record_audit_line(
                    "gameplay.imagery",
                    format!("IMAGERY presentation_missing person={}", person.id),
                );
            }
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
            self.select(
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
