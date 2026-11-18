//! The table's scenario (`serve --scenario <beats file>`): when a phone starts the game from the
//! lobby, the host starts the day for that seat's hero, with no command typed during play.
//!
//! It runs the dev runner (`development/market-thread.sh`) on the beats file in player mode for
//! the seat (`DF_RUN_PLAYER`), its hero as the roam's character (`DF_RUN_ROAM`, the persona id
//! `hero-<id>`, and `DF_RUN_HERO_*` for the runner to make the hero's persona, kit sheets and
//! starting plate: `DF_RUN_HERO_SHEET` is the reference sheet the look studio answers for the
//! hero, `DF_RUN_HERO_LOOK` the locked design in words), as a child process the host owns: its
//! output goes to a log file, it is stopped with its whole process tree when the host stops
//! (`taskkill /T` on Windows; elsewhere the runner alone is killed), and one runs at a time. It
//! never sets a paid switch (`DF_RUN_TTS`, `DF_RUN_MUSIC`): the runner uses what the toolbar says
//! and logs it (owner, 2026-11-16). The seat is bound to the hero's persona, so its phone plays the
//! day. A runner that exits with `PREPARE_FAILED_STATUS` could not make the hero ready.
//!
//! One runner plays the day for the whole party: every seat at the table, in seat order. Besides
//! the single-seat variables (of the first seat, unchanged for a one-seat day) it gets
//! `DF_RUN_PLAYERS` (`1,2`: the seats, in seat order) and, for each seat `n`, `DF_RUN_HERO_<n>_ID`,
//! `_NAME`,
//! `_SPECIES`, `_CLASS`, `_SHEET`, `_LOOK`, `_LOOK_KEY` and `_PERSONA`, which mean what the
//! single-seat `DF_RUN_HERO_*` variables (and `DF_RUN_ROAM`) mean. Every seat of the party is bound
//! at Start, unbound and told how the day ended at its end.
//!
//! The runner runs in Git Bash, named by path (`find_shell`): on Windows a bare `bash` is the
//! System32 WSL launcher, which CreateProcess finds before the PATH.
//!
//! Lines: `SCENARIO started seat= seats= hero= persona= log=`, `SCENARIO ended status= seat=
//! seats=`, `SCENARIO start_failed cause= seat= seats=`, `SCENARIO already_running`, `SCENARIO
//! stopped tree=` (`seat` is the party's first seat).
//! How the seat's day ended (`DayEnd`) is in its seat view, so its phone says so.
//!
//! The seat plays the hero's persona only for the day: Start binds it, and the end of the day
//! (and a host start, whose table is back in the lobby) clears it (`SCENARIO unbound seat=
//! persona=`). A seat's play screen shows only while its day runs or after it ended (`playing`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, PoisonError};

use df_playground::seat::wire::DayEnd;
use tokio::sync::Mutex;

use super::book::{Book, HeroRecord, SeatRecord};
use super::request::SeatError;
use crate::engine::request::ForceRequest;
use crate::io_cause::IoCause;
use crate::look::studio::LookStudio;
use crate::state::AppState;

/// The runner the scenario runs, relative to the checkout the host is started in.
const RUNNER: &str = "development/market-thread.sh";
/// Where a scenario's runner copy and logs go, relative to the checkout.
const SCENARIO_DIRECTORY: &str = "artifacts/tmp/scenario";

/// The scenario the table plays, and the run of it going now.
#[derive(Debug)]
pub struct Scenario {
    beats: PathBuf,
    /// The bash that runs the runner (`find_shell`).
    shell: PathBuf,
    /// The host's own loopback port, which the runner drives.
    port: u16,
    /// Held for a whole launch, so two Start taps never start two days.
    running: Mutex<Option<Running>>,
    /// How the last day ended; none while it runs.
    ended: std::sync::Mutex<Option<Ended>>,
}

