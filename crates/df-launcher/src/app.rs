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
    install::Install,
    platform, postgres,
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
}

struct Inner {
    phase: Phase,
    config: Config,
    settings_problem: Option<String>,
    view: View,
    display_open: bool,
    display_full_screen: bool,
    child: Option<Child>,
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
        let (config, settings_problem) = match config::load(&config_path) {
            Ok(config) => {
                // First run: write the full commented file so it can be edited by hand.
                if !config_path.exists() {
                    let _ = config::save(&config_path, &config);
                }
                (config, None)
            }
            Err(error) => (Config::default(), Some(error.to_string())),
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

    pub fn settings(&self) -> Settings {
        self.lock().config.settings.clone()
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

    /// Picks up a game started by the command line while this window was open.
    pub fn refresh_from_disk(self: &Arc<Self>) {
        if !self.startable() {
            return;
        }
        let Some(run) = self.read_run() else {
            return;
        };
        if platform::process_is(run.pid, &run.server_exe) {
            self.lock().phase = Phase::Running(run.clone());
            self.spawn_monitor(run);
        }
    }

    fn read_run(&self) -> Option<RunRecord> {
        let text = std::fs::read_to_string(self.data.join(RUN_FILE)).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn viewing(&self, view: View) {
        self.lock().view = view;
    }

    pub fn snapshot(&self) -> Snapshot {
        let (phase, settings, settings_problem, display_open, display_full_screen) = {
            let inner = self.lock();
            (
                inner.phase.clone(),
                inner.config.settings.clone(),
                inner.settings_problem.clone(),
                inner.display_open,
                inner.display_full_screen,
            )
        };
        let install = Install::locate(&settings).map_err(|missing| missing.0);
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
        }
    }

    /// Finds a game left running by an earlier launcher window (REF-006).
    fn reconnect(self: &Arc<Self>) {
        let path = self.data.join(RUN_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(run) = toml::from_str::<RunRecord>(&text) else {
            let _ = std::fs::remove_file(&path);
            return;
        };
        if platform::process_is(run.pid, &run.server_exe) {
            self.lock().phase = Phase::Running(run.clone());
            self.spawn_monitor(run);
            return;
        }
        let install = run.postgres_install();
        if run.postgres_started {
            let port = self.lock().config.settings.database_port;
            if postgres::ours_running(&install, port) {
                let _ = postgres::stop(&install);
            }
        }
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

    /// A new saved game with an automatic name (REF-009).
    pub fn new_game(&self, adventure: Adventure) -> SavedGame {
        let existing = saves::load(&self.data).unwrap_or_default();
        let now = chrono::Utc::now();
        let nanos = now.timestamp_subsec_nanos();
        let seed = u64::from(nanos) ^ (now.timestamp() as u64).rotate_left(17);
        SavedGame {
            database: saves::new_database(now, nanos ^ std::process::id()),
            name: saves::new_name(seed, &existing),
            adventure,
            created: now.timestamp(),
            last_played: now.timestamp(),
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
        self.begin(game);
    }

    pub fn resume(self: &Arc<Self>, database: &str) {
        if !self.startable() {
            return;
        }
        if let Some(game) = self.find_save(database) {
            self.begin(game);
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
        let mut settings = self.lock().config.settings.clone();
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
            .map_err(|missing| failed("DungeonFlux can't find the game", missing.0))?;
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
        let launch = server::Launch {
            install: &install,
            settings: &settings,
            keys: &keys,
            network: &network,
            database: &game.database,
            adventure: game.adventure,
            stop_dir: &stop_dir,
        };
        let mut child = match server::spawn(&launch, &log) {
            Ok(child) => child,
            Err(error) => {
                stop_postgres();
                return Err(failed("The game couldn't start", error.to_string()));
            }
        };
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
            dev_control: None,
        };
        // Written before the wait, so a launcher closed during startup still finds the game.
        let _ = self.write_run(&run);

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
                let _ = server::request_stop(&run.stop_dir);
                if child.wait_timeout(STOP_LIMIT).is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
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
        let _ = self.write_run(&run);
        let mut played = game.clone();
        played.last_played = chrono::Utc::now().timestamp();
        let _ = saves::record(&self.data, &played);
        run.game = played;
        {
            let mut inner = self.lock();
            inner.child = Some(child);
        }
        self.set_phase(Phase::Running(run.clone()));
        self.spawn_monitor(run.clone());
        if !options.quiet_display {
            self.show_display_per_settings(&run.display_url);
        }
        Ok(run)
    }

    fn show_display_per_settings(&self, url: &str) {
        let settings = self.lock().config.settings.clone();
        match settings.display {
            DisplayTarget::Window if self.windowed => (self.notify)(ShellEvent::OpenDisplay {
                url: url.to_owned(),
                screen: settings.display_screen,
                full_screen: settings.full_screen,
            }),
            DisplayTarget::Window | DisplayTarget::Browser => {
                let _ = open::that_detached(url);
            }
            DisplayTarget::Manual => {}
        }
    }

    fn display_url(&self) -> Option<String> {
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
        if self.windowed {
            (self.notify)(ShellEvent::OpenDisplay {
                url,
                screen: settings.display_screen,
                full_screen: settings.full_screen,
            });
        } else {
            let _ = open::that_detached(url);
        }
    }

    pub fn open_display_in_browser(&self) {
        if let Some(url) = self.display_url() {
            let _ = open::that_detached(url);
        }
    }

    pub fn set_display_full_screen(&self, full: bool) {
        (self.notify)(ShellEvent::DisplayFullScreen(full));
    }

    /// The desktop shell reports what the display window is doing.
    pub fn display_changed(&self, open: bool, full_screen: bool) {
        let refresh = {
            let mut inner = self.lock();
            let changed = inner.display_open != open || inner.display_full_screen != full_screen;
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
                    && let Some(path) = app.screenshot_path(&run.game.database)
                {
                    (app.notify)(ShellEvent::CaptureDisplay { path });
                }
                let alive = {
                    let mut inner = app.lock();
                    let watching =
                        matches!(&inner.phase, Phase::Running(current) if current.pid == run.pid);
                    if !watching {
                        return;
                    }
                    match inner.child.as_mut() {
                        Some(child) if child.id() == run.pid => {
                            matches!(child.try_wait(), Ok(None))
                        }
                        _ => platform::process_is(run.pid, &run.server_exe),
                    }
                };
                if alive {
                    continue;
                }
                app.lock().child = None;
                (app.notify)(ShellEvent::CloseDisplay);
                // Stopped from the command line: that command already saved and cleaned up.
                if app.read_run().is_none_or(|current| current.pid != run.pid) {
                    app.set_phase(Phase::Idle {
                        notice: Some(format!("{} was stopped and saved.", run.game.name)),
                    });
                    return;
                }
                if run.postgres_started {
                    let _ = postgres::stop(&run.postgres_install());
                }
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
        self.set_phase(Phase::Stopping(run.clone()));
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            app.capture_before_close(&run);
            (app.notify)(ShellEvent::CloseDisplay);
            if let Err(error) = server::request_stop(&run.stop_dir) {
                app.set_phase(Phase::Failed(Failure {
                    title: "The game can't be asked to stop".to_owned(),
                    message: format!("{error} You can force it to stop instead."),
                    log_tail: crate::files::tail(&run.log, 20),
                    recovery: Recovery::StopAgain(Box::new(run)),
                }));
                return;
            }
            if app.wait_for_exit(&run, STOP_LIMIT) {
                app.finish_stop(
                    &run,
                    format!("{} is saved. Resume it any time.", run.game.name),
                );
            } else {
                app.set_phase(Phase::Failed(Failure {
                    title: format!("{} hasn't stopped yet", run.game.name),
                    message: "It may still be saving. Try stopping again in a moment, or force it to stop. Forcing it can keep this game from resuming for about five minutes.".to_owned(),
                    log_tail: crate::files::tail(&run.log, 20),
                    recovery: Recovery::StopAgain(Box::new(run)),
                }));
            }
        });
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
        (self.notify)(ShellEvent::CaptureDisplay { path: path.clone() });
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let now = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok();
            if now.is_some() && now != before {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The command line's stop: graceful, on the caller's thread.
    pub fn stop_blocking(self: &Arc<Self>, force: bool) -> Result<String, String> {
        let run = self
            .read_run()
            .filter(|run| platform::process_is(run.pid, &run.server_exe))
            .ok_or_else(|| "No game is running.".to_owned())?;
        if force {
            platform::terminate(run.pid, &run.server_exe).map_err(|error| error.to_string())?;
            let _ = self.wait_for_exit(&run, Duration::from_secs(5));
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
        self.set_phase(Phase::Stopping(run.clone()));
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let notice = match platform::terminate(run.pid, &run.server_exe) {
                Ok(()) => format!(
                    "{} was forced to stop. It can be resumed in about five minutes.",
                    run.game.name
                ),
                Err(error) => format!("{} could not be forced to stop: {error}", run.game.name),
            };
            let _ = app.wait_for_exit(&run, Duration::from_secs(5));
            app.finish_stop(&run, notice);
        });
    }

    fn wait_for_exit(&self, run: &RunRecord, limit: Duration) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < limit {
            let alive = {
                let mut inner = self.lock();
                match inner.child.as_mut() {
                    Some(child) if child.id() == run.pid => matches!(child.try_wait(), Ok(None)),
                    _ => platform::process_is(run.pid, &run.server_exe),
                }
            };
            if !alive {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn finish_stop(&self, run: &RunRecord, mut notice: String) {
        self.lock().child = None;
        if run.postgres_started
            && let Err(error) = postgres::stop(&run.postgres_install())
        {
            notice.push_str(&format!(" The game database is still running: {error}"));
        }
        let mut played = run.game.clone();
        played.last_played = chrono::Utc::now().timestamp();
        let _ = saves::record(&self.data, &played);
        self.clear_run();
        self.set_phase(Phase::Idle {
            notice: Some(notice),
        });
    }

    /// Developer mode: use a server started elsewhere instead of the launcher's own.
    pub fn connect(&self) -> Result<(), String> {
        if !self.startable() {
            return Ok(());
        }
        let settings = self.lock().config.settings.clone();
        if settings.server_mode != ServerMode::External {
            return Err(
                "Choose \"Connect to a server that is already running\" in Settings first."
                    .to_owned(),
            );
        }
        self.connect_to(&settings.external_url, false)
    }

    /// Attaches to a server started elsewhere for this session only (`--connect`, or the
    /// Connect button in developer mode). Nothing is saved and the server is never stopped.
    /// `always_show` opens the display even when settings say not to (the `--connect` flag).
    pub fn connect_to(&self, url: &str, always_show: bool) -> Result<(), String> {
        if !self.startable() {
            return Ok(());
        }
        let origin = crate::settings::external_origin(url)?;
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
            self.set_phase(Phase::Failed(Failure {
                title: "No server answered".to_owned(),
                message: format!("Nothing is listening at {origin}. Start the server first (for example with development/run-gameplay.sh), then connect again."),
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

    pub fn disconnect(&self) {
        if matches!(self.lock().phase, Phase::Connected { .. }) {
            (self.notify)(ShellEvent::CloseDisplay);
            self.set_phase(Phase::Idle { notice: None });
        }
    }

    /// Applies `change` to a copy of the configuration, saves it, then keeps it.
    fn change_config(&self, change: impl FnOnce(&mut Config)) -> Result<(), io::Error> {
        let mut next = self.lock().config.clone();
        change(&mut next);
        config::save(&self.config_path, &next)?;
        let mut inner = self.lock();
        inner.config = next;
        inner.settings_problem = None;
        Ok(())
    }

    pub fn save_settings(&self, form: &Form) -> Result<(), Vec<FieldError>> {
        let next = self.settings().with_form(form)?;
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
        self.change_config(|config| {
            config.keys.insert(credential, value.to_owned());
        })
        .map_err(|error| format!("The key could not be saved: {error}"))
    }

    pub fn remove_key(&self, credential: Credential) -> Result<(), String> {
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
        let folder = folder.or_else(|| {
            Install::locate(&self.lock().config.settings)
                .ok()
                .map(|install| install.play_root.join("launcher-logs"))
        });
        if let Some(folder) = folder.filter(|folder| folder.is_dir()) {
            let _ = open::that_detached(folder);
        }
    }

    pub fn open_data_folder(&self) {
        let _ = open::that_detached(&self.data);
    }

    pub fn open_config_folder(&self) {
        if let Some(folder) = self.config_path.parent() {
            let _ = open::that_detached(folder);
        }
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
            dev_control: None,
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
            } => assert!(notice.contains("The Amber Bell")),
            other => panic!("unexpected phase {other:?}"),
        }
        assert!(!data.join(RUN_FILE).exists());
        std::fs::remove_dir_all(data).unwrap();
    }
}
