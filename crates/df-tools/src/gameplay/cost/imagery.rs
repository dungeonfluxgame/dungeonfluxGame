//! The tools-side adapter implementing the neutral imagery cost projection over the spend
//! ledger's book. Neutral `df-imagery` vocabulary in, book calls out; no finance or settlement
//! logic lives here.
//!
//! Recovery contract the adapter relies on: the dispatch journal rereads its row and replays
//! start then correction under the stable call id, so a crash between the supplier report and
//! the book write corrects instead of double-counting.

use df_imagery::cost::{
    CostError, CostFuture, CostRoute, CostUnits, ImageCostProjection, PinnedCorrection,
    PinnedStart, PinnedUnsent, UsageEvidence, UsageReport, UsageRevision,
};

use super::PinnedError;
use super::port::{MeteredSource, MeteredUnits, MeteringPort, PortRoute};

/// Maximum queued cost projection commands; overflow leaves raw journal usage for replay.
const COMMAND_CAPACITY: usize = 32;
/// Fixed length of the canonical dispatch-derived identity.
const PINNED_CALL_ID_BYTES: usize = super::PINNED_CALL_PREFIX.len() + 32;
/// Bounds owned route strings before queue admission.
const MAXIMUM_ROUTE_WORD_BYTES: usize = 120;
/// The longest unit vocabulary entry fits within this bound.
const MAXIMUM_CATEGORY_BYTES: usize = 32;

/// Existing cost-book attribution using actual prepared request sizes from the runtime.
pub struct GameplayImageCostPolicy;

impl df_imagery::runtime::ImageRuntimeCostPolicy for GameplayImageCostPolicy {
    fn route(
        &self,
        job: &df_imagery::ImageJob,
        config: &df_imagery::ImageConfig,
        backend: df_imagery::BackendId,
        model: &str,
    ) -> Result<CostRoute, df_imagery::runtime::RuntimeError> {
        let mut basis = df_imagery::runtime::retained_cost_basis(
            job,
            config,
            df_imagery::execution::StageKind::Generation,
        )?;
        basis.backend = Some(backend);
        basis.model = df_imagery::ModelName::new(model)
            .map_err(|_| df_imagery::runtime::RuntimeError::InvalidConfiguration)?;
        cost_route(&basis)
    }

    fn stage_route(
        &self,
        _job: &df_imagery::ImageJob,
        _config: &df_imagery::ImageConfig,
        basis: &df_imagery::runtime::RuntimeCostBasis,
    ) -> Result<CostRoute, df_imagery::runtime::RuntimeError> {
        cost_route(basis)
    }
}

fn cost_route(
    basis: &df_imagery::runtime::RuntimeCostBasis,
) -> Result<CostRoute, df_imagery::runtime::RuntimeError> {
    use super::{Category, Cause, Provider};
    use df_imagery::execution::StageKind;
    use df_imagery::runtime::RuntimeError;
    use df_imagery::{BackendId, GenerationQuality};
    let text = basis.prompt_bytes.div_ceil(2);
    let references = basis
        .reference_count
        .checked_mul(super::IMAGE_INPUT_TOKENS_PER_REFERENCE)
        .ok_or(RuntimeError::InvalidRecord)?;
    let (provider, cause, detail, maxima) = match basis.kind {
        StageKind::Classification => {
            if basis.analysis.is_none() {
                return Err(RuntimeError::InvalidRecord);
            }
            (
                Provider::TypeSafe,
                Cause::Judgement,
                "creator_classification".to_owned(),
                vec![(Category::TextInput, text)],
            )
        }
        StageKind::Inspection
        | StageKind::PhotoInspection
        | StageKind::HeadLocations
        | StageKind::StagingIdentity
        | StageKind::StagingFacing => {
            if basis.analysis.is_none() {
                return Err(RuntimeError::InvalidRecord);
            }
            let output = basis
                .maximum_output_tokens
                .or_else(|| {
                    basis
                        .analysis
                        .as_ref()
                        .map(|route| route.maximum_output_tokens)
                })
                .filter(|maximum| *maximum > 0)
                .ok_or(RuntimeError::InvalidRecord)?;
            (
                Provider::OpenAi,
                if basis.kind == StageKind::HeadLocations {
                    Cause::HeadLocation
                } else {
                    Cause::PictureJudgement
                },
                match basis.kind {
                    StageKind::HeadLocations => "head_locator",
                    StageKind::StagingIdentity => "staging_identity",
                    StageKind::StagingFacing => "staging_facing",
                    _ => "photo_check",
                }
                .to_owned(),
                vec![
                    (Category::TextInput, text),
                    (Category::ImageInput, references),
                    (Category::TextOutput, output),
                ],
            )
        }
        StageKind::Segmentation => {
            if basis.backend != Some(BackendId::Fal) || basis.analysis.is_some() {
                return Err(RuntimeError::InvalidRecord);
            }
            (
                Provider::Fal,
                Cause::Segmentation,
                "segmentation".to_owned(),
                vec![(Category::Calls, 1)],
            )
        }
        StageKind::LocalTransform => return Err(RuntimeError::UnsupportedCapability),
        _ => {
            let cause = image_cause(basis)?;
            let detail = format!("{}/{}", basis.purpose.purpose.label(), basis.kind.label());
            let (provider, maxima) = match basis.backend.ok_or(RuntimeError::InvalidRecord)? {
                BackendId::OpenAi => {
                    let dimensions = basis.generated.ok_or(RuntimeError::InvalidRecord)?;
                    let square_tokens = match basis.generation_quality {
                        Some(GenerationQuality::Low) => 272,
                        Some(GenerationQuality::High | GenerationQuality::Auto) => 4160,
                        None | Some(GenerationQuality::Medium) => 1056,
                        Some(GenerationQuality::LegacyNumeric(_)) => {
                            return Err(RuntimeError::UnsupportedCapability);
                        }
                    };
                    let output = dimensions
                        .pixel_count()
                        .checked_mul(square_tokens)
                        .ok_or(RuntimeError::InvalidRecord)?
                        .div_ceil(1024 * 1024);
                    (
                        Provider::OpenAi,
                        vec![
                            (Category::TextInput, text),
                            (Category::ImageInput, references),
                            (Category::ImageOutput, output),
                        ],
                    )
                }
                BackendId::Fal => (Provider::Fal, vec![(Category::Images, 1)]),
                BackendId::Bfl => (Provider::Bfl, vec![(Category::Calls, 1)]),
                // Subscription usage remains unpriced when no supplier usage/tariff is known.
                BackendId::CodexCli => (Provider::CodexSubscription, vec![(Category::Calls, 1)]),
            };
            (provider, cause, detail, maxima)
        }
    };
    Ok(CostRoute {
        provider: provider.label().to_owned(),
        model: basis.model.as_str().to_owned(),
        cause: cause.label().to_owned(),
        detail,
        maxima: maxima
            .into_iter()
            .map(|(category, quantity)| CostUnits {
                category: category.label().to_owned(),
                quantity,
            })
            .collect(),
    })
}

