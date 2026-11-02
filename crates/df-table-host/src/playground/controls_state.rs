//! What the playground is showing, and how many changes it has had.
//!
//! One lock holds the controls, the live scene overlay, the revision and a fixture generation. A
//! change is checked and applied under that lock, so it is all or nothing; a change computed
//! against a fixture that has since changed (a scene change, or a choice of fixture) is refused
//! instead of applied to the wrong scene; and the audit line of a change is written by a callback
//! that runs with the lock held, so the log is in revision order. Refusals are typed, so the routes
//! answer with the right status, the log names the reason as a token, and a bug is not mistaken
//! for a refusal.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use axum::http::StatusCode;
use df_playground::controls::{Controls, ControlsChange, ControlsError, ControlsReply};
use df_playground::fixture::{Camera, Fixture};
use df_playground::media::ClientReport;
use df_playground::scene::{MINIMUM_ZOOM_PERCENT, SceneChange, SceneError, SceneOverlay};

use super::clients::ClientTable;
use super::files::PlaygroundFiles;
use crate::limits::MAXIMUM_BATCH_CHANGES;
use crate::wire::ClientView;

/// Why a change of the controls was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlsRefusal {
    /// The fixture chosen is not among the files; `available` are the ones that are.
    NoSuchFixture {
        name: String,
        available: Vec<String>,
    },
    /// The fixture files changed (one was written or removed, or another was shown) between
    /// reading their names and applying the choice.
    FixtureChanged,
    /// A batch with no change in it.
    EmptyBatch,
    /// A batch with more changes than `MAXIMUM_BATCH_CHANGES`.
    TooManyChanges {
        given: usize,
    },
    Refused(ControlsError),
}

impl ControlsRefusal {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::FixtureChanged => StatusCode::CONFLICT,
            Self::NoSuchFixture { .. }
            | Self::EmptyBatch
            | Self::TooManyChanges { .. }
            | Self::Refused(_) => StatusCode::BAD_REQUEST,
        }
    }

    /// A closed word for the audit line, so a log says why without carrying free text.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NoSuchFixture { .. } => "no_such_fixture",
            Self::FixtureChanged => "fixture_changed",
            Self::EmptyBatch => "empty_batch",
            Self::TooManyChanges { .. } => "too_many_changes",
            Self::Refused(ControlsError::UnknownStatus(_)) => "unknown_status",
            Self::Refused(ControlsError::EmptyFixtureName) => "empty_fixture_name",
            Self::Refused(ControlsError::TransitionTooLong(_)) => "transition_too_long",
        }
    }
}

impl fmt::Display for ControlsRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFixture { name, available } => write!(
                formatter,
                "there is no fixture '{name}'; the fixtures are: {}",
                if available.is_empty() {
                    "none".to_owned()
                } else {
                    available.join(", ")
                }
            ),
            Self::FixtureChanged => {
                formatter.write_str("the fixtures changed while this was being checked, try again")
            }
            Self::EmptyBatch => formatter.write_str("a batch needs at least one change"),
            Self::TooManyChanges { given } => write!(
                formatter,
                "a batch holds at most {MAXIMUM_BATCH_CHANGES} changes, not {given}"
            ),
            Self::Refused(error) => write!(formatter, "{error}"),
        }
    }
}

/// Why a change of the scene was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneRefusal {
    Refused(SceneError),
    /// The fixture shown changed (another was chosen, or its file was written or removed) between
    /// reading it and applying the change.
    FixtureChanged,
}

impl SceneRefusal {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Refused(_) => StatusCode::BAD_REQUEST,
            Self::FixtureChanged => StatusCode::CONFLICT,
        }
    }

    /// A closed word for the audit line, so a log says why without carrying free text.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::FixtureChanged => "fixture_changed",
            Self::Refused(SceneError::Empty(_)) => "empty",
            Self::Refused(SceneError::TooLong { .. }) => "too_long",
            Self::Refused(SceneError::DuplicateMember(_)) => "duplicate_member",
            Self::Refused(SceneError::UnknownMember(_)) => "unknown_member",
            Self::Refused(SceneError::TooManyMembers) => "too_many_members",
            Self::Refused(SceneError::TooManyChoices { .. }) => "too_many_choices",
            Self::Refused(SceneError::BadVolume(_)) => "bad_volume",
            Self::Refused(SceneError::BadRotation(_)) => "bad_rotation",
            Self::Refused(SceneError::BadSlide | SceneError::SlideNeedsTheCard) => "bad_slide",
            Self::Refused(SceneError::BadZoom(_)) => "bad_zoom",
            Self::Refused(SceneError::BadCameraTime(_)) => "bad_camera_time",
            Self::Refused(_) => "invalid_scene_change",
        }
    }
}

