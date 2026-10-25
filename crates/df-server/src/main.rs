//! The `df-server` binary: `config check`, the hosted serving roles, `migrate` and
//! `healthcheck` (deploy/images/Dockerfile, deploy/compose/*.yml).

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::process::ExitCode;

    use df_server::{
        command_line::{self, Command},
        config::ServerConfig,
        health::Signal,
        serving,
    };

    pub fn main() -> ExitCode {
        let command = match command_line::parse(&command_line::arguments()) {
            Ok(command) => command,
            Err(usage) => {
                eprintln!(
                    "usage: df-server <config check|control-plane|campaign-runtime|egress|migrate|healthcheck> ({usage:?})"
                );
                return ExitCode::from(2);
            }
        };
        match command {
            Command::ConfigCheck { config } => config_check(config),
            Command::ControlPlane { config } => serve(config, ServeKind::ControlPlane),
            Command::CampaignRuntime { descriptor } => {
                serve(descriptor, ServeKind::CampaignRuntime)
            }
            Command::Egress { config } => serve(config, ServeKind::Egress),
            Command::Healthcheck {
                address,
                signal,
                ca_file: _,
                server_name: _,
            } => healthcheck(address, signal),
            Command::Migrate { action, config } => migrate(&action, config),
        }
    }

    #[derive(Clone, Copy)]
    enum ServeKind {
        ControlPlane,
        CampaignRuntime,
        Egress,
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

    fn load(argument: Option<std::path::PathBuf>) -> Result<ServerConfig, ExitCode> {
        let Some(path) = config_path(argument) else {
            eprintln!("serving needs --config/--descriptor <path>");
            return Err(ExitCode::from(2));
        };
        ServerConfig::load(&path).map_err(|error| {
            eprintln!("config refused: {error}");
            ExitCode::FAILURE
        })
    }

    fn serve(argument: Option<std::path::PathBuf>, kind: ServeKind) -> ExitCode {
        let config = match load(argument) {
            Ok(config) => config,
            Err(code) => return code,
        };
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return ExitCode::FAILURE,
        };
        runtime.block_on(async move {
            let serving = match kind {
                ServeKind::ControlPlane => serving::start_control_plane(config).await,
                ServeKind::CampaignRuntime => serving::start_campaign_runtime(config).await,
                ServeKind::Egress => serving::start_egress(config).await,
            };
            let serving = match serving {
                Ok(serving) => serving,
                Err(error) => {
                    eprintln!("startup refused: {error}");
                    return ExitCode::FAILURE;
                }
            };
            // Keep the recovery handle and workers alive until a shutdown signal.
            let _ = tokio::signal::ctrl_c().await;
            serving.shutdown().await;
            ExitCode::SUCCESS
        })
    }

    fn healthcheck(address: Option<std::net::SocketAddr>, signal: Option<String>) -> ExitCode {
        let (Some(address), Some(signal)) = (address, signal) else {
            eprintln!("healthcheck needs --address <addr> --signal <name>");
            return ExitCode::from(2);
        };
        let Some(signal) = Signal::from_name(&signal) else {
            eprintln!("unknown health signal: {signal}");
            return ExitCode::from(2);
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return ExitCode::FAILURE,
        };
        match runtime.block_on(serving::check_health(address, signal)) {
            Ok(true) => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        }
    }

    fn migrate(action: &str, argument: Option<std::path::PathBuf>) -> ExitCode {
        // Migration application is owned by the migration runner; this entry point only
        // validates the named config so a bad file fails before any deploy step.
        match load(argument) {
            Ok(_) => {
                println!("migrate {action}: config valid");
                ExitCode::SUCCESS
            }
            Err(code) => code,
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
