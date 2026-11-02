//! What the playground is showing, and how many changes it has had.
//!
//! One lock holds the controls, the live scene overlay, the revision and a fixture generation. A
//! change is checked and applied under that lock, so it is all or nothing, and a scene change that
//! was computed against a fixture that has since changed is refused instead of applied to the
//! wrong scene. Refusals are typed, so the routes answer with the right status and a bug is not
//! mistaken for a refusal.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use axum::http::StatusCode;
use df_playground::controls::{Controls, ControlsChange, ControlsError, ControlsReply};
use df_playground::fixture::Fixture;
use df_playground::media::ClientReport;
use df_playground::scene::{SceneChange, SceneError, SceneOverlay};

use super::clients::ClientTable;
use super::files::PlaygroundFiles;
use crate::wire::ClientView;

/// Why a change of the controls was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlsRefusal {
    /// The fixture chosen is not among the files; `available` are the ones that are.
    NoSuchFixture {
        name: String,
        available: Vec<String>,
    },
    Refused(ControlsError),
}

impl ControlsRefusal {
    pub fn status(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
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
    /// Rises whenever the fixture shown, or its file, changes. A scene change names the
    /// generation it was computed against.
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

    /// What is shown now, as it is, without looking at the fixture files. The frame and
    /// `controls.json` both answer with this, so they never disagree.
    pub fn snapshot(&self) -> ControlsReply {
        self.lock().reply.at(self.now_ms())
    }

    /// The fixture a scene change would sit on and its generation, or `None` while no fixture is
    /// shown.
    pub fn scene_target(&self) -> Option<(String, u64)> {
        let shown = self.lock();
        let name = shown.reply.controls.fixture.clone()?;
        Some((name, shown.generation))
    }

    /// Says that a fixture file was written or removed, so a scene change that read the old file
    /// is refused.
    pub fn fixture_files_changed(&self) {
        self.lock().generation += 1;
    }

    /// Makes one change to the live scene on top of `base`, the fixture shown at `generation`
    /// (from `scene_target`). A change that alters nothing does not count.
    pub fn change_scene(
        &self,
        change: SceneChange,
        base: &Fixture,
        generation: u64,
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
        }
        Ok(shown.reply.at(self.now_ms()))
    }

    /// Replaces the fixture shown by the first of `names` when the chosen one is missing from
    /// them (or none is chosen and one exists). That replacement counts as a change, and the
    /// reply says what is shown afterwards; `None` means nothing needed replacing.
    pub fn reconcile(&self, names: &[String]) -> Option<ControlsReply> {
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
        Some(shown.reply.at(self.now_ms()))
    }

    /// Makes one change.
    pub fn change(
        &self,
        change: ControlsChange,
        names: &[String],
    ) -> Result<ControlsReply, ControlsRefusal> {
        self.change_all(vec![change], names)
    }

    /// Makes several changes as one: if any is refused none is applied, and the whole is one
    /// numbered change. Choosing a fixture that does not exist is refused with the names that do.
    pub fn change_all(
        &self,
        changes: Vec<ControlsChange>,
        names: &[String],
    ) -> Result<ControlsReply, ControlsRefusal> {
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
        let mut shown = self.lock();
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
        }
        Ok(shown.reply.at(self.now_ms()))
    }
}
