//! The `persona perceive` and `persona thoughts` verbs of the dev command line: their grammar, the
//! request each sends, and the text of each answer.
//!
//! The text of an answer is the contract the market runner reads (`development/market-thread.sh`):
//! a header line, then one `see:` line for each thing seen, one `thing <id> (<name>): <look>` line
//! (the shape of the move's design lines) for each new thing now a thing of the place, and the
//! `thought:`, `attention:`, `feeling:`, `wants:` and `kept:` lines the answer has, each on one
//! line (a thought's own line breaks are folded to spaces).

use df_content::persona::PersonaId;

use super::super::cli::{flag, id_word};
use super::super::options::take_options;
use super::request::{
    Mode, PerceiveReply, PerceiveRequest, PerceiveSceneRequest, Perceived, SceneOutcome,
    ScenePerceived, Thoughts, ThoughtsQuery, Viewpoint,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json, reply_value};

/// `persona perceive`: what a character sees on screen and what they think.
pub const PERSONA_PERCEIVE_USAGE: &str = concat!(
    "persona perceive <who> [mode=glance|study] [picture=<name>] [focus=<region or item id>] ",
    "[recent=<text>]... [scene=<words>] [vision=true|false] [model=<name>] | ",
    "persona perceive scene [who=<id>,<id>...] [mode=glance|study] [picture=<name>] ",
    "[recent=<text>]... [scene=<words>] [vision=true|false] [parallel=<n>] [model=<name>]"
);
/// `persona thoughts`: the thoughts a character kept.
pub const PERSONA_THOUGHTS_USAGE: &str = "persona thoughts <who> [limit=<n>]";

/// A `persona perceive` or `persona thoughts` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerceiveCommand {
    One(PerceiveRequest),
    Scene(PerceiveSceneRequest),
    Thoughts {
        who: PersonaId,
        limit: Option<usize>,
    },
}

/// The value of the option `key` among `given`, refusing a key given twice.
fn single<'a>(given: &[(&'a str, &'a str)], key: &str) -> Result<Option<&'a str>, CommandError> {
    let mut values = given.iter().filter(|(seen, _)| *seen == key);
    let first = values.next().map(|(_, value)| *value);
    if values.next().is_some() {
        return Err(CommandError::DuplicateKey(key.to_owned()));
    }
    Ok(first)
}

fn mode_of(given: &[(&str, &str)]) -> Result<Mode, CommandError> {
    single(given, "mode")?.map_or(Ok(Mode::default()), |text| {
        Mode::from_key(text).ok_or_else(|| bad_choice("mode", text, "glance or study"))
    })
}

fn vision_of(given: &[(&str, &str)]) -> Result<bool, CommandError> {
    single(given, "vision")?.map_or(Ok(true), |text| flag("vision", text))
}

fn recent_of(given: &[(&str, &str)]) -> Vec<String> {
    given
        .iter()
        .filter(|(key, _)| *key == "recent")
        .map(|(_, value)| (*value).to_owned())
        .collect()
}

fn count_of(given: &[(&str, &str)], key: &'static str) -> Result<Option<usize>, CommandError> {
    single(given, key)?
        .map(|text| {
            text.parse::<usize>().map_err(|_| CommandError::BadNumber {
                what: key,
                text: text.to_owned(),
            })
        })
        .transpose()
}

const SHARED_KEYS: [&str; 6] = ["mode", "picture", "recent", "scene", "vision", "model"];

