//! Local-only placement: bounded, isolated game and launcher processes owned until shutdown.
use super::*;
use df_campaign::{
    lobby_state::SeatedMember,
    session_link::{HttpRuntime, Placement},
};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    sync::watch,
    task::JoinHandle,
};

const MAXIMUM_ROOMS: usize = 8;
const STARTUP_DEADLINE: Duration = Duration::from_secs(90);
/// A child's READY waits for its adventure, which can take minutes to generate.
fn child_ready_deadline(codex_mode: bool) -> Duration {
    Duration::from_secs(if codex_mode { 1200 } else { 600 })
}
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(2);
const STARTUP_POLL: Duration = Duration::from_millis(250);

#[derive(Clone)]
enum Phase {
    Starting,
    Ready(Allocated),
    Failed,
}

struct ChildRoom {
    allocation: Allocation,
    phase: watch::Receiver<Phase>,
    task: JoinHandle<()>,
}

impl Drop for ChildRoom {
    fn drop(&mut self) {
        // Aborting drops both kill_on_drop children, including during partial startup.
        self.task.abort();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SupervisedNetwork {
    listen: SocketAddr,
    public_origin: String,
    allowed_origins: String,
}

fn supervised_network(phones_on_network: bool, lan_address: Option<Ipv4Addr>) -> SupervisedNetwork {
    match (phones_on_network, lan_address) {
        (true, Some(address)) => SupervisedNetwork {
            listen: SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            public_origin: format!("http://{address}:0"),
            allowed_origins: format!("http://{address}:0,http://127.0.0.1:0,http://localhost:0"),
        },
        _ => SupervisedNetwork {
            listen: SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            public_origin: "http://127.0.0.1:0".to_owned(),
            allowed_origins: "http://127.0.0.1:0,http://localhost:0".to_owned(),
        },
    }
}

fn network_join_from_environment() -> (bool, Option<Ipv4Addr>) {
    let lan_address = std::env::var("DF_RUNTIME_LAN_ADDRESS")
        .ok()
        .and_then(|value| value.parse::<Ipv4Addr>().ok())
        .filter(|address| address.is_private())
        .or_else(|| {
            std::env::var("DF_GAMEPLAY_PUBLIC_URL")
                .ok()
                .and_then(|url| {
                    url.strip_prefix("http://")
                        .and_then(|rest| rest.split('/').next())
                        .and_then(|authority| authority.split(':').next())
                        .and_then(|host| host.parse::<Ipv4Addr>().ok())
                        .filter(|address| address.is_private())
                })
        });
    let phones_on_network = match std::env::var("DF_RUNTIME_PHONES_ON_NETWORK")
        .ok()
        .as_deref()
    {
        Some("1") => true,
        Some("0") => false,
        _ => lan_address.is_some(),
    };
    (phones_on_network, lan_address)
}

fn child_secret(secret: &LaunchSecret, session: &[u8; 16]) -> Result<LaunchSecret, std::io::Error> {
    let derived = super::super::hex(&secret.derive(b"local runtime child v1", session));
    LaunchSecret::new(&derived).map_err(|_| std::io::Error::other("local child secret unavailable"))
}

fn ready_origin(line: &str) -> Option<String> {
    let display = line
        .trim_end()
        .strip_prefix("READY display=")?
        .split(' ')
        .next()?;
    let origin = display.strip_suffix("/gameplay/display")?;
    let authority = origin.strip_prefix("http://")?;
    authority
        .rsplit(':')
        .next()?
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)?;
    Some(origin.to_owned())
}

fn loopback_control_address(ready_origin: &str) -> Option<SocketAddr> {
    let port = ready_origin
        .strip_prefix("http://")?
        .rsplit(':')
        .next()?
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)?;
    Some(SocketAddr::new(
        std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
    ))
}

fn links_from_ready(
    ready_origin: &str,
    launcher_address: SocketAddr,
    room_code: &str,
) -> LocalLinks {
    LocalLinks {
        launcher_url: format!("http://{launcher_address}/"),
        display_url: format!("{ready_origin}/gameplay/display"),
        player_url: format!("{ready_origin}/gameplay/player?room={room_code}"),
    }
}

fn adventure_title(adventure: &str) -> &'static str {
    match adventure {
        "random" => "A newly rolled adventure",
        "free-roam" => "Free roam",
        _ => "The Broken Seal",
    }
}

