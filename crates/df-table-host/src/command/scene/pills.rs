//! The `scene pill` verbs: the session, party, balance and rate pills of the top bar, the
//! wordmark and the profile mark.
//!
//! `scene pill set session <text>`, `party <joined>/<capacity>`, `balance <text>` and `rate <text>`
//! put a pill up; `scene pill clear <which>` takes one away. A balance or a rate is drawn only
//! when it is given: the public pricing shows what tables pay, so nothing here works one out.

use df_playground::lobby::change::{PillEdit, PillPart, ScreenEdit};
use df_playground::lobby::model::{LogoStyle, PartyCount};
use df_playground::scene::SceneChange;

use crate::command::{Command, CommandError, bad_choice, truth};

pub(super) const PILL_USAGE: &str = concat!(
    "scene pill set session <text> | party <joined>/<capacity> | balance <text> | rate <text> | ",
    "logo stacked|inline | profile on|off | scene pill clear ",
    "session|party|balance|rate|logo|profile",
);

/// The help lines of these verbs, as `help` prints them.
pub(in crate::command) const HELP: &str = concat!(
    "\n  scene pill ...\n      the top bar: set or clear the session, party, balance and rate ",
    "pills, the logo and the profile\n",
);

const PARTS: &str = "session, party, balance, rate, logo or profile";

fn party(value: &str) -> Result<PartyCount, CommandError> {
    let wrong = || bad_choice("party", value, "joined/capacity, such as 4/6");
    let (joined, capacity) = value.split_once('/').ok_or_else(wrong)?;
    let joined = joined.trim().parse::<u8>().map_err(|_| wrong())?;
    let capacity = capacity.trim().parse::<u8>().map_err(|_| wrong())?;
    Ok(PartyCount { joined, capacity })
}

fn set(which: &str, words: &[String]) -> Result<PillEdit, CommandError> {
    let text = words.join(" ");
    let mut pill = PillEdit::default();
    match which {
        "session" => pill.session = Some(text),
        "balance" => pill.balance = Some(text),
        "rate" => pill.rate = Some(text),
        "party" => pill.party = Some(party(&text)?),
        "logo" => {
            pill.logo = Some(match text.as_str() {
                "stacked" => LogoStyle::Stacked,
                "inline" => LogoStyle::Inline,
                other => return Err(bad_choice("logo", other, "stacked or inline")),
            });
        }
        "profile" => pill.profile = Some(truth("profile", &text)?),
        other => return Err(bad_choice("the pill", other, PARTS)),
    }
    Ok(pill)
}

/// Reads the words after `scene pill`.
pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    let edit = match words {
        [verb, which, rest @ ..] if verb == "set" && !rest.is_empty() => ScreenEdit::PillSet {
            pill: set(which, rest)?,
        },
        [verb, which] if verb == "clear" => ScreenEdit::PillClear {
            part: PillPart::from_key(which).ok_or_else(|| bad_choice("the pill", which, PARTS))?,
        },
        _ => return Err(CommandError::Usage(PILL_USAGE)),
    };
    Ok(Command::Scene(SceneChange::Screen { edit }))
}
