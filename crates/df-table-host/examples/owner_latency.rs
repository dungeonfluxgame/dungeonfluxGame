//! Runs spike S1: the owner-thread latency harness over fake ports (O.16).
//!
//! `cargo run --release -p df-table-host --example owner_latency -- help` lists the commands. A run
//! writes its report to standard output, one block per scenario. Always run it with `--release`:
//! a debug build measures the debugger, not the design. Nothing else should be busy on the
//! machine, and runs must not overlap, because they share the cores they are timing.

use std::collections::HashMap;
use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use df_table_host::owner_latency::commit::CommitMode;
use df_table_host::owner_latency::producers::Arrival;
use df_table_host::owner_latency::report::render;
use df_table_host::owner_latency::run::{RunConfig, SnapshotMode, Stop, run};
use df_table_host::owner_latency::sha256::sha256;
use df_table_host::owner_latency::state::WorkloadProfile;

type Failure = Box<dyn Error>;

const HELP: &str = "\
commands:
  scenario [key=value ...]   one scenario
      arrival=saturated|steady|burst   (default saturated)
      window=3                         outstanding per seat when saturated
      rate=20                          inputs per second when steady
      burst=6 period_ms=100            inputs at once and the gap between bursts
      profile=representative|combat|heavy    commit=memory|fsync|postgres
      checkpoint=200000                checkpoint bytes (at least; state alone may be larger)
      snapshot=in_place|copy_on_write
      inputs=5000 or seconds=60        when to stop (inputs are measured ones)
      warmup=300
  matrix [only=prefix]       the documented set of scenarios (about 30 minutes in all)
  busy [seconds=60]          the owner's busy share at 1, 3 and 6 inputs per second
  soak [minutes=10]          saturated fsync run with a table of each minute, for thermal drift
  hash                       throughput of the stand-in software SHA-256
  real                       the H3 rerun over the real owner actor, pipeline, production rules,
                             real projection and group commit (store: memory, fsync, postgres20ms)
";

fn main() -> Result<(), Failure> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    let options = Options::parse(arguments)?;
    let directory = tempfile::tempdir()?;
    let mut out = std::io::stdout().lock();
    match command.as_str() {
        "scenario" => {
            let config = options.scenario(directory.path())?;
            report(&mut out, &config)
        }
        "matrix" => matrix(&mut out, &options, directory.path()),
        "busy" => busy(&mut out, &options, directory.path()),
        "soak" => soak(&mut out, &options, directory.path()),
        "hash" => hash_speed(&mut out),
        "real" => real(&mut out, directory.path()),
        _ => Ok(out.write_all(HELP.as_bytes())?),
    }
}

struct Options(HashMap<String, String>);

impl Options {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, Failure> {
        let mut values = HashMap::new();
        for argument in arguments {
            let (key, value) = argument
                .split_once('=')
                .ok_or_else(|| format!("expected key=value, got {argument}"))?;
            values.insert(key.to_owned(), value.to_owned());
        }
        Ok(Self(values))
    }

    fn text(&self, key: &str, default: &str) -> String {
        self.0
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }

    fn number<T: std::str::FromStr>(&self, key: &str, default: T) -> Result<T, Failure> {
        match self.0.get(key) {
            Some(value) => value
                .parse()
                .map_err(|_| format!("{key}={value} is not a number").into()),
            None => Ok(default),
        }
    }

