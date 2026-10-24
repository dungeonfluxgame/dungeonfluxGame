//! The game's local PostgreSQL cluster, managed exactly as `run-gameplay.sh` manages it: the same
//! data folder, administrator role and options, so games saved either way stay resumable.
use std::{
    io,
    net::TcpStream,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use crate::install::Install;

const ADMIN: &str = "df_gameplay_demo_admin_20261004";

/// A command whose console window never flashes on screen.
pub fn quiet(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.stdin(std::process::Stdio::null());
    command
}

fn run(command: &mut Command, what: &str) -> Result<String, io::Error> {
    let started = std::time::Instant::now();
    let output = command.output()?;
    crate::log::event(
        "DATABASE_TOOL",
        format!(
            "what={} ok={} ms={}",
            crate::log::value(what),
            output.status.success(),
            started.elapsed().as_millis()
        ),
    );
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    Err(io::Error::other(format!(
        "{what} failed: {}",
        detail.trim()
    )))
}

fn pgdata(install: &Install) -> std::path::PathBuf {
    install.play_root.join("pgdata")
}

/// Our cluster is running on `port`. PostgreSQL publishes its port on line four of
/// `postmaster.pid`; `pg_ctl status` confirms the process.
pub fn ours_running(install: &Install, port: u16) -> bool {
    let published = std::fs::read_to_string(pgdata(install).join("postmaster.pid"))
        .ok()
        .and_then(|text| text.lines().nth(3).map(|line| line.trim().to_owned()));
    published.as_deref() == Some(port.to_string().as_str())
        && quiet(&install.postgres_tool("pg_ctl"))
            .arg("-D")
            .arg(pgdata(install))
            .arg("status")
            .output()
            .is_ok_and(|output| output.status.success())
}

fn port_answers(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).is_ok()
}

/// Starts the cluster unless ours already runs. Returns whether this call started it.
pub fn ensure_running(install: &Install, port: u16) -> Result<bool, io::Error> {
    if ours_running(install, port) {
        crate::log::event("DATABASE_REUSED", format!("port={port}"));
        return Ok(false);
    }
    if port_answers(port) {
        return Err(io::Error::other(format!(
            "Another program is using port {port}. Change the database port in Settings, under Players and network."
        )));
    }
    std::fs::create_dir_all(&install.play_root)?;
    let data = pgdata(install);
    if !data.is_dir() {
        run(
            quiet(&install.postgres_tool("initdb"))
                .arg("-D")
                .arg(&data)
                .args([
                    "-U",
                    ADMIN,
                    "--auth-host=trust",
                    "--auth-local=trust",
                    "-E",
                    "UTF8",
                    "--locale=C",
                ]),
            "Creating the game database folder",
        )?;
    }
    // The server pg_ctl starts inherits pg_ctl's standard handles and outlives it, so pipes
    // would never reach end-of-file: wait on the exit status and read the log on failure.
    let log = install.play_root.join("postgres.log");
    let started = std::time::Instant::now();
    let status = quiet(&install.postgres_tool("pg_ctl"))
        .arg("-D")
        .arg(&data)
        .arg("-l")
        .arg(&log)
        .arg("-o")
        .arg(format!("-p {port} -h 127.0.0.1 -c max_connections=120"))
        .args(["-w", "start"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    crate::log::event(
        "DATABASE_STARTED",
        format!(
            "port={port} ok={} ms={}",
            status.success(),
            started.elapsed().as_millis()
        ),
    );
    if !status.success() {
        let tail = crate::files::tail(&log, 1);
        return Err(io::Error::other(format!(
            "Starting the game database failed: {}",
            tail.trim()
        )));
    }
    Ok(true)
}

/// Creates the save's database when it does not exist yet. `name` is already validated by
/// `saves::valid_database`, so it is safe inside the SQL literal.
pub fn ensure_database(install: &Install, port: u16, name: &str) -> Result<(), io::Error> {
    if !crate::saves::valid_database(name) {
        return Err(io::Error::other(
            "The saved game has an invalid database name.",
        ));
    }
    let port = port.to_string();
    let exists = run(
        quiet(&install.postgres_tool("psql")).args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            ADMIN,
            "-d",
            "postgres",
            "-Atc",
            &format!("SELECT 1 FROM pg_database WHERE datname='{name}'"),
        ]),
        "Looking up the saved game",
    )?;
    if exists == "1" {
        return Ok(());
    }
    run(
        quiet(&install.postgres_tool("createdb")).args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            ADMIN,
            "-T",
            "template0",
            name,
        ]),
        "Creating the saved game",
    )
    .map(|_| ())
}

/// Stops our cluster (fast mode: sessions end, data is flushed).
pub fn stop(install: &Install) -> Result<(), io::Error> {
    run(
        quiet(&install.postgres_tool("pg_ctl"))
            .arg("-D")
            .arg(pgdata(install))
            .args(["-m", "fast", "-w", "stop"]),
        "Stopping the game database",
    )
    .map(|_| ())
}
