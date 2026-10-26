//! The launcher's state and actions: start or resume a game, follow it, stop it, and keep the
//! table display window in step. Long work runs on worker threads; every change of phase asks
//! the window to refresh.
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use crate::{
    config::{self, Config},
    credentials::{self, Credential, Keys},
    form::Form,
    install::{Install, Missing},
    log, platform, postgres,
    saves::{self, Adventure, SavedGame},
    server,
    settings::{DisplayScreen, DisplayTarget, FieldError, FullScreen, ServerMode, Settings},
};

/// Requests from the controller to whatever shows it (the desktop windows, or nothing in the
/// headless preview).
#[derive(Clone, Debug, PartialEq)]
pub enum ShellEvent {
    Refresh,
    OpenDisplay {
        url: String,
        screen: DisplayScreen,
        full_screen: FullScreen,
    },
    CloseDisplay,
    DisplayFullScreen(bool),
    /// Save a picture of the table display window to `path` (JPEG), if it is open.
    CaptureDisplay {
        path: PathBuf,
    },
}

/// Choices for one start that differ from the saved settings (the agent command line).
#[derive(Clone, Copy, Debug, Default)]
pub struct StartOptions {
    pub dev_control: bool,
    pub quiet_display: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Home,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Step {
    Checking,
    Database,
    Save,
    Server,
    Waiting,
}

impl Step {
    pub const ALL: [Self; 5] = [
        Self::Checking,
        Self::Database,
        Self::Save,
        Self::Server,
        Self::Waiting,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "Check the game files and settings",
            Self::Database => "Start the game database",
            Self::Save => "Open the saved game",
            Self::Server => "Start the game",
            Self::Waiting => "Prepare the table",
        }
    }
}

/// A running game the launcher started, kept on disk so a reopened launcher finds it again
/// (REF-006) and can still stop it and its database.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRecord {
    pub game: SavedGame,
    pub pid: u32,
    pub server_exe: PathBuf,
    pub stop_dir: PathBuf,
    pub log: PathBuf,
    pub display_url: String,
    pub player_url: String,
    pub postgres_started: bool,
    pub play_root: PathBuf,
    pub postgres_bin: PathBuf,
    /// The database port this game started with; settings may change while it runs.
    #[serde(default)]
    pub database_port: u16,
    /// Set by whoever is stopping the game, so other launchers read its end as a clean stop.
    #[serde(default)]
    pub stopping: bool,
    /// The secret this game's server answers launcher status requests with (REF-064); games
    /// started by an older launcher have none and show no table details.
    #[serde(default)]
    pub status_token: Option<String>,
    /// The server's `DEV CONTROL on url=… token_file=…` line, when developer control is on.
    #[serde(default)]
    pub dev_control: Option<String>,
}