/// The run going now: the task waiting on the runner, the runner's process id, and each seat of
/// the party with the persona it plays, in seat order.
#[derive(Debug)]
struct Running {
    task: tokio::task::JoinHandle<()>,
    pid: Option<u32>,
    party: Vec<(u64, String)>,
    /// The run's token (`DF_RUN_TOKEN`): the runner sends it with its seat calls.
    token: String,
}

/// A new run token: 128 random bits as hexadecimal digits. It tells this run's runner from one an
/// earlier host left behind; it is not a secret.
fn new_run_token() -> String {
    let bits = [
        super::room::random_u64(0x5255_4E31),
        super::room::random_u64(0x5255_4E32),
    ];
    bits.iter().map(|part| format!("{part:016x}")).collect()
}

/// How the last day ended, and for which seats, each with the persona it played (none when it
/// never got one).
#[derive(Debug, Clone)]
struct Ended {
    end: DayEnd,
    party: Vec<(u64, Option<String>)>,
    /// Why the day did not start, as its cause key, when it did not.
    cause: Option<String>,
}

impl Ended {
    /// Whether `seat` played the day (the outer `Option`), and the persona it played.
    fn persona_of(&self, seat: u64) -> Option<Option<String>> {
        self.party
            .iter()
            .find(|(played, _)| *played == seat)
            .map(|(_, persona)| persona.clone())
    }
}

/// `seats` as a log line and the runner write them: `1,2`.
pub(crate) fn seats_text(seats: impl IntoIterator<Item = u64>) -> String {
    seats
        .into_iter()
        .map(|seat| seat.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Where Git for Windows installs, when `git` does not say.
#[cfg(windows)]
const GIT_BASH: &str = r"C:\Program Files\Git\bin\bash.exe";
/// The variable that names the scenario's shell when `--scenario-shell` does not.
pub const SHELL_VARIABLE: &str = "DF_SCENARIO_SHELL";

/// No bash to run the scenario's runner was found: the paths tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellMissing {
    pub tried: Vec<PathBuf>,
}

impl std::fmt::Display for ShellMissing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tried: Vec<String> = self
            .tried
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        write!(
            formatter,
            "the scenario needs Git Bash to run its runner and found none (tried: {}); install \
             Git for Windows, or name the shell with --scenario-shell <path> or {SHELL_VARIABLE}",
            tried.join(", ")
        )
    }
}

impl std::error::Error for ShellMissing {}

/// The bash that runs the runner: the one named (`--scenario-shell`, else `DF_SCENARIO_SHELL`),
/// which must exist, else Git Bash of the Git install (`git --exec-path`, then its usual place).
pub async fn find_shell(named: Option<&Path>) -> Result<PathBuf, ShellMissing> {
    let named = named
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(SHELL_VARIABLE).map(PathBuf::from));
    let candidates = match named {
        Some(path) => vec![path],
        None => installed_shells().await,
    };
    for candidate in &candidates {
        if tokio::fs::metadata(candidate)
            .await
            .is_ok_and(|found| found.is_file())
        {
            return Ok(candidate.clone());
        }
    }
    Err(ShellMissing { tried: candidates })
}

/// Git Bash beside the Git on the PATH (`<git>/mingw64/libexec/git-core` → `<git>/bin/bash.exe`),
/// then where Git for Windows installs.
#[cfg(windows)]
async fn installed_shells() -> Vec<PathBuf> {
    let mut shells = Vec::new();
    let exec_path = tokio::process::Command::new("git")
        .arg("--exec-path")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;
    if let Some(output) = exec_path.ok().filter(|output| output.status.success()) {
        let exec_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if let Some(git) = exec_path.ancestors().nth(3) {
            shells.push(git.join("bin").join("bash.exe"));
        }
    }
    shells.push(PathBuf::from(GIT_BASH));
    shells
}

/// Elsewhere bash is where the system keeps it.
#[cfg(not(windows))]
async fn installed_shells() -> Vec<PathBuf> {
    vec![PathBuf::from("/bin/bash"), PathBuf::from("/usr/bin/bash")]
}

