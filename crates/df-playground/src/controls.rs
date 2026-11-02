//! What the person (or the dev CLI) has chosen for the playground: which scenario, which devices,
//! the reading pace, a status to try, and two display toggles.
//!
//! It is plain data plus the one rule for changing it, shared by the page and the host so both
//! agree on what a change means. It never draws and never reads the page.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::copy::{ReadingPace, status_from_key};
use crate::fixture::Status;

/// The key of the menu entry that keeps the fixture's own status.
pub const FIXTURE_STATUS_KEY: &str = "fixture";

/// Which devices are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Both,
    Tv,
    Phone,
}

impl Device {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "both" => Some(Self::Both),
            "tv" => Some(Self::Tv),
            "phone" => Some(Self::Phone),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Tv => "tv",
            Self::Phone => "phone",
        }
    }

    pub fn shows_tv(self) -> bool {
        matches!(self, Self::Both | Self::Tv)
    }

    pub fn shows_phone(self) -> bool {
        matches!(self, Self::Both | Self::Phone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// The fixture file shown, or none while no fixture exists.
    pub fixture: Option<String>,
    pub device: Device,
    pub pace: ReadingPace,
    /// A status key from the menu, or `FIXTURE_STATUS_KEY` to use the fixture's status.
    pub status_key: String,
    pub show_unavailable: bool,
    pub show_grid: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            fixture: None,
            device: Device::Both,
            pace: ReadingPace::Normal,
            status_key: FIXTURE_STATUS_KEY.to_owned(),
            show_unavailable: false,
            show_grid: false,
        }
    }
}

/// The controls and how many changes they have had, as the host answers: a reader that has the
/// same revision has nothing new to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlsReply {
    pub revision: u64,
    pub controls: Controls,
}

/// One change to the controls, as the page and the dev CLI both send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", content = "value", rename_all = "snake_case")]
pub enum ControlsChange {
    Fixture(String),
    Device(Device),
    Pace(ReadingPace),
    /// A status key (see `copy::STATUS_KEYS`) or `FIXTURE_STATUS_KEY`.
    Status(String),
    ShowUnavailable(bool),
    ShowGrid(bool),
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlsError {
    UnknownStatus(String),
    EmptyFixtureName,
}

impl fmt::Display for ControlsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStatus(key) => write!(
                formatter,
                "'{key}' is not a status; use '{FIXTURE_STATUS_KEY}' or one of {}",
                crate::copy::STATUS_KEYS.join(", ")
            ),
            Self::EmptyFixtureName => formatter.write_str("a fixture name cannot be empty"),
        }
    }
}

impl Controls {
    /// The status to draw: the chosen one, or the fixture's own when none is chosen.
    pub fn status_for(&self, fixture_status: &Status) -> Status {
        status_from_key(&self.status_key).unwrap_or_else(|| fixture_status.clone())
    }

    /// Makes one change. A refused change leaves the controls as they were.
    pub fn apply(&mut self, change: ControlsChange) -> Result<(), ControlsError> {
        match change {
            ControlsChange::Fixture(name) => {
                if name.is_empty() {
                    return Err(ControlsError::EmptyFixtureName);
                }
                self.fixture = Some(name);
            }
            ControlsChange::Device(device) => self.device = device,
            ControlsChange::Pace(pace) => self.pace = pace,
            ControlsChange::Status(key) => {
                if key != FIXTURE_STATUS_KEY && status_from_key(&key).is_none() {
                    return Err(ControlsError::UnknownStatus(key));
                }
                self.status_key = key;
            }
            ControlsChange::ShowUnavailable(shown) => self.show_unavailable = shown,
            ControlsChange::ShowGrid(shown) => self.show_grid = shown,
        }
        Ok(())
    }
}
