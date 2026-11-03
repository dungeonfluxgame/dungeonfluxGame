//! The `scene lobby`, `scene loading` and `scene roleplay` verbs: the live lobby, loading screen
//! and roleplay card of the playground's shared screen.
//!
//! Each verb reads its words into one `ScreenEdit` (`df_playground::lobby_change`). A key left out
//! leaves that field as it is, a key given an empty text (`name=`) takes the field away, and a
//! range is checked by the host when the change arrives, so the words here only say whether a
//! value is of the right kind.

use df_playground::lobby_change::{
    BeginEdit, ChipEdit, JoinEdit, LoadingEdit, LobbyLine, LobbyPart, PartyEdit, RoleplayEdit,
    ScreenEdit, SeatEdit,
};
use df_playground::lobby_model::{BubbleTone, CHARACTER_STEPS, SeatState, SummaryIcon};
use df_playground::overlay::Glyph;
use df_playground::scene::SceneChange;

use super::{Command, CommandError, bad_choice, count, keyed, truth};

pub(super) const LOBBY_USAGE: &str = "scene lobby set title|subtitle|eyebrow <text> | seats <n> | seat <n> [state=locked|creating|open|closed] [name=T] [class=T] [glyph=G] [level=N] [portrait=FILE] [step=class|race|finalize] [progress=0-100] [stage=T] [badge=T] [host=true|false] [controller=true|false] | join [code=T] [link=T] [hint=T] | party [joined=N] [capacity=N] [word=T] [detail=T] [icon=check|people|spinner] | begin [enabled=true|false] [label=T] [note=T] | focus begin|seat:<n>|none | remove title|seats|join|party|begin | clear";
pub(super) const LOADING_USAGE: &str = "scene loading set [kicker=T] [title=T] [subtitle=T] [label=T] [percent=0-100] [narrator=T] [status=T] [portrait=FILE] | seat <n> [name=T] [portrait=FILE] [ready=true|false] | clear";
pub(super) const ROLEPLAY_USAGE: &str = "scene roleplay set [title=T] [icon=G] [bubble=N] [speaker=T] [line=T] [emphasis=T] [speaker-icon=G] [portrait=FILE] [tone=hero|ally|mystic|npc|narrator|danger] [left=0-100] [top=0-100] [width=0-100] [tail-x=0-100] [tail-y=0-100] [caption=T] | clear";

/// The help lines of these verbs, as `help` prints them.
pub(super) const HELP: &str = concat!(
    "\n  scene lobby ...\n      the player lobby: its title words, its seats (seats <n> sets how many, seat <n> changes\n      one), the join code, the party summary, the begin button, focus <id> (the one element the\n      remote is on, begin or seat:2), remove <part>, clear\n",
    "\n  scene loading ...\n      the loading screen: its words, the bar's percent, the narrator and the seats\n",
    "\n  scene roleplay ...\n      the roleplay card: the title bar, a speech bubble (by number), the caption\n",
);

const GLYPHS: &str = "a glyph name such as sword, shield, bow, sun, book, scales, mug, mask, hammer, plus, voice, dice, crown, star, check, lock, pencil";

fn glyph(key: &str, value: &str) -> Result<Glyph, CommandError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| bad_choice(key, value, GLYPHS))
}

fn small(key: &str, value: &str) -> Result<u8, CommandError> {
    value.parse::<u8>().map_err(|_| {
        bad_choice(
            key,
            value,
            "a whole number from 0 to 255 (the host says the range)",
        )
    })
}

fn state(value: &str) -> Result<SeatState, CommandError> {
    match value {
        "locked" | "locked_in" | "locked-in" => Ok(SeatState::LockedIn),
        "creating" => Ok(SeatState::Creating),
        "open" => Ok(SeatState::Open),
        "closed" => Ok(SeatState::Closed),
        _ => Err(bad_choice(
            "state",
            value,
            "locked, creating, open or closed",
        )),
    }
}

fn step(value: &str) -> Result<u8, CommandError> {
    let named = CHARACTER_STEPS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(value));
    match named {
        Some(index) => Ok(u8::try_from(index).unwrap_or(0)),
        None => small("step", value)
            .map_err(|_| bad_choice("step", value, "class, race, finalize, or 0, 1 or 2")),
    }
}

/// The seat number that starts `words`, and the words after it.
fn number<'a>(
    words: &'a [String],
    usage: &'static str,
) -> Result<(usize, &'a [String]), CommandError> {
    let [number, rest @ ..] = words else {
        return Err(CommandError::Usage(usage));
    };
    let number = usize::try_from(count("the seat number", number)?)
        .map_err(|_| CommandError::Usage(usage))?;
    Ok((number, rest))
}