fn image_cause(
    basis: &df_imagery::runtime::RuntimeCostBasis,
) -> Result<super::Cause, df_imagery::runtime::RuntimeError> {
    use super::Cause;
    use df_imagery::{Purpose, runtime::RuntimeError};
    Ok(match basis.purpose.purpose {
        Purpose::CreatorPortrait | Purpose::CanonicalPerson => Cause::CharacterPortrait,
        Purpose::CanonicalRoom => Cause::PlacePortrait,
        Purpose::ScenePlate
            if basis.template_version
                == crate::gameplay::imagery::SCENE_REFRESH_TEMPLATE_REVISION =>
        {
            Cause::SceneRefresh
        }
        Purpose::ScenePlate | Purpose::PartyMural | Purpose::CombatStill => Cause::ScenePlate,
        Purpose::SceneEdit => Cause::PlateEdit,
        Purpose::Expression => Cause::CastExpression,
        Purpose::ItemCloseUp => Cause::ItemCloseUp,
        Purpose::AnimationFrame => Cause::ClipFrame,
        Purpose::ReferencePrep | Purpose::ImportedDerivative => {
            return Err(RuntimeError::UnsupportedCapability);
        }
    })
}

type Work = Box<dyn FnOnce(&MeteringPort) + Send>;

/// Projects imagery cost on one owned storage worker. Construct and drop outside the session
/// owner; drop drains the bounded queue and joins the worker.
pub struct ImageryCostAdapter {
    #[cfg(test)]
    port: MeteringPort,
    sender: Option<std::sync::mpsc::SyncSender<Work>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ImageryCostAdapter {
    /// Starts the bounded worker over the game's existing book.
    pub fn new(port: MeteringPort) -> std::io::Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Work>(COMMAND_CAPACITY);
        #[cfg(test)]
        let test_port = port.clone();
        let worker = std::thread::Builder::new()
            .name("imagery-cost".to_owned())
            .spawn(move || {
                for work in receiver {
                    work(&port);
                }
            })?;
        Ok(Self {
            #[cfg(test)]
            port: test_port,
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    async fn execute<T: Send + 'static>(
        &self,
        context: df_observe::AuditContext,
        work: impl FnOnce(&MeteringPort) -> Result<T, CostError> + Send + 'static,
    ) -> Result<T, CostError> {
        let (done, wait) = tokio::sync::oneshot::channel();
        let command = Box::new(move |port: &MeteringPort| {
            let _scope = df_observe::audit_scope(context);
            // Cancellation discards only the response; journal replay repairs the projection.
            let _cancelled = done.send(work(port));
        });
        self.sender
            .as_ref()
            .ok_or(CostError::ProjectionFull)?
            .try_send(command)
            .map_err(|_| CostError::ProjectionFull)?;
        wait.await.map_err(|_| CostError::ProjectionFull)?
    }

    #[cfg(test)]
    fn start_pinned(&self, id: &str, route: &CostRoute) -> Result<PinnedStart, CostError> {
        self.port
            .start_pinned(id, &port_route(route))
            .map(map_start)
            .map_err(map_error)
    }

    #[cfg(test)]
    fn correct(&self, id: &str, report: &UsageReport<'_>) -> Result<PinnedCorrection, CostError> {
        self.port
            .correct_pinned_at(
                id,
                report.revision.0,
                port_source(report.evidence),
                &port_units(report.units),
                report.usable,
                report.at_ms,
            )
            .map(map_correction)
            .map_err(map_error)
    }

    #[cfg(test)]
    fn record_unsent(&self, id: &str) -> Result<PinnedUnsent, CostError> {
        self.port
            .record_pinned_unsent(id)
            .map(map_unsent)
            .map_err(map_error)
    }
}

impl Drop for ImageryCostAdapter {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let _joined = worker.join();
        }
    }
}

