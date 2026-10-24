//! `dev place`: where the party stands and its exits; `dev place list` the places the run knows
//! and the moves made; `dev place go <exit|place>` moves the whole party for testing, through the
//! same path a hero's line takes. The server checks everything again; nothing here decides.

use super::{Client, CommandResult, get, post};
use serde_json::json;

#[derive(Debug, PartialEq)]
enum Command {
    Show,
    List,
    Go { target: String },
    Travel { target: String },
}

fn parse(positional: &[String], options: &[(String, String)]) -> Result<Command, String> {
    if options
        .iter()
        .any(|(key, _)| !matches!(key.as_str(), "--server" | "--token-file" | "--timeout"))
    {
        return Err("place takes no options".into());
    }
    match positional.get(1).map(String::as_str) {
        None => Ok(Command::Show),
        Some("list") if positional.len() == 2 => Ok(Command::List),
        Some(op @ ("go" | "travel")) => {
            let target = positional[2..].join(" ");
            if target.trim().is_empty() {
                return Err(
                    "place go needs an exit or a place, e.g. dev place go street-door".into(),
                );
            }
            Ok(if op == "travel" {
                Command::Travel { target }
            } else {
                Command::Go { target }
            })
        }
        _ => Err(
            "place takes nothing to show the current place, `list`, or `go <exit|place>`".into(),
        ),
    }
}

pub(super) async fn run(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match parse(positional, options) {
        Ok(Command::Show) => get(client, "/dev/place").await,
        Ok(Command::List) => get(client, "/dev/place/list").await,
        Ok(Command::Go { target }) => {
            post(client, "/dev/place/go", &json!({"target": target})).await
        }
        Ok(Command::Travel { target }) => {
            post(
                client,
                "/dev/place/go",
                &json!({"target":target,"travel":true}),
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

    #[test]
    fn place_shows_lists_and_goes() {
        assert_eq!(parse(&words("place"), &[]), Ok(Command::Show));
        assert_eq!(parse(&words("place list"), &[]), Ok(Command::List));
        assert_eq!(
            parse(&words("place go street-door"), &[]),
            Ok(Command::Go {
                target: "street-door".to_owned()
            })
        );
        assert_eq!(
            parse(&words("place go Marek's Cloth Shop"), &[]),
            Ok(Command::Go {
                target: "Marek's Cloth Shop".to_owned()
            })
        );
    }

    #[test]
    fn travel_preserves_destination_words_and_landmark_go_is_explicit() {
        assert_eq!(
            parse(&words("place travel the quiet riverside market"), &[]),
            Ok(Command::Travel {
                target: "the quiet riverside market".into()
            })
        );
        assert_eq!(
            parse(&words("place go landmark:Old Mill"), &[]),
            Ok(Command::Go {
                target: "landmark:Old Mill".into()
            })
        );
        assert!(parse(&words("place travel"), &[]).is_err());
        assert_eq!(
            parse(
                &words("place list"),
                &[("--server".into(), "http://localhost:5".into())]
            ),
            Ok(Command::List)
        );
    }

    #[test]
    fn place_refuses_a_missing_target_and_anything_else() {
        assert!(parse(&words("place go"), &[]).is_err());
        assert!(parse(&words("place list extra"), &[]).is_err());
        assert!(parse(&words("place fly"), &[]).is_err());
        assert!(parse(&words("place"), &[("--x".to_owned(), "1".to_owned())]).is_err());
    }
}