impl RunRecord {
    fn postgres_install(&self) -> Install {
        Install {
            root: PathBuf::new(),
            server_exe: self.server_exe.clone(),
            web_dir: PathBuf::new(),
            assets_dir: PathBuf::new(),
            postgres_bin: self.postgres_bin.clone(),
            play_root: self.play_root.clone(),
            env_file: PathBuf::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Recovery {
    Retry(SavedGame),
    StopAgain(Box<RunRecord>),
    Nothing,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub title: String,
    pub message: String,
    pub log_tail: String,
    pub recovery: Recovery,
}

#[derive(Clone, Debug)]
pub enum Phase {
    Idle {
        notice: Option<String>,
    },
    Starting {
        game: SavedGame,
        step: Step,
        detail: String,
    },
    Running(RunRecord),
    Connected {
        origin: String,
    },
    Stopping(RunRecord),
    Failed(Failure),
}

/// A consistent copy of what the pages show.
pub struct Snapshot {
    pub phase: Phase,
    pub settings: Settings,
    pub settings_problem: Option<String>,
    pub display_open: bool,
    pub display_full_screen: bool,
    pub saves: Vec<SavedGame>,
    pub keys: Result<Keys, String>,
    pub install: Result<Install, String>,
    pub data: PathBuf,
    pub config_path: PathBuf,
    pub windowed: bool,
    pub supervised: bool,
}

struct Inner {
    phase: Phase,
    config: Config,
    settings_problem: Option<String>,
    view: View,
    display_open: bool,
    display_full_screen: bool,
    child: Option<Child>,
    supervised: bool,
}

pub struct App {
    data: PathBuf,
    config_path: PathBuf,
    windowed: bool,
    inner: Mutex<Inner>,
    notify: Box<dyn Fn(ShellEvent) + Send + Sync>,
}

const RUN_FILE: &str = "running-game.toml";
const READY_LIMIT: Duration = Duration::from_secs(600);
const STOP_LIMIT: Duration = Duration::from_secs(30);

impl App {
    /// `windowed` is false for the headless preview, which cannot show a display window.
    pub fn new(
        data: PathBuf,
        config_path: PathBuf,
        windowed: bool,
        notify: Box<dyn Fn(ShellEvent) + Send + Sync>,
    ) -> Arc<Self> {
        log::event(
            "LAUNCHER_START",
            format!(
                "windowed={windowed} data={} config={} version={}",
                log::value(data.display().to_string()),
                log::value(config_path.display().to_string()),
                env!("CARGO_PKG_VERSION")
            ),
        );
        let (config, settings_problem) = match config::load(&config_path) {
            Ok(config) => {
                // First run: write the full commented file so it can be edited by hand.
                if !config_path.exists() {
                    let created = config::save(&config_path, &config);
                    log::event("CONFIG_CREATED", format!("ok={}", created.is_ok()));
                }
                (config, None)
            }
            Err(error) => {
                log::event(
                    "CONFIG_DAMAGED",
                    format!("error={}", log::value(error.to_string())),
                );
                (Config::default(), Some(error.to_string()))
            }
        };
        let app = Arc::new(Self {
            data,
            config_path,
            windowed,
            inner: Mutex::new(Inner {
                phase: Phase::Idle { notice: None },
                config,
                settings_problem,
                view: View::Home,
                display_open: false,
                display_full_screen: false,
                child: None,
                supervised: false,
            }),
            notify,
        });
        app.reconnect();
        app
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves plain data behind; keep serving it.
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn set_phase(&self, phase: Phase) {
        log_phase(&phase);
        let refresh = {
            let mut inner = self.lock();
            inner.phase = phase;
            inner.view == View::Home
        };
        if refresh {
            (self.notify)(ShellEvent::Refresh);
        }
    }

    pub fn data(&self) -> &Path {
        &self.data
    }

    /// The settings as the configuration file holds them now (another launcher or the command
    /// line may have changed it). A damaged file keeps the last good settings in memory.
    pub fn settings(&self) -> Settings {
        self.reload_config();
        self.lock().config.settings.clone()
    }

    fn reload_config(&self) {
        match config::load(&self.config_path) {
            Ok(fresh) => {
                let mut inner = self.lock();
                inner.config = fresh;
                inner.settings_problem = None;
            }
            Err(error) => self.lock().settings_problem = Some(error.to_string()),
        }
    }

    pub fn phase(&self) -> Phase {
        self.lock().phase.clone()
    }

    fn keys(&self, settings: &Settings, install: Option<&Install>) -> Result<Keys, io::Error> {
        let configured = self.lock().config.keys.clone();
        credentials::resolve(
            &configured,
            install
                .filter(|_| settings.keys_from_env_file)
                .map(|install| install.env_file.as_path()),
        )
    }

    /// Where the table display's last picture of `database` is kept.
    pub fn screenshot_path(&self, database: &str) -> Option<PathBuf> {
        saves::valid_database(database)
            .then(|| self.data.join("screens").join(format!("{database}.jpg")))
    }

    /// The running game's screenshot path, for the display window to save into.
    pub fn current_screenshot_path(&self) -> Option<PathBuf> {
        match &self.lock().phase {
            Phase::Running(run) | Phase::Stopping(run) => self.screenshot_path(&run.game.database),
            _ => None,
        }
    }

    /// Picks up a game started by the command line while this window was open. Only from Idle:
    /// a failed stop must keep showing its Force stop choice.
    pub fn refresh_from_disk(self: &Arc<Self>) {
        if !matches!(self.lock().phase, Phase::Idle { .. }) {
            return;
        }
        let Some(run) = self.read_run() else {
            return;
        };
        if !run.stopping && platform::process_is(run.pid, &run.server_exe) {
            log::event(
                "GAME_ADOPTED",
                format!("name={} pid={}", log::value(&run.game.name), run.pid),
            );
            self.set_phase(Phase::Running(run.clone()));
            self.spawn_monitor(run);
        }
    }

    fn read_run(&self) -> Option<RunRecord> {
        let text = std::fs::read_to_string(self.data.join(RUN_FILE)).ok()?;
        let mut run = toml::from_str::<RunRecord>(&text).ok()?;
        run.game.name = run.game.adventure.title().to_owned();
        Some(run)
    }

    pub fn viewing(&self, view: View) {
        self.lock().view = view;
    }

    pub fn snapshot(&self) -> Snapshot {
        self.reload_config();
        let (phase, settings, settings_problem, display_open, display_full_screen, supervised) = {
            let inner = self.lock();
            (
                inner.phase.clone(),
                inner.config.settings.clone(),
                inner.settings_problem.clone(),
                inner.display_open,
                inner.display_full_screen,
                inner.supervised,
            )
        };
        let install = Install::locate(&settings).map_err(|_| Missing::SHOWN.to_owned());
        Snapshot {
            keys: self
                .keys(&settings, install.as_ref().ok())
                .map_err(|error| error.to_string()),
            saves: saves::load(&self.data).unwrap_or_default(),
            install,
            phase,
            settings,
            settings_problem,
            display_open,
            display_full_screen,
            data: self.data.clone(),
            config_path: self.config_path.clone(),
            windowed: self.windowed,
            supervised,
        }
    }

    /// Finds a game left running by an earlier launcher window (REF-006).
    fn reconnect(self: &Arc<Self>) {
        let path = self.data.join(RUN_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(mut run) = toml::from_str::<RunRecord>(&text) else {
            let _ = std::fs::remove_file(&path);
            return;
        };
        run.game.name = run.game.adventure.title().to_owned();
        if platform::process_is(run.pid, &run.server_exe) {
            log::event(
                "GAME_RECONNECTED",
                format!("name={} pid={}", log::value(&run.game.name), run.pid),
            );
            self.lock().phase = Phase::Running(run.clone());
            self.spawn_monitor(run);
            return;
        }
        let install = run.postgres_install();
        if run.postgres_started {
            // The port the game started with, not today's setting.
            let port = if run.database_port == 0 {
                self.lock().config.settings.database_port
            } else {
                run.database_port
            };
            if postgres::ours_running(&install, port) {
                let stopped = postgres::stop(&install);
                log::event(
                    "DATABASE_STOPPED",
                    format!("reason=stale_record ok={}", stopped.is_ok()),
                );
            }
        }
        log::event(
            "GAME_ENDED_WHILE_CLOSED",
            format!("name={}", log::value(&run.game.name)),
        );
        let _ = std::fs::remove_file(&path);
        self.lock().phase = Phase::Idle {
            notice: Some(format!(
                "{} ended while DungeonFlux was closed. You can resume it below.",
                run.game.name
            )),
        };
    }

    fn startable(&self) -> bool {
        matches!(self.lock().phase, Phase::Idle { .. } | Phase::Failed(_))
    }

    /// A new saved game named after its adventure.
    pub fn new_game(&self, adventure: Adventure) -> SavedGame {
        let now = chrono::Utc::now();
        let nanos = now.timestamp_subsec_nanos();
        SavedGame {
            database: saves::new_database(now, nanos ^ std::process::id()),
            name: adventure.title().to_owned(),
            adventure,
            created: now.timestamp(),
            last_played: now.timestamp(),
            played_seconds: 0,
        }
    }

    /// A saved game by database name or by its name, ignoring case.
    pub fn find_save(&self, wanted: &str) -> Option<SavedGame> {
        saves::load(&self.data)
            .ok()?
            .into_iter()
            .find(|game| game.database == wanted || game.name.eq_ignore_ascii_case(wanted.trim()))
    }

    pub fn start_new(self: &Arc<Self>, adventure: Adventure) {
        if !self.startable() {
            return;
        }
        let game = self.new_game(adventure);
        log::event(
            "ACTION_START_NEW",
            format!(
                "adventure={} name={}",
                adventure.server_value(),
                log::value(&game.name)
            ),
        );
        self.begin(game);
    }

    pub fn resume(self: &Arc<Self>, database: &str) {
        if !self.startable() {
            return;
        }
        match self.find_save(database) {
            Some(game) => {
                log::event("ACTION_RESUME", format!("name={}", log::value(&game.name)));
                self.begin(game);
            }
            None => log::event(
                "ACTION_RESUME_UNKNOWN",
                format!("database={}", log::value(database)),
            ),
        }
    }

    /// The command line's start: runs on the caller's thread and leaves the game running.
    pub fn start_blocking(
        self: &Arc<Self>,
        game: &SavedGame,
        options: StartOptions,
    ) -> Result<RunRecord, Box<Failure>> {
        if !self.startable() {
            return Err(Box::new(Failure {
                title: "A game is already running".to_owned(),
                message: "Stop it first with: dungeonflux stop".to_owned(),
                log_tail: String::new(),
                recovery: Recovery::Nothing,
            }));
        }
        self.run_start(game, options)
    }

    pub fn retry(self: &Arc<Self>) {
        let recovery = match &self.lock().phase {
            Phase::Failed(failure) => failure.recovery.clone(),
            _ => return,
        };
        log::event("ACTION_RETRY", "");
        match recovery {
            Recovery::Retry(game) => self.begin(game),
            Recovery::StopAgain(run) => self.begin_stop(*run),
            Recovery::Nothing => self.set_phase(Phase::Idle { notice: None }),
        }
    }

    pub fn dismiss(&self) {
        if matches!(self.lock().phase, Phase::Failed(_) | Phase::Idle { .. }) {
            self.set_phase(Phase::Idle { notice: None });
        }
    }

    fn begin(self: &Arc<Self>, game: SavedGame) {
        self.set_phase(Phase::Starting {
            game: game.clone(),
            step: Step::Checking,
            detail: String::new(),
        });
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let outcome = app.run_start(&game, StartOptions::default());
            if let Err(failure) = outcome {
                app.set_phase(Phase::Failed(*failure));
            }
        });
    }

    fn step(&self, game: &SavedGame, step: Step, detail: &str) {
        // Only changes refresh the page; repeated status lines would reload it constantly.
        let unchanged = matches!(&self.lock().phase,
            Phase::Starting { step: current_step, detail: current_detail, .. }
                if *current_step == step && current_detail == detail);
        if unchanged {
            return;
        }
        log::event(
            "START_STEP",
            format!("step={step:?} detail={}", log::value(detail)),
        );
        self.set_phase(Phase::Starting {
            game: game.clone(),
            step,
            detail: detail.to_owned(),
        });
    }

    fn run_start(
        self: &Arc<Self>,
        game: &SavedGame,
        options: StartOptions,
    ) -> Result<RunRecord, Box<Failure>> {
        let started = std::time::Instant::now();
        let mut settings = self.settings();
        settings.dev_control |= options.dev_control;
        let failed = |title: &str, message: String| {
            Box::new(Failure {
                title: title.to_owned(),
                message,
                log_tail: String::new(),
                recovery: Recovery::Retry(game.clone()),
            })
        };
        let install = Install::locate(&settings)
            .map_err(|missing| failed("DungeonFlux can't find the game", missing.logged()))?;
        log::event(
            "INSTALL",
            format!(
                "root={} server={} postgres={} play={}",
                log::value(install.root.display().to_string()),
                log::value(install.server_exe.display().to_string()),
                log::value(install.postgres_bin.display().to_string()),
                log::value(install.play_root.display().to_string())
            ),
        );
        let keys = self
            .keys(&settings, Some(&install))
            .map_err(|error| failed("The keys can't be read", error.to_string()))?;
        if let Some(message) = server::missing_keys(&settings, &keys) {
            return Err(failed("A key is missing", message));
        }
        let network = server::network(&settings, server::private_address());
        if std::net::TcpListener::bind(network.listen.as_str()).is_err() {
            return Err(failed(
                "The game port is in use",
                format!(
                    "Another program is using port {}. Close it, or change the game port in Settings, under Players and network.",
                    settings.game_port
                ),
            ));
        }

        self.step(game, Step::Database, "");
        let postgres_started = postgres::ensure_running(&install, settings.database_port)
            .map_err(|error| failed("The game database didn't start", error.to_string()))?;
        let stop_postgres = || {
            if postgres_started {
                let _ = postgres::stop(&install);
            }
        };

        self.step(game, Step::Save, "");
        if let Err(error) =
            postgres::ensure_database(&install, settings.database_port, &game.database)
        {
            stop_postgres();
            return Err(failed(
                "The saved game couldn't be opened",
                error.to_string(),
            ));
        }

        self.step(game, Step::Server, "");
        let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let stop_dir = install
            .play_root
            .join("run")
            .join(format!("stop.launcher-{stamp}-{}", std::process::id()));
        let log = install
            .play_root
            .join("launcher-logs")
            .join(format!("{}-{stamp}.log", game.database));
        if let Err(error) = std::fs::create_dir_all(&stop_dir) {
            stop_postgres();
            return Err(failed("The game couldn't start", error.to_string()));
        }
        let status_token = match status_token() {
            Ok(token) => token,
            Err(error) => {
                stop_postgres();
                return Err(failed("The game couldn't start", error.to_string()));
            }
        };
        let launch = server::Launch {
            install: &install,
            settings: &settings,
            keys: &keys,
            network: &network,
            database: &game.database,
            adventure: game.adventure,
            stop_dir: &stop_dir,
            status_token: &status_token,
        };
        // Names only: values may be keys.
        let names: Vec<String> = server::environment(&launch)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mut child = match server::spawn(&launch, &log) {
            Ok(child) => child,
            Err(error) => {
                log::event(
                    "SERVER_SPAWN_FAILED",
                    format!("error={}", log::value(error.to_string())),
                );
                stop_postgres();
                return Err(failed("The game couldn't start", error.to_string()));
            }
        };
        log::event(
            "SERVER_SPAWNED",
            format!(
                "pid={} database={} listen={} log={} env={}",
                child.id(),
                game.database,
                network.listen,
                log::value(log.display().to_string()),
                names.join(",")
            ),
        );
        let origin = network
            .public_url
            .clone()
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", settings.game_port));
        let mut run = RunRecord {
            game: game.clone(),
            pid: child.id(),
            server_exe: install.server_exe.clone(),
            stop_dir,
            log: log.clone(),
            display_url: format!("{origin}/gameplay/display"),
            player_url: format!("{origin}/gameplay/player"),
            postgres_started,
            play_root: install.play_root.clone(),
            postgres_bin: install.postgres_bin.clone(),
            database_port: settings.database_port,
            stopping: false,
            dev_control: None,
            status_token: Some(status_token.clone()),
        };
        // Written before the wait, so a launcher closed during startup still finds the game.
        self.write_run_logged(&run, "start");

        self.step(game, Step::Waiting, "Loading the adventure");
        let waited = server::wait_ready(&mut child, &log, READY_LIMIT, |line| {
            if let Some(detail) = progress_detail(line) {
                self.step(game, Step::Waiting, &detail);
            }
        });
        let failure_with_log = |title: &str, message: &str| {
            Box::new(Failure {
                title: title.to_owned(),
                message: message.to_owned(),
                log_tail: crate::files::tail(&log, 40),
                recovery: Recovery::Retry(game.clone()),
            })
        };
        let ready = match waited {
            Ok(server::Wait::Ready(ready)) => ready,
            Ok(server::Wait::Exited(code)) => {
                log::event("SERVER_EXITED_STARTING", format!("code={code:?}"));
                stop_postgres();
                self.clear_run();
                return Err(failure_with_log(
                    "The game stopped while starting",
                    &match code {
                        Some(code) => format!(
                            "It exited with code {code}. The last lines it printed are below."
                        ),
                        None => {
                            "It ended without an exit code. The last lines it printed are below."
                                .to_owned()
                        }
                    },
                ));
            }
            Ok(server::Wait::TimedOut) | Err(_) => {
                log::event(
                    "SERVER_NOT_READY",
                    format!("limit_s={}", READY_LIMIT.as_secs()),
                );
                let stop_request = server::request_stop(&run.stop_dir).is_ok();
                let (killed, reaped) = if child.wait_timeout(STOP_LIMIT).is_none() {
                    (child.kill().is_ok(), child.wait().is_ok())
                } else {
                    (false, true)
                };
                log::event(
                    "SERVER_NOT_READY_CLEANUP",
                    format!("stop_request={stop_request} killed={killed} reaped={reaped}"),
                );
                stop_postgres();
                self.clear_run();
                return Err(failure_with_log(
                    "The game didn't get ready",
                    "It wasn't ready after 10 minutes, so DungeonFlux stopped it. The last lines it printed are below.",
                ));
            }
        };
        run.display_url = ready.display_url;
        if !ready.player_url.is_empty() {
            run.player_url = ready.player_url;
        }
        if settings.dev_control {
            run.dev_control = server::dev_control_line(&log, Duration::from_secs(5));
        }
        self.write_run_logged(&run, "start");
        let mut played = game.clone();
        played.last_played = chrono::Utc::now().timestamp();
        record_save(&self.data, &played, "started");
        run.game = played;
        {
            let mut inner = self.lock();
            inner.child = Some(child);
        }
        log::event(
            "GAME_READY",
            format!(
                "name={} pid={} ms={} display={}",
                log::value(&run.game.name),
                run.pid,
                started.elapsed().as_millis(),
                run.display_url
            ),
        );
        self.set_phase(Phase::Running(run.clone()));
        self.spawn_monitor(run.clone());
        if !options.quiet_display {
            self.show_display_per_settings(&run.display_url);
        }
        Ok(run)
    }

    fn show_display_per_settings(&self, url: &str) {
        let settings = self.lock().config.settings.clone();
        log::event(
            "DISPLAY_AUTO_OPEN",
            format!("target={:?}", settings.display),
        );
        match settings.display {
            DisplayTarget::Window if self.windowed => (self.notify)(ShellEvent::OpenDisplay {
                url: url.to_owned(),
                screen: settings.display_screen,
                full_screen: settings.full_screen,
            }),
            DisplayTarget::Window | DisplayTarget::Browser => open_logged(url, "display"),
            DisplayTarget::Manual => {}
        }
    }

    pub fn display_url(&self) -> Option<String> {
        match &self.lock().phase {
            Phase::Running(run) => Some(run.display_url.clone()),
            Phase::Connected { origin } => Some(format!("{origin}/gameplay/display")),
            _ => None,
        }
    }

    /// "Show table display": the launcher window when available, else the browser.
    pub fn open_display(&self) {
        let Some(url) = self.display_url() else {
            return;
        };
        let settings = self.lock().config.settings.clone();
        log::event("ACTION_SHOW_DISPLAY", format!("windowed={}", self.windowed));
        if self.windowed {
            (self.notify)(ShellEvent::OpenDisplay {
                url,
                screen: settings.display_screen,
                full_screen: settings.full_screen,
            });
        } else {
            open_logged(&url, "display");
        }
    }

    pub fn open_display_in_browser(&self) {
        if let Some(url) = self.display_url() {
            log::event("ACTION_DISPLAY_BROWSER", "");
            open_logged(&url, "display");
        }
    }

    pub fn set_display_full_screen(&self, full: bool) {
        log::event("ACTION_DISPLAY_FULL_SCREEN", format!("full={full}"));
        (self.notify)(ShellEvent::DisplayFullScreen(full));
    }

    /// The desktop shell reports what the display window is doing.
    pub fn display_changed(&self, open: bool, full_screen: bool) {
        let refresh = {
            let mut inner = self.lock();
            let changed = inner.display_open != open || inner.display_full_screen != full_screen;
            if changed {
                log::event(
                    "DISPLAY_STATE",
                    format!("open={open} full_screen={full_screen}"),
                );
            }
            inner.display_open = open;
            inner.display_full_screen = full_screen;
            changed && inner.view == View::Home
        };
        if refresh {
            (self.notify)(ShellEvent::Refresh);
        }
    }

    fn write_run(&self, run: &RunRecord) -> Result<(), io::Error> {
        let text = toml::to_string_pretty(run).map_err(io::Error::other)?;
        crate::files::write_atomic(&self.data.join(RUN_FILE), text.as_bytes())
    }

    /// Writes the running-game record; a failure is logged, since a launcher reopened later
    /// relies on that file to find the game.
    fn write_run_logged(&self, run: &RunRecord, moment: &str) {
        if let Err(error) = self.write_run(run) {
            log::event(
                "RUN_FILE",
                format!(
                    "write=failed moment={moment} reason={}",
                    log::value(error.to_string())
                ),
            );
        }
    }

    fn clear_run(&self) {
        let _ = std::fs::remove_file(self.data.join(RUN_FILE));
    }

    /// Watches the running game and reports it if it ends on its own.
    fn spawn_monitor(self: &Arc<Self>, run: RunRecord) {
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let mut seconds = 0u64;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                seconds += 1;
                // A fresh picture every two minutes, so a resume card stays current even if
                // the game ends without a clean stop.
                if seconds.is_multiple_of(120)
                    && app.lock().display_open
                    && let Some(path) = app.screenshot_path(&run.game.database)
                {
                    log::event("SCREENSHOT_REQUESTED", "reason=periodic");
                    (app.notify)(ShellEvent::CaptureDisplay { path });
                }
                let owned = {
                    let mut inner = app.lock();
                    let watching =
                        matches!(&inner.phase, Phase::Running(current) if current.pid == run.pid);
                    if !watching {
                        return;
                    }
                    match inner.child.as_mut() {
                        Some(child) if child.id() == run.pid => {
                            Some(matches!(child.try_wait(), Ok(None)))
                        }
                        _ => None,
                    }
                };
                // The process query runs without the lock, so pages never wait on it.
                let alive = owned.unwrap_or_else(|| platform::process_is(run.pid, &run.server_exe));
                if alive {
                    continue;
                }
                app.lock().child = None;
                (app.notify)(ShellEvent::CloseDisplay);
                // Stopped elsewhere (the command line or another window): that stop saved the
                // game and cleans up; this window only reports it.
                if app
                    .read_run()
                    .is_none_or(|current| current.pid != run.pid || current.stopping)
                {
                    log::event(
                        "GAME_STOPPED_ELSEWHERE",
                        format!("name={}", log::value(&run.game.name)),
                    );
                    app.set_phase(Phase::Idle {
                        notice: Some(format!("{} was stopped and saved.", run.game.name)),
                    });
                    return;
                }
                log::event(
                    "GAME_ENDED_UNEXPECTEDLY",
                    format!("name={} pid={}", log::value(&run.game.name), run.pid),
                );
                if run.postgres_started {
                    let _ = postgres::stop(&run.postgres_install());
                }
                record_save(
                    &app.data,
                    &saves::after_session(&run.game, chrono::Utc::now().timestamp()),
                    "ended_unexpectedly",
                );
                app.clear_run();
                app.set_phase(Phase::Failed(Failure {
                title: format!("{} stopped unexpectedly", run.game.name),
                message: "The game ended on its own. Its progress up to the last saved moment is kept; resume it to carry on. The last lines it printed are below.".to_owned(),
                log_tail: crate::files::tail(&run.log, 40),
                recovery: Recovery::Retry(run.game.clone()),
            }));
                return;
            }
        });
    }