fn bounded_route(route: &CostRoute) -> bool {
    route.provider.len() <= MAXIMUM_ROUTE_WORD_BYTES
        && route.model.len() <= MAXIMUM_ROUTE_WORD_BYTES
        && route.cause.len() <= MAXIMUM_ROUTE_WORD_BYTES
        && route.detail.len() <= MAXIMUM_ROUTE_WORD_BYTES
        && bounded_units(&route.maxima)
}

fn bounded_units(units: &[CostUnits]) -> bool {
    units.len() <= super::Category::ALL.len()
        && units
            .iter()
            .all(|unit| unit.category.len() <= MAXIMUM_CATEGORY_BYTES)
}

fn port_route(route: &CostRoute) -> PortRoute {
    PortRoute {
        provider: route.provider.clone(),
        model: route.model.clone(),
        cause: route.cause.clone(),
        detail: route.detail.clone(),
        maxima: port_units(&route.maxima),
    }
}

fn port_units(units: &[CostUnits]) -> Vec<MeteredUnits> {
    units
        .iter()
        .map(|unit| MeteredUnits {
            category: unit.category.clone(),
            quantity: unit.quantity,
        })
        .collect()
}

fn port_source(evidence: UsageEvidence) -> MeteredSource {
    match evidence {
        UsageEvidence::Reported => MeteredSource::Reported,
        UsageEvidence::Estimated => MeteredSource::Estimated,
        UsageEvidence::Unknown => MeteredSource::Unknown,
    }
}

fn map_start(outcome: super::PinnedStart) -> PinnedStart {
    match outcome {
        super::PinnedStart::Opened => PinnedStart::Opened,
        super::PinnedStart::ReplayedOpen => PinnedStart::ReplayedOpen,
        super::PinnedStart::AlreadySettled => PinnedStart::AlreadySettled,
    }
}

fn map_correction(outcome: super::PinnedCorrection) -> PinnedCorrection {
    match outcome {
        super::PinnedCorrection::Applied => PinnedCorrection::Applied,
        super::PinnedCorrection::Replayed => PinnedCorrection::Replayed,
    }
}

fn map_unsent(outcome: super::PinnedUnsent) -> PinnedUnsent {
    match outcome {
        super::PinnedUnsent::Recorded => PinnedUnsent::Recorded,
        super::PinnedUnsent::Replayed => PinnedUnsent::Replayed,
    }
}

fn map_error(error: PinnedError) -> CostError {
    match error {
        PinnedError::UnknownProvider => CostError::UnknownProvider,
        PinnedError::UnknownCause => CostError::UnknownCause,
        PinnedError::UnknownCategory => CostError::UnknownCategory,
        PinnedError::UnknownCall => CostError::UnknownCall,
        PinnedError::RouteConflict => CostError::RouteConflict,
        PinnedError::StaleRevision { stored, got } => CostError::StaleRevision {
            stored: UsageRevision(stored),
            got: UsageRevision(got),
        },
        PinnedError::RevisionConflict { revision } => CostError::RevisionConflict {
            revision: UsageRevision(revision),
        },
        PinnedError::AlreadySettled => CostError::AlreadySettled,
        PinnedError::BookNotOpen => CostError::BookNotOpen,
        PinnedError::ProjectionFull => CostError::ProjectionFull,
        PinnedError::PriceBasisUnavailable => CostError::PriceBasisUnavailable,
        PinnedError::CorruptLog => CostError::CorruptLog,
    }
}

