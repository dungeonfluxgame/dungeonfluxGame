//! Test-support transport captures genuine loopback responses with production raw retention and
//! usage policy.
use super::*;
use crate::cost::{CostRoute, UsageRevision};

/// Actual retained stage facts and source bytes for one loopback-only analytical exchange.
pub struct StandaloneUsageCapture<'a> {
    pub job: &'a ImageJob,
    pub stage: &'a StageView,
    pub dispatch: DispatchId,
    pub source: &'a [u8],
    pub credential: &'a str,
    pub route: &'a CostRoute,
    pub revision: UsageRevision,
    pub at_ms: u64,
    pub timeout: Duration,
    pub maximum_response_bytes: usize,
}

/// Executes the real prepared head-location transport; never creates a proof, journal row or
/// settlement.
pub async fn capture_standalone_usage(
    input: StandaloneUsageCapture<'_>,
) -> Result<RuntimeUsage, RuntimeError> {
    let StandaloneUsageCapture {
        job,
        stage,
        dispatch,
        source,
        credential,
        route,
        revision,
        at_ms,
        timeout,
        maximum_response_bytes,
    } = input;
    if stage.owner != job.owner
        || stage.job != job.job
        || stage.kind != StageKind::HeadLocations
        || stage.planned_publication.is_some()
        || revision.0 == 0
        || timeout.is_zero()
        || maximum_response_bytes == 0
        || maximum_response_bytes > 16_777_216
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let intent = analysis_codec::decode_analysis(job.intent.bytes())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let config = super::retained_config(job)?;
    let ImageAnalysisOperation::HeadLocations(spec) = &intent.operation else {
        return Err(RuntimeError::UnsupportedCapability);
    };
    if spec.face.is_some() || intent.owner != job.owner {
        return Err(RuntimeError::InvalidRecord);
    }
    let selected = chat_route(&config, &intent.operation)?;
    let endpoint = url::Url::parse(selected.endpoint.as_str())
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
    let loopback = endpoint
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if !loopback
        || selected.model.as_str() != route.model
        || source.len() as u64 > config.transforms.max_decoded_bytes
    {
        return Err(RuntimeError::InvalidConfiguration);
    }
    let operation = intent.operation.clone();
    let source = source.to_vec();
    let credential = credential.to_owned();
    let selected = selected.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        backend::PreparedStandaloneChat::prepare(
            backend::StandaloneChatInput {
                operation: &operation,
                source: &source,
                face: None,
                references: &[],
                segmentation: None,
                locations: None,
                identity: None,
                route: &selected,
                preparation: &config.analysis.preparation,
                max_decoded_bytes: config.transforms.max_decoded_bytes,
                timeout,
                maximum_response_bytes,
            },
            &credential,
        )
    })
    .await
    .map_err(|_| RuntimeError::WorkerFailed)??;
    let response = match prepared.execute().await {
        Ok(response) => RawAnalysis {
            response: AnalysisResponse::Http {
                status: response.status,
                body: response.body,
            },
            segmentation: None,
        },
        Err(RuntimeError::Expired) => RawAnalysis {
            response: AnalysisResponse::Transport(AnalysisTransportFailure::Deadline),
            segmentation: None,
        },
        Err(RuntimeError::ProviderUnknown) => RawAnalysis {
            response: AnalysisResponse::Transport(AnalysisTransportFailure::Transport),
            segmentation: None,
        },
        Err(error) => return Err(error),
    };
    let (evidence, units) = analysis::parse_response(response.body(), maximum_response_bytes)
        .map(|value| analysis::reported_usage(&value))
        .unwrap_or((UsageEvidence::Unknown, Vec::new()));
    let units = if evidence == UsageEvidence::Unknown {
        route.maxima.clone()
    } else {
        units
    };
    let dependencies = Dependencies::default();
    let raw_output = encode_raw(&response, &dependencies, None, 0, maximum_response_bytes)?;
    let usable = interpret(
        &response,
        &intent,
        &dependencies,
        None,
        0,
        maximum_response_bytes,
    )
    .is_ok();
    Ok(RuntimeUsage {
        owner: job.owner.clone(),
        job: job.job,
        stage: stage.stage,
        dispatch,
        revision,
        route: route.clone(),
        evidence,
        units,
        usable,
        at_ms,
        raw_output: Some(raw_output),
    })
}
