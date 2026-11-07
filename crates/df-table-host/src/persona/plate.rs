//! The plate of `persona converse plate=NAME`: a new tv backdrop made from what the characters
//! just said, put behind them while a spinner shows on the scene.
//!
//! A dev tool over routes that already exist, like the rest of `converse`: the image service
//! makes the picture (`media generate`), the imagery API finds the people in it (`media locate`),
//! the text service says which region is which character (`media locate identify`), and a scene
//! change puts the picture behind the figures. The prompt is built here from the table and the
//! latest lines, with no model call, so it is the same for the same talk. The title chip of the
//! scene is the spinner while the picture is made, and is put back whether the plate worked or not.

use df_playground::lobby::change::{RoleplayEdit, ScreenEdit};
use df_playground::lobby::model::SceneTitle;
use df_playground::overlay::Glyph;
use df_playground::scene::{ImageSlot, SceneChange};

use super::converse::{Table, Turn};
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};
use crate::imagegen::request::{GenerateReply, GenerateRequest, JobState};
use crate::imagegen::{Purpose, routes::path as image_path};
use crate::locate::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, LocateReply, LocateRequest, LocatedLayout,
};
use crate::locate::routes::path as locate_path;
use crate::movement::brief::BriefStyle;
use crate::playground::ChangeReply;
use crate::wire::{ControlsReply, route};

// TODO(DEV-12): tests for the prompt, the title put back after a failed step and the `plate=`
// option, and a live plate run once a dedicated signed-in Codex home exists (the image service
// refuses the user's own home: it has a config.toml).

/// The title of the scene while the picture is made.
const UPDATING_TITLE: &str = "Updating the scene...";
/// How many of the latest lines the picture is told.
const PICTURE_LINES: usize = 4;
/// The most characters of one line the picture is told.
const PICTURE_LINE_CHARS: usize = 300;

/// The step of the plate that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateStep {
    Generate,
    Locate,
    Identify,
    Backdrop,
}

impl PlateStep {
    pub fn key(self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::Locate => "locate",
            Self::Identify => "identify",
            Self::Backdrop => "backdrop",
        }
    }
}

/// The text the image service is asked to paint: the room, who sits in it, and what they have
/// just been talking about. It forbids every kind of writing, because the page draws the words.
pub(super) fn picture_prompt(table: &Table, history: &[Turn]) -> String {
    let people: Vec<String> = table
        .speakers()
        .iter()
        .map(|speaker| format!("{} ({})", speaker.display_name, speaker.job))
        .collect();
    let style = BriefStyle::default();
    let mut text = format!(
        concat!(
            "{} a lamplit tavern interior at night, warm candlelight and deep shadow, everyone ",
            "seated together at a long wooden table and facing the viewer. Seated, left to ",
            "right: {}. "
        ),
        style.direction,
        people.join("; ")
    );
    let skip = history.len().saturating_sub(PICTURE_LINES);
    if skip < history.len() {
        text.push_str("They have just been talking, and the scene shows what it was about:");
        for turn in &history[skip..] {
            let line: String = turn.line.chars().take(PICTURE_LINE_CHARS).collect();
            text.push_str(&format!(" \"{line}\""));
        }
        text.push(' ');
    }
    text.push_str("Show what they talk about as objects and mood in the room. ");
    text.push_str(&style.writing_ban);
    text
}

/// The step that failed and why.
pub(crate) type PlateFailure = (PlateStep, Box<DevError>);

fn at(step: PlateStep) -> impl FnOnce(DevError) -> PlateFailure {
    move |error| (step, Box::new(error))
}

/// A scene change sent to the host on `port`.
async fn change(port: u16, change: &SceneChange) -> Result<(), DevError> {
    let reply: ChangeReply = post_json(port, route::DEV_PLAYGROUND_SCENE, change).await?;
    reply_value(reply).map(|_| ())
}

fn title_change(title: &str, glyph: Glyph) -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::RoleplaySet {
            roleplay: RoleplayEdit {
                title: Some(title.to_owned()),
                icon: Some(glyph),
                ..RoleplayEdit::default()
            },
        },
    }
}

/// Takes the title bar away, for a scene that had none before the spinner.
fn title_removed() -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::RoleplaySet {
            roleplay: RoleplayEdit {
                title_remove: true,
                ..RoleplayEdit::default()
            },
        },
    }
}