/// The start of a run token, as a line writes it: enough to tell runs apart.
pub(crate) fn token_prefix(token: &str) -> &str {
    token.get(..8).unwrap_or(token)
}

/// The persona id a hero plays the scenario as.
pub(crate) fn persona_of(hero: u64) -> String {
    format!("hero-{hero}")
}

impl Scenario {
    pub fn new(beats: PathBuf, shell: PathBuf, port: u16) -> Self {
        Self {
            beats,
            shell,
            port,
            running: Mutex::new(None),
            ended: std::sync::Mutex::new(None),
        }
    }

    /// How `seat`'s day ended, when it ended.
    pub(crate) fn day_end(&self, seat: u64) -> Option<DayEnd> {
        self.ended_of(seat).map(|ended| ended.end)
    }

    /// Why `seat`'s day did not start, in the player's words, when it did not.
    pub(crate) fn start_problem(&self, seat: u64) -> Option<String> {
        let cause = self.ended_of(seat)?.cause?;
        Some(format!("The game couldn't start: {}.", cause_words(&cause)))
    }

    fn ended_of(&self, seat: u64) -> Option<Ended> {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .filter(|ended| ended.persona_of(seat).is_some())
    }

    fn set_ended(&self, ended: Option<Ended>) {
        *self.ended.lock().unwrap_or_else(PoisonError::into_inner) = ended;
    }

    /// The persona `seat` plays in the day running now or the day that ended: its play screen
    /// shows only then.
    pub(crate) async fn playing(&self, seat: u64) -> Option<String> {
        let running = self
            .running
            .lock()
            .await
            .as_ref()
            .filter(|running| !running.task.is_finished())
            .and_then(|running| {
                running
                    .party
                    .iter()
                    .find(|(played, _)| *played == seat)
                    .map(|(_, persona)| persona.clone())
            });
        running.or_else(|| {
            self.ended_of(seat)
                .and_then(|ended| ended.persona_of(seat).flatten())
        })
    }

    /// The token of the day running now, with its seats; none when no day runs.
    pub(crate) async fn run(&self) -> Option<(String, Vec<u64>)> {
        self.running
            .lock()
            .await
            .as_ref()
            .filter(|running| !running.task.is_finished())
            .map(|running| {
                let seats = running.party.iter().map(|(seat, _)| *seat).collect();
                (running.token.clone(), seats)
            })
    }

    /// A day running for `party` with `token`, with no runner: for the tests of the token check.
    #[cfg(test)]
    pub(crate) async fn run_for_test(&self, party: Vec<(u64, String)>, token: &str) {
        *self.running.lock().await = Some(Running {
            task: tokio::spawn(std::future::pending()),
            pid: None,
            party,
            token: token.to_owned(),
        });
    }

    /// The seats of the day running now, in seat order; none when no day runs.
    pub(crate) async fn party(&self) -> Vec<u64> {
        self.running
            .lock()
            .await
            .as_ref()
            .filter(|running| !running.task.is_finished())
            .map(|running| running.party.iter().map(|(seat, _)| *seat).collect())
            .unwrap_or_default()
    }

    /// Stops the run going now, if any, with the processes it started.
    pub async fn stop(&self, app: &AppState) {
        let running = self.running.lock().await.take();
        if let Some(running) = running.filter(|running| !running.task.is_finished()) {
            let tree = match running.pid {
                Some(pid) => kill_tree(pid).await,
                None => "unknown",
            };
            // The runner itself is killed when its task is dropped (`kill_on_drop`).
            running.task.abort();
            app.record_line(format!("SCENARIO stopped tree={tree}"));
        }
    }
}

/// Kills the runner's process tree (its curl and CLI children too); how it went.
#[cfg(windows)]
async fn kill_tree(pid: u32) -> &'static str {
    let killed = tokio::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match killed {
        Ok(status) if status.success() => "killed",
        Ok(_) => "kill_refused",
        Err(_) => "kill_failed",
    }
}

