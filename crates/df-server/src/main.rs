//! The `df-server` binary. Round 1 offers `config check`; the role subcommands arrive with the
//! runtime seam.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::process::ExitCode;

    use df_server::{
        command_line::{self, Command},
        config::ServerConfig,
    };

    pub fn main() -> ExitCode {
        let command = match command_line::parse(&command_line::arguments()) {
            Ok(command) => command,
            Err(usage) => {
                eprintln!("usage: df-server config check --config <path> ({usage:?})");
                return ExitCode::from(2);
            }
        };
        match command {
            Command::ConfigCheck { config } => config_check(config),
        }
    }

    fn config_path(argument: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
        #[cfg(feature = "local-dev")]
        {
            argument.or_else(df_server::environment::config_path)
        }
        #[cfg(not(feature = "local-dev"))]
        {
            argument
        }
    }

    fn config_check(argument: Option<std::path::PathBuf>) -> ExitCode {
        let Some(path) = config_path(argument) else {
            eprintln!("config check needs --config <path>");
            return ExitCode::from(2);
        };
        match ServerConfig::load(&path).and_then(|config| config.listing()) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("config check failed: {error}");
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::main()
}

/// The server is native-only; the browser target builds an empty program.
#[cfg(target_arch = "wasm32")]
fn main() {}