impl fmt::Display for SceneRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => write!(formatter, "{error}"),
            Self::FixtureChanged => {
                formatter.write_str("the scene changed while this was being checked, try again")
            }
        }
    }
}

/// What is shown, and which fixture it is built on.
#[derive(Debug)]
struct Shown {
    reply: ControlsReply,
    /// Rises whenever the fixture shown, or any fixture file, changes. A change that was checked
    /// against the files names the generation it read them at.
    generation: u64,
}

/// What the playground is showing, the pages that are open, and where its files are.
#[derive(Debug)]
pub struct PlaygroundState {
    files: PlaygroundFiles,
    shown: Mutex<Shown>,
    clients: ClientTable,
    /// When this run of the host began, so a move can say how long ago it started.
    started: Instant,
}

impl PlaygroundState {
    pub fn new(files: PlaygroundFiles, boot_id: String) -> Self {
        Self {
            files,
            shown: Mutex::new(Shown {
                reply: ControlsReply {
                    revision: 0,
                    boot_id,
                    controls: Controls::default(),
                    scene: SceneOverlay::default(),
                },
                generation: 0,
            }),
            clients: ClientTable::default(),
            started: Instant::now(),
        }
    }

    /// Milliseconds since this run of the host began.
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn lock(&self) -> MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn files(&self) -> &PlaygroundFiles {
        &self.files
    }

    /// Remembers what a page said. See `ClientTable::record`.
    pub fn record_client(&self, client: String, report: ClientReport) {
        self.clients.record(client, report);
    }

    /// The pages heard from, by name, with how long ago each last spoke.
    pub fn clients(&self) -> Vec<ClientView> {
        self.clients_at(Instant::now())
    }

    /// `clients` as of `now`, which tests move forward.
    pub fn clients_at(&self, now: Instant) -> Vec<ClientView> {
        self.clients.views_at(now)
    }

    pub fn revision(&self) -> u64 {
        self.lock().reply.revision
    }

    /// What is shown now, as it is, without looking at the fixture files, with each move saying how
    /// long ago it began as of this moment. The frame and `controls.json` both answer with this.
    pub fn snapshot(&self) -> ControlsReply {
        self.lock().reply.at(self.now_ms())
    }

    /// The generation of the fixture files as the playground knows them. Read it before reading
    /// the file names, and pass it with them to `change_all`.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// The fixture a scene change would sit on and its generation, or `None` while no fixture is
    /// shown.
    pub fn scene_target(&self) -> Option<(String, u64)> {
        let shown = self.lock();
        let name = shown.reply.controls.fixture.clone()?;
        Some((name, shown.generation))
    }

    /// Says that a fixture file was written or removed, so a change that read the old files is
    /// refused.
    pub fn fixture_files_changed(&self) {
        self.lock().generation += 1;
    }

    /// Makes one change to the live scene on top of `base`, the fixture shown at `generation`
    /// (from `scene_target`). A change that alters nothing does not count and `on_applied` is not
    /// called; otherwise it is called with the lock held, so its audit line is in revision order.
    pub fn change_scene(
        &self,
        change: SceneChange,
        base: &Fixture,
        generation: u64,
        on_applied: impl FnOnce(&ControlsReply),
    ) -> Result<ControlsReply, SceneRefusal> {
        let mut shown = self.lock();
        if shown.generation != generation {
            return Err(SceneRefusal::FixtureChanged);
        }
        let mut next = shown.reply.scene.clone();
        next.apply_at(change, base, self.now_ms())
            .map_err(SceneRefusal::Refused)?;
        if next != shown.reply.scene {
            shown.reply.scene = next;
            shown.reply.revision += 1;
            on_applied(&shown.reply);
        }
        Ok(shown.reply.at(self.now_ms()))
    }

