//! The range checks of the narration box and the seat strip, for a scene file.
//!
//! Each function walks one component and returns every value that is out of range with the path to
//! it, so a scene file points at the line. The limits are the very large ones in `limits` and the
//! domain's own (a scene is counted from 1 up to its total). A check never changes a value and
//! never draws.

use crate::checks;
use crate::fixture_checks::Step;
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_PARTY_MEMBERS};
use crate::overlay::{NarrationBox, SeatStrip};
use crate::scene::SceneError;

/// A value that is out of range, where it is and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) path: Vec<Step>,
    pub(crate) error: SceneError,
}

fn at(root: &[Step], more: &[Step]) -> Vec<Step> {
    root.iter().chain(more).copied().collect()
}

fn leaf(found: &mut Vec<Found>, root: &[Step], key: &'static str, result: Result<(), SceneError>) {
    if let Err(error) = result {
        found.push(Found {
            path: at(root, &[Step::Key(key)]),
            error,
        });
    }
}

fn optional(
    found: &mut Vec<Found>,
    root: &[Step],
    key: &'static str,
    what: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        leaf(found, root, key, checks::labelled(what, value));
    }
}

/// The problems of a narration box, at `root`.
pub(crate) fn narration(root: &[Step], narration: &NarrationBox) -> Vec<Found> {
    let mut found = Vec::new();
    leaf(
        &mut found,
        root,
        "text",
        checks::text("the narration", &narration.text, MAXIMUM_CAPTION_CHARACTERS),
    );
    optional(
        &mut found,
        root,
        "speaker",
        "the speaker",
        narration.speaker.as_deref(),
    );
    optional(
        &mut found,
        root,
        "continue_label",
        "the continue label",
        narration.continue_label.as_deref(),
    );
    if let Some(progress) = &narration.progress {
        let here = at(root, &[Step::Key("progress")]);
        if progress.index == 0 || progress.index > progress.total {
            found.push(Found {
                path: here.clone(),
                error: SceneError::BadProgress {
                    index: progress.index,
                    total: progress.total,
                },
            });
        }
        optional(
            &mut found,
            &here,
            "title",
            "the scene's name",
            progress.title.as_deref(),
        );
    }
    found
}

/// The problems of a seat strip, at `root`.
pub(crate) fn seat_strip(root: &[Step], strip: &SeatStrip) -> Vec<Found> {
    let mut found = Vec::new();
    let looks = at(root, &[Step::Key("looks")]);
    if strip.looks.len() > MAXIMUM_PARTY_MEMBERS {
        found.push(Found {
            path: looks.clone(),
            error: SceneError::TooMany {
                what: "the seat strip",
                limit: MAXIMUM_PARTY_MEMBERS,
            },
        });
    }
    for (index, look) in strip.looks.iter().enumerate() {
        let here = at(&looks, &[Step::Index(index)]);
        leaf(
            &mut found,
            &here,
            "name",
            checks::labelled("a hero's name", &look.name),
        );
        optional(&mut found, &here, "class", "a class", look.class.as_deref());
        optional(
            &mut found,
            &here,
            "portrait",
            "a media name",
            look.portrait.as_deref(),
        );
    }
    found
}

/// The first problem of a list, as the error a live change is refused with.
pub(crate) fn first_error(found: Vec<Found>) -> Result<(), SceneError> {
    found
        .into_iter()
        .next()
        .map_or(Ok(()), |problem| Err(problem.error))
}