impl ImageCostProjection for ImageryCostAdapter {
    fn start_pinned<'a>(
        &'a self,
        call_id: &'a str,
        route: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            if !bounded_route(route) {
                return Err(CostError::RouteConflict);
            }
            let id = call_id.to_owned();
            let route = port_route(route);
            self.execute(context, move |port| {
                port.start_pinned(&id, &route)
                    .map(map_start)
                    .map_err(map_error)
            })
            .await
        })
    }

    fn correct<'a>(
        &'a self,
        call_id: &'a str,
        report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            if !bounded_units(report.units) {
                return Err(CostError::UnknownCategory);
            }
            let id = call_id.to_owned();
            let units = port_units(report.units);
            let (revision, source, usable, at_ms) = (
                report.revision.0,
                port_source(report.evidence),
                report.usable,
                report.at_ms,
            );
            self.execute(context, move |port| {
                port.correct_pinned_at(&id, revision, source, &units, usable, at_ms)
                    .map(map_correction)
                    .map_err(map_error)
            })
            .await
        })
    }

    fn record_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            let id = call_id.to_owned();
            self.execute(context, move |port| {
                port.record_pinned_unsent(&id)
                    .map(map_unsent)
                    .map_err(map_error)
            })
            .await
        })
    }

    fn seal_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<df_imagery::cost::PinnedUnsentSeal, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            let id = call_id.to_owned();
            self.execute(context, move |port| {
                port.seal_pinned_unsent(&id).map_err(map_error)
            })
            .await
        })
    }

    fn read_receipt<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<Option<df_imagery::cost::PinnedCostReceipt>, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            let id = call_id.to_owned();
            self.execute(context, move |port| {
                port.read_pinned_receipt(&id).map_err(map_error)
            })
            .await
        })
    }

    fn is_durable<'a>(
        &'a self,
        call_id: &'a str,
        revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        let context = df_observe::current_audit_context();
        Box::pin(async move {
            if call_id.len() != PINNED_CALL_ID_BYTES {
                return Err(CostError::UnknownCall);
            }
            let id = call_id.to_owned();
            self.execute(context, move |port| {
                port.pinned_durable(&id, revision.map(|value| value.0))
                    .map_err(map_error)
            })
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRICES: &str = r#"{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [
{"provider": "openai", "model": "gpt-image-2.5-flare", "category": "text_input", "usd": "5", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
{"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_input", "usd": "8", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
{"provider": "openai", "model": "gpt-image-2.5-flare", "category": "image_output", "usd": "30", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"}]}"#;

    fn route() -> CostRoute {
        CostRoute {
            provider: "openai".to_owned(),
            model: "gpt-image-2.5-flare".to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "adapter".to_owned(),
            maxima: vec![
                CostUnits {
                    category: "text_input".to_owned(),
                    quantity: 2_000_000,
                },
                CostUnits {
                    category: "image_output".to_owned(),
                    quantity: 1_000_000,
                },
            ],
        }
    }

    fn actual_basis() -> df_imagery::runtime::RuntimeCostBasis {
        df_imagery::runtime::RuntimeCostBasis {
            purpose: df_imagery::PurposedUse {
                purpose: df_imagery::Purpose::ScenePlate,
                role: df_imagery::VariantRole::Primary,
            },
            kind: df_imagery::execution::StageKind::Generation,
            backend: Some(df_imagery::BackendId::OpenAi),
            model: df_imagery::ModelName::new("configured-image-model").unwrap(),
            template_version: 0xDF01_0006,
            prompt_bytes: 101,
            reference_count: 2,
            generated: Some(df_imagery::Dimensions::new(1024, 1024).unwrap()),
            generation_quality: Some(df_imagery::GenerationQuality::Low),
            analysis: None,
            maximum_output_tokens: None,
        }
    }

    #[test]
    fn actual_request_sizes_define_the_same_openai_maxima_as_the_old_image_meter() {
        let route = cost_route(&actual_basis()).unwrap();
        assert_eq!(route.provider, "openai");
        assert_eq!(route.cause, "scene_plate");
        assert_eq!(
            route.maxima,
            vec![
                CostUnits {
                    category: "text_input".into(),
                    quantity: 51
                },
                CostUnits {
                    category: "image_input".into(),
                    quantity: 3072
                },
                CostUnits {
                    category: "image_output".into(),
                    quantity: 272
                }
            ]
        );
        let mut refresh = actual_basis();
        refresh.template_version = crate::gameplay::imagery::SCENE_REFRESH_TEMPLATE_REVISION;
        assert_eq!(cost_route(&refresh).unwrap().cause, "scene_refresh");
    }

    #[test]
    fn classification_uses_its_own_jev_tariff_and_actual_input_not_an_image_price() {
        let mut basis = actual_basis();
        basis.kind = df_imagery::execution::StageKind::Classification;
        basis.backend = None;
        basis.analysis = Some(df_imagery::AnalysisRoute {
            account: df_imagery::AccountHandle::new("creator-jev").unwrap(),
            model: df_imagery::ModelName::new("jev-latest").unwrap(),
            endpoint: df_imagery::ApiEndpoint::new("http://127.0.0.1:55499").unwrap(),
            maximum_output_tokens: 1,
        });
        basis.model = basis.analysis.as_ref().unwrap().model.clone();
        let route = cost_route(&basis).unwrap();
        assert_eq!(
            (
                route.provider.as_str(),
                route.model.as_str(),
                route.cause.as_str()
            ),
            ("typesafe", "jev-latest", "judgement")
        );
        assert_eq!(
            route.maxima,
            vec![CostUnits {
                category: "text_input".into(),
                quantity: 51
            }]
        );
        basis.kind = df_imagery::execution::StageKind::Inspection;
        basis.maximum_output_tokens = Some(3000);
        assert_eq!(
            cost_route(&basis).unwrap().maxima.last().unwrap().quantity,
            3000
        );
    }

    #[tokio::test]
    async fn unified_segmentation_uses_its_actual_model_and_one_call_maximum() {
        let mut basis = actual_basis();
        basis.kind = df_imagery::execution::StageKind::Segmentation;
        basis.backend = Some(df_imagery::BackendId::Fal);
        basis.model = df_imagery::ModelName::new("fal-ai/sam-3-1/image-rle").unwrap();
        let route = cost_route(&basis).unwrap();
        assert_eq!(
            (
                route.provider.as_str(),
                route.model.as_str(),
                route.cause.as_str(),
                route.detail.as_str()
            ),
            (
                "fal",
                "fal-ai/sam-3-1/image-rle",
                "segmentation",
                "segmentation"
            )
        );
        assert_eq!(
            route.maxima,
            vec![CostUnits {
                category: "calls".to_owned(),
                quantity: 1
            }]
        );
        let port = MeteringPort::new();
        port.open(
            "segmentation-route",
            include_str!("../../../../../content/pricing.json"),
        )
        .unwrap();
        let adapter = ImageryCostAdapter::new(port).unwrap();
        let id = df_imagery::cost::pinned_call_id(&[0x53; 16]);
        ImageCostProjection::start_pinned(&adapter, &id, &route)
            .await
            .unwrap();
        let unknown = UsageReport {
            revision: UsageRevision(1),
            evidence: UsageEvidence::Unknown,
            units: &[],
            usable: false,
            at_ms: 1_000,
        };
        ImageCostProjection::correct(&adapter, &id, &unknown)
            .await
            .unwrap();
        assert_eq!(adapter.port.totals().calls, 1);
        assert_eq!(adapter.port.totals().micro_usd, 10_000);
        let receipt = ImageCostProjection::read_receipt(&adapter, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.route, route);
        assert_eq!(receipt.units, route.maxima);
        assert_eq!(receipt.evidence, UsageEvidence::Unknown);
        let reported = UsageReport {
            revision: UsageRevision(2),
            evidence: UsageEvidence::Reported,
            units: &route.maxima,
            usable: false,
            at_ms: 2_000,
        };
        ImageCostProjection::correct(&adapter, &id, &reported)
            .await
            .unwrap();
        assert_eq!(
            ImageCostProjection::correct(&adapter, &id, &reported)
                .await
                .unwrap(),
            PinnedCorrection::Replayed
        );
        assert_eq!(adapter.port.totals().calls, 1);
        assert_eq!(adapter.port.totals().micro_usd, 10_000);
        basis.backend = None;
        assert_eq!(
            cost_route(&basis),
            Err(df_imagery::runtime::RuntimeError::InvalidRecord)
        );
    }

    #[test]
    fn fal_alpha_extraction_is_its_own_image_call_and_local_transforms_are_not_supplier_calls() {
        let mut basis = actual_basis();
        basis.kind = df_imagery::execution::StageKind::Extraction;
        basis.backend = Some(df_imagery::BackendId::Fal);
        basis.model = df_imagery::ModelName::new("fal-ai/birefnet").unwrap();
        let route = cost_route(&basis).unwrap();
        assert_eq!(route.provider, "fal");
        assert_eq!(
            route.maxima,
            vec![CostUnits {
                category: "images".into(),
                quantity: 1
            }]
        );
        basis.kind = df_imagery::execution::StageKind::LocalTransform;
        basis.backend = None;
        assert_eq!(
            cost_route(&basis),
            Err(df_imagery::runtime::RuntimeError::UnsupportedCapability)
        );
    }

    fn adapter() -> ImageryCostAdapter {
        let port = MeteringPort::new();
        port.open("imagery-adapter", PRICES).expect("opens");
        ImageryCostAdapter::new(port).expect("storage worker")
    }

    #[test]
    fn the_neutral_call_id_format_matches_the_book_prefix() {
        let id = df_imagery::cost::pinned_call_id(&[7; 16]);
        assert!(id.starts_with(super::super::PINNED_CALL_PREFIX));
        assert!(id.starts_with(df_imagery::cost::PINNED_CALL_PREFIX));
    }

    #[test]
    fn the_adapter_opens_one_stable_call_and_names_unknown_vocabulary() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&[7; 16]);
        assert_eq!(call_id, df_imagery::cost::pinned_call_id(&[7; 16]));
        assert_eq!(
            adapter.start_pinned(&call_id, &route()),
            Ok(PinnedStart::Opened)
        );
        assert_eq!(
            adapter.start_pinned(&call_id, &route()),
            Ok(PinnedStart::ReplayedOpen)
        );
        let mut bad = route();
        bad.provider = "acme".to_owned();
        assert_eq!(
            adapter.start_pinned("img-other", &bad),
            Err(CostError::UnknownProvider)
        );
        bad = route();
        bad.cause = "sorcery".to_owned();
        assert_eq!(
            adapter.start_pinned("img-other", &bad),
            Err(CostError::UnknownCause)
        );
        bad = route();
        bad.maxima[0].category = "tokens".to_owned();
        assert_eq!(
            adapter.start_pinned("img-other", &bad),
            Err(CostError::UnknownCategory)
        );
    }

    #[test]
    fn the_adapter_projects_revisions_onto_the_same_book_call() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&[9; 16]);
        adapter.start_pinned(&call_id, &route()).expect("opens");
        let unknown = UsageReport {
            revision: UsageRevision(0),
            evidence: UsageEvidence::Unknown,
            units: &[],
            usable: true,
            at_ms: 1_000,
        };
        assert_eq!(
            adapter.correct(&call_id, &unknown),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().calls, 1);
        // Maxima at the test rates: 2M text tokens at 5 plus 1M output tokens at 30.
        assert_eq!(adapter.port.totals().micro_usd, 40_000_000);
        let reported = UsageReport {
            revision: UsageRevision(1),
            evidence: UsageEvidence::Reported,
            units: &[CostUnits {
                category: "image_output".to_owned(),
                quantity: 2_000_000,
            }],
            usable: true,
            at_ms: 2_000,
        };
        assert_eq!(
            adapter.correct(&call_id, &reported),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().calls, 1);
        assert_eq!(adapter.port.totals().micro_usd, 60_000_000);
        let recent = adapter.port.recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].call_id, call_id);
        assert!(recent[0].correction);
        assert_eq!(recent[0].at_ms, 2_000);
        assert_eq!(
            adapter.correct(&call_id, &unknown),
            Err(CostError::StaleRevision {
                stored: UsageRevision(1),
                got: UsageRevision(0),
            })
        );
        assert_eq!(
            adapter.port.start_pinned(&call_id, &port_route(&route())),
            Ok(super::super::PinnedStart::AlreadySettled)
        );
    }

    #[test]
    fn the_adapter_marks_proven_unsent_without_a_record() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&[11; 16]);
        adapter.start_pinned(&call_id, &route()).expect("opens");
        assert_eq!(adapter.record_unsent(&call_id), Ok(PinnedUnsent::Recorded));
        assert_eq!(adapter.record_unsent(&call_id), Ok(PinnedUnsent::Replayed));
        assert_eq!(adapter.port.totals().calls, 0);
        let reported = UsageReport {
            revision: UsageRevision(0),
            evidence: UsageEvidence::Reported,
            units: &[],
            usable: true,
            at_ms: 1_000,
        };
        assert_eq!(
            adapter.correct(&call_id, &reported),
            Err(CostError::AlreadySettled)
        );
    }

    #[test]
    fn the_adapter_needs_an_open_book() {
        let adapter = ImageryCostAdapter::new(MeteringPort::new()).expect("worker");
        assert_eq!(
            adapter.start_pinned("img-closed", &route()),
            Err(CostError::BookNotOpen)
        );
    }

    fn dispatch_bytes(n: u64) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&n.to_be_bytes());
        bytes
    }

    fn settle_reported(
        adapter: &ImageryCostAdapter,
        call_id: &str,
        revision: u64,
        units: &[CostUnits],
        at_ms: u64,
    ) -> Result<PinnedCorrection, CostError> {
        adapter.correct(
            call_id,
            &UsageReport {
                revision: UsageRevision(revision),
                evidence: UsageEvidence::Reported,
                units,
                usable: true,
                at_ms,
            },
        )
    }

    #[test]
    fn the_adapter_refuses_a_new_identity_past_the_retention_window() {
        let adapter = adapter();
        for n in 0..8u64 {
            let call_id = adapter.call_id_for(&dispatch_bytes(400 + n));
            adapter.start_pinned(&call_id, &route()).expect("opens");
            settle_reported(
                &adapter,
                &call_id,
                0,
                &[CostUnits {
                    category: "image_output".to_owned(),
                    quantity: 1_000_000,
                }],
                1_000,
            )
            .expect("settles");
        }
        assert_eq!(adapter.port.totals().calls, 8);
        let retired = adapter.call_id_for(&dispatch_bytes(400));
        assert_eq!(
            adapter.start_pinned(&retired, &route()),
            Ok(PinnedStart::AlreadySettled),
            "a retired id never reopens into a second charge"
        );
        assert_eq!(adapter.port.totals().calls, 8);
        let overflow = adapter.call_id_for(&dispatch_bytes(500));
        assert_eq!(
            adapter.start_pinned(&overflow, &route()),
            Err(CostError::ProjectionFull)
        );
        assert_eq!(adapter.port.totals().calls, 8);
    }

    const FRACTIONAL_PRICES: &str = r#"{"schema": "dungeonflux.pricing/1", "currency": "USD",