    /// Replaces the fixture shown by the first of `names` when the chosen one is missing from
    /// them (or none is chosen and one exists). That replacement counts as a change: `on_applied`
    /// is called with the lock held and the reply says what is shown afterwards; `None` means
    /// nothing needed replacing.
    pub fn reconcile(
        &self,
        names: &[String],
        on_applied: impl FnOnce(&ControlsReply),
    ) -> Option<ControlsReply> {
        let mut shown = self.lock();
        let present = shown
            .reply
            .controls
            .fixture
            .as_ref()
            .is_some_and(|chosen| names.contains(chosen));
        if present {
            return None;
        }
        let first = names.first().cloned();
        if shown.reply.controls.fixture == first {
            return None;
        }
        shown.reply.controls.fixture = first;
        shown.reply.scene = shown.reply.scene.cleared();
        shown.reply.revision += 1;
        shown.generation += 1;
        on_applied(&shown.reply);
        Some(shown.reply.at(self.now_ms()))
    }

    /// Pulls the camera back to the whole stage when it looks at an element that is among `gone`
    /// (despawned or cleared), as one numbered change. `on_applied` is called with the lock held,
    /// the name of the element the camera had, and the reply; `None` means the camera was not
    /// looking at any of them.
    pub fn release_camera_from(
        &self,
        gone: &[String],
        on_applied: impl FnOnce(&ControlsReply, &str),
    ) -> Option<ControlsReply> {
        let mut shown = self.lock();
        let target = shown
            .reply
            .scene
            .camera
            .as_ref()
            .and_then(|camera| camera.target.clone())
            .filter(|target| gone.contains(target))?;
        let now = self.now_ms();
        shown.reply.scene.camera_count += 1;
        let serial = shown.reply.scene.camera_count;
        shown.reply.scene.camera = Some(Camera {
            target: None,
            zoom_percent: MINIMUM_ZOOM_PERCENT,
            duration_ms: 0,
            serial,
            began_ms: now,
            elapsed_ms: 0,
        });
        shown.reply.revision += 1;
        on_applied(&shown.reply, &target);
        Some(shown.reply.at(now))
    }

    /// The name of the element the camera looks at, if it looks at one. A cheap look, so a
    /// despawn can skip collecting names when there is nothing to release.
    pub fn camera_target(&self) -> Option<String> {
        self.lock()
            .reply
            .scene
            .camera
            .as_ref()
            .and_then(|camera| camera.target.clone())
    }

    /// Makes several changes as one: if any is refused none is applied, and the whole is one
    /// numbered change. `names` are the fixture files as read at `generation` (from `generation`
    /// before the read); a choice of fixture is refused when the files changed since, or when the
    /// name is not among them. An empty batch and one with more than `MAXIMUM_BATCH_CHANGES`
    /// changes are refused. A batch that alters nothing does not count and `on_applied` is not
    /// called; otherwise it is called with the lock held, so its audit line is in revision order.
    pub fn change_all(
        &self,
        changes: Vec<ControlsChange>,
        names: &[String],
        generation: u64,
        on_applied: impl FnOnce(&ControlsReply),
    ) -> Result<ControlsReply, ControlsRefusal> {
        if changes.is_empty() {
            return Err(ControlsRefusal::EmptyBatch);
        }
        if changes.len() > MAXIMUM_BATCH_CHANGES {
            return Err(ControlsRefusal::TooManyChanges {
                given: changes.len(),
            });
        }
        let mut shown = self.lock();
        let chooses_fixture = changes
            .iter()
            .any(|change| matches!(change, ControlsChange::Fixture(_)));
        if chooses_fixture && shown.generation != generation {
            return Err(ControlsRefusal::FixtureChanged);
        }
        for change in &changes {
            if let ControlsChange::Fixture(name) = change
                && !names.contains(name)
            {
                return Err(ControlsRefusal::NoSuchFixture {
                    name: name.clone(),
                    available: names.to_vec(),
                });
            }
        }
        let mut next = shown.reply.controls.clone();
        for change in changes {
            next.apply(change).map_err(ControlsRefusal::Refused)?;
        }
        if next != shown.reply.controls {
            // Another scenario starts from its own file, without the last one's live changes.
            if next.fixture != shown.reply.controls.fixture {
                shown.reply.scene = shown.reply.scene.cleared();
                shown.generation += 1;
            }
            shown.reply.controls = next;
            shown.reply.revision += 1;
            on_applied(&shown.reply);
        }
        Ok(shown.reply.at(self.now_ms()))
    }
}
