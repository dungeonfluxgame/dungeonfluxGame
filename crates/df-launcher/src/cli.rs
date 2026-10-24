//! Commands for agents and scripts that skip the launcher screen. They use the same saved games,
//! settings and running-game record as the window, so a game started here shows up in the
//! launcher and a game started there can be stopped here. Output is one `KEY field=value` line
//! per fact on standard output; failures go to standard error with exit code 1.
use std::{process::Command as Process, sync::Arc};

use crate::{
    app::{App, Failure, Phase, RunRecord, StartOptions},
    saves::Adventure,
    server,
};

pub const HELP: &str = "\
usage: dungeonflux [--data-dir <folder>] [--config <file>] [command]

Without a command, opens the launcher window.

Commands (for agents and scripts; no window opens):
  start [harbor|random] [--dev-control] [--to <phase>] [--open-display]
        Start a new game and leave it running. Prints READY with the display and player links.
  resume <name or database> [--dev-control] [--to <phase>] [--open-display]
        Resume a saved game.
  stop [--force]     Save and stop the running game (and the database the launcher started).
  status             Print RUNNING ... or IDLE.
  saves              List saved games, newest first.
  logs [--lines N]   Print the launcher's log (last 200 lines unless N is given).

  --dev-control      Turn on the server's developer control for this game and print its
                     DEV_CONTROL url and token file (for df-transport-fixture dev).
  --to <phase>       After starting, drive the game to creation, opening, dialogue, combat,
                     complete or ended with `df-transport-fixture dev play --to` (implies
                     --dev-control).
  --open-display     Also open the table display in the default browser.

Developer options:
  --data-dir <folder>            Keep saved games, screenshots and the run record elsewhere.
  --config <file>                Use another configuration file than config/dungeonflux.toml.
  --ui-preview <127.0.0.1:port>  Serve the launcher pages over loopback HTTP, no window.
  --connect <http://host:port>   Open the window attached to a server started elsewhere.
  --show-display                 Open the window with the running game's table display.
";

const PHASES: [&str; 6] = [
    "creation", "opening", "dialogue", "combat", "complete", "ended",
];

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    Start { adventure: Adventure, flags: Flags },
    Resume { wanted: String, flags: Flags },
    Stop { force: bool },
    Status,
    Saves,
    Logs { lines: usize },
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Flags {
    pub dev_control: bool,
    pub to: Option<String>,
    pub open_display: bool,
}

