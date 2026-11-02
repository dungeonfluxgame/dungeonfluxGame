//! Checked ownership for native composition. An ownership declaration grants no runtime authority.
//! Consumer contracts remain in their owning crates; this module defines no substitute ports.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrateOwner {
    Session,
    Engine,
    Assets,
    Auth,
    Observe,
    Ai,
    ProviderApi,
    Model,
    Telemetry,
    Persistence,
    Tools,
    RpcBridge,
    Server,
}

impl CrateOwner {
    pub(crate) const fn crate_name(self) -> &'static str {
        match self {
            Self::Session => "df-session",
            Self::Engine => "df-engine",
            Self::Assets => "df-assets",
            Self::Auth => "df-auth",
            Self::Observe => "df-observe",
            Self::Ai => "df-ai",
            Self::ProviderApi => "df-provider-api",
            Self::Model => "df-model",
            Self::Telemetry => "df-telemetry",
            Self::Persistence => "df-persistence",
            Self::Tools => "df-tools",
            Self::RpcBridge => "df-rpc-bridge",
            Self::Server => "df-server",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoundaryKind {
    Consumer,
    Executor,
    Adapter,
    Bridge,
    Listener,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoundaryKey {
    pub(crate) kind: BoundaryKind,
    pub(crate) name: &'static str,
}

/// Source evidence distinguishes an existing definition from a working native adapter.
/// The two df-tools listeners are synthetic fixtures, not production game admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceEvidence {
    ConsumerContract,
    MissingEffectExecutor,
    NativeAdapterDeclaration,
    NativeBridgeContract,
    FixtureListenerOnly,
    MissingProductionListener,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoundarySpec {
    pub(crate) key: BoundaryKey,
    pub(crate) contract_owner: CrateOwner,
    pub(crate) lifetime_owner: CrateOwner,
    pub(crate) source_path: &'static str,
    pub(crate) source_symbol: &'static str,
    pub(crate) evidence: SourceEvidence,
}

const fn boundary(
    kind: BoundaryKind,
    name: &'static str,
    contract_owner: CrateOwner,
    lifetime_owner: CrateOwner,
    source_path: &'static str,
    source_symbol: &'static str,
    evidence: SourceEvidence,
) -> BoundarySpec {
    BoundarySpec {
        key: BoundaryKey { kind, name },
        contract_owner,
        lifetime_owner,
        source_path,
        source_symbol,
        evidence,
    }
}

/// Complete declared native ownership surface at the pinned source revision.
/// Session ActorInput, OperationScope and Reducer are included even though they
/// define input/serialization rather than external I/O. No public native port in
/// the cited consumer modules is silently excluded. Server lifetime ownership is
/// a composition responsibility; this catalogue does not construct those services.
pub(crate) const NATIVE_BOUNDARIES: [BoundarySpec; 40] = [
    boundary(
        BoundaryKind::Consumer,
        "df_session::inbox::ActorInput",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/inbox.rs",
        "ActorInput",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::inbox::Reducer",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/inbox.rs",
        "Reducer",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::submission::OperationScope",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/submission.rs",
        "OperationScope",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::submission::SessionRepository",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/submission.rs",
        "SessionRepository",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::submission::SessionEngine",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/submission.rs",
        "SessionEngine",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::submission::PublicationOwner",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/submission.rs",
        "PublicationOwner",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_session::effects::EffectRepository",
        CrateOwner::Session,
        CrateOwner::Server,
        "crates/df-session/src/effects.rs",
        "EffectRepository",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_engine::effect_emission::EffectRegistrationInspector",
        CrateOwner::Engine,
        CrateOwner::Server,
        "crates/df-engine/src/effect_emission.rs",
        "EffectRegistrationInspector",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_assets::publication::AssetStore",
        CrateOwner::Assets,
        CrateOwner::Server,
        "crates/df-assets/src/publication.rs",
        "AssetStore",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_assets::publication::AssetMetadataStore",
        CrateOwner::Assets,
        CrateOwner::Server,
        "crates/df-assets/src/publication.rs",
        "AssetMetadataStore",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_assets::range::AssetReadAuthority",
        CrateOwner::Assets,
        CrateOwner::Server,
        "crates/df-assets/src/range.rs",
        "AssetReadAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_assets::range::AssetReadStore",
        CrateOwner::Assets,
        CrateOwner::Server,
        "crates/df-assets/src/range.rs",
        "AssetReadStore",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::bootstrap::GuestBootstrapStore",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/bootstrap.rs",
        "GuestBootstrapStore",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::bootstrap::BootstrapObserver",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/bootstrap.rs",
        "BootstrapObserver",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::membership::MembershipAuthority",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/membership.rs",
        "MembershipAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::permissions::PermissionAuthority",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/permissions.rs",
        "PermissionAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::recovery::AccountAuthority",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/recovery.rs",
        "AccountAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_auth::rotation::PublicationAuthority",
        CrateOwner::Auth,
        CrateOwner::Server,
        "crates/df-auth/src/rotation.rs",
        "PublicationAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_observe::ingress::TelemetryIngress",
        CrateOwner::Observe,
        CrateOwner::Server,
        "crates/df-observe/src/ingress.rs",
        "TelemetryIngress",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_ai::admission::RecordingPublisher",
        CrateOwner::Ai,
        CrateOwner::Server,
        "crates/df-ai/src/admission.rs",
        "RecordingPublisher",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_ai::lookup::PreparedRead",
        CrateOwner::Ai,
        CrateOwner::Server,
        "crates/df-ai/src/lookup.rs",
        "PreparedRead",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_ai::lookup::ReadAuthority",
        CrateOwner::Ai,
        CrateOwner::Server,
        "crates/df-ai/src/lookup.rs",
        "ReadAuthority",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Consumer,
        "df_provider_api::budget::BudgetStore",
        CrateOwner::ProviderApi,
        CrateOwner::Server,
        "crates/df-provider-api/src/budget.rs",
        "BudgetStore",
        SourceEvidence::ConsumerContract,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::RunAi",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "RunAi",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::RunMedia",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "RunMedia",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::LoadMemoryCandidates",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "LoadMemoryCandidates",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::ArmTimer",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "ArmTimer",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::CancelJob",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "CancelJob",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::CancelTimer",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "CancelTimer",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Executor,
        "df_model::checkpoint::EffectKind::PublishPresentation",
        CrateOwner::Model,
        CrateOwner::Server,
        "crates/df-model/src/checkpoint.rs",
        "PublishPresentation",
        SourceEvidence::MissingEffectExecutor,
    ),
    boundary(
        BoundaryKind::Adapter,
        "df_assets::NativeFileStore",
        CrateOwner::Assets,
        CrateOwner::Server,
        "crates/df-assets/src/native_file_store.rs",
        "NativeFileStore",
        SourceEvidence::NativeAdapterDeclaration,
    ),
    boundary(
        BoundaryKind::Adapter,
        "df_telemetry::Store",
        CrateOwner::Telemetry,
        CrateOwner::Server,
        "crates/df-telemetry/src/lib.rs",
        "Store",
        SourceEvidence::NativeAdapterDeclaration,
    ),
    boundary(
        BoundaryKind::Adapter,
        "df_persistence::PostgresRepository",
        CrateOwner::Persistence,
        CrateOwner::Server,
        "crates/df-persistence/src/native_bridge.rs",
        "PostgresRepository",
        SourceEvidence::NativeAdapterDeclaration,
    ),
    boundary(
        BoundaryKind::Bridge,
        "df_rpc_bridge::native::NativeIncoming",
        CrateOwner::RpcBridge,
        CrateOwner::Server,
        "crates/df-rpc-bridge/src/native.rs",
        "NativeIncoming",
        SourceEvidence::NativeBridgeContract,
    ),
    boundary(
        BoundaryKind::Bridge,
        "df_rpc_bridge::native::NativeAdmission",
        CrateOwner::RpcBridge,
        CrateOwner::Server,
        "crates/df-rpc-bridge/src/native.rs",
        "NativeAdmission",
        SourceEvidence::NativeBridgeContract,
    ),
    boundary(
        BoundaryKind::Bridge,
        "df_rpc_bridge::native::NativeConnectionPermit",
        CrateOwner::RpcBridge,
        CrateOwner::Server,
        "crates/df-rpc-bridge/src/native.rs",
        "NativeConnectionPermit",
        SourceEvidence::NativeBridgeContract,
    ),
    boundary(
        BoundaryKind::Listener,
        "df_tools::fixture::serve",
        CrateOwner::Tools,
        CrateOwner::Tools,
        "crates/df-tools/src/fixture.rs",
        "TcpListener::bind",
        SourceEvidence::FixtureListenerOnly,
    ),
    boundary(
        BoundaryKind::Listener,
        "public game listener",
        CrateOwner::Server,
        CrateOwner::Server,
        "planning/subsystem-interfaces.md",
        "PublicServiceSet",
        SourceEvidence::MissingProductionListener,
    ),
    boundary(
        BoundaryKind::Listener,
        "restricted operator listener",
        CrateOwner::Server,
        CrateOwner::Server,
        "planning/subsystem-interfaces.md",
        "AdminServiceSet",
        SourceEvidence::MissingProductionListener,
    ),
    boundary(
        BoundaryKind::Listener,
        "payment webhook listener",
        CrateOwner::Server,
        CrateOwner::Server,
        "planning/commerce-service.md",
        "Webhook HTTPS handler",
        SourceEvidence::MissingProductionListener,
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OwnershipAssignment {
    pub(crate) boundary: BoundaryKey,
    pub(crate) contract_owner: Option<CrateOwner>,
    pub(crate) lifetime_owner: Option<CrateOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnerRole {
    Contract,
    Lifetime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnershipRefusal {
    Capacity,
    UnknownBoundary(BoundaryKey),
    DuplicateBoundary(BoundaryKey),
    MissingBoundary(BoundaryKey),
    MissingOwner {
        boundary: BoundaryKey,
        role: OwnerRole,
    },
    WrongOwner {
        boundary: BoundaryKey,
        role: OwnerRole,
        expected: CrateOwner,
        observed: CrateOwner,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeRefusal {
    MissingEffectExecutor(BoundaryKey),
    FixtureListenerOnly(BoundaryKey),
    MissingProductionListener(BoundaryKey),
    RuntimeQualificationUnperformed(BoundaryKey),
}

/// An immutable ownership witness borrowing a bounded caller-owned inventory.
/// Validation starts no I/O, task, timer, actor, listener, payment or provider.
/// Success means only that all declared boundaries have their exact owners.
#[derive(Debug)]
pub(crate) struct OwnershipRegistry<'a> {
    assignments: &'a [OwnershipAssignment],
}

impl<'a> OwnershipRegistry<'a> {
    pub(crate) fn validate(
        assignments: &'a [OwnershipAssignment],
    ) -> Result<Self, OwnershipRefusal> {
        if assignments.len() > NATIVE_BOUNDARIES.len() {
            return Err(OwnershipRefusal::Capacity);
        }
        for (position, assignment) in assignments.iter().enumerate() {
            let spec = NATIVE_BOUNDARIES
                .iter()
                .find(|spec| spec.key == assignment.boundary)
                .ok_or(OwnershipRefusal::UnknownBoundary(assignment.boundary))?;
            if assignments[..position]
                .iter()
                .any(|other| other.boundary == assignment.boundary)
            {
                return Err(OwnershipRefusal::DuplicateBoundary(assignment.boundary));
            }
            for (role, supplied, expected) in [
                (
                    OwnerRole::Contract,
                    assignment.contract_owner,
                    spec.contract_owner,
                ),
                (
                    OwnerRole::Lifetime,
                    assignment.lifetime_owner,
                    spec.lifetime_owner,
                ),
            ] {
                let observed = supplied.ok_or(OwnershipRefusal::MissingOwner {
                    boundary: spec.key,
                    role,
                })?;
                if observed != expected {
                    return Err(OwnershipRefusal::WrongOwner {
                        boundary: spec.key,
                        role,
                        expected,
                        observed,
                    });
                }
            }
        }
        for spec in NATIVE_BOUNDARIES {
            if !assignments
                .iter()
                .any(|assignment| assignment.boundary == spec.key)
            {
                return Err(OwnershipRefusal::MissingBoundary(spec.key));
            }
        }
        Ok(Self { assignments })
    }

    pub(crate) fn owner(&self, key: BoundaryKey) -> Result<&OwnershipAssignment, OwnershipRefusal> {
        self.assignments
            .iter()
            .find(|assignment| assignment.boundary == key)
            .ok_or(OwnershipRefusal::UnknownBoundary(key))
    }

    /// Every caller receives a classified refusal, never a readiness or dispatch token.
    /// Existing native types require independent current backing/auth/fence/health
    /// qualification; declared ownership cannot establish any of those conditions.
    pub(crate) fn native_admission(
        &self,
        key: BoundaryKey,
    ) -> Result<std::convert::Infallible, NativeRefusal> {
        let spec = NATIVE_BOUNDARIES.iter().find(|spec| spec.key == key);
        let refusal = match spec.map(|spec| spec.evidence) {
            Some(SourceEvidence::MissingEffectExecutor) => {
                NativeRefusal::MissingEffectExecutor(key)
            }
            Some(SourceEvidence::FixtureListenerOnly) => NativeRefusal::FixtureListenerOnly(key),
            Some(SourceEvidence::MissingProductionListener) => {
                NativeRefusal::MissingProductionListener(key)
            }
            Some(
                SourceEvidence::ConsumerContract
                | SourceEvidence::NativeAdapterDeclaration
                | SourceEvidence::NativeBridgeContract,
            )
            | None => NativeRefusal::RuntimeQualificationUnperformed(key),
        };
        Err(refusal)
    }
}