    fn scenario(&self, directory: &Path) -> Result<RunConfig, Failure> {
        let arrival = match self.text("arrival", "saturated").as_str() {
            "saturated" => Arrival::Saturated {
                window: self.number("window", 3)?,
            },
            "steady" => Arrival::Steady {
                inputs_per_s: self.number("rate", 20)?,
            },
            "burst" => Arrival::Burst {
                inputs: self.number("burst", 6)?,
                period_ms: self.number("period_ms", 100)?,
            },
            other => return Err(format!("unknown arrival {other}").into()),
        };
        let stop = if self.0.contains_key("seconds") {
            Stop::AfterSeconds(self.number("seconds", 60)?)
        } else {
            Stop::AfterInputs(self.number("inputs", 5_000)?)
        };
        let profile = match self.text("profile", "representative").as_str() {
            "representative" => WorkloadProfile::representative(),
            "combat" => WorkloadProfile::combat(),
            "heavy" => WorkloadProfile::heavy(),
            other => return Err(format!("unknown profile {other}").into()),
        };
        let commit = match self.text("commit", "fsync").as_str() {
            "memory" => CommitMode::Memory,
            "fsync" => CommitMode::Fsync,
            "postgres" => CommitMode::ModelledPostgres,
            other => return Err(format!("unknown commit {other}").into()),
        };
        let snapshot = match self.text("snapshot", "in_place").as_str() {
            "in_place" => SnapshotMode::InPlace,
            "copy_on_write" => SnapshotMode::CopyOnWrite,
            other => return Err(format!("unknown snapshot {other}").into()),
        };
        Ok(RunConfig {
            profile,
            commit,
            checkpoint_bytes: self.number("checkpoint", 200_000)?,
            snapshot,
            arrival,
            stop,
            warmup_inputs: self.number("warmup", 300)?,
            directory: directory.to_path_buf(),
        })
    }
}

fn report(out: &mut impl Write, config: &RunConfig) -> Result<(), Failure> {
    let started = Instant::now();
    let result = run(config)?;
    writeln!(out, "{}", render(&result)?)?;
    writeln!(
        out,
        "(wall time {:.1} s)\n",
        started.elapsed().as_secs_f64()
    )?;
    out.flush()?;
    Ok(())
}

fn base(directory: &Path) -> RunConfig {
    RunConfig {
        profile: WorkloadProfile::representative(),
        commit: CommitMode::Fsync,
        checkpoint_bytes: 200_000,
        snapshot: SnapshotMode::InPlace,
        arrival: Arrival::Saturated { window: 3 },
        stop: Stop::AfterInputs(5_000),
        warmup_inputs: 300,
        directory: directory.to_path_buf(),
    }
}

const SMALL_CHECKPOINT_BYTES: usize = 200_000;
const LARGE_CHECKPOINT_BYTES: usize = 2_000_000;

fn matrix(out: &mut impl Write, options: &Options, directory: &Path) -> Result<(), Failure> {
    let only = options.text("only", "");
    let with = |commit, checkpoint_bytes, arrival| RunConfig {
        commit,
        checkpoint_bytes,
        arrival,
        ..base(directory)
    };
    let saturated = Arrival::Saturated { window: 3 };
    let six_at_once = |period_ms| Arrival::Burst {
        inputs: 6,
        period_ms,
    };
    let combat = |commit, arrival| RunConfig {
        profile: WorkloadProfile::combat(),
        ..with(commit, SMALL_CHECKPOINT_BYTES, arrival)
    };
    let scenarios: Vec<(&str, RunConfig)> = vec![
        (
            "A1 flat out, memory commit",
            with(CommitMode::Memory, SMALL_CHECKPOINT_BYTES, saturated),
        ),
        (
            "A2 flat out, fsync ~290 KB",
            with(CommitMode::Fsync, SMALL_CHECKPOINT_BYTES, saturated),
        ),
        (
            "A3 flat out, fsync 2 MB",
            with(CommitMode::Fsync, LARGE_CHECKPOINT_BYTES, saturated),
        ),
        (
            "A4 flat out, postgres 20 ms, ~290 KB",
            with(
                CommitMode::ModelledPostgres,
                SMALL_CHECKPOINT_BYTES,
                saturated,
            ),
        ),
        (
            "A5 flat out, postgres 20 ms, 2 MB",
            with(
                CommitMode::ModelledPostgres,
                LARGE_CHECKPOINT_BYTES,
                saturated,
            ),
        ),
        (
            "B1 six seats at once, fsync ~290 KB",
            with(CommitMode::Fsync, SMALL_CHECKPOINT_BYTES, six_at_once(300)),
        ),
        (
            "B2 six seats at once, postgres ~290 KB",
            with(
                CommitMode::ModelledPostgres,
                SMALL_CHECKPOINT_BYTES,
                six_at_once(200),
            ),
        ),
        (
            "B3 eighteen at once (pending bound), fsync ~290 KB",
            with(
                CommitMode::Fsync,
                SMALL_CHECKPOINT_BYTES,
                Arrival::Burst {
                    inputs: 18,
                    period_ms: 800,
                },
            ),
        ),
        (
            "C1 steady 40 per second, fsync ~290 KB",
            with(
                CommitMode::Fsync,
                SMALL_CHECKPOINT_BYTES,
                Arrival::Steady { inputs_per_s: 40 },
            ),
        ),
        (
            "C2 steady 25 per second, postgres ~290 KB",
            with(
                CommitMode::ModelledPostgres,
                SMALL_CHECKPOINT_BYTES,
                Arrival::Steady { inputs_per_s: 25 },
            ),
        ),
        (
            "D1 combat views, flat out, fsync ~290 KB",
            combat(CommitMode::Fsync, saturated),
        ),
        (
            "D4 heavy views (about 5x the bytes), flat out, fsync ~290 KB",
            RunConfig {
                profile: WorkloadProfile::heavy(),
                ..with(CommitMode::Fsync, SMALL_CHECKPOINT_BYTES, saturated)
            },
        ),
        (
            "D3 copy-on-write snapshot, flat out, fsync ~290 KB",
            RunConfig {
                snapshot: SnapshotMode::CopyOnWrite,
                ..with(CommitMode::Fsync, SMALL_CHECKPOINT_BYTES, saturated)
            },
        ),
    ];
    for (label, config) in scenarios
        .iter()
        .filter(|(label, _)| label.starts_with(&only))
    {
        writeln!(out, "=== {label} ===")?;
        report(out, config)?;
    }
    Ok(())
}

