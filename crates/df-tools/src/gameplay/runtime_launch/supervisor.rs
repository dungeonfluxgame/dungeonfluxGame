//! Local-only placement: bounded, isolated game and launcher processes owned until shutdown.
use super::*;
use df_campaign::{
    lobby_state::SeatedMember,
    session_link::{HttpRuntime, Placement},
};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
};

const MAXIMUM_ROOMS: usize = 8;
const STARTUP_DEADLINE: Duration = Duration::from_secs(90);
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

pub(super) struct Supervisor {
    secret: LaunchSecret,
    database: tokio_postgres::Config,
    children: Mutex<HashMap<[u8; 16], ChildRoom>>,
}

impl Supervisor {
    pub(super) fn new(secret: LaunchSecret) -> Result<Self, std::io::Error> {
        Ok(Self {
            secret,
            database: super::super::database_config()?,
            children: Mutex::new(HashMap::new()),
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let runtime = HttpRuntime::new(address, self.secret.clone(), EXCHANGE_DEADLINE)
            .map_err(|_| std::io::Error::other("local child adapter unavailable"))?;
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
            drop(listener);
            let result = run_child(
                database,
                address,
                runtime,
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

async fn run_child(
    database: tokio_postgres::Config,
    address: SocketAddr,
    runtime: HttpRuntime,
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
    let origin = format!("http://{address}");
    let database_name = database
        .get_dbname()
        .ok_or_else(|| std::io::Error::other("child database missing"))?;
    let mut child = child_command(&executable)
        .args(std::env::args_os().skip(1))
        .env("DF_RUNTIME_CHILD", "1")
        .env_remove("DF_GAMEPLAY_ROOM_CODE")
        .env("DF_GAMEPLAY_LISTEN", address.to_string())
        .env("DF_GAMEPLAY_PUBLIC_URL", &origin)
        .env("DF_GAMEPLAY_ALLOWED_ORIGINS", &origin)
        .env("DF_GAMEPLAY_DEMO_DATABASE", database_name)
        .env("DF_GAMEPLAY_STOP_FILE", &stop_directory)
        .env("DF_GAMEPLAY_LAUNCHER_TOKEN", &status_token)
        .spawn()?;
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
    let links = LocalLinks {
        launcher_url: format!("http://{launcher_address}/"),
        display_url: format!("{origin}/gameplay/display"),
        player_url: format!("{origin}/gameplay/player?room={}", room.code),
    };
    let record = launcher_record(
        &links,
        &executable,
        child
            .id()
            .ok_or_else(|| std::io::Error::other("child pid missing"))?,
        database_name,
        &stop_directory,
        &status_token,
    )?;
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
    tokio::select! {
        result = child.wait() => {
            result?;
            updates.send_replace(Phase::Failed);
            // The launcher records the save after observing the game's exit. It remains
            // owned here until supervisor shutdown, so Stop and save can finish normally.
            launcher.wait().await?;
            Ok(())
        }
        result = launcher.wait() => { result?; child.kill().await?; Err(std::io::Error::other("local launcher stopped")) }
    }
}

fn launcher_record(
    links: &LocalLinks,
    executable: &std::path::Path,
    pid: u32,
    database: &str,
    stop: &std::path::Path,
    token: &str,
) -> Result<String, std::io::Error> {
    let quote = |text: &str| serde_json::to_string(text).map_err(std::io::Error::other);
    let adventure = std::env::var("DF_GAMEPLAY_ADVENTURE").unwrap_or_else(|_| "harbor".to_owned());
    if !matches!(adventure.as_str(), "harbor" | "random" | "free-roam") {
        return Err(std::io::Error::other("local adventure invalid"));
    }
    Ok(format!(
        "pid={pid}\nserver_exe={}\nstop_dir={}\nlog=\"\"\ndisplay_url={}\nplayer_url={}\npostgres_started=false\nplay_root=\"\"\npostgres_bin=\"\"\nstatus_token={}\n[game]\ndatabase={}\nname=\"Your game\"\nadventure=\"{adventure}\"\ncreated=0\nlast_played={}\n",
        quote(&executable.to_string_lossy())?,
        quote(&stop.to_string_lossy())?,
        quote(&links.display_url)?,
        quote(&links.player_url)?,
        quote(token)?,
        quote(database)?,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_secs()
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
