use std::collections::{BTreeMap, BTreeSet};

use prost::Message;
use prost_types::{FileDescriptorSet, MethodDescriptorProto};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RpcMode {
    Unary,
    ServerStreaming,
    ClientStreaming,
    BidirectionalStreaming,
}

impl RpcMode {
    fn from_descriptor(method: &MethodDescriptorProto) -> Self {
        match (
            method.client_streaming.unwrap_or(false),
            method.server_streaming.unwrap_or(false),
        ) {
            (false, false) => Self::Unary,
            (false, true) => Self::ServerStreaming,
            (true, false) => Self::ClientStreaming,
            (true, true) => Self::BidirectionalStreaming,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessClass {
    PublicIdentityBootstrap,
    PublicTrustedAccountTenant,
    PublicMembershipBinding,
    PublicMemberInput,
    PublicLiveMedia,
    PublicAudienceAudio,
    PublicImmutableAsset,
    PublicAuthorizedProjection,
    PublicBoundClient,
    PublicHostCapability,
    RestrictedAdmin,
    RestrictedTelemetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Namespace {
    Public,
    Admin,
    Telemetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ServiceOwner {
    service: &'static str,
    namespace: Namespace,
    access: AccessClass,
    request_owner: &'static str,
    result_owner: &'static str,
    stream_owner: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlannedMethod {
    service: &'static str,
    method: &'static str,
    request: &'static str,
    response: &'static str,
    mode: RpcMode,
    production_handler: ProductionHandlerEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MethodOwnerConstraint {
    service: &'static str,
    method: &'static str,
    access_requirement: &'static str,
    request_owner: &'static str,
    result_owner: &'static str,
    stream_owner: Option<&'static str>,
    source_ref: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductionHandlerEvidence {
    NotEstablishedByInventory,
    ClaimedImplemented,
}

const SERVICE_OWNERS: [ServiceOwner; 13] = [
    ServiceOwner {
        service: "IdentityService",
        namespace: Namespace::Public,
        access: AccessClass::PublicIdentityBootstrap,
        request_owner: "df-auth / native df-api",
        result_owner: "df-auth / native df-api",
        stream_owner: None,
    },
    ServiceOwner {
        service: "CustomerService",
        namespace: Namespace::Public,
        access: AccessClass::PublicTrustedAccountTenant,
        request_owner: "df-commerce / native df-api",
        result_owner: "df-commerce / native df-api",
        stream_owner: Some(
            "native df-api streams only a current ExportGrant and DataLifecycle scope within chunk, byte and item bounds; the durable export adapter is not fixed here",
        ),
    },
    ServiceOwner {
        service: "SessionService",
        namespace: Namespace::Public,
        access: AccessClass::PublicMembershipBinding,
        request_owner: "df-session / native df-api",
        result_owner: "df-session / native df-api",
        stream_owner: Some(
            "native df-api owns authorized Watch delivery; df-session owns subscription lifetime",
        ),
    },
    ServiceOwner {
        service: "ActionService",
        namespace: Namespace::Public,
        access: AccessClass::PublicMemberInput,
        request_owner: "native df-api maps authenticated input to df-session / df-engine",
        result_owner: "df-session owns commit; df-engine owns validation and staged outcome",
        stream_owner: None,
    },
    ServiceOwner {
        service: "VoiceService",
        namespace: Namespace::Public,
        access: AccessClass::PublicLiveMedia,
        request_owner: "native df-api admission; df-media owns live route",
        result_owner: "df-media / native df-api",
        stream_owner: Some(
            "native df-api owns bidi half-close and cancellation; df-media owns live capture/transcription; df-session owns accepted effects through durable session/run lifetime independent of RPC cancellation",
        ),
    },
    ServiceOwner {
        service: "AudioService",
        namespace: Namespace::Public,
        access: AccessClass::PublicAudienceAudio,
        request_owner: "native df-api authorizes audience; df-media owns routing",
        result_owner: "df-media / native df-api",
        stream_owner: Some(
            "native df-api authorizes the current binding lease/audience; df-media owns stream epoch, order and current timeline recovery; df-audio cancels old-epoch buffers and ignores late frames",
        ),
    },
    ServiceOwner {
        service: "AssetService",
        namespace: Namespace::Public,
        access: AccessClass::PublicImmutableAsset,
        request_owner: "df-assets / native df-api",
        result_owner: "df-assets / native df-api",
        stream_owner: Some(
            "native df-api owns bounded Get delivery; df-assets owns immutable byte access",
        ),
    },
    ServiceOwner {
        service: "JournalService",
        namespace: Namespace::Public,
        access: AccessClass::PublicAuthorizedProjection,
        request_owner: "native df-api / df-session",
        result_owner: "native df-api projects df-session read model",
        stream_owner: None,
    },
    ServiceOwner {
        service: "ClientService",
        namespace: Namespace::Public,
        access: AccessClass::PublicBoundClient,
        request_owner: "native df-api / df-client binding contract",
        result_owner: "native df-api with df-client / df-observe ports",
        stream_owner: Some(
            "native df-api owns control and bounded upload streams; UploadDiagnostics requires an expiring scoped CaptureTicket and approved target/binding, purpose, MIME, size, hash and chunk sequence",
        ),
    },
    ServiceOwner {
        service: "HostService",
        namespace: Namespace::Public,
        access: AccessClass::PublicHostCapability,
        request_owner: "native df-api validates capability; df-session owns admission",
        result_owner: "df-session / df-engine",
        stream_owner: None,
    },
    ServiceOwner {
        service: "DebugService",
        namespace: Namespace::Admin,
        access: AccessClass::RestrictedAdmin,
        request_owner: "native df-api / operator owner with independent df-auth authorization",
        result_owner: "native df-api / operator owner",
        stream_owner: Some("native df-api owns restricted WatchEvents delivery and cancellation"),
    },
    ServiceOwner {
        service: "TelemetryService",
        namespace: Namespace::Telemetry,
        access: AccessClass::RestrictedTelemetry,
        request_owner: "df-auth independently authorizes telemetry operators; trusted-source authorization applies to ingestion, not operator query/pin/export",
        result_owner: "df-telemetry",
        stream_owner: Some("df-telemetry owns bounded evidence export and stream lifecycle"),
    },
    ServiceOwner {
        service: "DialogueService",
        namespace: Namespace::Public,
        access: AccessClass::PublicMemberInput,
        request_owner: "df-intent / native df-tools authenticated Actor",
        result_owner: "df-intent disposition / native df-session decision receipt",
        stream_owner: None,
    },
];

const fn method(
    service: &'static str,
    method: &'static str,
    request: &'static str,
    response: &'static str,
    mode: RpcMode,
) -> PlannedMethod {
    PlannedMethod {
        service,
        method,
        request,
        response,
        mode,
        production_handler: ProductionHandlerEvidence::NotEstablishedByInventory,
    }
}

const PLANNED_METHODS: [PlannedMethod; 44] = [
    method(
        "IdentityService",
        "BeginGuest",
        "BeginGuestRequest",
        "IdentityGrant",
        RpcMode::Unary,
    ),
    method(
        "IdentityService",
        "Renew",
        "RenewIdentityRequest",
        "IdentityGrant",
        RpcMode::Unary,
    ),
    method(
        "IdentityService",
        "Revoke",
        "RevokeIdentityRequest",
        "RevocationReceipt",
        RpcMode::Unary,
    ),
    method(
        "CustomerService",
        "Command",
        "CustomerCommandRequest",
        "CustomerReceipt",
        RpcMode::Unary,
    ),
    method(
        "CustomerService",
        "Inspect",
        "CustomerInspectRequest",
        "CustomerView",
        RpcMode::Unary,
    ),
    method(
        "CustomerService",
        "GetOperation",
        "CustomerOperationRequest",
        "CustomerOperationLookup",
        RpcMode::Unary,
    ),
    method(
        "CustomerService",
        "Export",
        "CustomerExportRequest",
        "CustomerExportPart",
        RpcMode::ServerStreaming,
    ),
    method(
        "SessionService",
        "Create",
        "CreateSessionRequest",
        "SessionGrant",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "Join",
        "JoinSessionRequest",
        "SessionGrant",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "Resume",
        "ResumeSessionRequest",
        "SessionGrant",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "BindClient",
        "BindClientRequest",
        "ClientBinding",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "Leave",
        "LeaveSessionRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "SetPreferences",
        "SetPreferencesRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "SessionService",
        "Watch",
        "WatchViewRequest",
        "ViewMessage",
        RpcMode::ServerStreaming,
    ),
    method(
        "SessionService",
        "GetOperation",
        "GetOperationRequest",
        "OperationLookup",
        RpcMode::Unary,
    ),
    method(
        "ActionService",
        "Submit",
        "SubmitActionRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "ActionService",
        "Preview",
        "PreviewActionRequest",
        "ActionPreview",
        RpcMode::Unary,
    ),
    method(
        "ActionService",
        "ListOptions",
        "ListOptionsRequest",
        "OptionPage",
        RpcMode::Unary,
    ),
    method(
        "ActionService",
        "SubmitText",
        "SubmitTextRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "VoiceService",
        "Talk",
        "TalkInput",
        "TalkOutput",
        RpcMode::BidirectionalStreaming,
    ),
    method(
        "AudioService",
        "Listen",
        "ListenRequest",
        "AudioStreamMessage",
        RpcMode::ServerStreaming,
    ),
    method(
        "AssetService",
        "Manifest",
        "AssetManifestRequest",
        "AssetManifestPage",
        RpcMode::Unary,
    ),
    method(
        "AssetService",
        "Get",
        "GetAssetRequest",
        "AssetTransferMessage",
        RpcMode::ServerStreaming,
    ),
    method(
        "JournalService",
        "ListEntries",
        "JournalRequest",
        "JournalPage",
        RpcMode::Unary,
    ),
    method(
        "ClientService",
        "WatchControl",
        "WatchControlRequest",
        "ClientControl",
        RpcMode::ServerStreaming,
    ),
    method(
        "ClientService",
        "Report",
        "ClientReport",
        "ReportReceipt",
        RpcMode::Unary,
    ),
    method(
        "ClientService",
        "UploadTelemetry",
        "ClientTelemetryBatch",
        "IngestReceipt",
        RpcMode::ClientStreaming,
    ),
    method(
        "ClientService",
        "UploadDiagnostics",
        "DiagnosticPart",
        "DiagnosticReceipt",
        RpcMode::ClientStreaming,
    ),
    method(
        "HostService",
        "Command",
        "HostCommandRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "Command",
        "DebugCommandRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "Inspect",
        "InspectRequest",
        "DebugSnapshot",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "WatchEvents",
        "EventQuery",
        "DebugEvent",
        RpcMode::ServerStreaming,
    ),
    method(
        "DebugService",
        "SaveCheckpoint",
        "SaveCheckpointRequest",
        "CheckpointInfo",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "RestoreCheckpoint",
        "RestoreCheckpointRequest",
        "DecisionReceipt",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "SetFault",
        "SetFaultRequest",
        "FaultReceipt",
        RpcMode::Unary,
    ),
    method(
        "DebugService",
        "RequestCapture",
        "CaptureRequest",
        "CaptureTicket",
        RpcMode::Unary,
    ),
    method(
        "TelemetryService",
        "Query",
        "TelemetryQuery",
        "TelemetryPage",
        RpcMode::Unary,
    ),
    method(
        "TelemetryService",
        "Timeline",
        "TimelineQuery",
        "TimelinePage",
        RpcMode::Unary,
    ),
    method(
        "TelemetryService",
        "Health",
        "TelemetryHealthRequest",
        "TelemetryHealth",
        RpcMode::Unary,
    ),
    method(
        "TelemetryService",
        "PinEvidence",
        "PinEvidenceRequest",
        "EvidenceReference",
        RpcMode::Unary,
    ),
    method(
        "TelemetryService",
        "ExportEvidence",
        "ExportEvidenceRequest",
        "EvidenceChunk",
        RpcMode::ServerStreaming,
    ),
    method(
        "DialogueService",
        "Submit",
        "RawDialogueRequest",
        "DialogueResponse",
        RpcMode::Unary,
    ),
    method(
        "DialogueService",
        "Confirm",
        "ConfirmDialogueRequest",
        "SubmitActionResponse",
        RpcMode::Unary,
    ),
    method(
        "DialogueService",
        "Cancel",
        "CancelDialogueRequest",
        "DialogueResponse",
        RpcMode::Unary,
    ),
];

const METHOD_OWNER_CONSTRAINTS: [MethodOwnerConstraint; 10] = [
    MethodOwnerConstraint {
        service: "CustomerService",
        method: "Export",
        access_requirement: "current explicit ExportGrant and DataLifecycle scope; bounded chunks/bytes/items; payer status alone never grants player-secret export",
        request_owner: "native df-api binds trusted account/tenant; df-commerce owns customer export authorization policy; this contract does not assign the DataLifecycle adapter",
        result_owner: "native df-api shapes CustomerExportPart; durable export data adapter ownership is not fixed by this contract",
        stream_owner: Some(
            "native df-api owns the bounded server stream under the explicit grant and lifecycle scope",
        ),
        source_ref: "planning/rpc-api.md:652-666",
    },
    MethodOwnerConstraint {
        service: "VoiceService",
        method: "Talk",
        access_requirement: "current authorized capture lease and bounded admitted capture; stream half-close is independent of durable accepted work",
        request_owner: "native df-api admits the capture lease; df-media owns live capture/transcription routing",
        result_owner: "df-media owns stream transcript/terminal outcome; df-session owns any durably accepted action/job",
        stream_owner: Some(
            "df-session owns accepted effect deadlines/cancellation by session and run after durable acceptance, independent of Talk disconnect; native df-api owns bidi half-close/cancel",
        ),
        source_ref: "planning/rpc-api.md:169-174,512-524",
    },
    MethodOwnerConstraint {
        service: "AudioService",
        method: "Listen",
        access_requirement: "binding's current audio lease and authorized audience; no per-frame audience wildcard",
        request_owner: "native df-api validates the current binding lease and audience",
        result_owner: "df-media owns ordered messages with epoch, cue/track identity and timeline; df-audio owns playback buffers",
        stream_owner: Some(
            "df-media recovers lost/slow streams from a new epoch and current timeline; df-audio stops old-epoch buffers and ignores late frames",
        ),
        source_ref: "planning/rpc-api.md:455-460,526-539; planning/subsystem-interfaces.md:389-403",
    },
    MethodOwnerConstraint {
        service: "ClientService",
        method: "UploadDiagnostics",
        access_requirement: "expiring CaptureTicket bound to purpose and target/binding, with approved MIME/size/hash and chunk sequence",
        request_owner: "native df-api verifies the scoped ticket and bounded upload; DebugService capture authorization remains independently restricted",
        result_owner: "native df-api returns a retained evidence reference; durable diagnostic evidence adapter owner is unresolved in this API contract",
        stream_owner: Some(
            "native df-api owns bounded client-stream chunks and terminal receipt under ticket expiry; diagnostic bytes are not ordinary always-on logs",
        ),
        source_ref: "planning/rpc-api.md:552-571,598-614",
    },
    MethodOwnerConstraint {
        service: "ClientService",
        method: "UploadTelemetry",
        access_requirement: "current trusted client source identity for ingestion; this upload authorization is separate from telemetry operator query/pin/export authorization",
        request_owner: "native df-api validates/enriches trusted identity and calls the consumer-owned TelemetryIngress port in df-observe",
        result_owner: "df-telemetry implements TelemetryIngress and owns typed durability-stage and gap receipt facts",
        stream_owner: Some(
            "native df-api owns bounded client-stream half-close and terminal receipt; accepted durable batches survive cancellation",
        ),
        source_ref: "planning/rpc-api.md:557-567; planning/subsystem-interfaces.md:357-360,412-422",
    },
    MethodOwnerConstraint {
        service: "TelemetryService",
        method: "Query",
        access_requirement: "telemetry operator namespace with independent trusted operator authorization; trusted-source auth is ingestion-only",
        request_owner: "df-auth authorizes operator access; df-telemetry owns read-only filtering",
        result_owner: "df-telemetry owns bounded paginated telemetry results",
        stream_owner: None,
        source_ref: "planning/rpc-api.md:105-118,598-619; planning/subsystem-interfaces.md:412-422",
    },
    MethodOwnerConstraint {
        service: "TelemetryService",
        method: "Timeline",
        access_requirement: "telemetry operator namespace with independent trusted operator authorization; trusted-source auth is ingestion-only",
        request_owner: "df-auth authorizes operator access; df-telemetry owns read-only timeline filtering",
        result_owner: "df-telemetry owns bounded paginated timeline results",
        stream_owner: None,
        source_ref: "planning/rpc-api.md:105-118,598-619; planning/subsystem-interfaces.md:412-422",
    },
    MethodOwnerConstraint {
        service: "TelemetryService",
        method: "Health",
        access_requirement: "telemetry operator namespace with independent trusted operator authorization; trusted-source auth is ingestion-only",
        request_owner: "df-auth authorizes operator access; df-telemetry owns ingestion health",
        result_owner: "df-telemetry owns ingestion health facts",
        stream_owner: None,
        source_ref: "planning/rpc-api.md:105-118,598-619; planning/subsystem-interfaces.md:412-422",
    },
    MethodOwnerConstraint {
        service: "TelemetryService",
        method: "PinEvidence",
        access_requirement: "telemetry operator namespace with independent trusted operator authorization and retention policy",
        request_owner: "df-auth authorizes operator access; df-telemetry enforces evidence retention policy",
        result_owner: "df-telemetry owns the pinned evidence reference",
        stream_owner: None,
        source_ref: "planning/rpc-api.md:105-118,598-619; planning/subsystem-interfaces.md:412-422",
    },
    MethodOwnerConstraint {
        service: "TelemetryService",
        method: "ExportEvidence",
        access_requirement: "telemetry operator namespace with independent trusted operator authorization and retention policy",
        request_owner: "df-auth authorizes operator access; df-telemetry selects retained cited evidence",
        result_owner: "df-telemetry owns bounded evidence chunks",
        stream_owner: Some(
            "df-telemetry owns bounded export lifecycle under operator authorization and retention policy",
        ),
        source_ref: "planning/rpc-api.md:105-118,598-619; planning/subsystem-interfaces.md:412-422",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct DescriptorMethod {
    service: String,
    method: String,
    request: String,
    response: String,
    mode: RpcMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryError {
    WrongServiceCount,
    WrongMethodCount,
    MissingOwner,
    DuplicateQualifiedMethod,
    MissingMethod,
    UnknownMethod,
    TupleChanged,
    PublicOperatorClaim,
    MissingStreamOwner,
    UnknownDescriptorService,
    UnsupportedProductionHandlerClaim,
    WrongMethodOwnerConstraintCount,
    MissingMethodOwnerConstraint,
    MethodOwnerChanged,
}

fn validate_contract(
    owners: &[ServiceOwner],
    methods: &[PlannedMethod],
) -> Result<(), InventoryError> {
    if owners.len() != 13 {
        return Err(InventoryError::WrongServiceCount);
    }
    if methods.len() != 44 {
        return Err(InventoryError::WrongMethodCount);
    }

    let mut owner_by_service = BTreeMap::new();
    for owner in owners {
        if owner.request_owner.is_empty() || owner.result_owner.is_empty() {
            return Err(InventoryError::MissingOwner);
        }
        if owner.service.is_empty() || owner_by_service.insert(owner.service, owner).is_some() {
            return Err(InventoryError::DuplicateQualifiedMethod);
        }
        if matches!(owner.service, "DebugService" | "TelemetryService")
            && owner.namespace == Namespace::Public
        {
            return Err(InventoryError::PublicOperatorClaim);
        }
    }

    let expected_owners = SERVICE_OWNERS
        .iter()
        .map(|owner| owner.service)
        .collect::<BTreeSet<_>>();
    if owner_by_service.keys().copied().collect::<BTreeSet<_>>() != expected_owners {
        return Err(InventoryError::MissingOwner);
    }
    for owner in owners {
        let expected = SERVICE_OWNERS
            .iter()
            .find(|expected| expected.service == owner.service)
            .ok_or(InventoryError::MissingOwner)?;
        if owner.namespace != expected.namespace || owner.access != expected.access {
            return Err(InventoryError::PublicOperatorClaim);
        }
        if owner.request_owner != expected.request_owner
            || owner.result_owner != expected.result_owner
        {
            return Err(InventoryError::MissingOwner);
        }
        if owner.stream_owner != expected.stream_owner {
            return Err(InventoryError::MissingStreamOwner);
        }
    }

    let mut by_key = BTreeMap::new();
    for item in methods {
        let key = (item.service, item.method);
        if by_key.insert(key, item).is_some() {
            return Err(InventoryError::DuplicateQualifiedMethod);
        }
        owner_by_service
            .get(item.service)
            .ok_or(InventoryError::MissingOwner)?;
        if item.production_handler != ProductionHandlerEvidence::NotEstablishedByInventory {
            return Err(InventoryError::UnsupportedProductionHandlerClaim);
        }
    }

    for expected in PLANNED_METHODS {
        let Some(actual) = by_key.get(&(expected.service, expected.method)) else {
            return Err(InventoryError::MissingMethod);
        };
        if *actual != &expected {
            return Err(InventoryError::TupleChanged);
        }
    }
    if by_key.len() != PLANNED_METHODS.len() {
        return Err(InventoryError::UnknownMethod);
    }
    for item in methods {
        let owner = owner_by_service
            .get(item.service)
            .ok_or(InventoryError::MissingOwner)?;
        if item.mode != RpcMode::Unary && owner.stream_owner.is_none() {
            return Err(InventoryError::MissingStreamOwner);
        }
    }
    Ok(())
}

fn validate_full_contract(
    owners: &[ServiceOwner],
    methods: &[PlannedMethod],
    method_owners: &[MethodOwnerConstraint],
) -> Result<(), InventoryError> {
    validate_contract(owners, methods)?;
    validate_method_owner_constraints(methods, method_owners)
}

fn validate_method_owner_constraints(
    methods: &[PlannedMethod],
    method_owners: &[MethodOwnerConstraint],
) -> Result<(), InventoryError> {
    if method_owners.len() != METHOD_OWNER_CONSTRAINTS.len() {
        return Err(InventoryError::WrongMethodOwnerConstraintCount);
    }

    let method_keys = methods
        .iter()
        .map(|method| (method.service, method.method))
        .collect::<BTreeSet<_>>();
    let mut owners_by_key = BTreeMap::new();
    for owner in method_owners {
        let key = (owner.service, owner.method);
        if owners_by_key.insert(key, owner).is_some() {
            return Err(InventoryError::DuplicateQualifiedMethod);
        }
        if !method_keys.contains(&key) {
            return Err(InventoryError::MissingMethodOwnerConstraint);
        }
        if owner.access_requirement.is_empty()
            || owner.request_owner.is_empty()
            || owner.result_owner.is_empty()
            || owner.source_ref.is_empty()
        {
            return Err(InventoryError::MissingOwner);
        }
        if let Some(stream_owner) = owner.stream_owner
            && stream_owner.is_empty()
        {
            return Err(InventoryError::MissingStreamOwner);
        }
    }

    for expected in METHOD_OWNER_CONSTRAINTS {
        let key = (expected.service, expected.method);
        let Some(actual) = owners_by_key.get(&key) else {
            return Err(InventoryError::MissingMethodOwnerConstraint);
        };
        if *actual != &expected {
            return Err(InventoryError::MethodOwnerChanged);
        }
        let method = methods
            .iter()
            .find(|method| (method.service, method.method) == key)
            .ok_or(InventoryError::MissingMethodOwnerConstraint)?;
        if method.mode != RpcMode::Unary && actual.stream_owner.is_none() {
            return Err(InventoryError::MissingStreamOwner);
        }
    }
    Ok(())
}

fn descriptor_methods(bytes: &[u8]) -> Result<Vec<DescriptorMethod>, InventoryError> {
    let descriptor = FileDescriptorSet::decode(bytes).map_err(|_| InventoryError::TupleChanged)?;
    let mut methods = Vec::new();
    let mut services = BTreeSet::new();
    for file in descriptor.file {
        let package = file.package.as_deref().unwrap_or_default();
        for service in file.service {
            let service_name = service.name.unwrap_or_default();
            if service_name.is_empty() || package.is_empty() {
                return Err(InventoryError::UnknownDescriptorService);
            }
            if !services.insert(format!("{package}.{service_name}")) {
                return Err(InventoryError::DuplicateQualifiedMethod);
            }
            for method in service.method {
                let mode = RpcMode::from_descriptor(&method);
                let method_name = method.name.unwrap_or_default();
                let request = method.input_type.unwrap_or_default();
                let response = method.output_type.unwrap_or_default();
                if method_name.is_empty() || request.is_empty() || response.is_empty() {
                    return Err(InventoryError::TupleChanged);
                }
                methods.push(DescriptorMethod {
                    service: format!("{package}.{service_name}"),
                    method: method_name,
                    request,
                    response,
                    mode,
                });
            }
        }
    }
    if services != expected_current_service_keys() {
        return Err(InventoryError::UnknownDescriptorService);
    }
    Ok(methods)
}

fn expected_current_service_keys() -> BTreeSet<String> {
    [
        "dungeonflux.public.v1.ActionService",
        "dungeonflux.public.v1.SessionService",
        "dungeonflux.public.v1.RoomService",
        "dungeonflux.experimental.transport.v1.TransportFixture",
        "dungeonflux.public.v1.DialogueService",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn validate_current_descriptor(methods: &[DescriptorMethod]) -> Result<(), InventoryError> {
    let expected = expected_current_descriptor();
    let mut actual_by_key = BTreeMap::new();
    for method in methods {
        let key = (method.service.as_str(), method.method.as_str());
        if actual_by_key.insert(key, method).is_some() {
            return Err(InventoryError::DuplicateQualifiedMethod);
        }
    }
    if actual_by_key.len() != expected.len() {
        return Err(InventoryError::UnknownDescriptorService);
    }
    for expected_method in &expected {
        let key = (
            expected_method.service.as_str(),
            expected_method.method.as_str(),
        );
        let Some(actual) = actual_by_key.get(&key) else {
            return Err(InventoryError::MissingMethod);
        };
        if **actual != *expected_method {
            return Err(InventoryError::TupleChanged);
        }
    }
    Ok(())
}

fn expected_current_descriptor() -> Vec<DescriptorMethod> {
    let actual = |service: &str, method: &str, input: &str, output: &str, mode| DescriptorMethod {
        service: service.to_owned(),
        method: method.to_owned(),
        request: input.to_owned(),
        response: output.to_owned(),
        mode,
    };
    vec![
        actual(
            "dungeonflux.public.v1.ActionService",
            "Submit",
            ".dungeonflux.public.v1.SubmitActionRequest",
            ".dungeonflux.public.v1.SubmitActionResponse",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.public.v1.SessionService",
            "Watch",
            ".dungeonflux.public.v1.WatchViewRequest",
            ".dungeonflux.public.v1.ViewMessage",
            RpcMode::ServerStreaming,
        ),
        actual(
            "dungeonflux.public.v1.RoomService",
            "Join",
            ".dungeonflux.public.v1.JoinRoomRequest",
            ".dungeonflux.public.v1.JoinRoomResponse",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.experimental.transport.v1.TransportFixture",
            "Unary",
            ".dungeonflux.experimental.transport.v1.Sample",
            ".dungeonflux.experimental.transport.v1.Sample",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.experimental.transport.v1.TransportFixture",
            "ServerStream",
            ".dungeonflux.experimental.transport.v1.Sample",
            ".dungeonflux.experimental.transport.v1.Sample",
            RpcMode::ServerStreaming,
        ),
        actual(
            "dungeonflux.experimental.transport.v1.TransportFixture",
            "ClientStream",
            ".dungeonflux.experimental.transport.v1.Sample",
            ".dungeonflux.experimental.transport.v1.Sample",
            RpcMode::ClientStreaming,
        ),
        actual(
            "dungeonflux.experimental.transport.v1.TransportFixture",
            "Bidi",
            ".dungeonflux.experimental.transport.v1.Sample",
            ".dungeonflux.experimental.transport.v1.Sample",
            RpcMode::BidirectionalStreaming,
        ),
        actual(
            "dungeonflux.public.v1.DialogueService",
            "Submit",
            ".dungeonflux.public.v1.RawDialogueRequest",
            ".dungeonflux.public.v1.DialogueResponse",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.public.v1.DialogueService",
            "Confirm",
            ".dungeonflux.public.v1.ConfirmDialogueRequest",
            ".dungeonflux.public.v1.SubmitActionResponse",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.public.v1.DialogueService",
            "Cancel",
            ".dungeonflux.public.v1.CancelDialogueRequest",
            ".dungeonflux.public.v1.DialogueResponse",
            RpcMode::Unary,
        ),
        actual(
            "dungeonflux.public.v1.DialogueService",
            "Compose",
            ".dungeonflux.public.v1.ComposeRequest",
            ".dungeonflux.public.v1.ComposeResponse",
            RpcMode::Unary,
        ),
    ]
}

#[test]
fn original_twelve_services_and_forty_one_methods_have_typed_complete_contract_owners() {
    validate_full_contract(&SERVICE_OWNERS, &PLANNED_METHODS, &METHOD_OWNER_CONSTRAINTS).unwrap();
    assert_eq!(SERVICE_OWNERS.len(), 13);
    assert_eq!(PLANNED_METHODS.len(), 44);
    assert!(PLANNED_METHODS.iter().all(|method| {
        method.production_handler == ProductionHandlerEvidence::NotEstablishedByInventory
    }));
}

#[test]
fn method_specific_export_capture_stream_and_telemetry_owners_are_required() {
    validate_method_owner_constraints(&PLANNED_METHODS, &METHOD_OWNER_CONSTRAINTS).unwrap();

    let mut missing = METHOD_OWNER_CONSTRAINTS.to_vec();
    missing.retain(|owner| !(owner.service == "CustomerService" && owner.method == "Export"));
    assert_eq!(
        validate_method_owner_constraints(&PLANNED_METHODS, &missing),
        Err(InventoryError::WrongMethodOwnerConstraintCount)
    );

    for mutation in 0..6 {
        let mut changed = METHOD_OWNER_CONSTRAINTS.to_vec();
        match mutation {
            0 => changed[0].access_requirement = "payer status alone permits export",
            1 => changed[1].stream_owner = None,
            2 => changed[2].stream_owner = Some("recover without epoch or timeline"),
            3 => changed[3].access_requirement = "unscoped client capture upload",
            4 => {
                changed[4].request_owner = "generic source access with no trusted binding identity"
            }
            5 => {
                changed[5].request_owner =
                    "trusted source may query telemetry without operator authorization"
            }
            _ => unreachable!(),
        }
        assert_eq!(
            validate_method_owner_constraints(&PLANNED_METHODS, &changed),
            Err(InventoryError::MethodOwnerChanged),
            "mutation {mutation}"
        );
    }
}

#[test]
fn generated_descriptor_accounts_for_every_current_method_and_known_divergence() {
    let methods = descriptor_methods(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    validate_current_descriptor(&methods).unwrap();

    let action = methods
        .iter()
        .find(|method| method.service == "dungeonflux.public.v1.ActionService")
        .expect("the current ActionService Submit descriptor is explicitly inventoried");
    assert_eq!(action.method, "Submit");
    assert_eq!(
        action.response,
        ".dungeonflux.public.v1.SubmitActionResponse"
    );
    assert_ne!(action.response, ".dungeonflux.public.v1.DecisionReceipt");

    let room_join = methods
        .iter()
        .find(|method| method.service == "dungeonflux.public.v1.RoomService")
        .expect("RoomService.Join remains an explicit current divergence");
    assert_eq!(room_join.method, "Join");
    assert_eq!(room_join.request, ".dungeonflux.public.v1.JoinRoomRequest");
    assert_eq!(
        room_join.response,
        ".dungeonflux.public.v1.JoinRoomResponse"
    );
    assert!(!PLANNED_METHODS.iter().any(|method| {
        method.service == "SessionService"
            && method.method == "Join"
            && method.request == "JoinRoomRequest"
    }));

    let session_watch = methods
        .iter()
        .find(|method| method.service == "dungeonflux.public.v1.SessionService")
        .expect("the present SessionService.Watch tuple is inventoried");
    assert_eq!(session_watch.method, "Watch");
    assert_eq!(session_watch.mode, RpcMode::ServerStreaming);
    assert_eq!(
        session_watch.request,
        ".dungeonflux.public.v1.WatchViewRequest"
    );
    assert_eq!(session_watch.response, ".dungeonflux.public.v1.ViewMessage");
    assert_eq!(
        PLANNED_METHODS
            .iter()
            .filter(|method| method.service == "SessionService")
            .count(),
        8
    );

    let fixture_methods = methods
        .iter()
        .filter(|method| method.service == "dungeonflux.experimental.transport.v1.TransportFixture")
        .count();
    assert_eq!(fixture_methods, 4);
}

#[test]
fn missing_method_owner_duplicate_key_and_changed_planned_tuple_are_rejected() {
    let mut missing_method = PLANNED_METHODS.to_vec();
    missing_method.remove(0);
    assert_eq!(
        validate_contract(&SERVICE_OWNERS, &missing_method),
        Err(InventoryError::WrongMethodCount)
    );

    let mut missing_owner = SERVICE_OWNERS.to_vec();
    missing_owner.remove(0);
    assert_eq!(
        validate_contract(&missing_owner, &PLANNED_METHODS),
        Err(InventoryError::WrongServiceCount)
    );

    let mut owner_without_result = SERVICE_OWNERS.to_vec();
    owner_without_result[0].result_owner = "";
    assert_eq!(
        validate_contract(&owner_without_result, &PLANNED_METHODS),
        Err(InventoryError::MissingOwner)
    );

    let mut duplicate = PLANNED_METHODS.to_vec();
    duplicate[1] = duplicate[0];
    assert_eq!(
        validate_contract(&SERVICE_OWNERS, &duplicate),
        Err(InventoryError::DuplicateQualifiedMethod)
    );

    for mutation in 0..3 {
        let mut changed = PLANNED_METHODS.to_vec();
        match mutation {
            0 => changed[15].mode = RpcMode::ServerStreaming,
            1 => changed[15].request = "ForgedSubmitRequest",
            2 => changed[15].response = "SubmitActionResponse",
            _ => unreachable!(),
        }
        assert_eq!(
            validate_contract(&SERVICE_OWNERS, &changed),
            Err(InventoryError::TupleChanged),
            "mutation {mutation}"
        );
    }
}

#[test]
fn public_operator_namespace_missing_stream_owner_and_handler_claims_are_rejected() {
    let mut public_operator = SERVICE_OWNERS.to_vec();
    let debug = public_operator
        .iter_mut()
        .find(|owner| owner.service == "DebugService")
        .unwrap();
    debug.namespace = Namespace::Public;
    debug.access = AccessClass::PublicMemberInput;
    assert_eq!(
        validate_contract(&public_operator, &PLANNED_METHODS),
        Err(InventoryError::PublicOperatorClaim)
    );

    let mut ingestion_auth_for_operator_query = SERVICE_OWNERS.to_vec();
    let telemetry = ingestion_auth_for_operator_query
        .iter_mut()
        .find(|owner| owner.service == "TelemetryService")
        .unwrap();
    telemetry.request_owner = "df-telemetry accepts trusted source auth for operator query";
    assert_eq!(
        validate_contract(&ingestion_auth_for_operator_query, &PLANNED_METHODS),
        Err(InventoryError::MissingOwner)
    );

    let mut no_stream_owner = SERVICE_OWNERS.to_vec();
    let voice = no_stream_owner
        .iter_mut()
        .find(|owner| owner.service == "VoiceService")
        .unwrap();
    voice.stream_owner = None;
    assert_eq!(
        validate_contract(&no_stream_owner, &PLANNED_METHODS),
        Err(InventoryError::MissingStreamOwner)
    );

    let mut claimed_production = PLANNED_METHODS.to_vec();
    claimed_production[0].production_handler = ProductionHandlerEvidence::ClaimedImplemented;
    assert_eq!(
        validate_contract(&SERVICE_OWNERS, &claimed_production),
        Err(InventoryError::UnsupportedProductionHandlerClaim)
    );
}

#[test]
fn unclassified_room_method_and_forged_descriptor_completeness_are_rejected() {
    let mut room_removed = descriptor_methods(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    room_removed.retain(|method| method.service != "dungeonflux.public.v1.RoomService");
    assert_eq!(
        validate_current_descriptor(&room_removed),
        Err(InventoryError::UnknownDescriptorService)
    );

    let mut room_extended = descriptor_methods(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    room_extended.push(DescriptorMethod {
        service: "dungeonflux.public.v1.RoomService".to_owned(),
        method: "Invite".to_owned(),
        request: ".dungeonflux.public.v1.InviteRoomRequest".to_owned(),
        response: ".dungeonflux.public.v1.InviteRoomResponse".to_owned(),
        mode: RpcMode::Unary,
    });
    assert_eq!(
        validate_current_descriptor(&room_extended),
        Err(InventoryError::UnknownDescriptorService)
    );

    let mut forged_complete = descriptor_methods(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    for planned in PLANNED_METHODS {
        forged_complete.push(DescriptorMethod {
            service: format!("dungeonflux.public.v1.{}", planned.service),
            method: planned.method.to_owned(),
            request: format!(".dungeonflux.public.v1.{}", planned.request),
            response: format!(".dungeonflux.public.v1.{}", planned.response),
            mode: planned.mode,
        });
    }
    assert_eq!(
        validate_current_descriptor(&forged_complete),
        Err(InventoryError::DuplicateQualifiedMethod)
    );
}

#[test]
fn present_descriptor_tuple_mutations_fail_without_rewriting_the_generated_source() {
    let baseline = descriptor_methods(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    for mutation in 0..3 {
        let mut changed = baseline.clone();
        let target = if mutation == 2 { "Watch" } else { "Submit" };
        let method = changed
            .iter_mut()
            .find(|method| method.method == target)
            .expect("the targeted current method is present");
        match mutation {
            0 => method.mode = RpcMode::ServerStreaming,
            1 => method.request = ".dungeonflux.public.v1.ForgedRequest".to_owned(),
            2 => method.response = ".dungeonflux.public.v1.ForgedView".to_owned(),
            _ => unreachable!(),
        }
        assert_eq!(
            validate_current_descriptor(&changed),
            Err(InventoryError::TupleChanged),
            "mutation {mutation}"
        );
    }
}

#[test]
fn actual_descriptor_bytes_must_decode() {
    assert_eq!(
        descriptor_methods(&[10, 255]),
        Err(InventoryError::TupleChanged)
    );
}