"prices": [{"provider": "openai", "model": "gpt-6-luna", "category": "text_output",
"usd": "0.50", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"}]}"#;

    fn fractional_adapter() -> ImageryCostAdapter {
        let port = MeteringPort::new();
        port.open("fractional-adapter", FRACTIONAL_PRICES)
            .expect("opens");
        ImageryCostAdapter::new(port).expect("storage worker")
    }

    fn fractional_route() -> CostRoute {
        CostRoute {
            provider: "openai".to_owned(),
            model: "gpt-6-luna".to_owned(),
            cause: "storyteller".to_owned(),
            detail: "adapter".to_owned(),
            maxima: vec![CostUnits {
                category: "text_output".to_owned(),
                quantity: 10,
            }],
        }
    }

    #[test]
    fn the_adapter_keeps_a_repeated_revision_at_identical_money() {
        let adapter = fractional_adapter();
        let call_id = adapter.call_id_for(&dispatch_bytes(1));
        adapter
            .start_pinned(&call_id, &fractional_route())
            .expect("opens");
        let one = [CostUnits {
            category: "text_output".to_owned(),
            quantity: 1,
        }];
        let three = [CostUnits {
            category: "text_output".to_owned(),
            quantity: 3,
        }];
        // One token at half a micro-dollar rounds to one in isolation.
        assert_eq!(
            settle_reported(&adapter, &call_id, 0, &one, 1_000),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().micro_usd, 1);
        for revision in 1..4 {
            assert_eq!(
                settle_reported(&adapter, &call_id, revision, &one, 1_000 + revision),
                Ok(PinnedCorrection::Applied)
            );
            assert_eq!(
                adapter.port.totals(),
                super::super::port::MeterTotals {
                    calls: 1,
                    micro_usd: 1,
                    unpriced_calls: 0,
                },
                "a repeated revision never pumps rounding"
            );
        }
        // Three tokens are one and a half micro-dollars, corrected up then down.
        assert_eq!(
            settle_reported(&adapter, &call_id, 4, &three, 2_000),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().micro_usd, 2);
        assert_eq!(
            settle_reported(&adapter, &call_id, 5, &one, 3_000),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().micro_usd, 1);
    }

    #[test]
    fn the_adapter_restores_the_route_maxima_for_unknown_after_a_lower_report() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&dispatch_bytes(2));
        adapter.start_pinned(&call_id, &route()).expect("opens");
        let unknown = UsageReport {
            revision: UsageRevision(0),
            evidence: UsageEvidence::Unknown,
            units: &[],
            usable: true,
            at_ms: 1_000,
        };
        adapter.correct(&call_id, &unknown).expect("settles");
        assert_eq!(adapter.port.totals().micro_usd, 40_000_000);
        settle_reported(
            &adapter,
            &call_id,
            1,
            &[CostUnits {
                category: "image_output".to_owned(),
                quantity: 100,
            }],
            2_000,
        )
        .expect("corrects down");
        assert_eq!(adapter.port.totals().micro_usd, 3_000);
        let unknown = UsageReport {
            revision: UsageRevision(2),
            evidence: UsageEvidence::Unknown,
            units: &[],
            usable: true,
            at_ms: 3_000,
        };
        adapter.correct(&call_id, &unknown).expect("unknown again");
        assert_eq!(
            adapter.port.totals().micro_usd,
            40_000_000,
            "unknown stands for the original maxima, not the lower usage"
        );
    }

    #[test]
    fn the_adapter_names_conflicts_and_keeps_terminal_unsent() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&dispatch_bytes(3));
        adapter.start_pinned(&call_id, &route()).expect("opens");
        let units = [CostUnits {
            category: "image_output".to_owned(),
            quantity: 1_000_000,
        }];
        settle_reported(&adapter, &call_id, 0, &units, 1_000).expect("settles");
        let other = [CostUnits {
            category: "image_output".to_owned(),
            quantity: 2_000_000,
        }];
        assert_eq!(
            settle_reported(&adapter, &call_id, 0, &other, 2_000),
            Err(CostError::RevisionConflict {
                revision: UsageRevision(0),
            })
        );
        let unusable = UsageReport {
            revision: UsageRevision(1),
            evidence: UsageEvidence::Reported,
            units: &units,
            usable: false,
            at_ms: 3_000,
        };
        assert_eq!(
            adapter.correct(&call_id, &unusable),
            Ok(PinnedCorrection::Applied)
        );
        let recent = adapter.port.recent();
        assert_eq!(recent.len(), 1);
        assert!(!recent[0].usable, "the billed loss stays attributed");
        let unsent_id = adapter.call_id_for(&dispatch_bytes(4));
        adapter.start_pinned(&unsent_id, &route()).expect("opens");
        assert_eq!(
            adapter.record_unsent(&unsent_id),
            Ok(PinnedUnsent::Recorded)
        );
        assert_eq!(
            settle_reported(&adapter, &unsent_id, 0, &units, 4_000),
            Err(CostError::AlreadySettled)
        );
    }

    fn codex_route() -> CostRoute {
        CostRoute {
            provider: "codex_subscription".to_owned(),
            model: "codex-cli".to_owned(),
            cause: "storyteller".to_owned(),
            detail: "adapter".to_owned(),
            maxima: vec![
                CostUnits {
                    category: "calls".to_owned(),
                    quantity: 1,
                },
                CostUnits {
                    category: "text_input".to_owned(),
                    quantity: 1_000,
                },
                CostUnits {
                    category: "text_output".to_owned(),
                    quantity: 200,
                },
                CostUnits {
                    category: "text_reasoning_output".to_owned(),
                    quantity: 40,
                },
            ],
        }
    }

    #[test]
    fn the_adapter_projects_subscription_units_unpriced_and_refuses_invalid_routes() {
        let adapter = adapter();
        let call_id = adapter.call_id_for(&dispatch_bytes(5));
        adapter
            .start_pinned(&call_id, &codex_route())
            .expect("a real subscription route projects");
        let maxima = codex_route().maxima;
        let reported = UsageReport {
            revision: UsageRevision(0),
            evidence: UsageEvidence::Reported,
            units: &maxima,
            usable: true,
            at_ms: 1_000,
        };
        assert_eq!(
            adapter.correct(&call_id, &reported),
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(adapter.port.totals().micro_usd, 0, "no tariff is invented");
        let recent = adapter.port.recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].units.len(), 4, "no unpriced unit is lost");
        assert!(recent[0].unpriced);
        let mut bad = codex_route();
        bad.provider = "acme".to_owned();
        assert_eq!(
            adapter.start_pinned(&adapter.call_id_for(&dispatch_bytes(6)), &bad),
            Err(CostError::UnknownProvider),
            "an invalid route is refused, an unknown price is projected"
        );
    }

    #[tokio::test]
    async fn the_async_worker_projects_unknown_revisions_and_answers_only_actual_durability() {
        let adapter = adapter();
        let wide: &dyn ImageCostProjection = &adapter;
        let id = wide.call_id_for(&dispatch_bytes(900));
        assert_eq!(
            wide.start_pinned(&id, &route()).await,
            Ok(PinnedStart::Opened)
        );
        let one = [CostUnits {
            category: "image_output".to_owned(),
            quantity: 1,
        }];
        let report = UsageReport {
            revision: UsageRevision(1),
            evidence: UsageEvidence::Unknown,
            units: &one,
            usable: true,
            at_ms: 1_000,
        };
        assert_eq!(
            wide.correct(&id, &report).await,
            Ok(PinnedCorrection::Applied)
        );
        assert_eq!(
            wide.correct(&id, &report).await,
            Ok(PinnedCorrection::Replayed)
        );
        assert_eq!(adapter.port.totals().micro_usd, 40_000_000);
        assert_eq!(
            wide.is_durable(&id, Some(UsageRevision(1))).await,
            Ok(false),
            "memory-only is never a durable receipt"
        );
        let unsent = wide.call_id_for(&dispatch_bytes(901));
        wide.start_pinned(&unsent, &route()).await.unwrap();
        wide.record_unsent(&unsent).await.unwrap();
        assert_eq!(
            wide.correct(&unsent, &report).await,
            Err(CostError::AlreadySettled)
        );
    }

    #[tokio::test]
    async fn a_full_async_worker_queue_refuses_work_without_mutating_the_book() {
        let adapter = adapter();
        let (entered, wait) = std::sync::mpsc::sync_channel(1);
        let (release, resume) = std::sync::mpsc::sync_channel(1);
        adapter
            .sender
            .as_ref()
            .unwrap()
            .send(Box::new(move |_| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
            }))
            .unwrap();
        wait.recv().unwrap();
        for _ in 0..COMMAND_CAPACITY {
            adapter
                .sender
                .as_ref()
                .unwrap()
                .try_send(Box::new(|_| {}))
                .unwrap();
        }
        let id = adapter.call_id_for(&dispatch_bytes(902));
        let outcome = ImageCostProjection::start_pinned(&adapter, &id, &route()).await;
        release.send(()).unwrap();
        assert_eq!(outcome, Err(CostError::ProjectionFull));
        assert_eq!(adapter.port.totals().calls, 0);
    }

    #[tokio::test]
    async fn the_worker_carries_invocation_context_and_restores_it_between_commands() {
        struct Observer(std::sync::Mutex<Vec<Option<u64>>>);
        impl super::super::port::MeterObserver for Observer {
            fn started(&self, _: &super::super::port::MeterStart) {
                self.0
                    .lock()
                    .unwrap()
                    .push(df_observe::current_audit_context().operation);
            }
            fn settled(&self, _: &super::super::port::MeteredCall) {}
        }
        let adapter = adapter();
        let observer = std::sync::Arc::new(Observer(std::sync::Mutex::new(Vec::new())));
        adapter.port.observe(observer.clone());
        let id = adapter.call_id_for(&dispatch_bytes(903));
        let route = route();
        let future = {
            let _scope = df_observe::audit_scope(df_observe::AuditContext {
                operation: Some(7),
                ..Default::default()
            });
            ImageCostProjection::start_pinned(&adapter, &id, &route)
        };
        future.await.unwrap();
        let other = adapter.call_id_for(&dispatch_bytes(904));
        ImageCostProjection::start_pinned(&adapter, &other, &route)
            .await
            .unwrap();
        assert_eq!(*observer.0.lock().unwrap(), [Some(7), None]);
    }

    #[tokio::test]
    async fn a_shared_adapter_reads_its_same_actual_book_receipt_without_defaulting_to_absence() {
        let adapter = std::sync::Arc::new(adapter());
        let projection: std::sync::Arc<dyn ImageCostProjection> = adapter.clone();
        let id = projection.call_id_for(&dispatch_bytes(905));
        ImageCostProjection::start_pinned(&projection, &id, &route())
            .await
            .unwrap();
        let units = [CostUnits {
            category: "image_output".to_owned(),
            quantity: 100,
        }];
        let report = UsageReport {
            revision: UsageRevision(3),
            evidence: UsageEvidence::Reported,
            units: &units,
            usable: false,
            at_ms: 1_000,
        };
        ImageCostProjection::correct(&projection, &id, &report)
            .await
            .unwrap();
        let receipt = ImageCostProjection::read_receipt(&projection, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.call_id, id);
        assert_eq!(receipt.revision, UsageRevision(3));
        assert_eq!(receipt.units, units);
        assert_eq!(receipt.amount.unwrap().micros(), 3_000);
        assert!(!receipt.usable);
        assert!(!receipt.durable);
        assert_eq!(adapter.port.totals().calls, 1);
    }
    #[tokio::test]
    async fn an_actual_shared_storage_worker_seals_absence_before_a_late_start() {
        use df_imagery::cost::UnsentDurability;
        let adapter = std::sync::Arc::new(adapter());
        let projection: std::sync::Arc<dyn ImageCostProjection> = adapter.clone();
        let id = projection.call_id_for(&dispatch_bytes(9_500));
        let seal = projection.seal_unsent(&id).await.unwrap();
        assert_eq!(seal.call_id, id);
        assert_eq!(seal.durability, UnsentDurability::Pending);
        assert_eq!(
            ImageCostProjection::start_pinned(&projection, &id, &route()).await,
            Ok(PinnedStart::AlreadySettled)
        );
        let second = projection.call_id_for(&dispatch_bytes(9_501));
        ImageCostProjection::start_pinned(&projection, &second, &route())
            .await
            .unwrap();
        assert_eq!(
            projection.seal_unsent(&second).await.unwrap().durability,
            UnsentDurability::Pending
        );
        assert_eq!(
            ImageCostProjection::start_pinned(&projection, &second, &route()).await,
            Ok(PinnedStart::AlreadySettled)
        );
        assert_eq!(adapter.port.totals().calls, 0);
        assert!(adapter.port.recent().is_empty());
    }
}
