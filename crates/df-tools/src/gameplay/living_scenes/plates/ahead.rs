//! Run-owned speculative plates. Candidates survive reference warm-up; stale branches do not.
use super::*;
use crate::gameplay::{beat_script_runtime, journey, place_move};
use df_model::checkpoint::Checkpoint;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Source {
    Beat,
    Travel,
    Generated,
    Outcome,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::Beat => "beat",
            Self::Travel => "travel",
            Self::Generated => "generated",
            Self::Outcome => "outcome",
        }
    }
}

#[derive(Clone)]
struct Candidate {
    scene: PlateScene,
    source: Source,
    heroes: Vec<u8>,
    hero_names: Vec<(u8, String)>,
    revision: u64,
    id: Option<String>,
    queued: Instant,
    ready_ms: Option<u128>,
    failed: bool,
    used: Option<&'static str>,
}

#[derive(Default)]
pub(super) struct Queue {
    scope: String,
    origin: String,
    party: Vec<(u8, String)>,
    candidates: VecDeque<Candidate>,
    arrival: Option<Arrival>,
}

struct Forecast {
    current: pack::Scene,
    next: Vec<(pack::Scene, Source)>,
}

struct Arrival {
    id: String,
    scene: String,
    started: Instant,
    ready: bool,
}

impl Queue {
    pub(super) fn bind(&mut self, scene: &PlateScene, id: &str) {
        if let Some(candidate) = self
            .candidates
            .iter_mut()
            .find(|candidate| candidate.scene.source_identity() == scene.source_identity())
        {
            candidate.id = Some(id.to_owned());
        }
    }
}

fn diagnostic(candidate: &Candidate, status: &str, used: &str) {
    let scene = df_observe::presentation::local_identity(&candidate.scene.source_identity());
    let queued_ms = candidate.queued.elapsed().as_millis();
    let ready_ms = candidate.ready_ms;
    let source = candidate.source.label();
    record_plate_diagnostic(
        "gameplay.living_scene.plate_ahead",
        format!(
            "PLATE ahead scene={scene} source={source} queued_ms={queued_ms} ready_ms={} used={used} status={status}",
            ready_ms.map_or_else(|| "pending".to_owned(), |ms| ms.to_string())
        ),
        serde_json::Map::from_iter([
            ("scene".into(), serde_json::json!(scene)),
            ("source".into(), serde_json::json!(source)),
            ("queued_ms".into(), serde_json::json!(queued_ms)),
            ("ready_ms".into(), serde_json::json!(ready_ms)),
            ("used".into(), serde_json::json!(used)),
            ("status".into(), serde_json::json!(status)),
        ]),
    );
}

fn arrival_event(
    scene: &str,
    used: &str,
    wait_ms: u128,
    status: &str,
) -> (String, serde_json::Map<String, serde_json::Value>) {
    let scene = df_observe::presentation::local_identity(scene);
    (
        format!("PLATE shown scene={scene} used={used} wait_ms={wait_ms} status={status}"),
        serde_json::Map::from_iter([
            ("scene".into(), serde_json::json!(scene)),
            ("used".into(), serde_json::json!(used)),
            ("wait_ms".into(), serde_json::json!(wait_ms)),
            ("status".into(), serde_json::json!(status)),
        ]),
    )
}

fn arrival_diagnostic(scene: &str, used: &str, wait_ms: u128, status: &str) {
    let (line, fields) = arrival_event(scene, used, wait_ms, status);
    record_plate_diagnostic("gameplay.living_scene.plate_arrival", line, fields);
}

