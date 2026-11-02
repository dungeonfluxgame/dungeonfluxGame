//! Native gameplay composition: one configuration owner, runtime, ledger and telemetry lifetime.

use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use df_assets::{CatalogConfig, ObjectEnvironment};
use df_imagery::{ConfigControl, ImageContentRating, runtime::UsageProjectionLimits};
use df_observe::TelemetryLimits;
use df_persistence::postgres_tls::PostgresTransport;

use super::{
    cost::port::MeteringPort,
    image_configuration,
    image_host::{LocalHostPorts, LocalHostSettings, LocalImageHost},
    image_telemetry::{ImageTelemetry, TelemetrySettings},
    imagery::Imagery,
    imagery_identity::LocalRunGrant,
    resources::Hub,
    startup_configuration::{ContentRating, ImageryOff, ImagerySettings},
};

pub(super) struct Inputs<'a> {
    pub(super) database: tokio_postgres::Config,
    pub(super) run_grant: LocalRunGrant,
    pub(super) source_revision: u64,
    pub(super) legacy: &'a ImagerySettings,
    pub(super) rating: ContentRating,
    pub(super) hub: Hub,
    pub(super) qualification: bool,
    pub(super) fixture: Option<SocketAddr>,
    pub(super) manifest_directory: Option<&'a Path>,
}

pub(super) struct StartedImages {
    imagery: Imagery,
    live: Option<LiveImages>,
}
struct LiveImages {
    host: LocalImageHost,
    telemetry: ImageTelemetry,
    shutdown_timeout: Duration,
}
pub(super) struct ImageHealth {
    pub(super) runtime: (u64, bool, df_imagery::telemetry::ObserverHealth),
    pub(super) observer: df_imagery::telemetry::ObserverHealth,
    pub(super) sink: df_observe::DiagnosticHealth,
    pub(super) ingestion: df_telemetry::IngestionHealth,
}

impl StartedImages {
    /// The shared diagnostic sink of the image telemetry, when it is running.
    pub(super) fn telemetry_sink(&self) -> Option<Arc<dyn df_observe::DiagnosticSink>> {
        self.live
            .as_ref()
            .map(|live| live.telemetry.diagnostics().diagnostic_sink())
    }

    pub(super) fn diagnostics(&self) -> Option<ImageDiagnostics> {
        self.live.as_ref().map(|live| ImageDiagnostics {
            control: live.host.control().clone(),
            host: live.host.diagnostics(),
            telemetry: live.telemetry.diagnostics(),
        })
    }
    pub(super) fn imagery(&self) -> &Imagery {
        &self.imagery
    }
    pub(super) async fn shutdown(mut self) -> Result<(), io::Error> {
        let Some(mut live) = self.live.take() else {
            return Ok(());
        };
        let result = live.host.shutdown().await;
        // Telemetry drains even when the runtime already failed.
        let telemetry =
            tokio::task::spawn_blocking(move || live.telemetry.shutdown(live.shutdown_timeout))
                .await
                .map_err(|_| refused("image telemetry shutdown join failed"))?
                .map_err(|_| refused("image telemetry shutdown failed"));
        result.and(telemetry)
    }
}

