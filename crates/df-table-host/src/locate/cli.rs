//! `media locate` on the dev command line: its grammar, sending the request, and printing the
//! layout as one line per region. It never starts a server.
//!
//! `media locate <picture> [prompt=person|head] [masks=1-32]`,
//! `media locate show <picture>`, `media locate points <picture>`,
//! `media locate identify <picture> <name>[=<description>] ... [portrait.<name>=<file>]
//! [min=0-1000]`, `media locate label <picture> <region> <name|->`.

use df_scene_layout::{SceneLayout, percent_text};

use super::request::{
    CharacterRequest, IdentifiedLayout, IdentifyRequest, IdentityOutcome, LabelRequest,
    LocateReply, LocateRequest, LocatedLayout, ScanPrompt, StoredLayout,
};
use super::routes::path;
use crate::command::{Command, CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

pub const MEDIA_LOCATE_USAGE: &str = concat!(
    "media locate <picture> [prompt=person|head] [masks=1-32]",
    " | media locate show <picture>",
    " | media locate points <picture>",
    " | media locate identify <picture> <name>[=<description>] ... [portrait.<name>=<file>]",
    " [min=0-1000]",
    " | media locate label <picture> <number|id|name> <name|->"
);

/// A `media locate` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaLocate {
    /// Scan a picture and store its layout.
    Scan {
        picture: String,
        prompt: Option<ScanPrompt>,
        masks: Option<u32>,
    },
    /// Print a picture's stored layout.
    Show { picture: String },
    /// Name a region of a picture's layout (by number, id or current name), or clear its name.
    Label {
        picture: String,
        region: String,
        label: Option<String>,
    },
    /// Print every character point and camera shot of a picture's named characters.
    Points { picture: String },
    /// Name a picture's regions by asking a vision model which is which character.
    Identify {
        picture: String,
        characters: Vec<CharacterRequest>,
        min_confidence_permille: Option<u16>,
    },
}

/// Reads the words after `media locate`.
pub fn parse(words: &[String]) -> Result<Command, CommandError> {
    let command = match words {
        [verb, picture] if verb == "show" => MediaLocate::Show {
            picture: picture.clone(),
        },
        [verb, picture, rest @ ..] if verb == "identify" => parse_identify(picture, rest)?,
        [verb, picture] if verb == "points" => MediaLocate::Points {
            picture: picture.clone(),
        },
        [verb, picture, region, label] if verb == "label" => MediaLocate::Label {
            picture: picture.clone(),
            region: region.clone(),
            label: (label != "-").then(|| label.clone()),
        },
        [picture, options @ ..]
            if !matches!(picture.as_str(), "show" | "label" | "points" | "identify") =>
        {
            parse_scan(picture, options)?
        }
        _ => return Err(CommandError::Usage(MEDIA_LOCATE_USAGE)),
    };
    Ok(Command::MediaLocate(command))
}

/// Reads `media locate <picture> [prompt=person|head] [masks=N]`.
fn parse_scan(picture: &str, options: &[String]) -> Result<MediaLocate, CommandError> {
    let (mut prompt, mut masks) = (None, None);
    for option in options {
        match option.split_once('=') {
            Some(("prompt", value)) if prompt.is_none() => {
                prompt = Some(
                    ScanPrompt::from_key(value)
                        .ok_or_else(|| bad_choice("prompt", value, "person or head"))?,
                );
            }
            Some(("masks", value)) if masks.is_none() => {
                masks = Some(value.parse::<u32>().map_err(|_| {
                    bad_choice(
                        "masks",
                        value,
                        format!(
                            "a whole number from 1 to {}",
                            df_imagery::oneshot_segment::MAXIMUM_MASKS
                        ),
                    )
                })?);
            }
            Some((key @ ("prompt" | "masks"), _)) => {
                return Err(CommandError::DuplicateKey(key.to_owned()));
            }
            _ => return Err(CommandError::Usage(MEDIA_LOCATE_USAGE)),
        }
    }
    Ok(MediaLocate::Scan {
        picture: picture.to_owned(),
        prompt,
        masks,
    })
}