/// Elsewhere only the runner is killed, when its task is dropped.
#[cfg(not(windows))]
async fn kill_tree(_pid: u32) -> &'static str {
    "runner_only"
}

/// Records that the day did not start for the seats of `party`, whose seat views say so, and
/// clears each seat's binding to the day's persona.
pub(crate) async fn not_started(app: &Arc<AppState>, party: &[(u64, Option<String>)], cause: &str) {
    let Some(first) = party.first().map(|(seat, _)| *seat) else {
        return;
    };
    app.record_line_and_echo(format!(
        "SCENARIO start_failed cause={cause} seat={first} seats={}",
        seats_text(party.iter().map(|(seat, _)| *seat))
    ));
    if let Some(scenario) = app.seats().scenario() {
        scenario.set_ended(Some(Ended {
            end: DayEnd::NotStarted,
            party: party.to_vec(),
            cause: Some(cause.to_owned()),
        }));
    }
    for (seat, persona) in party {
        if let Some(persona) = persona {
            unbind(app, *seat, persona).await;
        }
    }
    back_to_lobby(app, first).await;
}

/// A day's start cause (a closed key of this host: a look's, a party's or a file's) in the player's
/// words.
fn cause_words(cause: &str) -> &'static str {
    match cause {
        "no_reference" => "a hero's reference sheet is not made yet",
        "no_look" => "a hero's look is not locked in",
        "no_hero" => "a seat has no hero to play",
        "no_party" => "nobody at the table is ready to play",
        "no_bag" => "a hero's bag could not be made",
        _ => "the table could not prepare the day",
    }
}

/// The party of `launches`, each seat with the persona its hero plays the day as.
fn party_of(launches: &[HeroLaunch]) -> Vec<(u64, Option<String>)> {
    launches
        .iter()
        .map(|launch| (launch.seat, Some(persona_of(launch.hero.id))))
        .collect()
}

/// The lobby's gathering state (`df_table::lobby`), named as the engine's force takes it.
const LOBBY_GATHERING: &str = "gathering";

/// Puts the table back in the lobby, gathering with nobody ready, once a seat's day ended: the
/// scene's machine stops and the lobby takes the focus (`engine force` with `clear_scene`; the
/// seats stay seated), so the players can change hero, get ready and start another day without a
/// host restart. Creation opens for the next start as the lobby gathers again. One `SCENARIO
/// lobby seat= result=gathering|already|<error>` line.
async fn back_to_lobby(app: &Arc<AppState>, seat: u64) {
    let result = async {
        let engine = app.engine()?;
        let view = super::table::quiet_view(engine, seat).await?;
        if super::table::is_gathering(&view) {
            return Ok(false);
        }
        engine
            .force(ForceRequest {
                machine: df_table::lobby::MACHINE_ID.to_owned(),
                state: LOBBY_GATHERING.to_owned(),
                payload: None,
                clear_scene: true,
            })
            .await?;
        Ok::<_, SeatError>(true)
    }
    .await;
    let reopened = matches!(result, Ok(true));
    let result = match &result {
        Ok(true) => "gathering",
        Ok(false) => "already",
        Err(error) => error.key(),
    };
    app.record_line_and_echo(format!("SCENARIO lobby seat={seat} result={result}"));
    // The lobby opens again: a new room code for the phones still to come.
    if reopened {
        app.seats().renew_code();
        super::room::open_lobby(app).await;
    }
}

/// Clears `seat`'s binding to the day's `persona`: Start binds it again.
async fn unbind(app: &AppState, seat: u64, persona: &str) {
    let unbound = async {
        let mut open = app.seats().open().await?;
        let bound = open
            .books()
            .record_at(seat)
            .is_some_and(|record| record.persona.as_deref() == Some(persona));
        if bound {
            open.books().bind_persona(seat, None)?;
            open.save(Book::Seats).await?;
        }
        Ok::<_, SeatError>(bound)
    }
    .await;
    match unbound {
        Ok(true) => app.record_line(format!("SCENARIO unbound seat={seat} persona={persona}")),
        Ok(false) => {}
        Err(error) => app.record_line(format!(
            "SCENARIO unbind_failed seat={seat} persona={persona} cause={}",
            error.key()
        )),
    }
}