    pub fn stop(self: &Arc<Self>) {
        let run = match &self.lock().phase {
            Phase::Running(run) => run.clone(),
            _ => return,
        };
        self.begin_stop(run);
    }

    fn begin_stop(self: &Arc<Self>, run: RunRecord) {
        log::event(
            "ACTION_STOP",
            format!("name={}", log::value(&run.game.name)),
        );
        self.set_phase(Phase::Stopping(run.clone()));
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            app.mark_stopping(&run);
            app.capture_before_close(&run);
            (app.notify)(ShellEvent::CloseDisplay);
            if let Err(error) = server::request_stop(&run.stop_dir) {
                log::event(
                    "STOP_REQUEST_FAILED",
                    format!("error={}", log::value(error.to_string())),
                );
                app.set_phase(Phase::Failed(Failure {
                    title: "The game can't be asked to stop".to_owned(),
                    message: format!("{error} You can force it to stop instead."),
                    log_tail: crate::files::tail(&run.log, 20),
                    recovery: Recovery::StopAgain(Box::new(run)),
                }));
                return;
            }
            log::event(
                "STOP_REQUESTED",
                format!("name={}", log::value(&run.game.name)),
            );
            if app.wait_for_exit(&run, STOP_LIMIT) {
                app.finish_stop(
                    &run,
                    format!("{} is saved. Resume it any time.", run.game.name),
                );
            } else {
                log::event(
                    "STOP_TIMED_OUT",
                    format!("limit_s={}", STOP_LIMIT.as_secs()),
                );
                app.set_phase(Phase::Failed(Failure {
                    title: format!("{} hasn't stopped yet", run.game.name),
                    message: "It may still be saving. Try stopping again in a moment, or force it to stop. Forcing it can keep this game from resuming for about five minutes.".to_owned(),
                    log_tail: crate::files::tail(&run.log, 20),
                    recovery: Recovery::StopAgain(Box::new(run)),
                }));
            }
        });
    }

    /// Records in the running-game file that this stop is deliberate, so a launcher watching
    /// the same game reads the server's exit as a clean stop rather than a crash.
    fn mark_stopping(&self, run: &RunRecord) {
        let mut marked = run.clone();
        marked.stopping = true;
        self.write_run_logged(&marked, "stopping");
    }

    /// Asks the display window for a last picture of the table and waits briefly for it.
    fn capture_before_close(&self, run: &RunRecord) {
        let open = self.lock().display_open;
        let Some(path) = self.screenshot_path(&run.game.database).filter(|_| open) else {
            return;
        };
        let before = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok();
        log::event("SCREENSHOT_REQUESTED", "reason=stop");
        (self.notify)(ShellEvent::CaptureDisplay { path: path.clone() });
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let now = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok();
            if now.is_some() && now != before {
                log::event(
                    "SCREENSHOT_SAVED",
                    format!("reason=stop ms={}", started.elapsed().as_millis()),
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        log::event("SCREENSHOT_MISSING", "reason=stop waited_ms=3000");
    }

    /// The command line's stop: graceful, on the caller's thread.
    pub fn stop_blocking(self: &Arc<Self>, force: bool) -> Result<String, String> {
        let run = self
            .read_run()
            .filter(|run| platform::process_is(run.pid, &run.server_exe))
            .ok_or_else(|| "No game is running.".to_owned())?;
        log::event(
            "CLI_STOP",
            format!("name={} force={force}", log::value(&run.game.name)),
        );
        self.set_phase(Phase::Stopping(run.clone()));
        self.mark_stopping(&run);
        if force {
            platform::terminate(run.pid, &run.server_exe).map_err(|error| error.to_string())?;
            if !self.wait_for_exit(&run, Duration::from_secs(5)) {
                return Err(format!(
                    "{} did not end after being forced to stop; it is left as it is.",
                    run.game.name
                ));
            }
        } else {
            server::request_stop(&run.stop_dir).map_err(|error| error.to_string())?;
            if !self.wait_for_exit(&run, STOP_LIMIT) {
                return Err(format!(
                    "{} has not stopped after 30 seconds. Run dungeonflux stop again, or dungeonflux stop --force.",
                    run.game.name
                ));
            }
        }
        self.finish_stop(&run, format!("{} is saved.", run.game.name));
        Ok(format!(
            "STOPPED name={:?} database={}",
            run.game.name, run.game.database
        ))
    }

    pub fn force_stop(self: &Arc<Self>) {
        let run = match &self.lock().phase {
            Phase::Failed(Failure {
                recovery: Recovery::StopAgain(run),
                ..
            }) => run.as_ref().clone(),
            _ => return,
        };
        log::event(
            "ACTION_FORCE_STOP",
            format!("name={}", log::value(&run.game.name)),
        );
        self.set_phase(Phase::Stopping(run.clone()));
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let terminated = platform::terminate(run.pid, &run.server_exe);
            // Only a confirmed exit ends tracking; otherwise the game is still running.
            if terminated.is_ok() && app.wait_for_exit(&run, Duration::from_secs(5)) {
                app.finish_stop(
                    &run,
                    format!(
                        "{} was forced to stop. It can be resumed in about five minutes.",
                        run.game.name
                    ),
                );
                return;
            }
            let reason = match terminated {
                Err(error) => error.to_string(),
                Ok(()) => "It was still running five seconds later.".to_owned(),
            };
            log::event(
                "FORCE_STOP_FAILED",
                format!("reason={}", log::value(&reason)),
            );
            app.set_phase(Phase::Failed(Failure {
                title: format!("{} could not be forced to stop", run.game.name),
                message: format!(
                    "{reason} Try again, or end df-transport-fixture in Task Manager."
                ),
                log_tail: crate::files::tail(&run.log, 20),
                recovery: Recovery::StopAgain(Box::new(run)),
            }));
        });
    }

    fn wait_for_exit(&self, run: &RunRecord, limit: Duration) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < limit {
            let owned = {
                let mut inner = self.lock();
                match inner.child.as_mut() {
                    Some(child) if child.id() == run.pid => {
                        Some(matches!(child.try_wait(), Ok(None)))
                    }
                    _ => None,
                }
            };
            let alive = owned.unwrap_or_else(|| platform::process_is(run.pid, &run.server_exe));
            if !alive {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn finish_stop(&self, run: &RunRecord, mut notice: String) {
        self.lock().child = None;
        if run.postgres_started {
            let stopped = postgres::stop(&run.postgres_install());
            log::event(
                "DATABASE_STOPPED",
                format!("reason=game_stopped ok={}", stopped.is_ok()),
            );
            if let Err(error) = stopped {
                notice.push_str(&format!(" The game database is still running: {error}"));
            }
        }
        let played = saves::after_session(&run.game, chrono::Utc::now().timestamp());
        log::event(
            "GAME_STOPPED",
            format!(
                "name={} session_s={} played_seconds={}",
                log::value(&run.game.name),
                played.played_seconds - run.game.played_seconds,
                played.played_seconds
            ),
        );
        record_save(&self.data, &played, "stopped");
        self.clear_run();
        self.set_phase(Phase::Idle {
            notice: Some(notice),
        });
    }

    /// Developer mode: use a server started elsewhere instead of the launcher's own. The name
    /// lookup and connection test run off the window's thread.
    pub fn connect(self: &Arc<Self>) -> Result<(), String> {
        if !self.startable() {
            return Ok(());
        }
        let settings = self.settings();
        if settings.server_mode != ServerMode::External {
            return Err(
                "Choose \"Connect to a server that is already running\" in Settings first."
                    .to_owned(),
            );
        }
        crate::settings::external_origin(&settings.external_url)?;
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(message) = app.connect_to(&settings.external_url, false) {
                log::event("CONNECT_REFUSED", format!("reason={}", log::value(message)));
            }
        });
        Ok(())
    }

    /// Attaches to a server started elsewhere for this session only (`--connect`, or the
    /// Connect button in developer mode). A supervisor-provided run record uses the normal
    /// running-game view and save/stop path; an ordinary attachment leaves the server external.
    /// `always_show` opens the display even when settings say not to (the `--connect` flag).
    pub fn connect_to(self: &Arc<Self>, url: &str, always_show: bool) -> Result<(), String> {
        if !self.startable() {
            return Ok(());
        }
        let origin = crate::settings::external_origin(url)?;
        log::event("ACTION_CONNECT", format!("origin={}", log::value(&origin)));
        if let Ok(record) = std::env::var("DF_RUNTIME_LAUNCHER_RECORD") {
            return self.attach_runtime(&origin, &record);
        }

        let host = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&origin);
        let host = if host.contains(':') {
            host.to_owned()
        } else if origin.starts_with("https://") {
            format!("{host}:443")
        } else {
            format!("{host}:80")
        };
        let reachable = std::net::ToSocketAddrs::to_socket_addrs(&host)
            .ok()
            .and_then(|mut addresses| addresses.next())
            .is_some_and(|address| {
                std::net::TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok()
            });
        if !reachable {
            log::event(
                "CONNECT_UNREACHABLE",
                format!(
                    "origin={} hint=start_the_server_with_development/run-gameplay.sh",
                    log::value(&origin)
                ),
            );
            self.set_phase(Phase::Failed(Failure {
                title: "No server answered".to_owned(),
                message: format!(
                    "No game server answered at {origin}. Check the address, then connect again."
                ),
                log_tail: String::new(),
                recovery: Recovery::Nothing,
            }));
            return Ok(());
        }
        self.set_phase(Phase::Connected {
            origin: origin.clone(),
        });
        if always_show {
            self.open_display();
        } else {
            self.show_display_per_settings(&format!("{origin}/gameplay/display"));
        }
        Ok(())
    }

    fn attach_runtime(self: &Arc<Self>, origin: &str, record: &str) -> Result<(), String> {
        let run: RunRecord = toml::from_str(record)
            .map_err(|_| "The session launcher could not read its running game.".to_owned())?;
        if !run.display_url.starts_with(&format!("{origin}/"))
            || !run.player_url.starts_with(&format!("{origin}/"))
            || !platform::process_is(run.pid, &run.server_exe)
        {
            return Err("The session's game is unavailable.".to_owned());
        }
        self.write_run(&run)
            .map_err(|_| "The session launcher could not retain its game.".to_owned())?;
        self.lock().supervised = true;
        self.set_phase(Phase::Running(run.clone()));
        self.spawn_monitor(run);
        Ok(())
    }

    pub fn disconnect(&self) {
        if matches!(self.lock().phase, Phase::Connected { .. }) {
            log::event("ACTION_DISCONNECT", "");
            (self.notify)(ShellEvent::CloseDisplay);
            self.set_phase(Phase::Idle { notice: None });
        }
    }

    /// Applies `change` to the configuration file as it is on disk now (the command line or a
    /// hand edit may have changed it), saves it, then keeps it. A damaged file is never
    /// overwritten: that would lose every setting and key in it.
    fn change_config(&self, change: impl FnOnce(&mut Config)) -> Result<(), io::Error> {
        let mut next = config::load(&self.config_path).map_err(|error| {
            io::Error::other(format!(
                "{error} Fix or remove the file first; nothing was changed."
            ))
        })?;
        change(&mut next);
        config::save(&self.config_path, &next)?;
        let mut inner = self.lock();
        inner.config = next;
        inner.settings_problem = None;
        Ok(())
    }

    /// Every setting and whether each key is set, from the file as it is now.
    pub fn config_list(&self) -> Result<Vec<(String, String)>, String> {
        let current = config::load(&self.config_path).map_err(|error| error.to_string())?;
        config::list(&current)
    }

    /// Sets one setting or key by name (see `config::set`) and saves the file.
    pub fn config_set(&self, name: &str, value: &str) -> Result<String, String> {
        self.try_change_config(|config| config::set(config, name, value))
            .inspect(|changed| log::event("CONFIG_SET", format!("name={changed}")))
    }

    /// Returns one setting to its default, or removes a key, and saves the file.
    pub fn config_reset(&self, name: &str) -> Result<String, String> {
        self.try_change_config(|config| config::reset(config, name))
            .inspect(|changed| log::event("CONFIG_RESET", format!("name={changed}")))
    }

    /// `change_config` for a change that may be refused; nothing is saved then.
    fn try_change_config(
        &self,
        change: impl FnOnce(&mut Config) -> Result<String, String>,
    ) -> Result<String, String> {
        let mut next = config::load(&self.config_path).map_err(|error| {
            format!("{error} Fix or remove the file first; nothing was changed.")
        })?;
        let changed = change(&mut next)?;
        config::save(&self.config_path, &next).map_err(|error| error.to_string())?;
        let mut inner = self.lock();
        inner.config = next;
        inner.settings_problem = None;
        Ok(changed)
    }

    pub fn save_settings(&self, form: &Form) -> Result<(), Vec<FieldError>> {
        let current = self.settings();
        let mut next = current.with_form(form)?;
        if self.lock().supervised {
            next.keys_from_env_file = current.keys_from_env_file;
            next.server_mode = current.server_mode;
            next.external_url.clone_from(&current.external_url);
            next.dev_control = current.dev_control;
            next.dev_overrides = current.dev_overrides;
            next.game_folder.clone_from(&current.game_folder);
            next.build_folder.clone_from(&current.build_folder);
            next.postgres_folder.clone_from(&current.postgres_folder);
            next.play_folder.clone_from(&current.play_folder);
        }
        log::event(
            "SETTINGS_SAVED",
            format!(
                "changed={}",
                crate::settings::changed_names(&current, &next).join(",")
            ),
        );
        self.change_config(|config| config.settings = next)
            .map_err(|error| {
                vec![FieldError {
                    field: "",
                    message: format!("Settings could not be saved: {error}"),
                }]
            })
    }

    pub fn store_key(&self, credential: Credential, value: &str) -> Result<(), String> {
        let value = value.trim();
        if !credentials::valid_key(value) {
            return Err(
                "A key is one line of letters, digits and symbols, without spaces.".to_owned(),
            );
        }
        // The credential's name only, never its value.
        log::event("KEY_SAVED", format!("credential={}", credential.id()));
        self.change_config(|config| {
            config.keys.insert(credential, value.to_owned());
        })
        .map_err(|error| format!("The key could not be saved: {error}"))
    }

    pub fn remove_key(&self, credential: Credential) -> Result<(), String> {
        log::event("KEY_REMOVED", format!("credential={}", credential.id()));
        self.change_config(|config| {
            config.keys.remove(&credential);
        })
        .map_err(|error| format!("The key could not be removed: {error}"))
    }

    pub fn open_logs(&self) {
        let folder = match &self.lock().phase {
            Phase::Running(run) | Phase::Stopping(run) => run.log.parent().map(Path::to_path_buf),
            _ => None,
        };
        let settings = self.settings();
        let folder = folder.or_else(|| {
            Install::locate(&settings)
                .ok()
                .map(|install| install.play_root.join("launcher-logs"))
        });
        log::event("ACTION_OPEN_LOGS", "");
        if let Some(folder) = folder.filter(|folder| folder.is_dir()) {
            open_logged(folder, "logs");
        }
    }

    pub fn open_data_folder(&self) {
        open_logged(&self.data, "data");
    }

    pub fn open_config_folder(&self) {
        if let Some(folder) = self.config_path.parent() {
            open_logged(folder, "config");
        }
    }
}

