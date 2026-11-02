//! The page's copy of the state: the loaded fixture, the controls, the live canvas, what is shown
//! and what was drawn, and the typed reasons something could not be read.
//!
//! The host owns the controls, so the dev CLI and this page change the same thing; this is only
//! the page's copy of what the host last said, and it never decides wording, timing or geometry.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use wasm_bindgen::JsValue;

use super::page::Page;
use crate::continuity::ReplyGate;
use crate::controls::{Controls, Transition};
use crate::draw_plan::Snapshot;
use crate::fixture::{ElementSource, Fixture, FixtureError, StageElement};
use crate::frame::LiveCanvas;
use crate::net::NetError;
use crate::scene::SceneOverlay;

pub const REVISION_ADDRESS: &str = "/playground/revision";
pub const THEME_ADDRESS: &str = "/playground/theme.css";
pub const FIXTURE_LIST_ADDRESS: &str = "/playground/fixtures.json";
pub const FIXTURE_DIRECTORY_ADDRESS: &str = "/playground/fixtures/";
pub const FRAME_ADDRESS: &str = "/playground/frame";
pub const CONTROLS_ADDRESS: &str = "/playground/controls.json";
pub const CANVAS_ADDRESS: &str = "/canvas.json";
pub const CHANGE_ADDRESS: &str = "/dev/playground/controls";
pub const REPORT_ADDRESS: &str = "/dev/playground/report";
pub const ROOT_ID: &str = "playground";
pub const THEME_ID: &str = "playground-theme";

/// Why the page cannot show what the host says: the host cannot be reached, or it answered with
/// something the page cannot read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    Net(NetError),
    /// An answer that is not what the address promises.
    NotRead {
        address: &'static str,
        wanted: &'static str,
        reason: String,
    },
    /// The page could not be drawn or changed.
    Dom(String),
}

impl fmt::Display for PageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(error) => write!(formatter, "{error}"),
            Self::NotRead {
                address,
                wanted,
                reason,
            } => write!(formatter, "{address} is not {wanted}: {reason}"),
            Self::Dom(message) => formatter.write_str(message),
        }
    }
}

impl From<NetError> for PageError {
    fn from(error: NetError) -> Self {
        Self::Net(error)
    }
}

impl From<JsValue> for PageError {
    fn from(error: JsValue) -> Self {
        Self::Dom(crate::dom::js_message(&error))
    }
}

/// Why a fixture could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureProblem {
    Fetch(NetError),
    /// The file was read and is not a valid scenario.
    Invalid {
        name: String,
        error: FixtureError,
    },
}

impl fmt::Display for FixtureProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(error) => write!(formatter, "{error}"),
            Self::Invalid { name, error } => write!(
                formatter,
                "{name}, line {}, column {}: {}",
                error.line, error.column, error.message
            ),
        }
    }
}

#[derive(Default)]
pub struct App {
    pub fixture_names: Vec<String>,
    /// The name of the fixture in `fixture`, so a change of choice is noticed.
    pub loaded_name: Option<String>,
    /// Set when the scene has just changed and the next draw should leave the old one with a
    /// transition.
    pub pending_transition: Option<Transition>,
    /// The loaded fixture, or why it could not be read.
    pub fixture: Option<Result<Fixture, FixtureProblem>>,
    /// The loaded fixture with the scene's live changes drawn on it, made when either changes and
    /// shared with what was drawn, so a frame does not copy a fixture.
    pub shown: Option<Rc<Result<Fixture, String>>>,
    pub live: Option<LiveCanvas>,
    /// The run of the host `live` came from.
    pub live_boot: String,
    /// The count of the newest frame drawn, which the next frame request names.
    pub frame: Option<u64>,
    /// This page's name for itself in reports to the host.
    pub client_id: String,
    /// The text of the last report sent, so an unchanged report is not sent again.
    pub last_report: Option<String>,
    pub polls_since_report: u32,
    /// How many times in a row the host could not be reached, for the retry wait.
    pub failures: u32,
    pub asset_revision: String,
    pub controls: Controls,
    /// Live changes to the scene, drawn on top of the loaded fixture.
    pub scene: SceneOverlay,
    /// Decides which answers of the host are taken: it knows the run and the count of changes of
    /// the answer `controls` came from.
    pub gate: ReplyGate,
    /// Set while the server cannot be reached.
    pub connection_problem: Option<PageError>,
    /// What the page is made from as drawn, to tell what a change touches.
    pub drawn: Option<Snapshot>,
    /// The pieces of the page, once it has been drawn.
    pub page: Option<Page>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
}

pub fn with_app<R>(read: impl FnOnce(&App) -> R) -> R {
    APP.with(|app| read(&app.borrow()))
}

pub fn with_app_mut<R>(change: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|app| change(&mut app.borrow_mut()))
}

pub fn log_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

/// Shows a problem in the notes until the next good answer.
pub fn set_problem(error: impl Into<PageError>) {
    let error = error.into();
    with_app_mut(|app| app.connection_problem = Some(error));
}

impl App {
    /// Makes `shown` from the loaded fixture and the scene's changes. Call it whenever either
    /// changes.
    pub fn reshow(&mut self) {
        self.shown = self.fixture.as_ref().map(|loaded| {
            Rc::new(match loaded {
                Ok(fixture) => {
                    let mut fixture = fixture.clone();
                    self.scene.apply_to(&mut fixture);
                    Ok(fixture)
                }
                Err(problem) => Err(problem.to_string()),
            })
        });
    }

    /// What the page is made from now.
    pub fn snapshot(&self, audio_unlocked: bool) -> Snapshot {
        Snapshot {
            names: self.fixture_names.clone(),
            controls: self.controls.clone(),
            loaded_name: self.loaded_name.clone(),
            shown: self.shown.clone(),
            connection_problem: self.connection_problem.as_ref().map(ToString::to_string),
            audio_unlocked,
        }
    }

    /// The elements the shared screen draws now: the scenario's own, or the live canvas's. `None`
    /// when the scenario wants the live canvas and none has arrived.
    pub fn stage_elements(&self) -> Option<&[StageElement]> {
        stage_elements_of(self.shown.as_deref(), self.live.as_ref())
    }
}

/// The elements the shared screen of `shown` draws: the scenario's own, or the live canvas's.
pub fn stage_elements_of<'a>(
    shown: Option<&'a Result<Fixture, String>>,
    live: Option<&'a LiveCanvas>,
) -> Option<&'a [StageElement]> {
    match shown {
        Some(Ok(fixture)) => match &fixture.tv.elements {
            ElementSource::Inline(inline) => Some(inline),
            ElementSource::Live(_) => live.map(|live| live.elements.as_slice()),
        },
        _ => None,
    }
}
