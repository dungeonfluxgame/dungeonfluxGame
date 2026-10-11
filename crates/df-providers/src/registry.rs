use std::fmt;

const OBSERVED_ON: &str = "2026-10-10";

/// The two routes selected for this bounded qualification record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateRouteId {
    ElevenFlashV25Tts,
    FalFluxSchnellDisposableImage,
    OpenAiResponsesText,
    ElevenScribeV2Stt,
}

/// Explicit source-qualified speech route factory, separate from the dated
/// two-candidate pricing record. It does not admit account rights or spend.
#[cfg(not(target_arch = "wasm32"))]
pub fn registered_speech_provider(
    route: CandidateRouteId,
    credential: &str,
    limits: crate::NativeHttpLimits,
) -> Result<crate::NativeSpeechProvider, crate::SpeechAdapterError> {
    crate::NativeSpeechProvider::new(route, credential, limits)
}

/// Constructs the actual native Responses adapter from explicit caller configuration.
/// This factory neither qualifies account rights nor admits spend. The historical
/// dated pricing registry remains separate from this configured transport route.
#[cfg(not(target_arch = "wasm32"))]
pub fn registered_text_provider(
    route: CandidateRouteId,
    config: crate::TextProviderConfig,
    credential: &str,
    limits: crate::NativeHttpLimits,
) -> Result<crate::NativeTextProvider, crate::TextResponseError> {
    if route != CandidateRouteId::OpenAiResponsesText {
        return Err(crate::TextResponseError::UnregisteredRoute);
    }
    crate::NativeTextProvider::new(config, credential, limits)
}

/// Capability covered by one selected route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Text,
    Stt,
    TextToSpeech,
    Image,
    Video,
    Sound,
    DisposableImage,
}

/// Public supplier billing unit, without implying a measured invoice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BillingUnit {
    ThousandCharacters,
    Megapixel,
}

/// Published rounding behavior for a rate, where the supplier states one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoundingRule {
    NonePublished,
    RoundUpToWholeMegapixels,
}

/// One dated vendor source used for an evidence statement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceReference {
    observed_on: &'static str,
    url: &'static str,
}

impl EvidenceReference {
    pub const fn observed_on(self) -> &'static str {
        self.observed_on
    }

    pub const fn url(self) -> &'static str {
        self.url
    }
}

/// Published USD rate represented exactly in microdollars.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishedRate {
    amount_micros: u64,
    unit: BillingUnit,
    rounding: RoundingRule,
    evidence: EvidenceReference,
}

impl PublishedRate {
    pub const fn amount_micros(self) -> u64 {
        self.amount_micros
    }

    pub const fn unit(self) -> BillingUnit {
        self.unit
    }

    pub const fn rounding(self) -> RoundingRule {
        self.rounding
    }

    pub const fn evidence(self) -> EvidenceReference {
        self.evidence
    }
}

/// Narrow observation of a published license or commercial-use statement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LicenseEvidence {
    summary: &'static str,
    evidence: EvidenceReference,
}

impl LicenseEvidence {
    pub const fn summary(self) -> &'static str {
        self.summary
    }

    pub const fn evidence(self) -> EvidenceReference {
        self.evidence
    }
}

/// Independent current qualification gate; documentation alone cannot pass it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    AccountEntitlement,
    RegionAvailability,
    Capacity,
    CurrentRateEntitlement,
    RightsApproval,
}

/// Explicit evidence state supplied for a qualification gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateStatus {
    Pending,
    Failed,
    Unperformed,
    Unsupported,
    Qualified,
}

/// The first non-qualified gate that prevents route dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchBlock {
    gate: Gate,
    status: GateStatus,
}

/// Capability with no selected route in this bounded registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedCapability {
    capability: Capability,
}

impl UnsupportedCapability {
    pub const fn capability(self) -> Capability {
        self.capability
    }
}

impl DispatchBlock {
    pub const fn gate(self) -> Gate {
        self.gate
    }

    pub const fn status(self) -> GateStatus {
        self.status
    }
}

/// Evidence for a selected supplier candidate, not a transport request or grant.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CandidateRoute {
    id: CandidateRouteId,
    capability: Capability,
    supplier: &'static str,
    model: &'static str,
    endpoint: &'static str,
    route_evidence: EvidenceReference,
    availability_evidence: EvidenceReference,
    published_rate: PublishedRate,
    license: LicenseEvidence,
    gates: [GateStatus; 5],
}

impl fmt::Debug for CandidateRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateRoute")
            .field("id", &self.id)
            .field("capability", &self.capability)
            .field("supplier", &self.supplier)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("route_evidence", &self.route_evidence)
            .field("published_rate", &self.published_rate)
            .field("license", &self.license)
            .field("dispatchable", &false)
            .finish()
    }
}

impl CandidateRoute {
    pub const fn id(self) -> CandidateRouteId {
        self.id
    }

    pub const fn capability(self) -> Capability {
        self.capability
    }

