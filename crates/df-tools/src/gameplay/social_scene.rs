//! Detached bounded room snapshots; the session checkpoint owns durable scene changes.

use super::{SocialError, SocialLedger};
use crate::gameplay::adventure::{
    scene_model::{SceneModel, SceneNow},
    setting::{ChapterSettings, SceneSetting},
};
use crate::gameplay::pack::{Scene, SceneKey};
use std::{collections::BTreeMap, fs, io, sync::PoisonError};

#[derive(Default)]
pub(super) struct Rooms {
    models: BTreeMap<String, SceneModel>,
    scenes: BTreeMap<String, (String, SceneKey)>,
    pub(super) projections: BTreeMap<String, SceneNow>,
    pub(super) active_scene: Option<String>,
}

impl Rooms {
    fn current(&self, scene: &str) -> Option<SceneNow> {
        if let Some(snapshot) = self.projections.get(scene) {
            return Some(snapshot.clone());
        }
        let (place, key) = self.scenes.get(scene)?;
        Some(SceneNow {
            model: self.models.get(place)?.for_scene(*key),
            changes: Vec::new(),
        })
    }
}

impl SocialLedger {
    /// Opening Influence uses a shared party baseline with explicit witnessed hero exceptions.
    pub(in crate::gameplay) fn opening_attitude(
        &self,
        current: &df_model::checkpoint::Checkpoint,
        member: df_types::MemberId,
    ) -> Result<df_rules::influence::Attitude, df_session::submission::RepositoryError> {
        let person = crate::gameplay::dialogue::opening_social_enforcer(current)?;
        let hero = crate::gameplay::journey::name(
            current.state(),
            crate::gameplay::journey::player_entity(member, current)?,
        )?;
        if let Some(attitude) = person.attitudes.get(&hero) {
            return Ok(crate::gameplay::dialogue::opening_scene_attitude(*attitude));
        }
        Ok(
            crate::gameplay::beat_script_runtime::opening_party_attitude(current.state())?
                .map(|(attitude, _)| attitude)
                .unwrap_or_else(|| {
                    crate::gameplay::dialogue::opening_scene_attitude(person.attitude)
                }),
        )
    }

    /// Read-only authored-opening projection for the development social endpoint. Unlike an
    /// emotional-state read, this never starts a scene or changes a relationship.
    pub(in crate::gameplay) fn development_opening_social(
        &self,
        current: &df_model::checkpoint::Checkpoint,
        requested: &str,
        hero: Option<&str>,
    ) -> Result<Option<serde_json::Value>, df_session::submission::RepositoryError> {
        use crate::gameplay::adventure::scene_model::SceneChangeKind;
        use crate::gameplay::{beat_script_runtime, dialogue, journey};
        if !current.state().narrative.active_beats.iter().any(|beat| {
            matches!(
                beat.entry.as_str(),
                "opening" | "courier-answer-seal" | "courier-answer-escort"
            )
        }) {
            return Ok(None);
        }
        let person = dialogue::opening_social_enforcer(current)?;
        if !person.id.eq_ignore_ascii_case(requested)
            && !person.name.eq_ignore_ascii_case(requested)
        {
            return Ok(None);
        }
        let party = beat_script_runtime::opening_party_attitude(current.state())?;
        let party_attitude = party
            .as_ref()
            .map(|(attitude, _)| *attitude)
            .unwrap_or_else(|| dialogue::opening_scene_attitude(person.attitude));
        let changes = journey::living_scene::scene_change_journal(current)?;
        let mut relationships = Vec::new();
        for character in &current.state().characters {
            let name = journey::name(current.state(), character.entity)?;
            if hero.is_some_and(|wanted| !name.eq_ignore_ascii_case(wanted)) {
                continue;
            }
            let exception = person.attitudes.get(&name);
            let cause = exception.map(|_| {
                changes
                    .iter()
                    .rev()
                    .find_map(|entry| {
                        (entry.change.entity_id == person.id
                            && matches!(&entry.change.kind,
                    SceneChangeKind::AttitudeChanged { hero_id, .. } if *hero_id == name))
                        .then(|| entry.change.description.clone())
                    })
                    .unwrap_or_else(|| "Authored starting attitude toward this hero".to_owned())
            });
            relationships.push(serde_json::json!({
                "hero": name,
                "attitude": exception.map(|attitude| dialogue::opening_scene_attitude(*attitude))
                    .unwrap_or(party_attitude).word(),
                "exception": exception.is_some(),
                "cause": cause,
            }));
        }
        Ok(Some(serde_json::json!({
            "npc": person.name, "npc_id": person.id,
            "party_attitude": party_attitude.word(),
            "party_cause": party.and_then(|(_, words)| words),
            "party_source": "canonical opening Influence receipts and authored starting attitude",
            "count": relationships.len(), "relationships": relationships,
        })))
    }

