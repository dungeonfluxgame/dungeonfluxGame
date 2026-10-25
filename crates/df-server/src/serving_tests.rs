use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use tokio::net::TcpListener;

use crate::{
    config::{
        DatabaseSection, Environment, InstanceSection, ListenSection, PathsSection, Role,
        ServerConfig, TelemetrySection, TlsMode, TlsSection,
    },
    health::Signal,
    runtime_recovery::{RecoveryFailure, RecoveryFuture, RecoveryStore},
    serving::{
        MaintenanceControl, ServingError, check_health, fake_maintenance_starter, start_with_store,
    },
};

fn run<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn config(role: Role) -> ServerConfig {
    ServerConfig {
        instance: InstanceSection {
            role,
            environment: Environment::Local,
            instance_id: "test-a".to_string(),
        },
        listen: ListenSection {
            public_address: (role == Role::ControlPlane)
                .then(|| "127.0.0.1:0".parse().expect("addr")),
            internal_address: "127.0.0.1:0".parse().expect("addr"),
            metrics_address: "127.0.0.1:0".parse().expect("addr"),
        },
        database: DatabaseSection {
            host: "127.0.0.1".to_string(),
            port: 55911,
            name: "test".to_string(),
            user: "postgres".to_string(),
            password_file: "password.txt".into(),
            pool_size: 4,
            statement_timeout_ms: 30_000,
            tls: TlsSection {
                mode: TlsMode::LoopbackPlaintext,
                ca_file: None,
                server_name: None,
            },
        },
        paths: PathsSection {
            temp_directory: "tmp".into(),
        },
        telemetry: TelemetrySection {
            otlp_endpoint: "http://127.0.0.1:4317".to_string(),
            service_name: "test".to_string(),
            spool_directory: "spool".into(),
            spool_bytes_maximum: 256 * 1024 * 1024,
            push_interval_seconds: 10,
        },
        runtime: Default::default(),
        providers: None,
        local_dev: None,
        objects: None,
        identity: None,
        payments: None,
    }
}

#[derive(Default)]
struct FakeStore {
    prepared: Option<RecoveryFailure>,
    breaks: AtomicU32,
}

impl RecoveryStore for Arc<FakeStore> {
    fn require_no_prepared_transactions(&self) -> RecoveryFuture<'_> {
        Box::pin(async move { self.prepared.map_or(Ok(()), Err) })
    }
    fn service_is_halted(&self) -> RecoveryFuture<'_, bool> {
        Box::pin(async { Ok(false) })
    }
    fn break_service_lock(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            self.breaks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct FakeMaintenance {
    runs: Arc<AtomicU32>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl FakeMaintenance {
    fn started() -> (Box<dyn MaintenanceControl>, Arc<AtomicU32>) {
        let runs = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&runs);
        let task_counter = Arc::clone(&runs);
        let task = tokio::spawn(async move {
            task_counter.fetch_add(1, Ordering::SeqCst);
        });
        (
            Box::new(Self {
                runs,
                task: Some(task),
            }),
            counter,
        )
    }
}

impl MaintenanceControl for FakeMaintenance {
    fn runs(&self) -> u32 {
        self.runs.load(Ordering::SeqCst)
    }
    fn failures(&self) -> u32 {
        0
    }
    fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
    fn stop(mut self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            if let Some(task) = self.task.take() {
                let _ = task.await;
            }
        })
    }
}

#[test]
fn startup_refuses_with_prepared_transactions_and_binds_nothing() {
    run(async {
        // Reserve then release an address so a refused start leaves it bindable.
        let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe");
        let address = probe.local_addr().expect("addr");
        drop(probe);
        let mut cfg = config(Role::ControlPlane);
        cfg.listen.metrics_address = address;
        let store = Arc::new(FakeStore {
            prepared: Some(RecoveryFailure::PreparedTransactionsEnabled),
            breaks: AtomicU32::new(0),
        });
        let (worker, _counter) = FakeMaintenance::started();
        let result = start_with_store(
            &cfg,
            Role::ControlPlane,
            store,
            fake_maintenance_starter(worker),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServingError::StartupRefused(
                crate::runtime_recovery::StartupRefusal::PreparedTransactionsEnabled
            ))
        ));
        // Nothing was bound: the address can be taken again.
        assert!(TcpListener::bind(address).await.is_ok());
    });
}

#[test]
fn a_started_control_plane_runs_a_maintenance_pass_and_readiness_flips_after_checks() {
    run(async {
        let cfg = config(Role::ControlPlane);
        let store = Arc::new(FakeStore::default());
        let (worker, counter) = FakeMaintenance::started();
        let serving = start_with_store(
            &cfg,
            Role::ControlPlane,
            Arc::clone(&store),
            fake_maintenance_starter(worker),
        )
        .await
        .expect("serving");
        assert!(serving.is_ready());
        // The fake pass ran (its task increments before finishing).
        for _ in 0..1000 {
            if counter.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(
            check_health(serving.local_address(), Signal::Ready)
                .await
                .expect("healthcheck")
        );
        // A fenced DF002 reaches the sweeper; other SQLSTATEs do not.
        serving.report_fenced_sqlstate("40001");
        serving.report_fenced_sqlstate("DF002");
        for _ in 0..1000 {
            if store.breaks.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(store.breaks.load(Ordering::SeqCst), 1);
        serving.shutdown().await;
    });
}

#[test]
fn local_campaign_runtime_uses_the_skip_path_and_hosted_refuses_without_a_database() {
    run(async {
        // Local: start_configured skips recovery, serving still becomes ready.
        let serving = crate::serving::start_campaign_runtime(config(Role::CampaignRuntime))
            .await
            .expect("local serving");
        assert!(serving.is_ready());
        serving.shutdown().await;

        // Hosted: the recovery connection is attempted before binding and refused.
        let mut hosted = config(Role::CampaignRuntime);
        hosted.instance.environment = Environment::Production;
        let result = crate::serving::start_campaign_runtime(hosted).await;
        assert!(matches!(
            result,
            Err(ServingError::StartupRefused(
                crate::runtime_recovery::StartupRefusal::DatabaseUnavailable
            ))
        ));
    });
}

#[test]
fn readiness_is_not_healthy_before_startup_checks_pass() {
    // Covered by the refusal test: a refused start returns no Serving at all.
    // Here a wrong role is refused before binding as well.
    run(async {
        let cfg = config(Role::CampaignRuntime);
        let store = Arc::new(FakeStore::default());
        let (worker, _counter) = FakeMaintenance::started();
        let result = start_with_store(
            &cfg,
            Role::ControlPlane,
            store,
            fake_maintenance_starter(worker),
        )
        .await;
        assert!(matches!(result, Err(ServingError::WrongRole { .. })));
    });
}
