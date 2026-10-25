//! Hosted runtime design 8.4 item 4: a runtime is launched with a cleared environment. This test
//! executable is its own runtime stand-in (no test harness): started by the launcher as
//! `<exe> campaign-runtime --descriptor <path>`, it writes the NAMES of the environment variables
//! it saw (never values) beside the descriptor and exits; started any other way it is the test.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{
        collections::BTreeSet,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use df_runner::{
        launcher::RuntimeLauncher,
        native::NativeLauncher,
        spec::{PlacementId, RuntimeLimits, RuntimeSpec},
    };

    /// Names a minimal runtime image may carry: its init adds nothing else (design 8.4 item 4).
    const ALLOWED_NAMES: [&str; 3] = ["PATH", "HOSTNAME", "HOME"];

    /// Windows starts every process with this one variable even when the environment is cleared.
    #[cfg(windows)]
    const ADDED_BY_THE_OPERATING_SYSTEM: [&str; 1] = ["PROCESSOR_ARCHITECTURE"];
    #[cfg(not(windows))]
    const ADDED_BY_THE_OPERATING_SYSTEM: [&str; 0] = [];

    pub fn main() {
        let arguments: Vec<OsString> = std::env::args_os().collect();
        if arguments
            .get(1)
            .is_some_and(|word| word == "campaign-runtime")
        {
            report_environment(&arguments);
            return;
        }
        run_test();
    }

    fn report_environment(arguments: &[OsString]) {
        let descriptor = arguments.get(3).expect("--descriptor <path>");
        if Path::new(descriptor)
            .file_name()
            .is_some_and(|name| name == "hang.toml")
        {
            // A runtime that never exits by itself: the launcher must kill it after the grace.
            std::thread::sleep(std::time::Duration::from_secs(120));
            return;
        }
        let names: Vec<String> = std::env::vars_os()
            .map(|(name, _value)| name.to_string_lossy().into_owned())
            .collect();
        let report = Path::new(descriptor).with_extension("environment");
        fs::write(report, names.join("\n")).expect("report the environment names");
    }

    fn run_test() {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("cleared_environment_{}", std::process::id()));
        fs::create_dir_all(&directory).expect("scratch directory");
        let descriptor = directory.join("runtime.toml");
        fs::write(&descriptor, "# descriptor stand-in").expect("descriptor");
        let spec = RuntimeSpec {
            placement_id: PlacementId::new([9; 16]),
            descriptor_path: descriptor.clone(),
            slot: 1,
            limits: RuntimeLimits::default(),
        };
        let ambient = std::env::vars_os().count();
        assert!(
            ambient > 0,
            "the runner's own environment is not empty, so clearing it means something"
        );

        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let launcher =
                    NativeLauncher::new(std::env::current_exe().expect("test executable"));
                let handle = launcher.start(&spec).await.expect("started");
                launcher
                    .stop(&handle, Duration::from_secs(60))
                    .await
                    .expect("exited")
            });
        assert!(
            outcome.is_clean(),
            "the stand-in runtime exits 0: {outcome:?}"
        );

        let seen: BTreeSet<String> = fs::read_to_string(descriptor.with_extension("environment"))
            .expect("the runtime reported its environment")
            .lines()
            .map(str::to_string)
            .collect();
        let extra: Vec<&String> = seen
            .iter()
            .filter(|name| {
                !ALLOWED_NAMES.contains(&name.as_str())
                    && !ADDED_BY_THE_OPERATING_SYSTEM.contains(&name.as_str())
            })
            .collect();
        assert!(
            extra.is_empty(),
            "the runtime inherited environment variables: {extra:?}"
        );
        stop_kills_a_runtime_that_ignores_the_grace_period(&directory);
        fs::remove_dir_all(&directory).expect("clean up");
        println!("test the_runtime_is_launched_with_a_cleared_environment ... ok");
    }

    fn stop_kills_a_runtime_that_ignores_the_grace_period(directory: &Path) {
        let descriptor = directory.join("hang.toml");
        fs::write(&descriptor, "# a runtime that hangs").expect("descriptor");
        let spec = RuntimeSpec {
            placement_id: PlacementId::new([8; 16]),
            descriptor_path: descriptor,
            slot: 2,
            limits: RuntimeLimits::default(),
        };
        let started = std::time::Instant::now();
        let report = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let launcher =
                    NativeLauncher::new(std::env::current_exe().expect("test executable"));
                let handle = launcher.start(&spec).await.expect("started");
                launcher
                    .stop(&handle, Duration::from_millis(300))
                    .await
                    .expect("killed after the grace")
            });
        assert!(
            !report.is_clean(),
            "a killed runtime is not a clean exit: {report:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the stop must not wait for the runtime's own 120 s"
        );
        println!("test stop_kills_a_runtime_that_ignores_the_grace_period ... ok");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::main();
}

/// The runtime and the launcher are native-only; the browser target builds an empty program.
#[cfg(target_arch = "wasm32")]
fn main() {}
