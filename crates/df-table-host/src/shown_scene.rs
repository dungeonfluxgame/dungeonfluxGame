//! The lookup the cast and persona routes share: the backdrop picture the page shows, its current
//! layout, and a character named in it.
//!
//! A route that works by character name resolves the layout once ([`shown_cast`]) and checks the
//! name against it ([`named`]); the refusal is a [`ShownSceneError`] that the route's own error
//! wraps, naming the characters the layout does have.

use std::fmt;

use axum::http::StatusCode;
use df_playground::fixture::{Fixture, PlateKind};
use df_scene_layout::{SceneLayout, SceneRegion};
use serde::{Deserialize, Serialize};

use crate::dev_http::RouteError;
use crate::locate::{LocateError, shown_backdrop};
use crate::playground::{PlaygroundChangeError, SceneApplyError};
use crate::state::AppState;

/// Why the shown scene could not give the character asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ShownSceneError {
    /// The shown scene could not be read, or refused the change.
    Scene { cause: PlaygroundChangeError },
    /// The layout of the shown backdrop could not be read.
    Layout { cause: LocateError },
    /// The shown scene has no backdrop picture.
    NoBackdrop,
    /// The shown backdrop has no current layout.
    NoLayout { picture: String },
    /// `character` is not a named character of `picture`'s layout, whose named ones are `names`.
    UnknownCharacter {
        character: String,
        picture: String,
        names: Vec<String>,
    },
}

impl ShownSceneError {
    /// The error's serde tag, for the audit line.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::Scene { .. } => "scene",
            Self::Layout { .. } => "layout",
            Self::NoBackdrop => "no_backdrop",
            Self::NoLayout { .. } => "no_layout",
            Self::UnknownCharacter { .. } => "unknown_character",
        }
    }
}

impl fmt::Display for ShownSceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene { cause } => write!(formatter, "{cause}"),
            Self::Layout { cause } => write!(formatter, "{cause}"),
            Self::NoBackdrop => formatter.write_str("the shown scene has no backdrop picture"),
            Self::NoLayout { picture } => write!(
                formatter,
                concat!(
                    "the shown scene's backdrop ({0}) has no current layout; ",
                    "scan it with: media locate {0}"
                ),
                picture
            ),
            Self::UnknownCharacter {
                character,
                picture,
                names,
            } => write!(
                formatter,
                "no character is named {character:?} in {picture}'s layout; the named ones are: {}",
                if names.is_empty() {
                    "(none: name them with media locate label or media locate identify)".to_owned()
                } else {
                    names.join(", ")
                }
            ),
        }
    }
}

impl std::error::Error for ShownSceneError {}

impl From<LocateError> for ShownSceneError {
    fn from(cause: LocateError) -> Self {
        Self::Layout { cause }
    }
}

impl From<SceneApplyError> for ShownSceneError {
    fn from(error: SceneApplyError) -> Self {
        Self::Scene {
            cause: PlaygroundChangeError::from(error),
        }
    }
}

impl RouteError for ShownSceneError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Scene { cause } => cause.status(),
            Self::Layout { cause } => cause.status(),
            Self::NoBackdrop | Self::NoLayout { .. } | Self::UnknownCharacter { .. } => {
                StatusCode::NOT_FOUND
            }
        }
    }
}

/// What the page shows, read once: the fixture, its backdrop picture, the layout it is shown with
/// and what kind of plate it is.
pub(crate) struct ShownCast {
    pub(crate) fixture: Fixture,
    pub(crate) picture: String,
    pub(crate) layout: SceneLayout,
    pub(crate) plate: PlateKind,
}

/// The fixture the page shows, its backdrop picture and the picture's layout
/// (`locate::shown_backdrop`).
pub(crate) async fn shown_cast(app: &AppState) -> Result<ShownCast, ShownSceneError> {
    let (fixture, _) = crate::playground::shown_fixture(app).await?;
    let backdrop = shown_backdrop(app, &fixture)
        .await?
        .ok_or(ShownSceneError::NoBackdrop)?;
    let picture = backdrop.picture;
    match backdrop.layout {
        Some(layout) => Ok(ShownCast {
            fixture,
            picture,
            layout,
            plate: backdrop.plate,
        }),
        None => Err(ShownSceneError::NoLayout { picture }),
    }
}

/// That `character` is a named character of `layout`, or the error naming the ones it has.
pub(crate) fn named<'a>(
    layout: &'a SceneLayout,
    picture: &str,
    character: &str,
) -> Result<&'a SceneRegion, ShownSceneError> {
    layout
        .region(character)
        .ok_or_else(|| ShownSceneError::UnknownCharacter {
            character: character.to_owned(),
            picture: picture.to_owned(),
            names: layout.names().into_iter().map(str::to_owned).collect(),
        })
}