/// Parses the words after the command name.
pub fn parse(name: &str, rest: &[String]) -> Result<Command, String> {
    let mut words = rest.iter().map(String::as_str);
    let mut flags = Flags::default();
    let mut positional = Vec::new();
    let mut force = false;
    let mut lines = None;
    while let Some(word) = words.next() {
        match word {
            "--lines" if name == "logs" => {
                let count = words.next().ok_or("--lines needs a number")?;
                lines = Some(
                    count
                        .parse::<usize>()
                        .ok()
                        .filter(|count| (1..=100_000).contains(count))
                        .ok_or("--lines takes a number from 1 to 100000")?,
                );
            }
            "--dev-control" => flags.dev_control = true,
            "--open-display" => flags.open_display = true,
            "--force" => force = true,
            "--to" => {
                let phase = words.next().ok_or("--to needs a phase")?;
                if !PHASES.contains(&phase) {
                    return Err(format!("--to takes one of: {}", PHASES.join(", ")));
                }
                flags.to = Some(phase.to_owned());
                flags.dev_control = true;
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => positional.push(other.to_owned()),
        }
    }
    let only_flags = |command: Command| {
        if positional.is_empty() {
            Ok(command)
        } else {
            Err(format!("{name} takes no {}", positional.join(" ")))
        }
    };
    match name {
        "start" => {
            let adventure = match positional.as_slice() {
                [] => Adventure::Harbor,
                [one] if one == "harbor" => Adventure::Harbor,
                [one] if one == "random" => Adventure::Random,
                _ => return Err("start takes harbor or random".to_owned()),
            };
            Ok(Command::Start { adventure, flags })
        }
        "resume" => match positional.as_slice() {
            [] => Err(
                "resume needs a saved game's name or database (see: dungeonflux saves)".to_owned(),
            ),
            words => Ok(Command::Resume {
                wanted: words.join(" "),
                flags,
            }),
        },
        "stop" if flags == Flags::default() => only_flags(Command::Stop { force }),
        "status" if !force && flags == Flags::default() => only_flags(Command::Status),
        "saves" if !force && flags == Flags::default() => only_flags(Command::Saves),
        "logs" if !force && flags == Flags::default() => only_flags(Command::Logs {
            lines: lines.unwrap_or(200),
        }),
        _ => Err(format!("{name} does not take these options")),
    }
}

pub fn is_command(word: &str) -> bool {
    matches!(
        word,
        "start" | "resume" | "stop" | "status" | "saves" | "logs"
    )
}

pub fn run(app: &Arc<App>, command: Command) -> Result<(), String> {
    crate::log::event(
        "CLI_COMMAND",
        format!("command={}", crate::log::value(format!("{command:?}"))),
    );
    match command {
        Command::Logs { lines } => {
            println!("{}", crate::log::tail(app.data(), lines));
            Ok(())
        }
        Command::Start { adventure, flags } => {
            let game = app.new_game(adventure);
            started(app.start_blocking(&game, options(&flags)), &flags)
        }
        Command::Resume { wanted, flags } => {
            let game = app.find_save(&wanted).ok_or_else(|| {
                format!("No saved game is called {wanted:?}. List them with: dungeonflux saves")
            })?;
            started(app.start_blocking(&game, options(&flags)), &flags)
        }
        Command::Stop { force } => {
            println!("{}", app.stop_blocking(force)?);
            Ok(())
        }
        Command::Status => {
            app.refresh_from_disk();
            match app.phase() {
                Phase::Running(run) => print_run("RUNNING", &run),
                _ => println!("IDLE"),
            }
            Ok(())
        }
        Command::Saves => {
            for game in crate::saves::load(app.data()).map_err(|error| error.to_string())? {
                let played = chrono::DateTime::from_timestamp(game.last_played, 0)
                    .map(|moment| moment.to_rfc3339())
                    .unwrap_or_default();
                println!(
                    "SAVE name={:?} database={} adventure={} last_played={played}",
                    game.name,
                    game.database,
                    game.adventure.server_value()
                );
            }
            Ok(())
        }
    }
}

fn options(flags: &Flags) -> StartOptions {
    StartOptions {
        dev_control: flags.dev_control,
        quiet_display: true,
    }
}

fn print_run(label: &str, run: &RunRecord) {
    println!(
        "{label} name={:?} database={} pid={} display={} player={}",
        run.game.name, run.game.database, run.pid, run.display_url, run.player_url
    );
    if let Some((url, token_file)) = run
        .dev_control
        .as_deref()
        .and_then(server::dev_control_parts)
    {
        println!("DEV_CONTROL url={url} token_file={token_file}");
    }
}

fn started(outcome: Result<RunRecord, Box<Failure>>, flags: &Flags) -> Result<(), String> {
    let run = outcome.map_err(|failure| {
        let mut message = format!("{}: {}", failure.title, failure.message);
        if !failure.log_tail.is_empty() {
            message.push('\n');
            message.push_str(&failure.log_tail);
        }
        message
    })?;
    print_run("READY", &run);
    if flags.open_display {
        let _ = open::that_detached(&run.display_url);
    }
    if let Some(phase) = &flags.to {
        let (url, token_file) = run
            .dev_control
            .as_deref()
            .and_then(server::dev_control_parts)
            .ok_or("The game started without developer control, so --to cannot drive it.")?;
        let output = Process::new(&run.server_exe)
            .args(["dev", "--server", &url, "--token-file", &token_file])
            .args(["--timeout", "180", "play", "--to", phase])
            .output()
            .map_err(|error| format!("df-transport-fixture dev could not run: {error}"))?;
        let text = String::from_utf8_lossy(if output.stdout.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        })
        .trim()
        .to_owned();
        if !output.status.success() {
            return Err(format!(
                "The game is running, but it could not reach {phase}: {text}"
            ));
        }
        println!("PHASE {phase} {}", text.replace('\n', " "));
    }
    // The game keeps running after this process exits (REF-005).
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn commands_parse_with_their_flags() {
        assert_eq!(
            parse("start", &words("random --to creation")).unwrap(),
            Command::Start {
                adventure: Adventure::Random,
                flags: Flags {
                    dev_control: true,
                    to: Some("creation".to_owned()),
                    open_display: false
                }
            }
        );
        assert_eq!(
            parse("resume", &words("The Iron Map --dev-control")).unwrap(),
            Command::Resume {
                wanted: "The Iron Map".to_owned(),
                flags: Flags {
                    dev_control: true,
                    ..Flags::default()
                }
            }
        );
        assert_eq!(
            parse("stop", &words("--force")).unwrap(),
            Command::Stop { force: true }
        );
        assert_eq!(parse("status", &[]).unwrap(), Command::Status);
        for refused in [
            ("start", "castle"),
            ("start", "--to moon"),
            ("resume", ""),
            ("status", "--force"),
            ("saves", "extra"),
            ("stop", "--to creation"),
            ("start", "--fast"),
        ] {
            assert!(parse(refused.0, &words(refused.1)).is_err(), "{refused:?}");
        }
    }
}