    /// Replaces a detached projection. The canonical checkpoint owns every accepted change;
    /// loaded sidecar deltas must never be appended to or replayed over this snapshot.
    pub(in crate::gameplay) fn sync_room(
        &self,
        scene: &str,
        snapshot: SceneNow,
    ) -> Result<(), SocialError> {
        // This is the trusted session projection, not a generator candidate: party heroes
        // have no NPC knowledge/aliases and owners may be off stage. Bound its payload while
        // retaining canonical fact validation in install_cached_rooms/record::reload.
        if snapshot.model.people.len() > 20
            || snapshot.model.things.len()
                > crate::gameplay::journey::living_scene::MAXIMUM_ROOM_THINGS
            || snapshot.model.ways_out.len() > 8
            || snapshot.model.offers.len() > 12
            || serde_json::to_vec(&snapshot)
                .map_err(|_| SocialError::Invalid("scene serialization"))?
                .len()
                > crate::gameplay::journey::living_scene::MAXIMUM_ROOM_BYTES
        {
            return Err(SocialError::Invalid("scene snapshot bounds"));
        }
        if snapshot.changes.len() > crate::gameplay::journey::living_scene::MAXIMUM_ROOM_CHANGES
            || snapshot
                .changes
                .iter()
                .any(|line| line.trim().is_empty() || line.chars().count() > 180)
        {
            return Err(SocialError::Invalid("scene snapshot changes"));
        }
        let witnesses = snapshot.model.witnessed_thefts.clone();
        {
            let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
            if !rooms.scenes.contains_key(scene) {
                return Err(SocialError::Invalid("unknown scene"));
            }
            rooms.projections.insert(scene.to_owned(), snapshot);
            rooms.active_scene = Some(scene.to_owned());
        }
        // The journal-derived witnesses are idempotent, so a repeated projection records nothing.
        if self.record_witnessed_thefts(&witnesses)? {
            self.save()?;
        }
        Ok(())
    }

    /// Native composition has already verified these immutable bytes against PostgreSQL.
    /// Revalidate their bindings here because compact pack scenes omit the canonical models.
    pub(super) fn install_cached_rooms(&self) -> Result<(), SocialError> {
        use std::io::Read;
        let Some(directory) = self.file.as_ref().and_then(|file| file.parent()) else {
            return Ok(());
        };
        let record = match fs::File::open(directory.join("adventure.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(SocialError::Io(error.kind())),
        };
        let read = |file: fs::File| -> Result<Vec<u8>, SocialError> {
            let limit = df_persistence::local_demo_scope::ADVENTURE_DOCUMENT_BYTES as u64;
            let mut bytes = Vec::new();
            file.take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| SocialError::Io(error.kind()))?;
            if bytes.len() as u64 > limit {
                return Err(SocialError::Invalid("room record size"));
            }
            Ok(bytes)
        };
        let documents = df_persistence::local_demo_scope::DemoAdventure {
            record: read(record)?,
            pack: read(
                fs::File::open(directory.join("pack.json"))
                    .map_err(|error| SocialError::Io(error.kind()))?,
            )?,
            tuning: read(
                fs::File::open(directory.join("tuning.json"))
                    .map_err(|error| SocialError::Io(error.kind()))?,
            )?,
        };
        let (loaded, record) = crate::gameplay::adventure::record::reload(&documents)
            .map_err(|_| SocialError::Invalid("room record binding"))?;
        if record.seed != self.adventure_seed || record.dossiers != self.dossiers {
            return Err(SocialError::Invalid("room dossier binding"));
        }
        if let Some(settings) = &record.settings {
            self.install_rooms(settings, &loaded.pack.scenes)?;
            crate::gameplay::journey::living_scene::install(settings);
        }
        Ok(())
    }

    pub(in crate::gameplay) fn install_rooms(
        &self,
        settings: &ChapterSettings,
        scenes: &[Scene],
    ) -> Result<(), SocialError> {
        self.install_scene_settings(scenes, |scene| Some(settings.scene(scene.key)))
    }

