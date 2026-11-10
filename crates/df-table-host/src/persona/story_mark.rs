//! The story's mark on the shared screen: its title in the chip at the top right and, under it,
//! the place the party has reached out of the story's places (`kit::title_chip`,
//! `kit::progress_line`). The host words it from what the table may see: the title, how many
//! places the story has, which one the party is at and that place's name; on the way between two
//! places it says so and names neither, and the places ahead stay unnamed. A scene's danger,
//! dressing and beats never reach it.
//!
//! The count of places is shown: a story's places are the ground its party walks, and the count
//! says nothing of what happened, who did it or which clue goes with which.
//!
//! Audit lines: `PERSONA story_mark places=.. at=.. way=..` and, when the screen refuses it,
//! `PERSONA story_mark_refused status=..`; no story text reaches a log line.

use df_playground::overlay::{Progress, StoryMark};
use df_playground::scene::SceneChange;

use super::story::Story;
use super::story_way::PlayPhase;
use crate::state::AppState;

/// What the progress line says while the party is on the way between two places.
const ON_THE_WAY: &str = "On the way";

/// The mark of `story` at its phase: the title alone before play begins, then the place the party
/// is at (or left, while on the way) out of the story's places. A phase naming a place the story
/// does not have shows the title alone.
pub(super) fn story_mark(story: &Story) -> StoryMark {
    let progress = story.phase.as_ref().and_then(|phase| {
        let index = story
            .scenes
            .iter()
            .position(|scene| scene.id == phase.place())?;
        let name = match phase {
            PlayPhase::Place { .. } => story.scenes.get(index)?.place.clone(),
            PlayPhase::Way { .. } => ON_THE_WAY.to_owned(),
        };
        Some(Progress {
            index: u16::try_from(index + 1).ok()?,
            total: u16::try_from(story.scenes.len()).ok()?,
            title: Some(name),
        })
    });
    StoryMark {
        title: story.title.clone(),
        progress,
    }
}

/// Puts the mark of `story` on the shared screen. A screen that refuses it loses nothing the
/// story keeps: the next phase shows it again.
pub(super) async fn show_story_mark(app: &AppState, story: &Story) {
    let mark = story_mark(story);
    let at = mark.progress.as_ref().map_or(0, |progress| progress.index);
    let line = format!(
        "PERSONA story_mark places={} at={at} way={}",
        story.scenes.len(),
        matches!(story.phase, Some(PlayPhase::Way { .. }))
    );
    let changes = vec![SceneChange::Story { mark }];
    match crate::playground::apply_scene_changes(app, changes).await {
        Ok(_) => app.record_line(line),
        Err(error) => app.record_line(format!(
            "PERSONA story_mark_refused status={}",
            error.status.as_u16()
        )),
    }
}
