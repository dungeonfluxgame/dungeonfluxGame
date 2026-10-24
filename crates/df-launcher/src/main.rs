//! DungeonFlux desktop launcher (`dungeonflux.exe`).
//!
//! Double-click: the launcher window, where you start, resume and stop games and change
//! settings; the table display opens in its own window. The game server runs as a separate
//! process, so phones stay connected and the game keeps running when the launcher closes.
//! Settings and keys live in the central `config/dungeonflux.toml` (see `config.rs`); agents
//! and scripts can skip the window with the commands in `cli.rs` (`dungeonflux --help`).
#![windows_subsystem = "windows"]

mod app;
mod cli;
mod config;
mod control;
mod credentials;
mod desktop;
mod files;
mod form;
mod install;
mod log;
mod platform;
mod postgres;
mod preview;
mod saves;
mod server;
mod settings;
mod ui;

use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

#[derive(Debug, Default)]
struct Arguments {
    data_dir: Option<PathBuf>,
    config: Option<PathBuf>,
    preview: Option<SocketAddr>,
    connect: Option<String>,
    show_display: bool,
    command: Option<cli::Command>,
    help: bool,
}

fn arguments(words: Vec<String>) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    let mut items = words.into_iter();
    while let Some(item) = items.next() {
        let mut value = |name: &str| items.next().ok_or(format!("{name} needs a value"));
        match item.as_str() {
            "--data-dir" => parsed.data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--config" => parsed.config = Some(PathBuf::from(value("--config")?)),
            "--ui-preview" => {
                parsed.preview = Some(
                    value("--ui-preview")?
                        .parse()
                        .map_err(|_| "--ui-preview takes an address such as 127.0.0.1:63400")?,
                );
            }
            "--connect" => {
                let url = value("--connect")?;
                settings::external_origin(&url)?;
                parsed.connect = Some(url);
            }
            "--show-display" => parsed.show_display = true,
            "--help" | "-h" | "help" => parsed.help = true,
            word if cli::is_command(word) => {
                let rest: Vec<String> = items.by_ref().collect();
                parsed.command = Some(cli::parse(word, &rest)?);
            }
            other => return Err(format!("unknown argument {other:?}\n\n{}", cli::HELP)),
        }
    }
    Ok(parsed)
}

/// A short stable key for the data folder, so test instances never block the real launcher.
fn instance_key(folder: &std::path::Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in folder.to_string_lossy().to_lowercase().bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    // Commands, help and the preview print to the terminal that started them.
    let windowed = !words.iter().any(|word| {
        cli::is_command(word) || matches!(word.as_str(), "--ui-preview" | "--help" | "-h" | "help")
    });
    if !windowed {
        platform::attach_parent_console();
    }
    platform::keep_std_handles_private();
    match launch(words) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            // Double-clicked launches have no console, so show the reason there.
            if windowed {
                platform::error_box(&message);
            }
            ExitCode::FAILURE
        }
    }
}

fn launch(words: Vec<String>) -> Result<(), String> {
    let arguments = arguments(words)?;
    if arguments.help {
        println!("{}", cli::HELP);
        return Ok(());
    }
    let data = files::data_folder(arguments.data_dir)
        .map_err(|error| format!("DungeonFlux could not open its data folder: {error}"))?;
    log::init(&data);
    let config = config::locate(arguments.config, &data);
    if let Some(command) = arguments.command {
        let app = app::App::new(data, config, false, Box::new(|_| {}));
        return cli::run(&app, command);
    }
    if let Some(address) = arguments.preview {
        let app = app::App::new(data, config, false, Box::new(|_| {}));
        if let Some(url) = &arguments.connect {
            app.connect_to(url, true)?;
        }
        return preview::serve(app, address).map_err(|error| format!("Preview failed: {error}"));
    }
    match platform::claim_instance(&instance_key(&data))
        .map_err(|error| format!("DungeonFlux could not start: {error}"))?
    {
        platform::Instance::AlreadyRunning => Ok(()),
        platform::Instance::First(guard) => desktop::run(
            data,
            config,
            guard,
            arguments.connect,
            arguments.show_display,
        )
        .map_err(|error| format!("DungeonFlux could not open its window: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_come_before_the_command_and_commands_take_the_rest() {
        let parsed = arguments(words(
            "--data-dir D --config C.toml start random --to opening",
        ))
        .unwrap();
        assert_eq!(parsed.data_dir, Some(PathBuf::from("D")));
        assert_eq!(parsed.config, Some(PathBuf::from("C.toml")));
        assert!(matches!(parsed.command, Some(cli::Command::Start { .. })));
        assert!(arguments(words("--connect ftp://x")).is_err());
        assert!(arguments(words("--ui-preview nowhere")).is_err());
        assert!(arguments(words("launch")).is_err());
        assert!(arguments(words("--help")).unwrap().help);
    }
}