pub(super) async fn start(inputs: Inputs<'_>) -> Result<StartedImages, io::Error> {
    // Qualification launches have no image storage, workers, configuration reads or DB effects.
    if inputs.qualification {
        return Ok(StartedImages {
            imagery: Imagery::disabled(ImageryOff::Qualification),
            live: None,
        });
    }
    let control = image_configuration::control(inputs.legacy, false)?;
    if let Some(address) = inputs.fixture {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(refused("image fixture must be loopback"));
        }
        let mut patch = df_imagery::RuntimePatch::empty();
        patch.mode = Some(df_imagery::GlobalMode::Live);
        patch.default_backend = Some(df_imagery::BackendId::OpenAi);
        patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
        patch.backend_enabled[df_imagery::BackendId::OpenAi.index()] = Some(true);
        patch.backend_endpoint[df_imagery::BackendId::OpenAi.index()] = Some(Some(
            df_imagery::ApiEndpoint::new(&format!("http://{address}"))
                .map_err(|_| refused("image fixture endpoint invalid"))?,
        ));
        patch.analysis_classification = Some(df_imagery::AnalysisRoutePatch::Clear);
        patch.analysis_inspection = Some(df_imagery::AnalysisRoutePatch::Clear);
        for purpose in (0..df_imagery::Purpose::COUNT).filter_map(df_imagery::Purpose::from_index) {
            patch.purpose_backend[purpose.index()] = Some(Some(df_imagery::BackendId::OpenAi));
        }
        let mut owner = control
            .lock()
            .map_err(|_| refused("image config unavailable"))?;
        let revision = owner.revision();
        owner
            .apply(patch, revision)
            .map_err(|_| refused("image fixture configuration refused"))?;
    }
    let analysis_fixture = optional("DF_IMAGERY_ANALYSIS_FIXTURE")?
        .map(|value| {
            value
                .parse::<SocketAddr>()
                .map_err(|_| refused("image analysis fixture invalid"))
        })
        .transpose()?;
    if analysis_fixture.is_some_and(|address| {
        inputs.fixture.is_none() || !address.ip().is_loopback() || address.port() == 0
    }) {
        return Err(refused(
            "image analysis fixture requires the captured loopback development gate",
        ));
    }
    let adventure = match (inputs.manifest_directory, inputs.legacy) {
        (Some(directory), ImagerySettings::On(legacy)) => Some(
            super::imagery::AdventureManifest::new(directory, legacy.cache_dir.clone()),
        ),
        _ => None,
    };
    let shutdown_timeout = millis("DF_IMAGERY_SHUTDOWN_TIMEOUT_MS", 30_000)?;
    let database_timeout = millis("DF_IMAGERY_DATABASE_TIMEOUT_MS", 30_000)?;
    let poll_interval = millis("DF_IMAGERY_POLL_INTERVAL_MS", 100)?;
    let operator_database = match optional("DF_IMAGERY_OPERATOR_DATABASE")? {
        Some(value) => value
            .parse()
            .map_err(|_| refused("image operator database invalid"))?,
        None => inputs.database.clone(),
    };
    let transport = if df_persistence::postgres_tls::all_hosts_loopback(&inputs.database) {
        PostgresTransport::PlaintextLoopbackOnly
    } else {
        PostgresTransport::VerifiedTls {
            ca_file: PathBuf::from(
                optional("DF_IMAGERY_DATABASE_CA")?
                    .ok_or_else(|| refused("image verified database CA required"))?,
            ),
        }
    };
    let object_root = PathBuf::from(
        optional("DF_IMAGERY_OBJECT_ROOT")?
            .unwrap_or_else(|| ".dungeonflux/private-images".to_owned()),
    );
    let environment = ObjectEnvironment::new(
        &optional("DF_IMAGERY_OBJECT_ENV")?.unwrap_or_else(|| "local".to_owned()),
    )
    .map_err(|_| refused("image object environment invalid"))?;
    let telemetry_root = PathBuf::from(
        optional("DF_TELEMETRY_DIR")?.unwrap_or_else(|| ".dungeonflux/telemetry".to_owned()),
    );
    let level = match optional("DF_IMAGERY_LOG_LEVEL")?
        .as_deref()
        .unwrap_or("info")
    {
        "off" => df_imagery::telemetry::ImageLogLevel::Off,
        "error" => df_imagery::telemetry::ImageLogLevel::Error,
        "warn" => df_imagery::telemetry::ImageLogLevel::Warn,
        "info" => df_imagery::telemetry::ImageLogLevel::Info,
        "debug" => df_imagery::telemetry::ImageLogLevel::Debug,
        "trace" => df_imagery::telemetry::ImageLogLevel::Trace,
        _ => return Err(refused("image log level invalid")),
    };
    let settings = LocalHostSettings {
        worker_database: inputs.database,
        fixture: inputs.fixture,
        analysis_fixture,
        adventure,
        operator_database,
        transport,
        connections: positive("DF_IMAGERY_DATABASE_CONNECTIONS", 2)?,
        max_current_grants: positive("DF_IMAGERY_MAX_CURRENT_GRANTS", 512)?,
        object_root,
        environment,
        catalog: CatalogConfig::default(),
        reviewed_maximum_bytes: positive("DF_IMAGERY_REVIEWED_MAX_BYTES", 16 * 1024 * 1024)? as u64,
        annotation_maximum_bytes: positive("DF_IMAGERY_ANNOTATION_MAX_BYTES", 1024 * 1024)? as u64,
        control,
        run_grant: inputs.run_grant,
        source_revision: inputs.source_revision,
        rating: match inputs.rating {
            ContentRating::Restrained => ImageContentRating::Restrained,
            ContentRating::Mature => ImageContentRating::Mature,
        },
        poll_interval,
        shutdown_timeout,
        database_timeout,
        usage_limits: UsageProjectionLimits {
            max_attempts: positive("DF_IMAGERY_USAGE_MAX_ATTEMPTS", 64)?,
            max_observations: positive("DF_IMAGERY_USAGE_MAX_OBSERVATIONS", 1024)?,
            max_unit_lines: positive("DF_IMAGERY_USAGE_MAX_UNIT_LINES", 64)?,
            max_word_bytes: positive("DF_IMAGERY_USAGE_MAX_WORD_BYTES", 256)?,
        },
    };
    let mut telemetry = ImageTelemetry::open(
        &telemetry_root,
        TelemetrySettings {
            limits: TelemetryLimits::default(),
            maximum_pending: positive("DF_IMAGERY_TELEMETRY_PENDING", 1024)?,
            level,
        },
    )
    .map_err(|_| refused("image telemetry unavailable"))?;
    let opened = LocalImageHost::open(
        settings,
        LocalHostPorts {
            observer: telemetry.observer(),
            costs: MeteringPort::from_ledger(inputs.hub.ledger().clone()),
        },
        inputs.hub,
    )
    .await;
    let host = match opened {
        Ok(host) => host,
        Err(error) => {
            let drained =
                tokio::task::spawn_blocking(move || telemetry.shutdown(shutdown_timeout)).await;
            if !matches!(drained, Ok(Ok(()))) {
                return Err(refused("image startup and telemetry drain failed"));
            }
            return Err(error);
        }
    };
    Ok(StartedImages {
        imagery: host.imagery().clone(),
        live: Some(LiveImages {
            host,
            telemetry,
            shutdown_timeout,
        }),
    })
}