/// At a host start the table is back in the lobby and no day runs: every seat bound to its
/// hero's day persona is unbound.
pub(crate) async fn clear_bindings(app: &AppState) {
    let bound: Vec<(u64, String)> = match app.seats().open().await {
        Ok(mut open) => open
            .books()
            .records()
            .iter()
            .filter_map(|record| {
                let persona = persona_of(record.hero?);
                (record.persona.as_deref() == Some(persona.as_str()))
                    .then_some((record.seat, persona))
            })
            .collect(),
        Err(error) => {
            app.record_line(format!("SCENARIO unbind_failed cause={}", error.key()));
            return;
        }
    };
    for (seat, persona) in bound {
        unbind(app, seat, &persona).await;
    }
}

/// The runner's exit status when the hero's setup failed (`roam_prepare_hero`): the day did not
/// start.
pub const PREPARE_FAILED_STATUS: i32 = 3;

/// What the runner needs of the hero: whose seat and who they are.
pub(crate) struct HeroLaunch {
    pub(crate) seat: u64,
    pub(crate) hero: HeroRecord,
}

impl HeroLaunch {
    /// The launch of `record`'s hero, when it chose one that the library holds.
    pub(crate) fn of(record: &SeatRecord, hero: Option<&HeroRecord>) -> Option<Self> {
        Some(Self {
            seat: record.seat,
            hero: hero?.clone(),
        })
    }
}

/// A short key of the hero's locked look (its words and its sheet): the runner keys what it makes
/// once and reuses (the opening plate) to it, so a hero locked again in a new look opens on it.
fn look_key(sheet: &str, look: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{sheet}\n{look}").as_bytes());
    df_playground::seat::wire::hex_digits(&digest[..4])
}

/// What the runner is told of the hero's looks: the reference sheet the pictures of `persona`
/// follow (the look studio's one reference lookup, which answers the hero's pack sheet for the
/// seat bound to it) and the locked design in words. `Err` with the cause key when either is
/// missing: the day cannot start without them.
async fn hero_looks(
    app: &AppState,
    persona: &str,
    hero: &HeroRecord,
) -> Result<(String, String), &'static str> {
    let sheet = LookStudio::reference_of(app, persona)
        .await
        .map_err(|error| error.key())?
        .ok_or("no_reference")?;
    let look = LookStudio::locked_look(app, hero)
        .await
        .map_err(|error| error.key())?
        .ok_or("no_look")?;
    Ok((sheet, look))
}

/// Copies the runner beside its logs (bash reads a script as it goes, so an edit mid-run would
/// garble the one being run) and opens the log. Blocking.
fn prepare(stamp: u128) -> std::io::Result<(PathBuf, std::fs::File, PathBuf)> {
    let directory = Path::new(SCENARIO_DIRECTORY);
    std::fs::create_dir_all(directory)?;
    let runner = directory.join("runner.sh");
    std::fs::copy(RUNNER, &runner)?;
    let log_path = directory.join(format!("play-{stamp}.log"));
    let log = std::fs::File::create(&log_path)?;
    Ok((runner, log, log_path))
}