pub(super) struct Supervisor {
    secret: LaunchSecret,
    database: tokio_postgres::Config,
    children: Mutex<HashMap<[u8; 16], ChildRoom>>,
    phones_on_network: bool,
    lan_address: Option<Ipv4Addr>,
}

impl Supervisor {
    pub(super) fn new(secret: LaunchSecret) -> Result<Self, std::io::Error> {
        let (phones_on_network, lan_address) = network_join_from_environment();
        Ok(Self {
            secret,
            database: super::super::database_config()?,
            children: Mutex::new(HashMap::new()),
            phones_on_network,
            lan_address,
        })
    }

    pub(super) async fn allocate(&self, allocation: Allocation) -> Response {
        let mut phase = {
            let mut children = self.children.lock().await;
            if let Some(child) = children.get(&allocation.session) {
                if child.allocation != allocation {
                    return refusal(Refusal::Occupied);
                }
                child.phase.clone()
            } else {
                // Reservations are never recycled into another session during this supervisor's life.
                if children.len() >= MAXIMUM_ROOMS {
                    return refusal(Refusal::Occupied);
                }
                let child = match self.reserve(allocation).await {
                    Ok(child) => child,
                    Err(_) => return refusal(Refusal::Unavailable),
                };
                let phase = child.phase.clone();
                children.insert(child.allocation.session, child);
                phase
            }
        };
        loop {
            match phase.borrow().clone() {
                Phase::Starting => {}
                Phase::Ready(allocated) => return private(Json(allocated).into_response()),
                Phase::Failed => return refusal(Refusal::Unavailable),
            }
            if phase.changed().await.is_err() {
                return refusal(Refusal::Unavailable);
            }
        }
    }

    async fn reserve(&self, allocation: Allocation) -> Result<ChildRoom, std::io::Error> {
        let placement = placement(&allocation)?;
        let network = supervised_network(self.phones_on_network, self.lan_address);
        let child_secret = child_secret(&self.secret, &allocation.session)?;
        let mut database = self.database.clone();
        database.dbname(format!(
            "df_gameplay_demo_rtlocal_{}",
            super::super::hex(&allocation.session)
        ));
        let (updates, phase) = watch::channel(Phase::Starting);
        let status_token = super::super::hex(
            &self
                .secret
                .derive(b"local launcher status", &allocation.session),
        );
        // No await between dispatch and retaining the task; HTTP cancellation retains the reservation.
        let task = tokio::spawn(async move {
            let result = run_child(
                database,
                network,
                child_secret,
                placement.clone(),
                status_token,
                updates.clone(),
            )
            .await;
            updates.send_replace(Phase::Failed);
            actor::record_audit_line(
                "session.runtime_child_stopped",
                format!(
                    "SESSION runtime_child_stopped session={} reason={}",
                    super::super::hex(placement.session.as_bytes()),
                    if result.is_ok() {
                        "ended"
                    } else {
                        "unavailable"
                    }
                ),
            );
        });
        Ok(ChildRoom {
            allocation,
            phase,
            task,
        })
    }
}

fn placement(allocation: &Allocation) -> Result<Placement, std::io::Error> {
    let invalid = || std::io::Error::other("invalid local placement");
    Ok(Placement {
        campaign: df_types::CampaignId::from_bytes(&allocation.campaign).map_err(|_| invalid())?,
        session: df_types::SessionId::from_bytes(&allocation.session).map_err(|_| invalid())?,
        roster: allocation
            .roster
            .iter()
            .map(|seat| {
                Ok(SeatedMember {
                    account: df_types::AccountId::from_bytes(&seat.account)
                        .map_err(|_| invalid())?,
                    member: df_types::MemberId::from_bytes(&seat.member).map_err(|_| invalid())?,
                })
            })
            .collect::<Result<Vec<_>, std::io::Error>>()?,
    })
}

