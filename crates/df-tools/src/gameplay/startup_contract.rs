use super::{join_grant_driver, startup_configuration};
// Symlink, mode and non-Unicode path construction below are Unix-only cases.
#[cfg(unix)]
use std::os::unix::{
    ffi::OsStringExt,
    fs::{DirBuilderExt, symlink},
};
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::{
    env::VarError,
    ffi::OsString,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::oneshot;
// Only the Unix-gated fixture below needs filesystem paths and its counter.
#[cfg(unix)]
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicUsize,
};

// The startup fixture lives under the author's macOS namespace, so the
// fixture and every test that needs it only run on Unix.
#[cfg(unix)]
const ROOT: &str = "/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/server-d02-startup-contract-20261009-a1/native-fixtures-01";
#[cfg(unix)]
const PAYLOAD: &[u8] = b"startup contract byte-load fixture";
#[cfg(unix)]
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[cfg(unix)]
struct Fixture {
    root: PathBuf,
    web: PathBuf,
    assets: PathBuf,
}

#[cfg(unix)]
impl Fixture {
    fn new() -> Self {
        let granted = PathBuf::from(
            std::env::var_os("DF_SERVER_D02_FIXTURE_ROOT")
                .expect("Root must grant the exact startup fixture namespace"),
        );
        assert_eq!(granted, Path::new(ROOT));
        assert!(fs::symlink_metadata(&granted).unwrap().is_dir());
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        assert!(number < 64, "finite startup fixture count");
        let root = granted.join(format!("startup-{}-{number}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let web = root.join("web");
        let assets = root.join("assets");
        for directory in [
            &web,
            &assets,
            &assets.join("scenes"),
            &root.join("concept-art"),
        ] {
            fs::DirBuilder::new().mode(0o700).create(directory).unwrap();
        }
        let fixture = Self { root, web, assets };
        for (path, _) in fixture.required() {
            fs::write(path, PAYLOAD).unwrap();
        }
        fixture
    }

    fn required(&self) -> [(PathBuf, u64); 5] {
        [
            (self.web.join("df_tools.js"), 1024 * 1024),
            (self.web.join("df_tools_bg.wasm"), 32 * 1024 * 1024),
            (
                self.assets.join("scenes/mara-harbor-v4.png"),
                16 * 1024 * 1024,
            ),
            (
                self.root
                    .join("concept-art/scene-campfire-under-stars.webp"),
                4 * 1024 * 1024,
            ),
            (self.root.join("concept-art/vell-avatar.webp"), 1024 * 1024),
        ]
    }

    fn inn(&self) -> PathBuf {
        self.root
            .join("concept-art/scene-tavern-barkeep-talk-rain.webp")
    }

    fn load(&self) -> Result<startup_configuration::StartupAssets, io::Error> {
        startup_configuration::load_assets(&self.web, &self.assets)
    }
}

fn assert_fixed_endpoint(config: &tokio_postgres::Config, database: &str) {
    assert_eq!(
        config.get_hosts(),
        &[tokio_postgres::config::Host::Tcp("127.0.0.1".into())]
    );
    assert_eq!(config.get_ports(), &[55517]);
    assert_eq!(config.get_user(), Some("df_gameplay_demo_admin_20261004"));
    assert_eq!(config.get_dbname(), Some(database));
    assert!(config.get_password().is_none());
}

#[test]
fn database_defaults_and_valid_names_preserve_fixed_local_endpoint() {
    let default = startup_configuration::database_config(Err(VarError::NotPresent)).unwrap();
    assert_fixed_endpoint(&default, "df_gameplay_demo_20261004_r03");
    for name in ["df_gameplay_demo_", "df_gameplay_demo_Test_123"] {
        assert_fixed_endpoint(
            &startup_configuration::database_config(Ok(name.into())).unwrap(),
            name,
        );
    }
    let exact = format!(
        "df_gameplay_demo_{}",
        "a".repeat(63 - "df_gameplay_demo_".len())
    );
    assert_eq!(exact.len(), 63);
    assert_fixed_endpoint(
        &startup_configuration::database_config(Ok(exact.clone())).unwrap(),
        &exact,
    );
    assert!(startup_configuration::database_config(Ok(exact + "a")).is_err());
}

#[test]
fn invalid_database_input_refuses_without_echoing_secret_or_mutating_environment() {
    for name in [
        "",
        "other_database",
        "df_gameplay_demo_/tmp",
        "df_gameplay_demo_;DROP",
        "df_gameplay_demo_é",
        "postgres://user:private-test-sentinel@host/db",
        "df_gameplay_demo_password=private-test-sentinel",
    ] {
        let error = startup_configuration::database_config(Ok(name.into())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "local demonstration database invalid");
        assert!(!error.to_string().contains("private-test-sentinel"));
    }
    // Non-Unicode input is built per platform: an invalid UTF-8 byte on Unix,
    // a lone surrogate on Windows.
    #[cfg(unix)]
    let bytes = OsString::from_vec(vec![0xff]);
    #[cfg(windows)]
    let bytes = OsString::from_wide(&[0xD800]);
    assert!(startup_configuration::database_config(Err(VarError::NotUnicode(bytes))).is_err());
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[test]
fn all_five_required_assets_load_and_only_missing_inn_is_optional() {
    let fixture = Fixture::new();
    let loaded = fixture.load().unwrap();
    for bytes in [
        &loaded.glue,
        &loaded.wasm,
        &loaded.art,
        &loaded.campfire,
        &loaded.portrait,
    ] {
        assert_eq!(bytes.as_ref(), PAYLOAD);
    }
    assert!(loaded.inn.is_none());
    fs::write(fixture.inn(), b"optional bytes").unwrap();
    assert_eq!(
        fixture.load().unwrap().inn.unwrap().as_ref(),
        b"optional bytes"
    );
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[test]
fn each_missing_required_asset_refuses_the_actual_loader() {
    let fixture = Fixture::new();
    for (path, _) in fixture.required() {
        fs::remove_file(&path).unwrap();
        let error = match fixture.load() {
            Err(error) => error,
            Ok(_) => panic!("missing mandatory asset cannot admit startup"),
        };
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        fs::write(path, PAYLOAD).unwrap();
    }
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[test]
fn each_required_asset_refuses_oversize_directory_and_symlink() {
    let fixture = Fixture::new();
    let target = fixture.root.join("link-target");
    fs::write(&target, PAYLOAD).unwrap();
    for (path, maximum) in fixture.required() {
        fs::File::create(&path)
            .unwrap()
            .set_len(maximum + 1)
            .unwrap();
        assert!(fixture.load().is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(fixture.load().is_err());
        fs::remove_dir(&path).unwrap();
        symlink(&target, &path).unwrap();
        assert!(fixture.load().is_err());
        fs::remove_file(&path).unwrap();
        fs::write(path, PAYLOAD).unwrap();
    }
    assert!(fixture.load().is_ok());
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[test]
fn optional_inn_refuses_oversize_directory_and_symlink_without_falling_back() {
    let fixture = Fixture::new();
    let inn = fixture.inn();
    fs::File::create(&inn)
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert!(fixture.load().is_err());
    fs::remove_file(&inn).unwrap();
    fs::create_dir(&inn).unwrap();
    assert!(fixture.load().is_err());
    fs::remove_dir(&inn).unwrap();
    let target = fixture.root.join("inn-target");
    fs::write(&target, PAYLOAD).unwrap();
    symlink(&target, &inn).unwrap();
    assert!(fixture.load().is_err());
    fs::remove_file(&inn).unwrap();
    assert!(fixture.load().unwrap().inn.is_none());
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[test]
fn actual_byte_bound_accepts_exact_limit_and_refuses_one_more() {
    let fixture = Fixture::new();
    let file = fixture.root.join("bounded-byte-input");
    fs::write(&file, b"abc").unwrap();
    assert_eq!(
        super::bounded_asset(file.clone(), 3).unwrap().as_ref(),
        b"abc"
    );
    fs::write(&file, b"abcd").unwrap();
    assert!(super::bounded_asset(file, 3).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn actual_grant_driver_joins_success_protocol_error_and_safe_panic() {
    let success = tokio::spawn(async { Ok::<(), tokio_postgres::Error>(()) });
    assert!(join_grant_driver(success).await.is_ok());
    let failed = tokio::spawn(async {
        let error = "invalid-connection-option"
            .parse::<tokio_postgres::Config>()
            .unwrap_err();
        Err::<(), tokio_postgres::Error>(error)
    });
    let error = join_grant_driver(failed).await.unwrap_err();
    assert_eq!(error.to_string(), "grant driver protocol failed");
    let panicked: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>> =
        tokio::spawn(async { panic!("controlled startup contract panic") });
    let error = join_grant_driver(panicked).await.unwrap_err();
    assert_eq!(error.to_string(), "grant driver join failed");
}

struct PendingDriver {
    polled: Option<oneshot::Sender<()>>,
    dropped: Arc<AtomicBool>,
}

impl Future for PendingDriver {
    type Output = Result<(), tokio_postgres::Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(polled) = self.polled.take() {
            polled.send(()).unwrap();
        }
        Poll::Pending
    }
}

impl Drop for PendingDriver {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

async fn pending_driver_is_aborted_and_joined() -> bool {
    let dropped = Arc::new(AtomicBool::new(false));
    let (sender, first_poll) = oneshot::channel();
    let driver = tokio::spawn(PendingDriver {
        polled: Some(sender),
        dropped: dropped.clone(),
    });
    first_poll.await.unwrap();
    assert!(!dropped.load(Ordering::SeqCst));
    let error = join_grant_driver(driver).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "grant driver deadline; aborted task joined"
    );
    dropped.load(Ordering::SeqCst)
}

#[tokio::test(flavor = "current_thread")]
async fn pending_grant_driver_is_polled_then_aborted_joined_and_dropped() {
    assert!(pending_driver_is_aborted_and_joined().await);
}

#[cfg(unix)] // Needs the author's macOS fixture root.
#[tokio::test(flavor = "current_thread")]
async fn whole_original_contract_consumes_actual_boundary_results_and_explicit_gaps() {
    let fixture = Fixture::new();
    let loaded = fixture.load().unwrap();
    let mandatory = [
        &loaded.glue,
        &loaded.wasm,
        &loaded.art,
        &loaded.campfire,
        &loaded.portrait,
    ];
    let five_loaded = mandatory.iter().all(|bytes| bytes.as_ref() == PAYLOAD);
    let optional_absent = loaded.inn.is_none();
    fs::remove_file(fixture.web.join("df_tools.js")).unwrap();
    let mandatory_missing_refused = fixture.load().is_err();
    let config = startup_configuration::database_config(Err(VarError::NotPresent)).unwrap();
    assert_fixed_endpoint(&config, "df_gameplay_demo_20261004_r03");
    let invalid = startup_configuration::database_config(Ok(
        "postgres://user:private-test-sentinel@host/db".into(),
    ))
    .unwrap_err();
    let safe_refusal = invalid.to_string() == "local demonstration database invalid";
    let abort_join_drop_observed = pending_driver_is_aborted_and_joined().await;
    let rendered = format!(
        concat!(
            "{{\n",
            "  \"contract\": \"B-C-df-server-D02\",\n",
            "  \"consumer\": \"df-tools::main -> gameplay::serve\",\n",
            "  \"scope\": \"selected loopback demonstration; no general production readiness\",\n",
            "  \"decision\": {{\"choice\": \"extract and qualify actual serve configuration and byte-loading operations\", \"rationale\": \"real consumer executes the same policy; no process environment mutation or duplicate test model\", \"alternatives\": [\"new df-server/readiness registry rejected: not the current consumer and would invent authority\", \"copied test-only loader/configuration rejected: would not qualify actual serve\", \"general hosted readiness deferred: required production composition and secrets do not exist\"]}},\n",
            "  \"configuration\": {{\"database_host\": \"127.0.0.1\", \"database_port\": 55517, \"database_role\": \"df_gameplay_demo_admin_20261004\", \"database_name\": \"prefix + ASCII alphanumeric/underscore + at most 63 bytes\", \"listener\": \"127.0.0.1:63309\", \"credential_URI\": false, \"test_environment_mutation\": false}},\n",
            "  \"observed\": {{\"five_mandatory_byte_loads\": {}, \"optional_Inn_NotFound\": {}, \"missing_required_refused\": {}, \"safe_configuration_refusal\": {}, \"pending_driver_abort_join_drop\": {}}},\n",
            "  \"secrets\": {{\"current_origin\": \"model::random local fence/player/display bytes before PostgreSQL admission\", \"diagnostics\": \"supplied invalid database input omitted\", \"external_secret_backend\": \"absent\", \"credential_issuance_fault\": \"unperformed\"}},\n",
            "  \"startup_order\": [\"mandatory assets\", \"source checkpoint and recovery inventory\", \"local entropy and database configuration\", \"grant connect + durable admission\", \"repository connect + configure reconnect\", \"listener bind\", \"issuer + configure reconnect\", \"DurableOwner + actor thread handoff\", \"HTTP/gRPC serving select\"],\n",
            "  \"readiness\": {{\"authority\": \"no public readiness API\", \"source_policy\": \"failure before serving cannot assert ready\", \"executors\": \"only current prepared local gameplay owners; no general dispatcher\", \"decoded_media\": \"unperformed\", \"PostgreSQL_faults\": \"unperformed\"}},\n",
            "  \"cleanup\": {{\"grant_driver\": \"2s join wait; then abort plus 2s join wait; actual pending task destruction observed\", \"repository_and_issuer_startup_failure\": \"source-declared release/close; fault execution unperformed\", \"actor_thread_spawn_or_handoff_failure\": \"source-declared close_unstarted_actor; fault execution unperformed\", \"optional_qualification_task_await\": \"no timeout at this call site\", \"actor_thread_join\": \"spawn_blocking thread.join; no timeout at this call site\", \"optional_phase_tasks\": \"courier/NPC/Inn/restart/rest source-declare 47s wait then abort plus 2s join wait; fault execution unperformed; join-pending error remains possible\", \"HTTP_gRPC_futures_and_listener\": \"owned by serving select and dropped on select exit; not proven graceful in-flight request drain\", \"preview\": \"unchanged OwnedPreview tests; separate qualification command\", \"fixture_files\": \"retained in Root-granted namespace; Root owns cleanup\"}},\n",
            "  \"unsupported\": [\"general hosted server\", \"external secret provider\", \"commerce/provider egress readiness\", \"arbitrary required-executor registry\", \"native startup on WASM\"]\n",
            "}}\n"
        ),
        five_loaded,
        optional_absent,
        mandatory_missing_refused,
        safe_refusal,
        abort_join_drop_observed
    );
    assert_eq!(rendered, include_str!("startup_contract.json"));
}