/// Gives hero `hero` its bag for the day, a fresh journey (`planning/town-to-tomb.md` part C): its
/// inventory set empty through the inventory's one reset, then its starting kit and coins through
/// creation's one hand-out, given again. Starting twice gives the same bag. One `SCENARIO kit hero=
/// given|waits` line (`waits` with its reason); `Err` with the cause key `no_bag` when either step
/// fails.
pub(crate) async fn kit_for_day(app: &Arc<AppState>, hero: u64) -> Result<(), &'static str> {
    use crate::creation::KitOutcome;
    use crate::world::inventory::OwnerKind;
    let owner = persona_of(hero);
    crate::world::inventory::routes::reset_owner(app, &owner, OwnerKind::Character)
        .await
        .map_err(|error| {
            app.record_line(format!(
                "SCENARIO kit hero={hero} failed=inventory cause={}",
                error.key()
            ));
            "no_bag"
        })?;
    let given = crate::creation::give_kit_again(app, hero)
        .await
        .map_err(|error| {
            app.record_line(format!(
                "SCENARIO kit hero={hero} failed=kit cause={}",
                error.key()
            ));
            "no_bag"
        })?;
    app.record_line(match given {
        KitOutcome::Given | KitOutcome::AlreadyGiven => format!("SCENARIO kit hero={hero} given"),
        KitOutcome::Waits(reason) => format!("SCENARIO kit hero={hero} waits reason={reason}"),
    });
    Ok(())
}

/// What the runner is told of one hero of the party: who they are and their looks.
struct Prepared {
    launch: HeroLaunch,
    persona: String,
    sheet: String,
    look: String,
}

impl Prepared {
    /// The variables of this hero, numbered by its seat: `DF_RUN_HERO_<seat>_*`.
    fn numbered(&self) -> Vec<(String, String)> {
        let hero = &self.launch.hero;
        let prefix = format!("DF_RUN_HERO_{}", self.launch.seat);
        [
            ("ID", hero.id.to_string()),
            ("NAME", hero.name.clone()),
            ("SPECIES", hero.species.clone()),
            ("CLASS", hero.class.clone()),
            ("SHEET", self.sheet.clone()),
            ("LOOK", self.look.clone()),
            ("LOOK_KEY", look_key(&self.sheet, &self.look)),
            ("PERSONA", self.persona.clone()),
        ]
        .into_iter()
        .map(|(name, value)| (format!("{prefix}_{name}"), value))
        .collect()
    }
}