fn child_command(executable: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(executable);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

async fn read_ready(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<String, std::io::Error> {
    loop {
        let Some(line) = lines
            .next_line()
            .await
            .map_err(|_| std::io::Error::other("child output unavailable"))?
        else {
            return Err(std::io::Error::other("child exited before ready"));
        };
        if let Some(origin) = ready_origin(&line) {
            return Ok(origin);
        }
        println!("{line}");
    }
}

async fn run_child(
    database: tokio_postgres::Config,
    network: SupervisedNetwork,
    child_secret: LaunchSecret,
    placement: Placement,
    status_token: String,
    updates: watch::Sender<Phase>,
) -> Result<(), std::io::Error> {
    let executable = std::env::current_exe()?;
    let launcher_executable = std::env::var_os("DF_RUNTIME_LAUNCHER_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable.with_file_name("dungeonflux.exe"));
    let directory = std::env::current_dir()?
        .join("artifacts/tmp/runtime-sessions")
        .join(super::super::hex(placement.session.as_bytes()));
    let stop_directory = directory.join("stop");
    tokio::fs::create_dir_all(&stop_directory).await?;
    tokio::time::timeout(STARTUP_DEADLINE, prepare_database(&database))
        .await
        .map_err(|_| std::io::Error::other("local database startup deadline"))??;
    let database_name = database
        .get_dbname()
        .ok_or_else(|| std::io::Error::other("child database missing"))?;
    let child_secret_text = String::from_utf8(child_secret.header_value().to_vec())
        .map_err(|_| std::io::Error::other("local child secret unavailable"))?;
    let mut child = child_command(&executable)
        .args(std::env::args_os().skip(1))
        .env("DF_RUNTIME_CHILD", "1")
        .env_remove("DF_RUNTIME_SUPERVISOR")
        .env_remove("DF_GAMEPLAY_ROOM_CODE")
        .env("DF_RUNTIME_LAUNCH_SECRET", &child_secret_text)
        .env("DF_GAMEPLAY_LISTEN", network.listen.to_string())
        .env("DF_GAMEPLAY_PUBLIC_URL", &network.public_origin)
        .env("DF_GAMEPLAY_ALLOWED_ORIGINS", &network.allowed_origins)
        .env("DF_GAMEPLAY_DEMO_DATABASE", database_name)
        .env("DF_GAMEPLAY_STOP_FILE", &stop_directory)
        .env("DF_GAMEPLAY_LAUNCHER_TOKEN", &status_token)
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child output missing"))?;
    let mut lines = BufReader::new(stdout).lines();
    // The READY line on this child's own pipe is its identity and bound address; no
    // allocation (and no secret) is sent to any address before it arrives.
    let deadline =
        child_ready_deadline(std::env::var("DF_GAMEPLAY_CODEX_MODE").as_deref() == Ok("on"));
    let ready = tokio::time::timeout(deadline, read_ready(&mut lines))
        .await
        .map_err(|_| std::io::Error::other("child startup deadline"))??;
    let drain = tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    let control_address = loopback_control_address(&ready)
        .ok_or_else(|| std::io::Error::other("child ready address invalid"))?;
    let runtime = HttpRuntime::new(control_address, child_secret, EXCHANGE_DEADLINE)
        .map_err(|_| std::io::Error::other("local child adapter unavailable"))?;
    let room = tokio::time::timeout(STARTUP_DEADLINE, async {
        loop {
            if let Ok(room) = runtime.allocate_with_links(&placement).await {
                return Ok(room.0);
            }
            if child.try_wait()?.is_some() {
                return Err(std::io::Error::other("child exited before ready"));
            }
            tokio::time::sleep(STARTUP_POLL).await;
        }
    })
    .await
    .map_err(|_| std::io::Error::other("child startup deadline"))??;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let launcher_address = listener.local_addr()?;
    drop(listener);
    let origin = ready;
    let links = links_from_ready(&origin, launcher_address, &room.code);
    let adventure = std::env::var("DF_GAMEPLAY_ADVENTURE").unwrap_or_else(|_| "harbor".to_owned());
    let created_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_secs();
    let record = launcher_record(&LauncherRecord {
        links: &links,
        executable: &executable,
        pid: child
            .id()
            .ok_or_else(|| std::io::Error::other("child pid missing"))?,
        database: database_name,
        stop: &stop_directory,
        token: &status_token,
        adventure: &adventure,
        created_unix_seconds,
    })?;
    let mut launcher = child_command(launcher_executable)
        .arg("--data-dir")
        .arg(&directory)
        .args([
            "--ui-preview",
            &launcher_address.to_string(),
            "--connect",
            &origin,
        ])
        .env("DF_RUNTIME_LAUNCHER_RECORD", record)
        .spawn()?;
    wait_launcher(&mut launcher, launcher_address).await?;
    actor::record_audit_line(
        "session.runtime_child_ready",
        format!(
            "SESSION runtime_child_ready session={} room={} launcher={launcher_address}",
            super::super::hex(placement.session.as_bytes()),
            room.code
        ),
    );
    updates.send_replace(Phase::Ready(Allocated {
        code: room.code,
        generation: room.run_identity,
        links: Some(links),
    }));
    let outcome = tokio::select! {
        result = child.wait() => {
            result?;
            updates.send_replace(Phase::Failed);
            // The launcher records the save after observing the game's exit. It remains
            // owned here until supervisor shutdown, so Stop and save can finish normally.
            launcher.wait().await?;
            Ok(())
        }
        result = launcher.wait() => { result?; child.kill().await?; Err(std::io::Error::other("local launcher stopped")) }
    };
    drain.abort();
    outcome
}

struct LauncherRecord<'a> {
    links: &'a LocalLinks,
    executable: &'a std::path::Path,
    pid: u32,
    database: &'a str,
    stop: &'a std::path::Path,
    token: &'a str,
    adventure: &'a str,
    created_unix_seconds: u64,
}

fn launcher_record(record: &LauncherRecord<'_>) -> Result<String, std::io::Error> {
    let quote = |text: &str| serde_json::to_string(text).map_err(std::io::Error::other);
    if !matches!(record.adventure, "harbor" | "random" | "free-roam") {
        return Err(std::io::Error::other("local adventure invalid"));
    }
    Ok(format!(
        "pid={}\nserver_exe={}\nstop_dir={}\nlog=\"\"\ndisplay_url={}\nplayer_url={}\npostgres_started=false\nplay_root=\"\"\npostgres_bin=\"\"\nstatus_token={}\n[game]\ndatabase={}\nname={}\nadventure=\"{}\"\ncreated={}\nlast_played={}\n",
        record.pid,
        quote(&record.executable.to_string_lossy())?,
        quote(&record.stop.to_string_lossy())?,
        quote(&record.links.display_url)?,
        quote(&record.links.player_url)?,
        quote(record.token)?,
        quote(record.database)?,
        quote(adventure_title(record.adventure))?,
        record.adventure,
        record.created_unix_seconds,
        record.created_unix_seconds,
    ))
}

async fn wait_launcher(launcher: &mut Child, address: SocketAddr) -> Result<(), std::io::Error> {
    tokio::time::timeout(STARTUP_DEADLINE, async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                return Ok(());
            }
            if launcher.try_wait()?.is_some() {
                return Err(std::io::Error::other("local launcher exited"));
            }
            tokio::time::sleep(STARTUP_POLL).await;
        }
    })
    .await
    .map_err(|_| std::io::Error::other("local launcher deadline"))?
}