fn busy(out: &mut impl Write, options: &Options, directory: &Path) -> Result<(), Failure> {
    let seconds = options.number("seconds", 60)?;
    for inputs_per_s in [1u32, 3, 6] {
        for commit in [CommitMode::Fsync, CommitMode::ModelledPostgres] {
            writeln!(
                out,
                "=== busy share at {inputs_per_s} inputs per second, {} ===",
                commit.name()
            )?;
            let config = RunConfig {
                commit,
                arrival: Arrival::Steady { inputs_per_s },
                stop: Stop::AfterSeconds(seconds),
                warmup_inputs: 5,
                ..base(directory)
            };
            report(out, &config)?;
        }
    }
    Ok(())
}

fn soak(out: &mut impl Write, options: &Options, directory: &Path) -> Result<(), Failure> {
    let minutes: u64 = options.number("minutes", 10)?;
    writeln!(
        out,
        "=== soak, {minutes} minutes flat out, fsync ~290 KB ==="
    )?;
    let config = RunConfig {
        stop: Stop::AfterSeconds(minutes * 60),
        ..base(directory)
    };
    report(out, &config)
}

fn real(out: &mut impl Write, directory: &Path) -> Result<(), Failure> {
    use df_table_host::owner_latency::real::{
        lobby_burst, lobby_single, projection_cost, rules_path,
    };
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let cost = projection_cost(2_000, directory).await?;
        writeln!(out, "{}", cost.render())?;
        for mode in [
            CommitMode::Memory,
            CommitMode::Fsync,
            CommitMode::ModelledPostgres,
        ] {
            let name = mode.name();
            let reports = [
                lobby_single(&format!("one at a time, {name}"), mode, 400, 20, directory).await?,
                lobby_burst(&format!("six at once, {name}"), mode, 6, 200, 50, directory).await?,
                lobby_burst(
                    &format!("eighteen at once, {name}"),
                    mode,
                    18,
                    100,
                    100,
                    directory,
                )
                .await?,
                rules_path(&format!("rules path, {name}"), mode, 200, directory).await?,
            ];
            for report in &reports {
                writeln!(out, "{}", report.render())?;
            }
            out.flush()?;
        }
        Ok::<(), Failure>(())
    })
}

fn hash_speed(out: &mut impl Write) -> Result<(), Failure> {
    let data = vec![0x5au8; 2_000_000];
    let rounds = 50;
    let started = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(sha256(std::hint::black_box(&data)));
    }
    let seconds = started.elapsed().as_secs_f64();
    let megabytes_per_s = (data.len() * rounds) as f64 / 1e6 / seconds;
    writeln!(
        out,
        "software sha256: {megabytes_per_s:.0} MB/s ({:.2} ms for 2 MB)",
        seconds * 1e3 / rounds as f64
    )?;
    Ok(())
}