/// `persona perceive ...`.
pub(in crate::persona) fn parse_perceive(
    words: &[String],
) -> Result<PerceiveCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_PERCEIVE_USAGE);
    let [first, rest @ ..] = words else {
        return Err(usage());
    };
    if first == "scene" {
        let keys: Vec<&str> = SHARED_KEYS.into_iter().chain(["who", "parallel"]).collect();
        let (given, left) = take_options(rest, &keys);
        if !left.is_empty() {
            return Err(usage());
        }
        let who = single(&given, "who")?
            .map(|list| {
                list.split(',')
                    .map(|word| id_word(word).map(|id| id.to_string()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        return Ok(PerceiveCommand::Scene(PerceiveSceneRequest {
            who,
            mode: mode_of(&given)?,
            picture: single(&given, "picture")?.map(str::to_owned),
            recent: recent_of(&given),
            scene: single(&given, "scene")?.map(str::to_owned),
            vision: vision_of(&given)?,
            parallel: count_of(&given, "parallel")?,
            model: single(&given, "model")?.map(str::to_owned),
        }));
    }
    let keys: Vec<&str> = SHARED_KEYS.into_iter().chain(["focus"]).collect();
    let (given, left) = take_options(rest, &keys);
    if !left.is_empty() {
        return Err(usage());
    }
    Ok(PerceiveCommand::One(PerceiveRequest {
        who: id_word(first)?.to_string(),
        mode: mode_of(&given)?,
        picture: single(&given, "picture")?.map(str::to_owned),
        focus: single(&given, "focus")?.map(str::to_owned),
        recent: recent_of(&given),
        scene: single(&given, "scene")?.map(str::to_owned),
        vision: vision_of(&given)?,
        model: single(&given, "model")?.map(str::to_owned),
    }))
}

/// `persona thoughts ...`.
pub(in crate::persona) fn parse_thoughts(
    words: &[String],
) -> Result<PerceiveCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_THOUGHTS_USAGE);
    let [who, rest @ ..] = words else {
        return Err(usage());
    };
    let (given, left) = take_options(rest, &["limit"]);
    if !left.is_empty() {
        return Err(usage());
    }
    Ok(PerceiveCommand::Thoughts {
        who: id_word(who)?,
        limit: count_of(&given, "limit")?,
    })
}

/// `text` on one line: its line breaks and runs of spaces folded to single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What one character saw and thought, as the lines the runner reads.
pub(in crate::persona) fn format_perceived(perceived: &Perceived) -> String {
    let mut lines = vec![format!(
        concat!(
            "{} perceives (mode={} vision={} picture={} regions={} viewpoint={} attention={} ",
            "duration_ms={})"
        ),
        perceived.who,
        perceived.mode.key(),
        perceived.vision,
        perceived.picture.as_deref().unwrap_or("-"),
        perceived.regions,
        match perceived.viewpoint {
            Viewpoint::Pictured { .. } => "pictured",
            Viewpoint::Viewer => "viewer",
        },
        perceived.attention.label(),
        perceived.duration_ms
    )];
    for seen in &perceived.sees {
        lines.push(format!(
            "see: {} | region={} item={} salience={} subject={}",
            one_line(&seen.what),
            seen.region.as_deref().unwrap_or("-"),
            seen.item.as_deref().unwrap_or("-"),
            seen.salience,
            seen.subject.key()
        ));
    }
    for thing in perceived.new_things.iter().filter(|thing| thing.added) {
        lines.push(format!(
            "thing {} ({}): {}",
            thing.id,
            one_line(&thing.name),
            one_line(&thing.look)
        ));
    }
    lines.push(format!("thought: {}", one_line(&perceived.thinking_about)));
    lines.push(format!("attention: {}", perceived.attention.label()));
    if let Some(feeling) = &perceived.feeling {
        lines.push(format!("feeling: {} {:+}", feeling.emotion, feeling.delta));
    }
    if !perceived.wants.is_empty() {
        lines.push(format!("wants: {}", one_line(&perceived.wants.join("; "))));
    }
    if perceived.leak_suspect {
        lines.push(
            "leak_suspect: the thought may state the private side; do not show it".to_owned(),
        );
    }
    if let Some(thought) = &perceived.thought {
        lines.push(format!("kept: {thought}"));
    }
    lines.join("\n")
}

fn format_scene(scene: &ScenePerceived) -> String {
    let perceived = scene
        .results
        .iter()
        .filter(|row| matches!(row.outcome, SceneOutcome::Perceived { .. }))
        .count();
    let mut lines = vec![format!(
        "scene: picture={} vision={} characters={} perceived={perceived}",
        scene.picture.as_deref().unwrap_or("-"),
        scene.vision,
        scene.results.len()
    )];
    for row in &scene.results {
        lines.push(format!("== {}", row.who));
        match &row.outcome {
            SceneOutcome::Perceived { perceived } => lines.push(format_perceived(perceived)),
            SceneOutcome::Refused { error } => {
                lines.push(format!("refused ({}): {error}", error.key()));
            }
        }
    }
    lines.join("\n")
}

fn format_thoughts(thoughts: &Thoughts) -> String {
    if thoughts.thoughts.is_empty() {
        return format!("{} has kept no thoughts", thoughts.who);
    }
    let mut lines = vec![format!(
        "{} has kept {} thoughts",
        thoughts.who,
        thoughts.thoughts.len()
    )];
    for thought in &thoughts.thoughts {
        lines.push(format!(
            "thought {} seq={} place={} picture={}: {}",
            thought.id,
            thought.seq,
            thought.place.as_deref().unwrap_or("-"),
            thought.picture.as_deref().unwrap_or("-"),
            one_line(&thought.text)
        ));
    }
    lines.join("\n")
}

/// Runs one perception command against the host on `port`.
pub(in crate::persona) async fn run(
    port: u16,
    command: PerceiveCommand,
) -> Result<String, DevError> {
    match command {
        PerceiveCommand::One(request) => {
            let reply: PerceiveReply<Perceived> =
                post_json(port, path::PERSONA_PERCEIVE, &request).await?;
            Ok(format_perceived(&reply_value(reply)?))
        }
        PerceiveCommand::Scene(request) => {
            let reply: PerceiveReply<ScenePerceived> =
                post_json(port, path::PERSONA_PERCEIVE_SCENE, &request).await?;
            Ok(format_scene(&reply_value(reply)?))
        }
        PerceiveCommand::Thoughts { who, limit } => {
            let query = ThoughtsQuery {
                who: who.to_string(),
                limit,
            };
            let mut address = format!("{}?who={}", path::PERSONA_THOUGHTS, query.who);
            if let Some(limit) = query.limit {
                address.push_str(&format!("&limit={limit}"));
            }
            let reply: PerceiveReply<Thoughts> = get_json(port, &address).await?;
            Ok(format_thoughts(&reply_value(reply)?))
        }
    }
}