fn text(value: &str) -> Option<String> {
    Some(value.to_owned())
}

fn screen(edit: ScreenEdit) -> Result<Command, CommandError> {
    Ok(Command::Scene(SceneChange::Screen { edit }))
}

fn lobby_seat(rest: &[String]) -> Result<Command, CommandError> {
    let (number, pairs) = number(rest, LOBBY_USAGE)?;
    let mut seat = SeatEdit::default();
    let keys = [
        "state",
        "name",
        "class",
        "glyph",
        "level",
        "portrait",
        "step",
        "progress",
        "stage",
        "badge",
        "host",
        "controller",
    ];
    for (key, value) in keyed(pairs, "the keys in `scene lobby seat`", &keys)? {
        match key {
            "state" => seat.state = Some(state(value)?),
            "name" => seat.name = text(value),
            "class" => seat.class = text(value),
            "glyph" => seat.glyph = Some(glyph(key, value)?),
            "level" => seat.level = Some(small(key, value)?),
            "portrait" => seat.portrait = text(value),
            "step" => seat.step = Some(step(value)?),
            "progress" => seat.progress_percent = Some(small(key, value)?),
            "stage" => seat.stage = text(value),
            "badge" => seat.badge = text(value),
            "host" => seat.host = Some(truth(key, value)?),
            _ => seat.controller = Some(truth(key, value)?),
        }
    }
    screen(ScreenEdit::LobbySeat { number, seat })
}

fn lobby_join(pairs: &[String]) -> Result<Command, CommandError> {
    let mut join = JoinEdit::default();
    for (key, value) in keyed(
        pairs,
        "the keys in `scene lobby join`",
        &["code", "link", "hint"],
    )? {
        match key {
            "code" => join.game_code = text(value),
            "link" => join.join_link = text(value),
            _ => join.hint = text(value),
        }
    }
    screen(ScreenEdit::LobbyJoin { join })
}

fn lobby_party(pairs: &[String]) -> Result<Command, CommandError> {
    let mut party = PartyEdit::default();
    let keys = ["joined", "capacity", "word", "detail", "icon"];
    for (key, value) in keyed(pairs, "the keys in `scene lobby party`", &keys)? {
        match key {
            "joined" => party.joined = Some(small(key, value)?),
            "capacity" => party.capacity = Some(small(key, value)?),
            "word" => party.word = text(value),
            "detail" => party.detail = text(value),
            _ => {
                party.icon = Some(
                    serde_json::from_value::<SummaryIcon>(serde_json::Value::String(
                        value.to_owned(),
                    ))
                    .map_err(|_| bad_choice(key, value, "check, people or spinner"))?,
                );
            }
        }
    }
    screen(ScreenEdit::LobbyParty { party })
}

fn lobby(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => screen(ScreenEdit::LobbyClear {}),
        [verb, id] if verb == "focus" => screen(ScreenEdit::LobbyFocus {
            id: (id != "none").then(|| id.clone()),
        }),
        [verb, name] if verb == "remove" => {
            let part = LobbyPart::from_key(name)
                .ok_or_else(|| bad_choice("the part", name, LobbyPart::keys_text()))?;
            screen(ScreenEdit::LobbyRemove { part })
        }
        [verb, number] if verb == "seats" => screen(ScreenEdit::LobbySeats {
            count: usize::try_from(count("the number of seats", number)?)
                .map_err(|_| CommandError::Usage(LOBBY_USAGE))?,
        }),
        [verb, which, words @ ..] if verb == "set" && !words.is_empty() => {
            let field = match which.as_str() {
                "title" => LobbyLine::Title,
                "subtitle" => LobbyLine::Subtitle,
                "eyebrow" => LobbyLine::Eyebrow,
                other => return Err(bad_choice("the line", other, "title, subtitle or eyebrow")),
            };
            screen(ScreenEdit::LobbyLine {
                field,
                text: words.join(" "),
            })
        }
        [verb, rest @ ..] if verb == "seat" => lobby_seat(rest),
        [verb, pairs @ ..] if verb == "join" && !pairs.is_empty() => lobby_join(pairs),
        [verb, pairs @ ..] if verb == "party" && !pairs.is_empty() => lobby_party(pairs),
        [verb, pairs @ ..] if verb == "begin" && !pairs.is_empty() => {
            let mut begin = BeginEdit::default();
            for (key, value) in keyed(
                pairs,
                "enabled=true|false, label=T, note=T",
                &["enabled", "label", "note"],
            )? {
                match key {
                    "enabled" => begin.enabled = Some(truth(key, value)?),
                    "label" => begin.label = text(value),
                    _ => begin.note = text(value),
                }
            }
            screen(ScreenEdit::LobbyBegin { begin })
        }
        _ => Err(CommandError::Usage(LOBBY_USAGE)),
    }
}