    /// Installs the scene models authored directly in the active gameplay pack.
    pub(in crate::gameplay) fn install_pack_rooms(
        &self,
        scenes: &[Scene],
    ) -> Result<(), SocialError> {
        self.install_scene_settings(scenes, |scene| scene.setting.as_ref())
    }

    /// Registers the room of a trusted runtime scene (the place overlay attaches a model the
    /// authored scene lists do not carry). An already known scene keeps its installed model.
    pub(in crate::gameplay) fn ensure_projected_room(&self, scene: &Scene) {
        let Some(setting) = &scene.setting else {
            return;
        };
        let Some(model) = &setting.model else {
            return;
        };
        let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        let name = format!("{:?}", scene.key);
        if rooms.scenes.contains_key(&name) {
            return;
        }
        rooms
            .models
            .entry(setting.place_id.clone())
            .or_insert_with(|| model.clone());
        rooms
            .scenes
            .insert(name, (setting.place_id.clone(), scene.key));
    }

    fn install_scene_settings<'a>(
        &self,
        scenes: &'a [Scene],
        setting_for_scene: impl Fn(&'a Scene) -> Option<&'a SceneSetting>,
    ) -> Result<(), SocialError> {
        let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        let mut candidate = Rooms::default();
        for scene in scenes {
            let Some(setting) = setting_for_scene(scene) else {
                continue;
            };
            if let Some(model) = &setting.model {
                candidate
                    .models
                    .insert(setting.place_id.clone(), model.clone());
                candidate.scenes.insert(
                    format!("{:?}", scene.key),
                    (setting.place_id.clone(), scene.key),
                );
            }
        }
        *rooms = candidate;
        Ok(())
    }

    pub(in crate::gameplay) fn scene_now(&self, scene: &str) -> Option<SceneNow> {
        self.rooms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .current(scene)
    }

    pub(super) fn active_room(&self) -> Option<SceneNow> {
        let rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        rooms.current(rooms.active_scene.as_deref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_scene_opening_read_has_party_attitude_without_a_dossier_and_names_authored_exceptions()
     {
        let mut loaded = crate::gameplay::pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        let person = loaded
            .pack
            .scenes
            .iter_mut()
            .find(|scene| scene.key == SceneKey::Opening)
            .unwrap()
            .setting
            .as_mut()
            .unwrap()
            .model
            .as_mut()
            .unwrap()
            .people
            .iter_mut()
            .find(|person| person.id == "bandit")
            .unwrap();
        person.attitudes.insert(
            "Vale".to_owned(),
            crate::gameplay::adventure::scene_model::StartingAttitude::Friendly,
        );
        crate::gameplay::pack::with_test_pack(loaded, || {
            let current = crate::gameplay::beat_script_runtime::opening_social_tests::opening(2);
            let social = SocialLedger::none();
            let document = social
                .development_opening_social(&current, "Dain Rusk", None)
                .unwrap()
                .unwrap();
            assert_eq!(document["party_attitude"], "hostile");
            let rows = document["relationships"].as_array().unwrap();
            let vale = rows.iter().find(|row| row["hero"] == "Vale").unwrap();
            assert_eq!(vale["attitude"], "friendly");
            assert_eq!(vale["exception"], true);
            assert_eq!(vale["cause"], "Authored starting attitude toward this hero");
            assert!(
                social
                    .development_opening_social(&current, "unknown", None)
                    .unwrap()
                    .is_none()
            );
            let filtered = social
                .development_opening_social(&current, "bandit", Some("Vale"))
                .unwrap()
                .unwrap();
            assert_eq!(filtered["count"], 1);
        });
    }

    #[test]
    fn built_in_pack_opening_projection_installs_and_unknown_scenes_are_refused() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/harbor-journey");
        let loaded = crate::gameplay::pack::load(&directory).expect("built-in harbor pack loads");
        let scenes = loaded.pack.scenes.clone();

        crate::gameplay::pack::with_test_pack(loaded, || {
            let social = SocialLedger::none();
            social
                .install_pack_rooms(&scenes)
                .expect("built-in scene settings install");

            let opening = format!("{:?}", SceneKey::Opening);
            let projection = social
                .scene_now(&opening)
                .expect("built-in opening scene model is installed");
            social
                .sync_room(&opening, projection.clone())
                .expect("known opening projection installs");
            assert_eq!(social.scene_now(&opening), Some(projection.clone()));
            assert!(social.sync_room("unknown-scene", projection).is_err());
        });
    }
}
