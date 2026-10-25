//! `dev dice`: the forced-face queue. `dev dice next <face…> [--die d20]` queues faces for the next
//! draws of that die size, `dev dice` lists the queue and `dev dice clear` empties it. The server
//! checks every face again; the local check only gives the answer without a round trip.

use super::{Client, CommandResult, get, post};
use serde_json::json;

const DICE: [(&str, u32); 7] = [
    ("d4", 4),
    ("d6", 6),
    ("d8", 8),
    ("d10", 10),
    ("d12", 12),
    ("d20", 20),
    ("d100", 100),
];

#[derive(Debug, PartialEq)]
enum Command {
    List,
    Next { sides: u32, faces: Vec<u32> },
    Clear,
}

fn die_sides(name: &str) -> Result<u32, String> {
    DICE.iter()
        .find(|(die_name, _)| *die_name == name)
        .map(|(_, sides)| *sides)
        .ok_or_else(|| {
            format!("--die must be one of d4, d6, d8, d10, d12, d20, d100 (got {name:?})")
        })
}

fn parse(positional: &[String], options: &[(String, String)]) -> Result<Command, String> {
    let die = options
        .iter()
        .rev()
        .find(|(key, _)| key == "--die")
        .map(|(_, value)| value.as_str());
    match positional.get(1).map(String::as_str) {
        None if positional.len() == 1 && die.is_none() => Ok(Command::List),
        Some("clear") if positional.len() == 2 && die.is_none() => Ok(Command::Clear),
        Some("next") => {
            let sides = die_sides(die.unwrap_or("d20"))?;
            let faces = positional[2..]
                .iter()
                .map(|word| {
                    word.parse::<u32>()
                        .ok()
                        .filter(|face| (1..=sides).contains(face))
                        .ok_or_else(|| {
                            format!("face {word:?} must be a whole number from 1 to {sides} for d{sides}")
                        })
                })
                .collect::<Result<Vec<u32>, String>>()?;
            if faces.is_empty() {
                return Err("dice next needs one or more faces, e.g. dev dice next 17 3 --die d20".into());
            }
            Ok(Command::Next { sides, faces })
        }
        _ => Err(
            "dice takes nothing to list, `clear`, or `next <face…> [--die d4|d6|d8|d10|d12|d20|d100]`; --die applies to next only".into(),
        ),
    }
}

pub(super) async fn run(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match parse(positional, options) {
        Ok(Command::List) => get(client, "/dev/dice").await,
        Ok(Command::Clear) => post(client, "/dev/dice/clear", &json!({})).await,
        Ok(Command::Next { sides, faces }) => {
            post(
                client,
                "/dev/dice/next",
                &json!({"sides": sides, "faces": faces}),
            )
            .await
        }
        Err(message) => client.print_error(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn die(name: &str) -> Vec<(String, String)> {
        vec![("--die".to_owned(), name.to_owned())]
    }

    #[test]
    fn next_queues_faces_for_the_named_die_and_defaults_to_d20() {
        assert_eq!(
            parse(&words("dice next 17 3"), &[]),
            Ok(Command::Next {
                sides: 20,
                faces: vec![17, 3]
            })
        );
        assert_eq!(
            parse(&words("dice next 1 4"), &die("d4")),
            Ok(Command::Next {
                sides: 4,
                faces: vec![1, 4]
            })
        );
        assert_eq!(
            parse(&words("dice next 100"), &die("d100")),
            Ok(Command::Next {
                sides: 100,
                faces: vec![100]
            })
        );
    }

    #[test]
    fn list_and_clear_take_no_faces_and_no_die() {
        assert_eq!(parse(&words("dice"), &[]), Ok(Command::List));
        assert_eq!(parse(&words("dice clear"), &[]), Ok(Command::Clear));
        assert!(parse(&words("dice clear"), &die("d6")).is_err());
        assert!(parse(&words("dice clear 3"), &[]).is_err());
        assert!(parse(&words("dice"), &die("d6")).is_err());
    }

    #[test]
    fn refuses_unknown_dice_out_of_range_faces_and_empty_or_unknown_commands() {
        assert!(parse(&words("dice next 3"), &die("d7")).is_err());
        assert!(parse(&words("dice next 0"), &[]).is_err());
        assert!(parse(&words("dice next 21"), &[]).is_err());
        assert!(parse(&words("dice next 5"), &die("d4")).is_err());
        assert!(parse(&words("dice next seventeen"), &[]).is_err());
        assert!(parse(&words("dice next -1"), &[]).is_err());
        assert!(parse(&words("dice next"), &[]).is_err());
        assert!(parse(&words("dice roll"), &[]).is_err());
    }
}