/// Deployment settings are consumed here rather than by the core image policy parser.
pub(super) fn is_host_setting(name: &str) -> bool {
    matches!(
        name,
        "DF_IMAGERY_ANALYSIS_FIXTURE"
            | "DF_IMAGERY_ANNOTATION_MAX_BYTES"
            | "DF_IMAGERY_REVIEWED_MAX_BYTES"
            | "DF_IMAGERY_CREDENTIAL_MAP"
            | "DF_IMAGERY_CONFIG"
            | "DF_IMAGERY_OBJECT_ROOT"
            | "DF_IMAGERY_OBJECT_ENV"
            | "DF_IMAGERY_OPERATOR_DATABASE"
            | "DF_IMAGERY_DATABASE_CA"
            | "DF_IMAGERY_DATABASE_CONNECTIONS"
            | "DF_IMAGERY_MAX_CURRENT_GRANTS"
            | "DF_IMAGERY_DATABASE_TIMEOUT_MS"
            | "DF_IMAGERY_SHUTDOWN_TIMEOUT_MS"
            | "DF_IMAGERY_POLL_INTERVAL_MS"
            | "DF_IMAGERY_LOG_LEVEL"
            | "DF_IMAGERY_TELEMETRY_PENDING"
            | "DF_IMAGERY_USAGE_MAX_ATTEMPTS"
            | "DF_IMAGERY_USAGE_MAX_OBSERVATIONS"
            | "DF_IMAGERY_USAGE_MAX_UNIT_LINES"
            | "DF_IMAGERY_USAGE_MAX_WORD_BYTES"
    )
}
fn optional(name: &'static str) -> Result<Option<String>, io::Error> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(refused("image deployment setting invalid")),
    }
}
fn positive(name: &'static str, default: usize) -> Result<usize, io::Error> {
    optional(name)?
        .map(|value| {
            value
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| refused("image deployment bound invalid"))
        })
        .unwrap_or(Ok(default))
}
fn millis(name: &'static str, default: u64) -> Result<Duration, io::Error> {
    optional(name)?
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .map(Duration::from_millis)
                .ok_or_else(|| refused("image deployment duration invalid"))
        })
        .unwrap_or(Ok(Duration::from_millis(default)))
}
fn refused(message: &'static str) -> io::Error {
    io::Error::other(message)
}

#[derive(Clone)]
pub(super) struct ImageDiagnostics {
    control: Arc<Mutex<ConfigControl>>,
    host: super::image_host::HostDiagnostics,
    telemetry: super::image_telemetry::TelemetryDiagnostics,
}
impl ImageDiagnostics {
    pub(super) fn grants(
        &self,
    ) -> Result<Option<super::image_sources::GrantDiagnostics>, df_imagery::runtime::RuntimeError>
    {
        self.host.grants()
    }
    pub(super) fn quota(
        &self,
    ) -> Result<Option<df_imagery::runtime::QuotaSummary>, df_imagery::runtime::QuotaRefusal> {
        self.host.quota()
    }
    pub(super) fn control(&self) -> Arc<Mutex<ConfigControl>> {
        self.control.clone()
    }
    pub(super) fn recent_events(
        &self,
        limit: usize,
    ) -> Vec<df_imagery::telemetry::RecentImageEvent> {
        self.telemetry.recent(limit)
    }
    pub(super) fn health(&self) -> Result<Option<ImageHealth>, io::Error> {
        let Some(runtime) = self.host.health() else {
            return Ok(None);
        };
        let Some((observer, sink, ingestion)) = self
            .telemetry
            .health()
            .map_err(|_| refused("image telemetry health unavailable"))?
        else {
            return Ok(None);
        };
        Ok(Some(ImageHealth {
            runtime,
            observer,
            sink,
            ingestion,
        }))
    }
}