/// Reads `media locate identify <picture> <name>[=<description>] ... [portrait.<name>=<file>]
/// [min=0-1000]`: each character is a name with its look in words after `=`, a portrait is a
/// library picture of a character named in the same command, and `min` is how sure the model must
/// be, in thousandths (600 when not given).
fn parse_identify(picture: &str, words: &[String]) -> Result<MediaLocate, CommandError> {
    let mut characters: Vec<CharacterRequest> = Vec::new();
    let mut portraits: Vec<(String, String)> = Vec::new();
    let mut min = None;
    for word in words {
        if let Some(value) = word.strip_prefix("min=") {
            if min.is_some() {
                return Err(CommandError::DuplicateKey("min".to_owned()));
            }
            min = Some(
                value
                    .parse::<u16>()
                    .ok()
                    .filter(|value| *value <= 1000)
                    .ok_or_else(|| bad_choice("min", value, "thousandths from 0 to 1000"))?,
            );
        } else if let Some(rest) = word.strip_prefix("portrait.") {
            let (name, file) = rest
                .split_once('=')
                .ok_or(CommandError::Usage(MEDIA_LOCATE_USAGE))?;
            portraits.push((name.to_owned(), file.to_owned()));
        } else {
            characters.push(parse_character(word, &characters)?);
        }
    }
    for (name, file) in portraits {
        let character = characters
            .iter_mut()
            .find(|character| character.name == name)
            .ok_or_else(|| {
                bad_choice("portrait", &name, "a character named in the same command")
            })?;
        character.portrait = Some(file);
    }
    if characters.is_empty() {
        return Err(CommandError::Usage(MEDIA_LOCATE_USAGE));
    }
    Ok(MediaLocate::Identify {
        picture: picture.to_owned(),
        characters,
        min_confidence_permille: min,
    })
}

/// Reads one `<name>[=<description>]` word, refusing a name already among `known`.
fn parse_character(
    word: &str,
    known: &[CharacterRequest],
) -> Result<CharacterRequest, CommandError> {
    let (name, description) = match word.split_once('=') {
        Some((name, description)) => (name, Some(description.trim().to_owned())),
        None => (word, None),
    };
    if name.is_empty() || known.iter().any(|character| character.name == name) {
        return Err(bad_choice(
            "character",
            name,
            "a name given once, such as innkeeper or innkeeper=bearded man in a vest",
        ));
    }
    Ok(CharacterRequest {
        name: name.to_owned(),
        description: description.filter(|text| !text.is_empty()),
        portrait: None,
    })
}

/// The address of `GET /dev/media/locate/show` for `picture`.
fn show_address(picture: &str) -> String {
    format!("{}?picture={}", path::LOCATE_SHOW, query_value(picture))
}

