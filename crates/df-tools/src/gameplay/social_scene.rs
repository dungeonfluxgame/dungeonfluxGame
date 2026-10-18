//! Detached bounded room snapshots; the session checkpoint owns durable scene changes.

use super::{SocialError, SocialLedger};
use crate::gameplay::adventure::{
    scene_model::{SceneModel, SceneNow},
    setting::ChapterSettings,
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
        let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        let mut candidate = Rooms::default();
        for scene in scenes {
            let setting = settings.scene(scene.key);
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