/// Starts the day for the party of `launches` (in seat order), unless a run is going already.
pub(crate) async fn launch(app: &Arc<AppState>, scenario: &Scenario, launches: Vec<HeroLaunch>) {
    let mut running = scenario.running.lock().await;
    if running
        .as_ref()
        .is_some_and(|running| !running.task.is_finished())
    {
        app.record_line("SCENARIO already_running".to_owned());
        return;
    }
    scenario.set_ended(None);
    let party = party_of(&launches);
    let mut prepared = Vec::with_capacity(launches.len());
    for launch in launches {
        let persona = persona_of(launch.hero.id);
        match hero_looks(app, &persona, &launch.hero).await {
            Ok((sheet, look)) => prepared.push(Prepared {
                launch,
                persona,
                sheet,
                look,
            }),
            Err(cause) => {
                not_started(app, &party, cause).await;
                return;
            }
        }
    }
    // A Start is a fresh journey: the scenario's quests start unoffered, and each hero carries its
    // starting kit and coins alone before the runner reads its bag.
    crate::world::clear_quests(app).await;
    for hero in &prepared {
        if let Err(cause) = kit_for_day(app, hero.launch.hero.id).await {
            not_started(app, &party, cause).await;
            return;
        }
    }
    let Some(first) = prepared.first() else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let files =
        crate::blocking::run_blocking(move || prepare(stamp), std::io::ErrorKind::Other.into())
            .await;
    let (runner, log, log_path) = match files {
        Ok(files) => files,
        Err(error) => {
            let cause = IoCause::from_kind(error.kind()).key();
            not_started(app, &party, cause).await;
            return;
        }
    };
    let token = new_run_token();
    let mut command = tokio::process::Command::new(&scenario.shell);
    command
        .arg(&runner)
        .arg(&scenario.beats)
        .env(super::key::RUN_TOKEN_VARIABLE, &token)
        .env("DF_HOST_PID", std::process::id().to_string())
        .env("DF_RUN_PLAYER", first.launch.seat.to_string())
        .env("DF_RUN_ROAM", &first.persona)
        .env("DF_RUN_HERO_ID", first.launch.hero.id.to_string())
        .env("DF_RUN_HERO_NAME", &first.launch.hero.name)
        .env("DF_RUN_HERO_SPECIES", &first.launch.hero.species)
        .env("DF_RUN_HERO_CLASS", &first.launch.hero.class)
        .env("DF_RUN_HERO_SHEET", &first.sheet)
        .env("DF_RUN_HERO_LOOK", &first.look)
        .env("DF_RUN_HERO_LOOK_KEY", look_key(&first.sheet, &first.look))
        .env(
            "DF_RUN_PLAYERS",
            seats_text(party.iter().map(|(seat, _)| *seat)),
        )
        .envs(prepared.iter().flat_map(Prepared::numbered))
        .env("DF_TABLE_PORT", scenario.port.to_string())
        .env(
            "FRAMES_LOG",
            Path::new(SCENARIO_DIRECTORY).join(format!("frames-{stamp}.log")),
        )
        .env_remove("DF_RUN_TTS")
        .env_remove("DF_RUN_MUSIC")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    if let Ok(root) = std::env::current_dir() {
        command.env("DF_RUN_ROOT", root);
    }
    if let Ok(exe) = std::env::current_exe() {
        command.env("DF_TABLE_EXE", exe);
    }
    let started = log.try_clone().and_then(|errors| {
        command.stdout(log).stderr(errors);
        command.spawn()
    });
    let mut child = match started {
        Ok(child) => child,
        Err(error) => {
            let cause = IoCause::from_kind(error.kind()).key();
            not_started(app, &party, cause).await;
            return;
        }
    };
    let pid = child.id();
    app.record_line_and_echo(format!(
        "SCENARIO started seat={} seats={} hero={} persona={} token={} log={}",
        first.launch.seat,
        seats_text(party.iter().map(|(seat, _)| *seat)),
        first.launch.hero.id,
        first.persona,
        token_prefix(&token),
        log_path.display()
    ));
    let watcher = Arc::clone(app);
    let played: Vec<(u64, String)> = prepared
        .iter()
        .map(|hero| (hero.launch.seat, hero.persona.clone()))
        .collect();
    let ending = played.clone();
    let task = tokio::spawn(async move {
        // The child is killed when this task is aborted (`kill_on_drop`).
        let status = child.wait().await;
        let end = match status.as_ref().map(std::process::ExitStatus::code) {
            Ok(Some(0)) => DayEnd::Finished,
            Ok(Some(PREPARE_FAILED_STATUS)) => DayEnd::NotStarted,
            _ => DayEnd::Failed,
        };
        let status = status.map_or_else(
            |_| "unknown".to_owned(),
            |status| {
                status
                    .code()
                    .map_or_else(|| "killed".to_owned(), |code| code.to_string())
            },
        );
        let Some(first) = ending.first().map(|(seat, _)| *seat) else {
            return;
        };
        watcher.record_line_and_echo(format!(
            "SCENARIO ended status={status} seat={first} seats={}",
            seats_text(ending.iter().map(|(seat, _)| *seat))
        ));
        if let Some(scenario) = watcher.seats().scenario() {
            scenario.set_ended(Some(Ended {
                end,
                cause: None,
                party: ending
                    .iter()
                    .map(|(seat, persona)| (*seat, Some(persona.clone())))
                    .collect(),
            }));
        }
        for (seat, persona) in &ending {
            unbind(&watcher, *seat, persona).await;
            // Talk the day never took is dropped with it, and the phone is told by its view.
            let dropped = watcher.seats().talk().drop_talk(*seat);
            if dropped > 0 {
                watcher.record_line(format!("SCENARIO talk_dropped seat={seat} count={dropped}"));
            }
        }
        back_to_lobby(&watcher, first).await;
    });
    *running = Some(Running {
        task,
        pid,
        party: played,
        token,
    });
}