async fn prepare_database(database: &tokio_postgres::Config) -> Result<(), std::io::Error> {
    let mut admin = database.clone();
    admin.dbname("postgres").connect_timeout(EXCHANGE_DEADLINE);
    let (client, connection) = admin
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(std::io::Error::other)?;
    let mut driver = tokio::task::JoinSet::new();
    driver.spawn(connection);
    let result = tokio::time::timeout(
        STARTUP_DEADLINE,
        df_persistence::local_demo_scope::ensure_run_database(
            &client,
            database
                .get_dbname()
                .ok_or_else(|| std::io::Error::other("child database missing"))?,
        ),
    )
    .await;
    drop(client);
    driver.shutdown().await;
    match result {
        Ok(Ok(())) => Ok(()),
        _ => Err(std::io::Error::other(
            "local child database preparation failed",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_child_ready_deadline_is_bounded() {
        assert_eq!(child_ready_deadline(false), Duration::from_secs(600));
        assert_eq!(child_ready_deadline(true), Duration::from_secs(1200));
    }

    #[tokio::test]
    async fn supervised_child_inherits_the_openai_block() {
        match std::env::var("DF_TEST_SUPERVISOR_GUARD").as_deref() {
            Ok("child") => {
                assert_eq!(
                    std::env::var("DF_GAMEPLAY_DISABLE_OPENAI_API").as_deref(),
                    Ok("on")
                );
            }
            Ok("parent") => {
                let status = child_command(std::env::current_exe().unwrap())
                    .args(["--exact", "gameplay::runtime_launch::supervisor::tests::supervised_child_inherits_the_openai_block"])
                    .env("DF_TEST_SUPERVISOR_GUARD", "child")
                    .status().await.unwrap();
                assert!(status.success());
            }
            _ => {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "gameplay::runtime_launch::supervisor::tests::supervised_child_inherits_the_openai_block"])
                    .env("DF_TEST_SUPERVISOR_GUARD", "parent")
                    .env("DF_GAMEPLAY_DISABLE_OPENAI_API", "on")
                    .status().unwrap();
                assert!(status.success());
            }
        }
    }

    #[test]
    fn network_joining_on_with_a_lan_address_binds_all_interfaces_and_links_use_it() {
        let network = supervised_network(true, Some(Ipv4Addr::new(192, 168, 0, 96)));
        assert_eq!(network.listen, "0.0.0.0:0".parse().unwrap());
        assert_eq!(network.public_origin, "http://192.168.0.96:0");
        assert!(
            network
                .allowed_origins
                .starts_with("http://192.168.0.96:0,")
        );
        let ready = ready_origin(
            "READY display=http://192.168.0.96:63309/gameplay/display player=http://192.168.0.96:63309/gameplay/player?room=ABCD room=ABCD",
        )
        .unwrap();
        assert_eq!(ready, "http://192.168.0.96:63309");
        let links = links_from_ready(&ready, "127.0.0.1:4001".parse().unwrap(), "ABCD");
        assert_eq!(
            links.display_url,
            "http://192.168.0.96:63309/gameplay/display"
        );
        assert_eq!(
            links.player_url,
            "http://192.168.0.96:63309/gameplay/player?room=ABCD"
        );
        assert!(!links.display_url.contains("127.0.0.1"));
        assert_eq!(
            loopback_control_address(&ready),
            Some("127.0.0.1:63309".parse().unwrap())
        );
    }

    #[test]
    fn network_joining_off_or_without_a_lan_address_stays_loopback() {
        for network in [
            supervised_network(false, Some(Ipv4Addr::new(192, 168, 0, 96))),
            supervised_network(true, None),
            supervised_network(false, None),
        ] {
            assert_eq!(network.listen, "127.0.0.1:0".parse().unwrap());
            assert_eq!(network.public_origin, "http://127.0.0.1:0");
        }
        let links = links_from_ready(
            "http://127.0.0.1:54321",
            "127.0.0.1:4001".parse().unwrap(),
            "WXYZ",
        );
        assert!(links.display_url.starts_with("http://127.0.0.1:54321/"));
        assert!(links.player_url.contains("127.0.0.1:54321"));
    }

    #[test]
    fn ready_origin_refuses_non_ready_and_ephemeral_lines() {
        assert_eq!(ready_origin("ROOM ABCD"), None);
        assert_eq!(
            ready_origin("READY display=http://127.0.0.1:0/gameplay/display player=x"),
            None
        );
    }

    #[test]
    fn child_secret_is_derived_per_session_and_is_not_the_master_secret() {
        let master = LaunchSecret::new(&"m".repeat(32)).unwrap();
        let first = child_secret(&master, &[1; 16]).unwrap();
        let second = child_secret(&master, &[2; 16]).unwrap();
        assert_ne!(first.header_value(), master.header_value());
        assert_ne!(first.header_value(), second.header_value());
        assert!(first.authenticates(first.header_value()));
        assert!(!first.authenticates(master.header_value()));
    }

    #[test]
    fn launcher_record_uses_the_adventure_title_and_the_real_start_time() {
        let links = LocalLinks {
            launcher_url: "http://127.0.0.1:4001/".to_owned(),
            display_url: "http://192.168.0.96:63309/gameplay/display".to_owned(),
            player_url: "http://192.168.0.96:63309/gameplay/player?room=ABCD".to_owned(),
        };
        let record = launcher_record(&LauncherRecord {
            links: &links,
            executable: std::path::Path::new("game.exe"),
            pid: 42,
            database: "df_gameplay_demo_test",
            stop: std::path::Path::new("stop"),
            token: "token",
            adventure: "harbor",
            created_unix_seconds: 1_792_800_000,
        })
        .unwrap();
        assert!(record.contains("name=\"The Broken Seal\""), "{record}");
        assert!(record.contains("created=1792800000"), "{record}");
        assert!(record.contains("last_played=1792800000"), "{record}");
        assert!(!record.contains("Your game"));
        assert!(!record.contains("created=0\n"));
    }
}