/// The title bar the scene shows now through a live change, if one is.
async fn current_title(port: u16) -> Result<Option<SceneTitle>, DevError> {
    let controls: ControlsReply = get_json(port, route::PLAYGROUND_CONTROLS).await?;
    Ok(controls
        .scene
        .roleplay
        .set_value()
        .and_then(|card| card.title.clone()))
}

/// Makes the plate `name` from the talk and puts it behind the scene: the prompt is the talk's
/// (see `picture_prompt`) and the characters are the table's.
pub(super) async fn make_plate(
    port: u16,
    name: &str,
    table: &Table,
    history: &[Turn],
) -> Result<String, PlateFailure> {
    let characters = table
        .speakers()
        .iter()
        .map(|speaker| CharacterRequest {
            name: speaker.id.clone(),
            description: Some(speaker.job.clone()),
            portrait: None,
        })
        .collect();
    deliver_plate(port, name, &picture_prompt(table, history), characters).await
}

/// Makes the plate `name` from `prompt` and puts it behind the scene, with `characters` located and
/// named in it. The spinner title is shown from the start and the title that was there is put back
/// at the end, also when a step failed; the failure is that of the step, and a title that could not
/// be put back is not hidden by it.
pub(crate) async fn deliver_plate(
    port: u16,
    name: &str,
    prompt: &str,
    characters: Vec<CharacterRequest>,
) -> Result<String, PlateFailure> {
    let before = current_title(port).await.map_err(at(PlateStep::Generate))?;
    change(port, &title_change(UPDATING_TITLE, Glyph::Spinner))
        .await
        .map_err(at(PlateStep::Generate))?;
    let made = build(port, name, prompt, characters).await;
    let put_back = before.map_or_else(title_removed, |title| {
        title_change(&title.text, title.glyph)
    });
    let restored = change(port, &put_back).await;
    match (made, restored) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(failure), _) => Err(failure),
        (Ok(_), Err(error)) => Err(at(PlateStep::Backdrop)(error)),
    }
}

async fn build(
    port: u16,
    name: &str,
    prompt: &str,
    characters: Vec<CharacterRequest>,
) -> Result<String, PlateFailure> {
    let request = GenerateRequest {
        prompt: prompt.to_owned(),
        name: name.to_owned(),
        purpose: Some(Purpose::Plate),
        aspect: None,
        wait: true,
    };
    let generated: GenerateReply<crate::imagegen::request::JobView> =
        post_json(port, image_path::GENERATE, &request)
            .await
            .map_err(at(PlateStep::Generate))?;
    let job = reply_value(generated).map_err(at(PlateStep::Generate))?;
    let JobState::Stored { provenance, .. } = job.state else {
        return Err(at(PlateStep::Generate)(DevError::PlateNotStored {
            name: job.name,
        }));
    };
    let picture = provenance.media_file;
    let located: LocateReply<LocatedLayout> = post_json(
        port,
        locate_path::LOCATE,
        &LocateRequest {
            picture: picture.clone(),
            prompt: None,
            masks: None,
        },
    )
    .await
    .map_err(at(PlateStep::Locate))?;
    let located = reply_value(located).map_err(at(PlateStep::Locate))?;
    let identified: LocateReply<IdentifiedLayout> = post_json(
        port,
        locate_path::LOCATE_IDENTIFY,
        &IdentifyRequest {
            picture: picture.clone(),
            characters,
            min_confidence_permille: None,
        },
    )
    .await
    .map_err(at(PlateStep::Identify))?;
    let identified = reply_value(identified).map_err(at(PlateStep::Identify))?;
    change(
        port,
        &SceneChange::Image {
            slot: ImageSlot::TvBackdrop,
            media: picture.clone(),
            caption: None,
            rotation_degrees: 0,
            slide: None,
            layout: None,
        },
    )
    .await
    .map_err(at(PlateStep::Backdrop))?;
    let named = identified
        .rows
        .iter()
        .filter(|row| row.region.is_some() && row.outcome.key() == "named")
        .count();
    Ok(format!(
        "plate {name}: {picture} is behind the scene ({} regions found, {named} of {} characters named)",
        located.layout.regions().len(),
        identified.rows.len()
    ))
}