/// `text` percent-encoded for a query value: every byte but the unreserved ones is escaped.
fn query_value(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The value of a locate reply, or its refusal (the typed `LocateError`) as the command's failure.
fn reply_value<T>(reply: LocateReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

async fn show(port: u16, picture: &str) -> Result<StoredLayout, DevError> {
    reply_value(get_json(port, &show_address(picture)).await?)
}

/// Runs one `media locate` command against the host on `port`.
pub async fn run(port: u16, command: MediaLocate) -> Result<String, DevError> {
    match command {
        MediaLocate::Scan {
            picture,
            prompt,
            masks,
        } => {
            let request = LocateRequest {
                picture,
                prompt,
                masks,
            };
            let located: LocatedLayout =
                reply_value(post_json(port, path::LOCATE, &request).await?)?;
            Ok(format_located(&located))
        }
        MediaLocate::Points { picture } => Ok(format_points(&show(port, &picture).await?.layout)),
        MediaLocate::Show { picture } => Ok(format_stored(&show(port, &picture).await?)),
        MediaLocate::Identify {
            picture,
            characters,
            min_confidence_permille,
        } => {
            let request = IdentifyRequest {
                picture,
                characters,
                min_confidence_permille,
            };
            let identified: IdentifiedLayout =
                reply_value(post_json(port, path::LOCATE_IDENTIFY, &request).await?)?;
            Ok(format_identified(&identified))
        }
        MediaLocate::Label {
            picture,
            region,
            label,
        } => {
            let request = LabelRequest {
                picture,
                region,
                label,
            };
            let stored: StoredLayout =
                reply_value(post_json(port, path::LOCATE_LABEL, &request).await?)?;
            Ok(format_stored(&stored))
        }
    }
}

/// A duration in seconds with one decimal, as a person reads it.
fn seconds_text(duration_ms: u64) -> String {
    format!("{:.1} s", duration_ms as f64 / 1000.0)
}

fn format_located(located: &LocatedLayout) -> String {
    let layout = &located.layout;
    format!(
        "ok stored {}: {} {} region(s) of {} ({}x{}) by {} in {}{}\n{}",
        located.layout_file,
        layout.regions().len(),
        layout.prompt(),
        located.picture,
        layout.picture().width_px,
        layout.picture().height_px,
        layout.model(),
        seconds_text(located.duration_ms),
        if located.kept_labels > 0 {
            format!("; kept {} name(s)", located.kept_labels)
        } else {
            String::new()
        },
        format_regions(layout)
    )
}

fn format_stored(stored: &StoredLayout) -> String {
    let layout = &stored.layout;
    format!(
        "{} ({} of {}, prompt {}, {}x{})\n{}",
        stored.layout_file,
        layout.model(),
        stored.picture,
        layout.prompt(),
        layout.picture().width_px,
        layout.picture().height_px,
        format_regions(layout)
    )
}

fn outcome_text(outcome: IdentityOutcome) -> &'static str {
    match outcome {
        IdentityOutcome::Named => "-> named",
        IdentityOutcome::Unsure => "-> left as it was (below the confidence threshold)",
        IdentityOutcome::Conflict => {
            "-> left as it was (the model gave this region to two characters)"
        }
        IdentityOutcome::NotFound => "-> not in the picture",
    }
}

fn format_identified(identified: &IdentifiedLayout) -> String {
    let lines = identified
        .rows
        .iter()
        .map(|row| {
            format!(
                "  {}: {} (confidence {}) {}",
                row.name,
                row.region.map_or_else(
                    || "no region".to_owned(),
                    |region| format!("region {region}")
                ),
                percent_text(row.confidence_permille),
                outcome_text(row.outcome)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        concat!(
            "ok asked the text service ({}) about {} character(s) in {} ",
            "({}{})\n{}\n{}"
        ),
        identified.profile,
        identified.rows.len(),
        identified.picture,
        seconds_text(identified.duration_ms),
        if identified.replayed {
            ", a replay of an answer it already checked"
        } else {
            ""
        },
        lines,
        format_regions(&identified.layout)
    )
}

/// One line per region: number, name, box, head point and score, in percent of the picture.
pub fn format_regions(layout: &SceneLayout) -> String {
    if layout.regions().is_empty() {
        return format!("  no {} found", layout.prompt());
    }
    layout
        .regions()
        .iter()
        .enumerate()
        .map(|(number, region)| {
            format!(
                "  {number}: {} [{}] box {}..{} x {}..{} % head {},{} % score {}",
                region.label().unwrap_or("(unnamed)"),
                region.id(),
                percent_text(region.left_permille()),
                percent_text(region.right_permille()),
                percent_text(region.top_permille()),
                percent_text(region.bottom_permille()),
                percent_text(region.head_x_permille()),
                percent_text(region.head_y_permille()),
                percent_text(region.score_permille())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every character point and camera shot of each named character (each region, by number, when
/// none is named), in percent of the picture: what bubbles, markers and the camera use.
pub fn format_points(layout: &SceneLayout) -> String {
    use df_scene_layout::{
        CharacterPoint, Shot, point_of, region_target, shot_focus, shot_zoom_percent,
    };
    let named = layout
        .regions()
        .iter()
        .any(|region| region.label().is_some());
    let mut lines = Vec::new();
    for (number, region) in layout.regions().iter().enumerate() {
        if named && region.label().is_none() {
            continue;
        }
        let name = region
            .label()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("region {number} (unnamed)"));
        lines.push(format!("{name}:"));
        let points = CharacterPoint::ALL
            .into_iter()
            .map(|point| {
                let (x, y) = point_of(region, point);
                format!("{} {},{}", point.key(), percent_text(x), percent_text(y))
            })
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(format!("  points (%): {points}"));
        for shot in Shot::ALL {
            let (x, y) = shot_focus(region, shot);
            let target = region.label().map_or_else(
                || "(name it to aim the camera)".to_owned(),
                |label| region_target(label, Some(shot)),
            );
            lines.push(format!(
                "  shot {}: focus {},{} % zoom {}%  target {target}",
                shot.key(),
                percent_text(x),
                percent_text(y),
                shot_zoom_percent(region, shot)
            ));
        }
    }
    if lines.is_empty() {
        format!("no {} in the layout", layout.prompt())
    } else {
        lines.join("\n")
    }
}