fn same_picture(left: &PlateScene, right: &PlateScene) -> bool {
    let picture = |scene: &PlateScene| {
        scene
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.model.picture_state())
            .transpose()
    };
    match (picture(left), picture(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

impl Shared {
    fn queue_ahead(
        &self,
        book: &mut Book,
        mut scene: PlateScene,
        source: Source,
        heroes: &[u8],
        hero_names: &[(u8, String)],
        revision: u64,
    ) {
        let identity = scene.source_identity();
        if heroes.is_empty() {
            return;
        }
        if let Some(candidate) = book.ahead.candidates.iter_mut().find(|candidate| {
            candidate.scene.source_identity() == identity && candidate.heroes == heroes
        }) {
            if source != Source::Beat {
                candidate.source = source;
            }
            return;
        }
        scene.hero_names = hero_names.iter().map(|(_, name)| name.clone()).collect();
        let candidate = Candidate {
            scene,
            source,
            heroes: heroes.to_vec(),
            hero_names: hero_names.to_vec(),
            revision,
            id: None,
            queued: Instant::now(),
            ready_ms: None,
            failed: false,
            used: None,
        };
        book.ahead
            .candidates
            .retain(|candidate| candidate.used.is_none() || candidate.ready_ms.is_none());
        if book.ahead.candidates.len() >= MAXIMUM_SUCCESSORS {
            // A concrete storyteller plan takes precedence over the furthest static prediction.
            if source == Source::Beat {
                diagnostic(&candidate, "queue_full", "cancelled");
                return;
            }
            let position = book
                .ahead
                .candidates
                .iter()
                .rposition(|candidate| candidate.source == Source::Beat);
            let Some(position) = position else {
                diagnostic(&candidate, "queue_full", "cancelled");
                return;
            };
            if let Some(candidate) = book.ahead.candidates.remove(position) {
                self.cancel_ahead(book, candidate);
            }
        }
        diagnostic(&candidate, "queued", "miss");
        book.ahead.candidates.push_back(candidate);
    }

    fn cancel_ahead(&self, book: &mut Book, candidate: Candidate) {
        if candidate.used.is_none() {
            diagnostic(&candidate, "cancelled", "cancelled");
        }
        if let Some(id) = candidate.id
            && book
                .current
                .as_ref()
                .and_then(|current| current.id.as_ref())
                != Some(&id)
            && matches!(book.entries.get(&id), Some(Entry::Pending))
        {
            book.entries.remove(&id);
            book.request_started.remove(&id);
            book.order.retain(|known| known != &id);
        }
    }

    pub(super) fn replace_beat_ahead(&self, book: &mut Book, view: &View, next: &[pack::Scene]) {
        let allowed: Vec<String> = next
            .iter()
            .filter_map(PlateScene::of)
            .map(|scene| scene.source_identity())
            .chain(std::iter::once(view.scene_key.clone()))
            .collect();
        if book.ahead.origin != view.scene_key {
            self.retain_ahead(book, &allowed, &view.heroes);
            book.ahead.origin = view.scene_key.clone();
        } else if book
            .ahead
            .candidates
            .iter()
            .any(|candidate| candidate.heroes != view.heroes)
        {
            let known = book
                .ahead
                .candidates
                .iter()
                .map(|candidate| candidate.scene.source_identity())
                .collect::<Vec<_>>();
            self.retain_ahead(book, &known, &view.heroes);
        }
        for scene in next
            .iter()
            .filter_map(PlateScene::of)
            .take(MAXIMUM_SUCCESSORS)
        {
            self.queue_ahead(
                book,
                scene,
                Source::Beat,
                &view.heroes,
                &view.hero_names,
                view.graph_revision,
            );
        }
    }

    fn retain_ahead(&self, book: &mut Book, allowed: &[String], heroes: &[u8]) {
        let old = std::mem::take(&mut book.ahead.candidates);
        for candidate in old {
            if candidate.heroes == heroes && allowed.contains(&candidate.scene.source_identity()) {
                book.ahead.candidates.push_back(candidate);
            } else {
                self.cancel_ahead(book, candidate);
            }
        }
    }

    pub(super) fn poll_ahead_locked(&self, book: &mut Book) {
        // Current is always admitted before speculative work, even after dependencies finish.
        if let Some(current) = &book.current
            && !current.in_combat
            && let Some(scene) = current.scene.clone()
        {
            let heroes = current.heroes.clone();
            let faces = current.faces.clone();
            let revision = current.graph_revision;
            self.ensure(book, &scene, &heroes, &faces, revision, false);
        }
        let current_waiting = book
            .current
            .as_ref()
            .is_some_and(|current| !current.in_combat && current.scene.is_some())
            && book.current_entry().is_none();
        for candidate in book.ahead.candidates.clone() {
            if current_waiting {
                if let Some(model) = &candidate.scene.authored_room {
                    self.inner.admit_canonical_room(
                        &candidate.scene.place_id,
                        model,
                        &candidate.scene.cast,
                        candidate.scene.authored_place_photo(),
                    );
                }
                continue;
            }
            let faces = self.inner.hero_face_keys(&candidate.hero_names);
            self.ensure(
                book,
                &candidate.scene,
                &candidate.heroes,
                &faces,
                candidate.revision,
                true,
            );
            let id = book
                .ahead
                .candidates
                .iter()
                .find(|known| known.scene.source_identity() == candidate.scene.source_identity())
                .and_then(|known| known.id.clone());
            if let Some(id) = id.as_ref()
                && matches!(book.entries.get(id), Some(Entry::Ready { .. }))
                && candidate.ready_ms.is_none()
            {
                self.finish_ahead(book, id, true, Instant::now());
            } else if let Some(id) = id
                && matches!(
                    book.entries.get(&id),
                    Some(Entry::Failed | Entry::NotPlanned)
                )
                && !candidate.failed
            {
                self.finish_ahead(book, &id, false, Instant::now());
            }
        }
    }

    pub(super) fn reject_stale_ahead(&self, book: &mut Book, actual: &PlateScene) {
        let old = std::mem::take(&mut book.ahead.candidates);
        for candidate in old {
            if candidate.used.is_none()
                && candidate.scene.source_identity() == actual.source_identity()
                && !same_picture(&candidate.scene, actual)
            {
                if let Some(id) = candidate.id.as_ref() {
                    book.entries.remove(id);
                    book.request_started.remove(id);
                    book.order.retain(|known| known != id);
                }
                self.cancel_ahead(book, candidate);
            } else {
                book.ahead.candidates.push_back(candidate);
            }
        }
    }

    pub(super) fn ahead_matches(&self, id: &str, scene: &PlateScene) -> bool {
        if !self.ahead_wanted(id) {
            return false;
        }
        let book = locked(&self.book);
        if !book.entries.contains_key(id) {
            return false;
        }
        if let Some(candidate) = book
            .ahead
            .candidates
            .iter()
            .find(|candidate| candidate.id.as_deref() == Some(id))
        {
            return same_picture(&candidate.scene, scene);
        }
        book.current
            .as_ref()
            .filter(|current| current.id.as_deref() == Some(id))
            .and_then(|current| current.scene.as_ref())
            .is_some_and(|current| same_picture(current, scene))
    }

    pub(super) fn ahead_wanted(&self, id: &str) -> bool {
        let book = locked(&self.book);
        book.entries.contains_key(id)
            && (book
                .ahead
                .candidates
                .iter()
                .any(|candidate| candidate.id.as_deref() == Some(id))
                || book
                    .current
                    .as_ref()
                    .and_then(|current| current.id.as_deref())
                    == Some(id))
    }

    pub(super) async fn wait_until_unwanted(&self, id: &str, scene: &PlateScene) {
        loop {
            if !self.ahead_matches(id, scene) {
                return;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    pub(super) fn finish_ahead(&self, book: &mut Book, id: &str, ready: bool, now: Instant) {
        if let Some(candidate) = book
            .ahead
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id.as_deref() == Some(id))
        {
            if ready && candidate.ready_ms.is_none() {
                candidate.ready_ms =
                    Some(now.saturating_duration_since(candidate.queued).as_millis());
            }
            candidate.failed = !ready;
            diagnostic(
                candidate,
                if ready { "ready" } else { "failed" },
                candidate.used.unwrap_or("miss"),
            );
        }
    }

    pub(super) fn note_arrival(&self, book: &mut Book) {
        if let Some(arrival) = book.ahead.arrival.take().filter(|arrival| !arrival.ready) {
            arrival_diagnostic(
                &arrival.scene,
                "miss",
                arrival.started.elapsed().as_millis(),
                "left",
            );
        }
        let Some(current) = &book.current else {
            return;
        };
        let Some(id) = current.id.clone() else {
            return;
        };
        let ready = matches!(book.entries.get(&id), Some(Entry::Ready { .. }));
        let used = if ready { "hit" } else { "miss" };
        if let Some(candidate) = book
            .ahead
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id.as_ref() == Some(&id))
        {
            candidate.used = Some(used);
            diagnostic(candidate, "arrived", used);
        }
        arrival_diagnostic(
            &current.scene_key,
            used,
            0,
            if ready { "ready" } else { "waiting" },
        );
        book.ahead.arrival = Some(Arrival {
            id,
            scene: current.scene_key.clone(),
            started: Instant::now(),
            ready,
        });
    }

    pub(super) fn note_arrival_ready(&self, book: &mut Book, id: &str, now: Instant) {
        if !matches!(book.entries.get(id), Some(Entry::Ready { .. })) {
            return;
        }
        if let Some(arrival) = book
            .ahead
            .arrival
            .as_mut()
            .filter(|arrival| arrival.id == id && !arrival.ready)
        {
            arrival.ready = true;
            arrival_diagnostic(
                &arrival.scene,
                "miss",
                now.saturating_duration_since(arrival.started).as_millis(),
                "ready",
            );
        }
    }

    /// Checkpoint notifications run independently of browser views and never await image work.
    pub(super) fn plan_checkpoint(&self, checkpoint: &Checkpoint) {
        self.inner.observe_checkpoint(checkpoint);
        let scope = format!(
            "{:?}|{:?}",
            checkpoint.basis().session,
            checkpoint.basis().run
        );
        let mut book = locked(&self.book);
        if book.ahead.scope != scope {
            if !book.ahead.scope.is_empty() {
                book.current = None;
                book.ahead.arrival = None;
                book.ahead.origin.clear();
                book.entries.clear();
                book.order.clear();
                book.request_started.clear();
                book.image_layouts.clear();
                book.edit = Edit::Idle;
                book.direct.reset();
                book.state_images = state_images::Book::default();
                book.face_refresh = FaceRefresh::default();
                book.refresh_decisions.clear();
                self.inner.set_plate_covers(false);
            }
            let old = std::mem::take(&mut book.ahead.candidates);
            for candidate in old {
                self.cancel_ahead(&mut book, candidate);
            }
            book.ahead.scope = scope;
        }
        let mut heroes = Vec::new();
        for (index, member) in journey::participants(checkpoint).into_iter().enumerate() {
            let Some(entity) = member.character else {
                continue;
            };
            let (Ok(seat), Ok(name)) = (
                u8::try_from(index + 1),
                journey::name(checkpoint.state(), entity),
            ) else {
                record(
                    "gameplay.living_scene.plate_plan_failed",
                    &[("failure", "invalid_party".to_owned())],
                );
                return;
            };
            heroes.push((seat, name));
        }
        let seats: Vec<u8> = heroes.iter().map(|(seat, _)| *seat).collect();
        if book.ahead.party != heroes {
            let old = std::mem::take(&mut book.ahead.candidates);
            for candidate in old {
                if candidate.heroes == seats {
                    book.ahead.candidates.push_back(candidate);
                } else {
                    self.cancel_ahead(&mut book, candidate);
                }
            }
        }
        book.ahead.party = heroes.clone();
        let result = self.checkpoint_scenes(checkpoint);
        match result {
            Ok(Forecast {
                current,
                next: scenes,
            }) => {
                if scenes.is_empty() && current.key == SceneKey::Room {
                    let old = std::mem::take(&mut book.ahead.candidates);
                    for candidate in old {
                        self.cancel_ahead(&mut book, candidate);
                    }
                }
                let current_identity = projection_scene_identity(
                    current.key.source_label(),
                    current.source_variant_id.as_deref().unwrap_or_default(),
                    &current.title,
                    &current.asset,
                );
                let offered: Vec<String> = scenes
                    .iter()
                    .filter(|(_, source)| *source == Source::Travel)
                    .filter_map(|(scene, _)| PlateScene::of(scene))
                    .map(|scene| scene.source_identity())
                    .collect();
                let old = std::mem::take(&mut book.ahead.candidates);
                for candidate in old {
                    if candidate.source == Source::Travel
                        && !offered.contains(&candidate.scene.source_identity())
                        && candidate.used.is_none()
                        && current_identity != candidate.scene.source_identity()
                    {
                        self.cancel_ahead(&mut book, candidate);
                    } else {
                        book.ahead.candidates.push_back(candidate);
                    }
                }
                if book.ahead.origin != current_identity {
                    let allowed: Vec<String> = scenes
                        .iter()
                        .filter_map(|(scene, _)| PlateScene::of(scene))
                        .map(|scene| scene.source_identity())
                        .chain(std::iter::once(current_identity.clone()))
                        .collect();
                    self.retain_ahead(&mut book, &allowed, &seats);
                    book.ahead.origin = current_identity;
                }
                for (scene, source) in scenes {
                    if let Some(scene) = PlateScene::of(&scene) {
                        self.queue_ahead(&mut book, scene, source, &seats, &heroes, 0);
                    }
                }
                self.poll_ahead_locked(&mut book);
            }
            Err(error) => record(
                "gameplay.living_scene.plate_plan_failed",
                &[("failure", format!("{error:?}"))],
            ),
        }
    }

    fn checkpoint_scenes(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<Forecast, df_session::submission::RepositoryError> {
        if journey::phase(checkpoint)? == rpc::JourneyPhase::Room {
            let next = if journey::party_ready(checkpoint) {
                let opening = successors(&pack::scene(SceneKey::Room))
                    .into_iter()
                    .find(|scene| scene.key == SceneKey::Opening)
                    .unwrap_or_else(|| pack::scene(SceneKey::Opening));
                vec![(opening, Source::Beat)]
            } else {
                Vec::new()
            };
            return Ok(Forecast {
                current: pack::scene(SceneKey::Room),
                next,
            });
        }
        let current = beat_script_runtime::scene(checkpoint.state())?;
        let mut scenes = Vec::new();
        // Only destinations actually offered by the authoritative travel projection are primed.
        let party = place::whereabouts(checkpoint.state(), &current)?;
        let snapshot = journey::living_scene::snapshot(checkpoint.state(), &current)?;
        for option in place_move::travel_options(checkpoint)? {
            let destination = match option.target {
                place_move::MoveTarget::Known(id) => {
                    place::destination_of(checkpoint.state(), &party.home, &id)?
                }
                place_move::MoveTarget::Exit(id) => {
                    let Some(exit) = snapshot.as_ref().and_then(|snapshot| {
                        snapshot.model.ways_out.iter().find(|exit| exit.id == id)
                    }) else {
                        continue;
                    };
                    place::destination(checkpoint.state(), &party.home, &party.here, exit)?
                }
                place_move::MoveTarget::Landmark(_)
                | place_move::MoveTarget::Spot(_)
                | place_move::MoveTarget::New => continue,
            };
            match destination {
                place::Destination::Created(record) => scenes.push((
                    place_move::destination_scene(checkpoint, &record, &current)?,
                    Source::Travel,
                )),
                place::Destination::Home => {
                    if let Some(scene) = pack::with_content(|content| {
                        content
                            .scenes
                            .iter()
                            .find(|scene| {
                                scene
                                    .setting
                                    .as_ref()
                                    .is_some_and(|setting| setting.place_id == party.home)
                            })
                            .cloned()
                    }) {
                        scenes.push((scene, Source::Travel));
                    }
                }
                place::Destination::Story { .. } | place::Destination::Create { .. } => {}
            }
        }
        let beat_id = beat_script::with_script(|script| {
            script
                .current(checkpoint.state())
                .map(|beat| beat.id.clone())
        })?;
        let next = pack::with_content(|content| {
            let script = content
                .beat_script
                .as_ref()
                .unwrap_or_else(|| beat_script::canonical());
            successors_in_from(script, &content.scenes, &current, Some(&beat_id))
        });
        scenes.extend(next.into_iter().map(|scene| (scene, Source::Beat)));
        for ready in crate::gameplay::adventure::free_roam::prepared(checkpoint) {
            if current
                .setting
                .as_ref()
                .is_some_and(|setting| setting.place_id == ready.from)
            {
                let scene = place_move::destination_scene(checkpoint, &ready.place, &current)?;
                let outcome = match crate::gameplay::place_music::installed() {
                    Some(music) => match music.prepare_scene(&scene) {
                        Ok(Some(plan)) => format!("outcome=planned plan_state={}", plan.state),
                        Ok(None) => "outcome=no_plan".to_owned(),
                        Err(error) => format!("outcome=projection_failed reason={error:?}"),
                    },
                    None => "outcome=disabled".to_owned(),
                };
                crate::gameplay::actor::record_audit_line(
                    "gameplay.free_roam",
                    format!(
                        "FREE_ROAM ahead_media place={} kind=music_ambience {outcome}",
                        ready.place.place_id
                    ),
                );
                scenes.push((scene, Source::Generated));
            }
        }
        Ok(Forecast {
            current,
            next: scenes,
        })
    }
}

impl super::super::LivingScenes {
    /// A validated storyteller plan can prime media without publishing or committing that plan.
    pub(in crate::gameplay) fn plan_plate(
        &self,
        scene: Result<pack::Scene, df_session::submission::RepositoryError>,
        source: Source,
    ) {
        let Some(plates) = &self.plates else {
            return;
        };
        let scene = match scene {
            Ok(scene) => scene,
            Err(error) => {
                record(
                    "gameplay.living_scene.plate_plan_failed",
                    &[("failure", format!("{error:?}"))],
                );
                return;
            }
        };
        let mut book = locked(&plates.shared.book);
        let names = if book.ahead.party.is_empty() {
            book.current
                .as_ref()
                .and_then(|current| {
                    current.scene.as_ref().map(|scene| {
                        current
                            .heroes
                            .iter()
                            .copied()
                            .zip(scene.hero_names.iter().cloned())
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default()
        } else {
            book.ahead.party.clone()
        };
        let heroes = names.iter().map(|(seat, _)| *seat).collect::<Vec<_>>();
        let revision = book
            .current
            .as_ref()
            .map_or(0, |current| current.graph_revision);
        if let Some(scene) = PlateScene::of(&scene) {
            plates
                .shared
                .queue_ahead(&mut book, scene, source, &heroes, &names, revision);
            plates.shared.poll_ahead_locked(&mut book);
        }
    }
}

#[cfg(test)]
#[path = "ahead_tests.rs"]
mod tests;

#[cfg(test)]
mod free_roam_tests {
    use super::super::tests::{drain, finish_canonical_references, key, shared, view};
    use super::*;
    use crate::gameplay::{
        adventure::scene_model::{self, SceneNow},
        adventure::{RulesMode, free_roam},
        beat_script_variety_tests as native,
        place_creation::tests as fixtures,
        startup_configuration::ContentRating,
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    use serde_json::{Value, json};

    #[test]
    fn prepared_free_roam_forecast_preserves_checkpoint_picture_and_hits_on_arrival() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _runtime_scope = runtime.enter();
        let compute: Arc<dyn Fn(&str) -> Value + Send + Sync> =
            Arc::new(|prompt| {
                let facts: Value = prompt
                    .lines()
                    .find_map(|line| line.strip_prefix("FACTS: "))
                    .map(|line| serde_json::from_str(line).unwrap())
                    .unwrap();
                if facts["task"] == "free_roam_city" {
                    return json!({
                        "name": "Alder", "character": "a canal city of workshops",
                        "mood": "unhurried", "time_of_day": "afternoon", "weather": "dry and cool"
                    });
                }
                let identity = format!("{:x}", sha2::Sha256::digest(facts.to_string().as_bytes()));
                let identity = &identity[..8];
                let gate = facts["origin"]["name"] == "Road outside Alder";
                let mut place = fixtures::place_slot();
                place["name"] = json!(if gate {
                    "Alder Main Gate".to_owned()
                } else {
                    format!("Alder Walk {identity}")
                });
                place["speaker"] = json!("");
                place["greeting"] = json!("");
                place["wares"] = json!([]);
                place["services"] = json!([]);
                let people = (0..if gate { 4 } else { 2 })
                .map(|index| json!({
                    "name": format!("Local{identity}{index}"), "role": "potter",
                    "description": "An adult potter in a plain coat.",
                    "position": format!("beside post {index}"), "gender": "they",
                    "wants": "Finish the day's work.", "attitude": "indifferent",
                    "stat_block": "commoner", "purse_cp": 20,
                    "knowledge": ["This is Alder.", "The Bell Tower is uphill.", "I work here."]
                }))
                .collect::<Vec<_>>();
                fixtures::answer(&place, &json!(people), &fixtures::things_slot())
            });
        let endpoint =
            FakeEndpoint::start((0..8).map(|_| Reply::Compute(compute.clone())).collect());
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS42/tmp/forecast-{}",
            std::process::id()
        ));
        let storyteller = teller(endpoint.address, 0, Duration::from_secs(10), &cache);
        let generated = runtime
            .block_on(free_roam::generate(
                842,
                RulesMode::Standard,
                ContentRating::default(),
                &storyteller,
            ))
            .unwrap();
        let (loaded, _) =
            crate::gameplay::adventure::record::reload(&generated.documents(true).unwrap())
                .unwrap();
        pack::with_test_pack(loaded, || {
            let checkpoint = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let before = checkpoint.clone();
            let (shared, mut jobs) = shared(16);
            let forecast = shared.checkpoint_scenes(&checkpoint).unwrap();
            let generated: Vec<_> = forecast
                .next
                .iter()
                .filter(|(_, source)| *source == Source::Generated)
                .collect();
            assert_eq!(generated.len(), 2);
            assert!(place::created(checkpoint.state()).unwrap().is_empty());
            let destination = generated[0].0.clone();
            let expected = destination.setting.as_ref().unwrap().model.clone().unwrap();
            assert!(expected.people.iter().any(|person| person.role == "hero"));
            assert!(!expected.things.is_empty());
            assert!(!expected.ways_out.is_empty());
            let place_id = destination.setting.as_ref().unwrap().place_id.clone();
            let mut stale = expected.clone();
            stale.people.retain(|person| person.role != "hero");
            stale.things.clear();
            stale.ways_out.clear();
            stale.weather = "stale weather".into();
            scene_model::publish_scene(
                &place_id,
                SceneNow {
                    model: stale,
                    changes: Vec::new(),
                },
            );
            let queued = PlateScene::of(&destination).unwrap();
            assert_eq!(queued.snapshot.as_ref().unwrap().model, expected);
            shared.plan_checkpoint(&checkpoint);
            assert_eq!(locked(&shared.book).ahead.candidates.len(), 2);
            assert!(
                locked(&shared.book)
                    .ahead
                    .candidates
                    .iter()
                    .all(|candidate| candidate.source == Source::Generated)
            );
            finish_canonical_references(&shared.inner);
            shared.poll_ahead_locked(&mut locked(&shared.book));
            let id = drain(&mut jobs)
                .into_iter()
                .find_map(|job| match job {
                    Job::Fresh {
                        id,
                        scene,
                        ahead: true,
                        ..
                    } if scene.place_id == place_id => {
                        assert_eq!(scene.snapshot.as_ref().unwrap().model, expected);
                        Some(id)
                    }
                    _ => None,
                })
                .expect("the prepared neighbour reaches the plate worker");
            shared.finish(Done::Fresh {
                id: id.clone(),
                graph_revision: 0,
                ahead: true,
                elapsed: Duration::from_secs(1),
                result: Ok((key('b'), None)),
            });
            scene_model::publish_scene(
                &place_id,
                SceneNow {
                    model: expected.clone(),
                    changes: Vec::new(),
                },
            );
            let heroes = locked(&shared.book)
                .ahead
                .candidates
                .iter()
                .find(|candidate| candidate.scene.place_id == place_id)
                .unwrap()
                .heroes
                .clone();
            let mut arrival = view(&destination, &heroes, false);
            arrival.hero_names = locked(&shared.book).ahead.party.clone();
            shared.observe_with(&arrival, Some(destination), Vec::new());
            let book = locked(&shared.book);
            assert!(matches!(book.current_entry(), Some(Entry::Ready { .. })));
            let candidate = book
                .ahead
                .candidates
                .iter()
                .find(|candidate| candidate.scene.place_id == place_id)
                .unwrap();
            assert_eq!(candidate.used, Some("hit"));
            assert_eq!(candidate.scene.snapshot.as_ref().unwrap().model, expected);
            assert_eq!(
                checkpoint, before,
                "forecasting never commits a visit or revision"
            );
            drop(book);

            // Restore a durable generated-place checkpoint with no runtime neighbours cached.
            let ready = free_roam::prepared(&checkpoint)[0].clone();
            let restored = place::plan(
                &checkpoint,
                "Brynn",
                df_types::OperationId::from_bytes(&[77; 16]).unwrap(),
                &ready.from,
                &ready.exit,
                &ready.place.place_id,
                Some(&ready.place),
            )
            .unwrap_or_else(|_| panic!("the prepared place is admissible in the saved checkpoint"))
            .candidate;
            assert!(
                free_roam::prepared(&restored)
                    .iter()
                    .all(|known| known.from != ready.place.place_id)
            );
            let (updates_in, updates) = tokio::sync::watch::channel(restored.clone());
            let (sender, _inbox) = df_session::inbox::bounded_inbox();
            let service = crate::gameplay::Service {
                actor: sender,
                display_credential: [0; 32],
                updates,
                streams: Arc::new(tokio::sync::Semaphore::new(2)),
                voice: crate::gameplay::voice::Voice::disabled(
                    crate::gameplay::startup_configuration::VoiceOff::Qualification,
                ),
                npc_voice: Arc::new(crate::gameplay::npc_reply::NpcVoice::new(Arc::new(
                    storyteller,
                ))),
                table_log: crate::gameplay::table_log::TableLog::new(),
                social: crate::gameplay::social::SocialLedger::none(),
                living: crate::gameplay::living_scenes::LivingScenes::disabled(),
                layout: crate::gameplay::dev_control::layout::LayoutState::new(),
                portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
            };
            runtime
                .block_on(service.prepare_free_roam_current())
                .unwrap();
            assert_eq!(*updates_in.borrow(), restored);
            assert_eq!(
                free_roam::prepared(&restored)
                    .iter()
                    .filter(|known| known.from == ready.place.place_id)
                    .count(),
                2
            );
            assert!(service.table_log.lines().is_empty());
            let calls = endpoint.connections();
            runtime
                .block_on(service.prepare_free_roam_current())
                .unwrap();
            assert_eq!(
                endpoint.connections(),
                calls,
                "restored neighbours are reused"
            );
            assert_eq!(*updates_in.borrow(), restored);
        });
    }
}