/// Records a saved game's times; a failure is logged instead of passing unnoticed.
fn record_save(data: &Path, game: &SavedGame, moment: &str) {
    if let Err(error) = saves::record(data, game) {
        log::event(
            "SAVE_RECORD",
            format!(
                "outcome=failed moment={moment} database={} reason={}",
                game.database,
                log::value(error.to_string())
            ),
        );
    }
}

/// Opens a link or folder with the system; a failure is logged.
fn open_logged(target: impl AsRef<std::ffi::OsStr>, what: &str) {
    if let Err(error) = open::that_detached(target) {
        log::event(
            "OPEN_FAILED",
            format!("what={what} reason={}", log::value(error.to_string())),
        );
    }
}

/// 32 random bytes in hex: the secret a started server answers launcher status requests with.
fn status_token() -> io::Result<String> {
    let mut secret = [0_u8; 32];
    getrandom::getrandom(&mut secret).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(secret.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn log_phase(phase: &Phase) {
    let (name, game) = match phase {
        Phase::Idle { .. } => ("idle", None),
        Phase::Starting { game, .. } => ("starting", Some(game.name.as_str())),
        Phase::Running(run) => ("running", Some(run.game.name.as_str())),
        Phase::Connected { origin } => ("connected", Some(origin.as_str())),
        Phase::Stopping(run) => ("stopping", Some(run.game.name.as_str())),
        Phase::Failed(failure) => ("failed", Some(failure.title.as_str())),
    };
    if name != "starting" {
        log::event(
            "PHASE",
            format!("to={name} about={}", log::value(game.unwrap_or(""))),
        );
    }
}

/// Plain-language progress for the server's startup status lines; other lines are not shown.
fn progress_detail(line: &str) -> Option<String> {
    let first = line.split(' ').next().unwrap_or("");
    // After a hard stop the server waits for the old session's lease before reopening the save:
    // `SESSION waiting for the previous owner's lease, 234 s`.
    if first == "SESSION" && line.contains("previous owner's lease") {
        let seconds = line
            .rsplit(", ")
            .next()
            .and_then(|rest| rest.trim_end_matches(" s").trim().parse::<u64>().ok());
        return Some(match seconds {
            Some(seconds) if seconds >= 90 => format!(
                "Waiting for the last session to let go of this save, about {} minutes",
                seconds.div_ceil(60)
            ),
            Some(seconds) => format!(
                "Waiting for the last session to let go of this save, about {seconds} seconds"
            ),
            None => "Waiting for the last session to let go of this save".to_owned(),
        });
    }
    Some(
        match first {
            "ADVENTURE" | "ROOM" => "Setting the scene",
            "STORYTELLER" | "MEMORY" => "Waking the storyteller",
            "VOICE" | "MUSIC" | "SPEECH" => "Tuning voices and music",
            "IMAGERY" | "PLATES" | "HOME" => "Preparing pictures",
            "JOIN" => "Opening the table to players",
            _ => return None,
        }
        .to_owned(),
    )
}

trait WaitTimeout {
    fn wait_timeout(&mut self, limit: Duration) -> Option<std::process::ExitStatus>;
}

impl WaitTimeout for Child {
    fn wait_timeout(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let started = std::time::Instant::now();
        while started.elapsed() < limit {
            if let Ok(Some(status)) = self.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_are_translated_and_others_hidden() {
        assert_eq!(
            progress_detail("ROOM ABCD").as_deref(),
            Some("Setting the scene")
        );
        assert_eq!(
            progress_detail("STORYTELLER on model=x").as_deref(),
            Some("Waking the storyteller")
        );
        assert_eq!(
            progress_detail("SESSION waiting for the previous owner's lease, 234 s").as_deref(),
            Some("Waiting for the last session to let go of this save, about 4 minutes")
        );
        assert_eq!(
            progress_detail("SESSION waiting for the previous owner's lease, 40 s").as_deref(),
            Some("Waiting for the last session to let go of this save, about 40 seconds")
        );
        assert_eq!(progress_detail("RESOURCE kind=x"), None);
        assert_eq!(progress_detail(""), None);
    }

    #[test]
    fn new_harbor_saves_use_the_game_content_title() {
        let data =
            std::env::temp_dir().join(format!("df-launcher-new-save-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let app = App::new(
            data.clone(),
            data.join("dungeonflux.toml"),
            false,
            Box::new(|_| {}),
        );

        let game = app.new_game(Adventure::Harbor);

        assert_eq!(game.name, "The Broken Seal");
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn a_stale_run_record_is_cleared_with_a_notice() {
        let data = std::env::temp_dir().join(format!("df-launcher-app-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let run = RunRecord {
            game: SavedGame {
                database: "df_gameplay_demo_play_x".into(),
                name: "The Amber Bell".into(),
                adventure: Adventure::Harbor,
                created: 1,
                last_played: 1,
                played_seconds: 0,
            },
            pid: u32::MAX - 1,
            server_exe: data.join("missing.exe"),
            stop_dir: data.join("stop"),
            log: data.join("log"),
            display_url: "http://127.0.0.1:1/gameplay/display".into(),
            player_url: String::new(),
            postgres_started: false,
            play_root: data.join("play"),
            postgres_bin: data.join("pg"),
            database_port: 55517,
            stopping: false,
            dev_control: None,
            status_token: None,
        };
        std::fs::write(data.join(RUN_FILE), toml::to_string(&run).unwrap()).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&events);
        let app = App::new(
            data.clone(),
            data.join("dungeonflux.toml"),
            false,
            Box::new(move |event| seen.lock().unwrap().push(event)),
        );
        match app.snapshot().phase {
            Phase::Idle {
                notice: Some(notice),
            } => assert!(notice.contains("The Broken Seal")),
            other => panic!("unexpected phase {other:?}"),
        }
        assert!(!data.join(RUN_FILE).exists());
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn a_placed_session_uses_the_running_launcher_and_rejects_another_origin() {
        let data = std::env::temp_dir().join(format!("df-launcher-placed-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let app = App::new(
            data.clone(),
            data.join("dungeonflux.toml"),
            false,
            Box::new(|_| {}),
        );
        let run = RunRecord {
            game: SavedGame {
                database: "df_gameplay_demo_rtlocal_test".into(),
                name: "Your game".into(),
                adventure: Adventure::Harbor,
                created: 1,
                last_played: chrono::Utc::now().timestamp(),
                played_seconds: 0,
            },
            pid: std::process::id(),
            server_exe: std::env::current_exe().unwrap(),
            stop_dir: data.join("stop"),
            log: data.join("game.log"),
            display_url: "http://127.0.0.1:45678/gameplay/display".into(),
            player_url: "http://127.0.0.1:45678/gameplay/player?room=BADEFO".into(),
            postgres_started: false,
            play_root: data.clone(),
            postgres_bin: data.clone(),
            database_port: 55911,
            stopping: false,
            status_token: None,
            dev_control: None,
        };
        let record = toml::to_string(&run).unwrap();
        assert!(
            app.attach_runtime("http://127.0.0.1:45679", &record)
                .is_err()
        );
        assert!(!app.snapshot().supervised);
        app.attach_runtime("http://127.0.0.1:45678", &record)
            .unwrap();
        let snapshot = app.snapshot();
        assert!(matches!(snapshot.phase, Phase::Running(_)));
        assert!(snapshot.supervised);
        let settings = crate::ui::handle(&app, "GET", "/settings", &[]);
        let settings = String::from_utf8(settings.body).unwrap();
        for section in [
            "Players and network",
            "Table display",
            "Story and sound",
            "Pictures",
            "Video",
        ] {
            assert!(
                settings.contains(&format!("<h2>{section}</h2>")),
                "{section}"
            );
        }
        for hidden in [
            "Developer",
            "Files and command line",
            ".env",
            "dungeonflux start",
        ] {
            assert!(
                !settings.contains(hidden),
                "unexpected supervised copy: {hidden}"
            );
        }
        let page = crate::ui::handle(&app, "GET", "/", &[]);
        let body = String::from_utf8(page.body).unwrap();
        assert!(body.contains("BADEFO"));
        assert!(body.contains("Stop and save"));
        assert!(!body.contains("Developer mode: the game"));
        assert!(!body.contains("/settings#developer"));
        assert!(!body.contains("dungeonflux --help"));
        assert_eq!(app.read_run().unwrap().player_url, run.player_url);
        app.set_phase(Phase::Idle { notice: None });
        std::fs::remove_dir_all(data).unwrap();
    }
}