    pub const fn supplier(self) -> &'static str {
        self.supplier
    }

    pub const fn model(self) -> &'static str {
        self.model
    }

    pub const fn endpoint(self) -> &'static str {
        self.endpoint
    }

    pub const fn route_evidence(self) -> EvidenceReference {
        self.route_evidence
    }

    pub const fn availability_evidence(self) -> EvidenceReference {
        self.availability_evidence
    }

    pub const fn published_rate(self) -> PublishedRate {
        self.published_rate
    }

    pub const fn license(self) -> LicenseEvidence {
        self.license
    }

    /// Returns the first unresolved gate in stable policy order.
    pub const fn dispatch_block(self) -> Result<(), DispatchBlock> {
        assess_dispatch_gates(self.gates)
    }
}

/// Classifies caller-supplied gate observations without establishing their truth.
///
/// Only the owner of current account, capacity, price and rights evidence can
/// provide qualified observations. This function neither authorizes nor dispatches.
pub const fn assess_dispatch_gates(statuses: [GateStatus; 5]) -> Result<(), DispatchBlock> {
    let gates = [
        Gate::AccountEntitlement,
        Gate::RegionAvailability,
        Gate::Capacity,
        Gate::CurrentRateEntitlement,
        Gate::RightsApproval,
    ];
    let mut index = 0;
    while index < gates.len() {
        if !matches!(statuses[index], GateStatus::Qualified) {
            return Err(DispatchBlock {
                gate: gates[index],
                status: statuses[index],
            });
        }
        index += 1;
    }
    Ok(())
}

const ELEVEN_RATE: PublishedRate = PublishedRate {
    amount_micros: 40_000,
    unit: BillingUnit::ThousandCharacters,
    rounding: RoundingRule::NonePublished,
    evidence: EvidenceReference {
        observed_on: OBSERVED_ON,
        url: "https://elevenlabs.io/pricing/api",
    },
};

const ELEVEN_LICENSE: LicenseEvidence = LicenseEvidence {
    summary: "Paid-plan commercial use is subject to the customer terms and excludes Beta Services; the exact account, entity, voice and product rights remain unqualified.",
    evidence: EvidenceReference {
        observed_on: OBSERVED_ON,
        url: "https://elevenlabs.io/terms-of-use",
    },
};

const FAL_RATE: PublishedRate = PublishedRate {
    amount_micros: 3_000,
    unit: BillingUnit::Megapixel,
    rounding: RoundingRule::RoundUpToWholeMegapixels,
    evidence: EvidenceReference {
        observed_on: OBSERVED_ON,
        url: "https://fal.ai/models/fal-ai/flux/schnell",
    },
};

const FAL_LICENSE: LicenseEvidence = LicenseEvidence {
    summary: "The model page labels commercial use; fal terms retain customer responsibility for rights and provide no originality or noninfringement guarantee.",
    evidence: EvidenceReference {
        observed_on: OBSERVED_ON,
        url: "https://fal.ai/legal/terms-of-service",
    },
};

const CANDIDATES: [CandidateRoute; 2] = [
    CandidateRoute {
        id: CandidateRouteId::ElevenFlashV25Tts,
        capability: Capability::TextToSpeech,
        supplier: "ElevenLabs",
        model: "eleven_flash_v2_5",
        endpoint: "POST https://api.elevenlabs.io/v1/text-to-speech/:voice_id/stream",
        route_evidence: EvidenceReference {
            observed_on: OBSERVED_ON,
            url: "https://elevenlabs.io/docs/api-reference/text-to-speech/stream",
        },
        availability_evidence: EvidenceReference {
            observed_on: OBSERVED_ON,
            url: "https://elevenlabs.io/elevenapi-terms",
        },
        published_rate: ELEVEN_RATE,
        license: ELEVEN_LICENSE,
        gates: [
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Pending,
        ],
    },
    CandidateRoute {
        id: CandidateRouteId::FalFluxSchnellDisposableImage,
        capability: Capability::DisposableImage,
        supplier: "fal",
        model: "fal-ai/flux/schnell",
        endpoint: "fal model endpoint fal-ai/flux/schnell",
        route_evidence: EvidenceReference {
            observed_on: OBSERVED_ON,
            url: "https://fal.ai/models/fal-ai/flux/schnell/api",
        },
        availability_evidence: EvidenceReference {
            observed_on: OBSERVED_ON,
            url: "https://fal.ai/legal/api-services",
        },
        published_rate: FAL_RATE,
        license: FAL_LICENSE,
        gates: [
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Unperformed,
            GateStatus::Pending,
        ],
    },
];

/// Returns the two dated route candidates. Neither is qualified for dispatch.
pub const fn candidate_routes() -> &'static [CandidateRoute; 2] {
    &CANDIDATES
}

/// Looks up the selected route for a capability or returns an explicit refusal.
pub const fn candidate_for(
    capability: Capability,
) -> Result<&'static CandidateRoute, UnsupportedCapability> {
    match capability {
        Capability::TextToSpeech => Ok(&CANDIDATES[0]),
        Capability::DisposableImage => Ok(&CANDIDATES[1]),
        _ => Err(UnsupportedCapability { capability }),
    }
}