fn loading(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => screen(ScreenEdit::LoadingClear {}),
        [verb, rest @ ..] if verb == "seat" => {
            let (number, pairs) = number(rest, LOADING_USAGE)?;
            let mut chip = ChipEdit::default();
            for (key, value) in keyed(
                pairs,
                "name=T, portrait=FILE, ready=true|false",
                &["name", "portrait", "ready"],
            )? {
                match key {
                    "name" => chip.name = text(value),
                    "portrait" => chip.portrait = text(value),
                    _ => chip.ready = Some(truth(key, value)?),
                }
            }
            screen(ScreenEdit::LoadingSeat { number, chip })
        }
        [verb, pairs @ ..] if verb == "set" && !pairs.is_empty() => {
            let mut loading = LoadingEdit::default();
            let keys = [
                "kicker", "title", "subtitle", "label", "percent", "narrator", "status", "portrait",
            ];
            for (key, value) in keyed(pairs, "the keys in `scene loading set`", &keys)? {
                match key {
                    "kicker" => loading.kicker = text(value),
                    "title" => loading.title = text(value),
                    "subtitle" => loading.subtitle = text(value),
                    "label" => loading.label = text(value),
                    "percent" => loading.percent = Some(small(key, value)?),
                    "narrator" => loading.narrator_name = text(value),
                    "status" => loading.narrator_status = text(value),
                    _ => loading.narrator_portrait = text(value),
                }
            }
            screen(ScreenEdit::LoadingSet { loading })
        }
        _ => Err(CommandError::Usage(LOADING_USAGE)),
    }
}

fn tone(key: &str, value: &str) -> Result<BubbleTone, CommandError> {
    serde_json::from_value::<BubbleTone>(serde_json::Value::String(value.to_owned()))
        .map_err(|_| bad_choice(key, value, "hero, ally, mystic, npc, narrator or danger"))
}

fn roleplay_set(pairs: &[String]) -> Result<Command, CommandError> {
    let mut roleplay = RoleplayEdit::default();
    let keys = [
        "title",
        "icon",
        "bubble",
        "speaker",
        "line",
        "emphasis",
        "speaker-icon",
        "portrait",
        "tone",
        "left",
        "top",
        "width",
        "tail-x",
        "tail-y",
        "caption",
    ];
    for (key, value) in keyed(pairs, "the keys in `scene roleplay set`", &keys)? {
        match key {
            "title" => roleplay.title = text(value),
            "icon" => roleplay.icon = Some(glyph(key, value)?),
            "bubble" => {
                roleplay.bubble = Some(
                    usize::try_from(count("the bubble number", value)?)
                        .map_err(|_| CommandError::Usage(ROLEPLAY_USAGE))?,
                );
            }
            "speaker" => roleplay.speaker = text(value),
            "line" => roleplay.line = text(value),
            "emphasis" => roleplay.emphasis = text(value),
            "speaker-icon" => roleplay.speaker_icon = Some(glyph(key, value)?),
            "portrait" => roleplay.portrait = text(value),
            "tone" => roleplay.tone = Some(tone(key, value)?),
            "left" => roleplay.left_percent = Some(small(key, value)?),
            "top" => roleplay.top_percent = Some(small(key, value)?),
            "width" => roleplay.width_percent = Some(small(key, value)?),
            "tail-x" => roleplay.tail_x_percent = Some(small(key, value)?),
            "tail-y" => roleplay.tail_y_percent = Some(small(key, value)?),
            _ => roleplay.caption = text(value),
        }
    }
    screen(ScreenEdit::RoleplaySet { roleplay })
}

fn roleplay(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => screen(ScreenEdit::RoleplayClear {}),
        [verb, pairs @ ..] if verb == "set" && !pairs.is_empty() => roleplay_set(pairs),
        _ => Err(CommandError::Usage(ROLEPLAY_USAGE)),
    }
}

/// Reads the words after `scene lobby`, `scene loading` or `scene roleplay`.
pub(super) fn parse(verb: &str, words: &[String]) -> Result<Command, CommandError> {
    match verb {
        "lobby" => lobby(words),
        "loading" => loading(words),
        _ => roleplay(words),
    }
}
