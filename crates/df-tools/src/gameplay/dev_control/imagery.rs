//! Authorized views and revision-fenced edits of the existing image configuration owner.
//! A local commit is distinct from runtime acknowledgement; credentials stay in host custody.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use df_imagery::{
    ConfigApplyError, ConfigControl, GenerationQuality, ImageConfig, OverlaySource, ResolvedConfig,
    RuntimePatch, parse_runtime_value,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::image_startup::{ImageDiagnostics, ImageHealth};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigRequest {
    expected_revision: u64,
    patch: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResetRequest {
    expected_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventsQuery {
    last: Option<usize>,
}

/// Most events one read returns; the observer keeps a fixed window beyond this.
const EVENTS_DEFAULT: usize = 50;
const EVENTS_MAXIMUM: usize = df_imagery::telemetry::RECENT_EVENT_CAPACITY;

pub(super) fn commands() -> [Value; 4] {
    [
        json!({
            "command":"HTTP GET /dev/imagery", "interface":"HTTP", "method":"GET",
            "path":"/dev/imagery", "args":{},
            "output":"Structured effective configuration/provenance, desired/applied runtime revision and actual observer/sink/ingestion health; unavailable stays explicit",
        }),
        json!({
            "command":"HTTP GET /dev/imagery/events", "interface":"HTTP", "method":"GET",
            "path":"/dev/imagery/events", "args":{"last":"1 to 256, default 50"},
            "output":"Newest enabled image events oldest first: sequence, event, level, failure label, safe job/stage/attempt ids, elapsed and output bytes; never prompts, paths or credentials",
        }),
        json!({
            "command":"HTTP POST /dev/imagery/config", "interface":"HTTP", "method":"POST",
            "path":"/dev/imagery/config",
            "args":{"expected_revision":"current configuration revision", "patch":"flat dotted fields with scalar values; mode live|off|local-only|cache-only"},
            "example":{"expected_revision":7,"patch":{"mode":"off"}},
            "output":"HTTP202 exact applied_local mutation receipt plus current view; runtime acknowledgement remains separate; stale409 and invalid400 leave configuration unchanged; runtime override only",
        }),
        json!({
            "command":"HTTP POST /dev/imagery/reset", "interface":"HTTP", "method":"POST",
            "path":"/dev/imagery/reset", "args":{"expected_revision":"current configuration revision"},
            "output":"HTTP202 applied_local receipt after clearing runtime overrides to file/environment/launch selections; separate runtime acknowledgement; no file persistence",
        }),
    ]
}

pub(super) async fn show(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let Some(diagnostics) = state.image_diagnostics.as_ref() else {
        return Ok((StatusCode::OK, Json(unmounted())));
    };
    Ok((StatusCode::OK, Json(read(diagnostics)?)))
}

pub(super) async fn events(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let diagnostics = state.image_diagnostics.as_ref().ok_or_else(unavailable)?;
    let last = query.last.unwrap_or(EVENTS_DEFAULT);
    if last == 0 || last > EVENTS_MAXIMUM {
        return Err(bad_request("imagery events last must be 1 to 256"));
    }
    let events = diagnostics
        .recent_events(last)
        .into_iter()
        .map(|event| {
            json!({
                "sequence": event.sequence,
                "at_unix_ms": event.at_unix_ms,
                "event": event.name,
                "level": format!("{:?}", event.level).to_lowercase(),
                "failure": event.failure,
                "job": event.job,
                "stage": event.stage,
                "attempt": event.attempt,
                "elapsed_ms": event.elapsed_ms,
                "output_bytes": event.output_bytes,
            })
        })
        .collect::<Vec<_>>();
    Ok((
        StatusCode::OK,
        Json(json!({"events": events, "returned": events.len(), "window": EVENTS_MAXIMUM})),
    ))
}

pub(super) async fn config(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ConfigRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let diagnostics = state.image_diagnostics.as_ref().ok_or_else(unavailable)?;
    let revision = apply_values(&diagnostics.control(), body.expected_revision, &body.patch)?;
    committed(diagnostics, revision)
}

pub(super) async fn reset(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ResetRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let diagnostics = state.image_diagnostics.as_ref().ok_or_else(unavailable)?;
    let control = diagnostics.control();
    let revision = control
        .lock()
        .map_err(|_| failed("imagery configuration owner unavailable"))?
        .reset(body.expected_revision)
        .map_err(apply_error)?
        .revision;
    committed(diagnostics, revision)
}

fn apply_values(
    control: &Mutex<ConfigControl>,
    expected_revision: u64,
    values: &BTreeMap<String, Value>,
) -> Result<u64, (StatusCode, Json<Value>)> {
    let mut patch = RuntimePatch::empty();
    for (path, value) in values {
        let value = match value {
            Value::String(value) => value.clone(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::Null => "none".to_owned(),
            _ => return Err(bad_request("imagery patch values must be scalar")),
        };
        parse_runtime_value(&mut patch, path, &value)
            .map_err(|_| bad_request("imagery patch is invalid"))?;
    }
    Ok(control
        .lock()
        .map_err(|_| failed("imagery configuration owner unavailable"))?
        .apply(patch, expected_revision)
        .map_err(apply_error)?
        .revision)
}

fn apply_error(error: ConfigApplyError) -> (StatusCode, Json<Value>) {
    match error {
        ConfigApplyError::StaleRevision { expected, current } => (
            StatusCode::CONFLICT,
            Json(json!({"error":"stale imagery revision", "edited":false,
                "expected_revision":expected, "current_revision":current})),
        ),
        ConfigApplyError::Invalid(_) => bad_request("imagery patch is invalid"),
        ConfigApplyError::RevisionOverflow => (
            StatusCode::CONFLICT,
            Json(json!({"error":"imagery revision exhausted", "edited":false})),
        ),
        ConfigApplyError::PersistFailed => failed("imagery configuration change unavailable"),
    }
}

fn committed(diagnostics: &ImageDiagnostics, revision: u64) -> Outcome {
    let mut response = read(diagnostics).unwrap_or_else(|_| {
        json!({"status":"committed_view_unavailable", "configured":true,
            "configuration":null,"runtime":null,"telemetry":null})
    });
    response["mutation"] = json!({"status":"applied_local", "edited":true, "revision":revision});
    Ok((StatusCode::ACCEPTED, Json(response)))
}

fn read(diagnostics: &ImageDiagnostics) -> Result<Value, (StatusCode, Json<Value>)> {
    let control = diagnostics.control();
    let snapshot = control
        .lock()
        .map_err(|_| failed("imagery configuration owner unavailable"))?
        .read_effective();
    let health = diagnostics.health();
    let mut value = state_value(
        &snapshot,
        health.as_ref().ok().and_then(|value| value.as_ref()),
    );
    value["authority_capacity"] = match diagnostics.grants() {
        Ok(Some(grants)) => {
            json!({"status":"available", "registered":grants.registered,"maximum":grants.maximum,
            "capacity_refusals":grants.capacity_refusals,"setting":"DF_IMAGERY_MAX_CURRENT_GRANTS","requires_restart":true})
        }
        Ok(None) => json!({"status":"unavailable", "reason":"runtime_not_mounted"}),
        Err(_) => json!({"status":"unavailable", "reason":"authority_state_unavailable"}),
    };
    value["quota"] = match diagnostics.quota() {
        Ok(Some(summary)) => {
            json!({"status":"available", "closed_accounts":summary.closed_accounts,
            "open_accounts":summary.open_accounts,"probing_accounts":summary.probing_accounts})
        }
        Ok(None) => json!({"status":"unavailable", "reason":"runtime_not_mounted"}),
        Err(_) => json!({"status":"unavailable", "reason":"quota_state_unavailable"}),
    };
    Ok(value)
}

fn unmounted() -> Value {
    json!({"status":"unavailable", "configured":false, "reason":"image runtime not mounted",
        "configuration":null, "runtime":null, "telemetry":null})
}

fn unavailable() -> (StatusCode, Json<Value>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(unmounted()))
}

fn state_value(snapshot: &ResolvedConfig, health: Option<&ImageHealth>) -> Value {
    let desired = snapshot.provenance.revision;
    let applied = health.map(|health| health.runtime.0);
    let running = health.map(|health| health.runtime.1);
    json!({
        "status":if running == Some(true) {"available"} else {"unavailable"},
        "configured":true,
        "configuration":{"revision":desired, "effective":effective(&snapshot.config),
            "provenance":{"revision":desired, "fields":snapshot.provenance.fields.iter()
                .map(|field| json!({"field":field.field,"source":source(&field.source)})).collect::<Vec<_>>(),
                "ignored":snapshot.provenance.ignored}},
        "runtime":propagation(desired, applied, running),
        "telemetry":health.map(health_value).unwrap_or_else(||json!({"status":"unavailable"})),
    })
}

fn propagation(desired: u64, applied: Option<u64>, running: Option<bool>) -> Value {
    let status = match (applied, running) {
        (_, None) => "unobserved",
        (_, Some(false)) => "runtime_unavailable",
        (Some(applied), Some(true)) if applied == desired => "applied",
        (Some(applied), Some(true)) if applied > desired => "configuration_changed_during_read",
        _ => "pending",
    };
    json!({"desired_revision":desired, "applied_revision":applied, "running":running,
        "propagation":status, "acknowledged":status == "applied"})
}

fn source(source: &OverlaySource) -> Value {
    match source {
        OverlaySource::Default => json!({"layer":"default"}),
        OverlaySource::FileImagery => json!({"layer":"file_imagery"}),
        OverlaySource::FilePictures => json!({"layer":"file_pictures"}),
        OverlaySource::LegacyEnv(name) => json!({"layer":"legacy_environment", "name":name}),
        OverlaySource::NewEnv(name) => json!({"layer":"environment", "name":name}),
        OverlaySource::Launch => json!({"layer":"launch"}),
        OverlaySource::Runtime => json!({"layer":"runtime"}),
    }
}

fn effective(config: &ImageConfig) -> Value {
    let purposes: [df_imagery::Purpose; df_imagery::Purpose::COUNT] = [
        df_imagery::Purpose::CreatorPortrait,
        df_imagery::Purpose::CanonicalPerson,
        df_imagery::Purpose::CanonicalRoom,
        df_imagery::Purpose::ScenePlate,
        df_imagery::Purpose::SceneEdit,
        df_imagery::Purpose::Expression,
        df_imagery::Purpose::ItemCloseUp,
        df_imagery::Purpose::PartyMural,
        df_imagery::Purpose::CombatStill,
        df_imagery::Purpose::AnimationFrame,
        df_imagery::Purpose::ReferencePrep,
        df_imagery::Purpose::ImportedDerivative,
    ];
    let backends: [df_imagery::BackendId; df_imagery::BackendId::COUNT] = [
        df_imagery::BackendId::OpenAi,
        df_imagery::BackendId::Fal,
        df_imagery::BackendId::Bfl,
        df_imagery::BackendId::CodexCli,
    ];
    let mode = match config.mode {
        df_imagery::GlobalMode::Live => "live",
        df_imagery::GlobalMode::LocalOnly => "local-only",
        df_imagery::GlobalMode::CacheOnly => "cache-only",
        df_imagery::GlobalMode::Off => "off",
    };
    json!({"mode":mode, "default_backend":config.default_backend.label(),
        "purposes":purposes.into_iter().map(|purpose| (purpose.label().to_owned(),purpose_value(config.purpose(purpose))))
            .collect::<serde_json::Map<_,_>>(),
        "backends":backends.into_iter().map(|backend| (backend.label().to_owned(),backend_value(config.backend(backend))))
            .collect::<serde_json::Map<_,_>>(),
        "transforms":transform_value(config), "scheduler":scheduler_value(config),
        "request_limits":{"max_prompt_chars":config.request_limits.max_prompt_chars,
            "max_references":config.request_limits.max_references,
            "max_transform_steps":config.request_limits.max_transform_steps,
            "max_composite_layers":config.request_limits.max_composite_layers},
        "codex":{"executable":config.codex.executable.as_str(), "workdir":config.codex.workdir.as_str(),
            "cli_profile":config.codex.cli_profile.as_str(), "agent_model":config.codex.agent_model.as_ref().map(|value|value.as_str()),
            "max_stdout_bytes":config.codex.max_stdout_bytes,"max_stderr_bytes":config.codex.max_stderr_bytes,
            "max_message_bytes":config.codex.max_message_bytes},
        "analysis":analysis_value(config),
        "creator":{"bfl_account":config.creator_bfl_account.as_ref().map(|account|account.as_str())},
        "quota":{"enabled":config.quota.enabled,"cooldown_ms":config.quota.cooldown_ms,"maximum_accounts":config.quota.maximum_accounts},
        "default_output_profile":config.default_output_profile.as_str(),
        "output_profiles":config.output_profiles.iter().map(|profile|(profile.name.as_str().to_owned(),profile_value(profile.profile)))
            .collect::<serde_json::Map<_,_>>(),
        "retry":{"max_attempts":config.retry.max_attempts,"retry_deadline_secs":config.retry.retry_deadline_secs,
            "backoff_secs":config.retry.backoff_secs},
        "accounting":{"mode":config.accounting.mode.label(),"price_file":config.accounting.price_file.as_ref().map(|value|value.as_str())},
        "quality":{"jpeg_quality":config.quality.jpeg_quality.value(),"provider_quality":config.quality.provider_quality.map(quality_value)},
    })
}

fn purpose_value(gate: &df_imagery::PurposeGate) -> Value {
    json!({"enabled":gate.enabled,"backend_override":gate.backend_override.map(|value|value.label()),
        "account_override":gate.account_override.as_ref().map(|value|value.as_str()),
        "model_override":gate.model_override.as_ref().map(|value|value.as_str()),
        "output_profile":gate.output_profile.as_ref().map(|value|value.as_str()),
        "quality_override":gate.quality_override.map(quality_value),
        "moderation_override":gate.moderation_override.map(|value| match value {
            df_imagery::ProviderModeration::Auto => "auto", df_imagery::ProviderModeration::Low => "low"})})
}

fn backend_value(gate: &df_imagery::BackendGate) -> Value {
    json!({"enabled":gate.enabled,"account":gate.account.as_str(),"max_concurrent_jobs":gate.max_concurrent_jobs,
        "model":gate.model.as_ref().map(|value|value.as_str()),
        "generation_model":gate.generation_model.as_ref().map(|value|value.as_str()),
        "edit_model":gate.edit_model.as_ref().map(|value|value.as_str()),
        "alpha_model":gate.alpha_model.as_ref().map(|value|value.as_str()),
        "endpoint":gate.endpoint.as_ref().map(|value|value.as_str())})
}

fn quality_value(quality: GenerationQuality) -> Value {
    match quality {
        GenerationQuality::Low => json!("low"),
        GenerationQuality::Medium => json!("medium"),
        GenerationQuality::High => json!("high"),
        GenerationQuality::Auto => json!("auto"),
        GenerationQuality::LegacyNumeric(value) => json!({"legacy_numeric":value.value()}),
    }
}

fn transform_value(config: &ImageConfig) -> Value {
    let value = config.transforms;
    json!({"enabled":value.enabled,"max_concurrency":value.max_concurrency,
        "max_decoded_bytes":value.max_decoded_bytes,"max_output_bytes":value.max_output_bytes,
        "queue_capacity":value.queue_capacity,"deadline_secs":value.deadline_secs})
}

fn scheduler_value(config: &ImageConfig) -> Value {
    let value = config.scheduler;
    json!({"foreground_limit":value.foreground_limit,"presence_limit":value.presence_limit,
        "background_limit":value.background_limit,"max_pending_jobs":value.max_pending_jobs,
        "max_pending_bytes":value.max_pending_bytes,"queue_age_secs":value.queue_age_secs,
        "job_deadline_secs":value.job_deadline_secs,"stage_deadline_secs":value.stage_deadline_secs})
}

fn analysis_route(route: Option<&df_imagery::AnalysisRoute>) -> Value {
    route
        .map(|route| {
            json!({"account":route.account.as_str(),"model":route.model.as_str(),
        "endpoint":route.endpoint.as_str(),"maximum_output_tokens":route.maximum_output_tokens})
        })
        .unwrap_or(Value::Null)
}

fn preparation(value: df_imagery::AnalysisPreparationLimits) -> Value {
    json!({"max_pixels":value.max_pixels,"max_picture_bytes":value.max_picture_bytes,"max_request_bytes":value.max_request_bytes})
}

fn analysis_value(config: &ImageConfig) -> Value {
    let value = &config.analysis;
    json!({"preparation":preparation(value.preparation),
        "classification":analysis_route(value.classification.as_ref()),"inspection":analysis_route(value.inspection.as_ref()),
        "head_locations":analysis_route(value.head_locations.as_ref()),"staging_identity":analysis_route(value.staging_identity.as_ref()),
        "staging_facing":analysis_route(value.staging_facing.as_ref()),
        "segmentation":value.segmentation.as_ref().map(|route|json!({"provider":match route.provider {df_imagery::SegmentationProvider::Fal=>"fal"},"account":route.account.as_str(),
            "model":route.model.as_str(),"endpoint":route.endpoint.as_str()})),
        "segmentation_preparation":preparation(value.segmentation_preparation)})
}

fn dimensions(value: df_imagery::Dimensions) -> Value {
    json!({"width_px":value.width_px(),"height_px":value.height_px()})
}

fn profile_value(profile: df_imagery::OutputProfile) -> Value {
    let delivered = profile.delivered();
    let alpha = match delivered.alpha() {
        df_imagery::AlphaHandling::Opaque => json!({"mode":"opaque"}),
        df_imagery::AlphaHandling::PreserveAlpha => json!({"mode":"preserve_alpha"}),
        df_imagery::AlphaHandling::Matte(color) => {
            json!({"mode":"matte","rgb":[color.red,color.green,color.blue]})
        }
    };
    let quality = match delivered.encoding() {
        df_imagery::Encoding::Jpeg { quality } => Some(quality.value()),
        df_imagery::Encoding::WebP { quality } => quality.map(|value| value.value()),
        df_imagery::Encoding::Png => None,
    };
    json!({"generated":profile.generated().map(dimensions),"delivered":{"dimensions":dimensions(delivered.dimensions()),
        "alpha":alpha,"encoding":{"format":delivered.encoding().format().label(),"quality":quality,
            "lossless":delivered.encoding().is_lossless()}}})
}

fn observer_value(value: df_imagery::telemetry::ObserverHealth) -> Value {
    use df_imagery::telemetry::ImageLogLevel;
    let level = match value.level {
        ImageLogLevel::Off => "off",
        ImageLogLevel::Error => "error",
        ImageLogLevel::Warn => "warn",
        ImageLogLevel::Info => "info",
        ImageLogLevel::Debug => "debug",
        ImageLogLevel::Trace => "trace",
    };
    json!({"level":level,"filtered":value.filtered,"admitted":value.admitted,"failures":value.failures})
}

fn health_value(health: &ImageHealth) -> Value {
    let sink = health.sink;
    let ingestion = &health.ingestion;
    json!({"status":"available", "observer":observer_value(health.observer),
        "runtime_observer":observer_value(health.runtime.2),
        "sink":{"attempted":sink.attempted,"admitted":sink.admitted,"confirmed":sink.confirmed,
            "refused":sink.refused,"receipt_failures":sink.receipt_failures,"pending":sink.pending,
            "last_reserved":sink.last_reserved,"last_confirmed":sink.last_confirmed},
        "ingestion":{"admitted":ingestion.admitted,"spooled":ingestion.spooled,"committed":ingestion.committed,
            "retries":ingestion.retries,"duplicates":ingestion.duplicates,"rejected":ingestion.rejected,
            "capacity":ingestion.capacity,"unconfirmed_gaps":ingestion.unconfirmed_gaps,
            "pending_items":ingestion.pending_items,"pending_bytes":ingestion.pending_bytes,
            "backlog_files":ingestion.backlog_files,"spool_bytes":ingestion.spool_bytes,
            "checkpoint":ingestion.checkpoint,"committed_watermark":ingestion.committed_watermark,
            "last_commit_millis":ingestion.last_commit_millis,"lag_millis":ingestion.lag_millis,
            "readiness":match ingestion.readiness {df_telemetry::Readiness::Healthy=>"healthy",
                df_telemetry::Readiness::Warning=>"warning",df_telemetry::Readiness::Degraded=>"degraded"},
            "emergency_records":ingestion.emergency_records}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> Mutex<ConfigControl> {
        let mut launch = RuntimePatch::empty();
        launch.mode = Some(df_imagery::GlobalMode::LocalOnly);
        Mutex::new(ConfigControl::build("", &[], &launch, 7).unwrap())
    }

    #[test]
    fn stale_and_invalid_runtime_patches_preserve_the_real_owner() {
        let control = control();
        let before = control.lock().unwrap().read_effective();
        let patch = BTreeMap::from([("mode".to_owned(), json!("off"))]);
        assert_eq!(
            apply_values(&control, 6, &patch).unwrap_err().0,
            StatusCode::CONFLICT
        );
        assert_eq!(control.lock().unwrap().read_effective(), before);
        let invalid = BTreeMap::from([
            ("mode".to_owned(), json!("off")),
            ("scheduler.max_pending_jobs".to_owned(), json!(0)),
        ]);
        assert_eq!(
            apply_values(&control, 7, &invalid).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(control.lock().unwrap().read_effective(), before);
        let secret = BTreeMap::from([(
            "private-token-sentinel".to_owned(),
            json!("private-value-sentinel"),
        )]);
        let error = apply_values(&control, 7, &secret)
            .unwrap_err()
            .1
            .0
            .to_string();
        assert!(!error.contains("private-token-sentinel"));
        assert!(!error.contains("private-value-sentinel"));
        assert_eq!(control.lock().unwrap().read_effective(), before);
    }

    #[test]
    fn local_commit_preserves_provenance_and_does_not_invent_runtime_acknowledgement() {
        let control = control();
        let patch = BTreeMap::from([("mode".to_owned(), json!("off"))]);
        assert_eq!(apply_values(&control, 7, &patch).unwrap(), 8);
        let snapshot = control.lock().unwrap().read_effective();
        assert_eq!(
            snapshot.provenance.source_of("mode"),
            Some(OverlaySource::Runtime)
        );
        let state = propagation(snapshot.provenance.revision, Some(7), Some(true));
        assert_eq!(state["desired_revision"], 8);
        assert_eq!(state["applied_revision"], 7);
        assert_eq!(state["propagation"], "pending");
        assert_eq!(state["acknowledged"], false);
        assert_eq!(effective(&snapshot.config)["mode"], "off");
        assert_eq!(propagation(8, Some(8), Some(false))["acknowledged"], false);
        assert_eq!(propagation(8, None, None)["applied_revision"], Value::Null);
        assert_eq!(control.lock().unwrap().reset(8).unwrap().revision, 9);
        let restored = control.lock().unwrap().read_effective();
        assert_eq!(effective(&restored.config)["mode"], "local-only");
        assert_eq!(
            restored.provenance.source_of("mode"),
            Some(OverlaySource::Launch)
        );
    }

    #[test]
    fn structured_effective_view_keeps_profiles_route_selections_and_unknown_health() {
        let control = control();
        let snapshot = control.lock().unwrap().read_effective();
        let view = state_value(&snapshot, None);
        assert!(view["configuration"]["effective"]["backends"].is_object());
        assert!(view["configuration"]["effective"]["output_profiles"].is_object());
        assert!(view["configuration"]["provenance"]["fields"].is_array());
        assert_eq!(view["runtime"]["propagation"], "unobserved");
        assert_eq!(view["telemetry"]["status"], "unavailable");
        assert_eq!(unmounted()["configured"], false);
        let mode = view["configuration"]["effective"]["mode"].clone();
        assert_eq!(
            apply_values(&control, 7, &BTreeMap::from([("mode".to_owned(), mode)])).unwrap(),
            8
        );
    }

    #[test]
    fn health_view_preserves_disabled_capture_loss_receipts_and_real_ingestion_lag() {
        let observer = df_imagery::telemetry::ObserverHealth {
            level: df_imagery::telemetry::ImageLogLevel::Off,
            filtered: 41,
            admitted: 7,
            failures: 3,
        };
        let health = ImageHealth {
            runtime: (6, true, observer),
            observer,
            sink: df_observe::DiagnosticHealth {
                attempted: 7,
                admitted: 5,
                confirmed: 4,
                refused: 2,
                receipt_failures: 1,
                pending: 1,
                last_reserved: 7,
                last_confirmed: 6,
            },
            ingestion: df_telemetry::IngestionHealth {
                admitted: 5,
                spooled: 4,
                committed: 3,
                retries: 2,
                duplicates: 1,
                rejected: 2,
                capacity: 1,
                unconfirmed_gaps: 2,
                pending_items: 1,
                pending_bytes: 88,
                backlog_files: 1,
                spool_bytes: 99,
                checkpoint: 11,
                committed_watermark: 12,
                last_commit_millis: Some(13),
                lag_millis: 123_456,
                readiness: df_telemetry::Readiness::Degraded,
                emergency_records: 1,
            },
        };
        let snapshot = control().lock().unwrap().read_effective();
        let view = state_value(&snapshot, Some(&health));
        assert_eq!(view["runtime"]["desired_revision"], 7);
        assert_eq!(view["runtime"]["applied_revision"], 6);
        assert_eq!(view["runtime"]["acknowledged"], false);
        assert_eq!(view["telemetry"]["observer"]["level"], "off");
        assert_eq!(view["telemetry"]["observer"]["filtered"], 41);
        assert_eq!(view["telemetry"]["sink"]["admitted"], 5);
        assert_eq!(view["telemetry"]["sink"]["confirmed"], 4);
        assert_eq!(view["telemetry"]["sink"]["receipt_failures"], 1);
        assert_eq!(view["telemetry"]["ingestion"]["committed"], 3);
        assert_eq!(view["telemetry"]["ingestion"]["lag_millis"], 123_456);
        assert_eq!(view["telemetry"]["ingestion"]["readiness"], "degraded");
    }
}
