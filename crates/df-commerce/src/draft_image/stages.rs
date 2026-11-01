//! One explicit creator consent covers a closed set of paid routes for the whole job.

use super::{
    AssetId, ConsentId, DraftId, JobId, PublicationId, Refusal, Scope, StageId, account_serde,
};
use crate::digest::business_key;
use df_types::AccountId;
use serde::{Deserialize, Serialize};

/// Maximum pinned routes in one creator job grant.
pub const MAXIMUM_STAGE_ROUTES: usize = 16;

/// Local transforms have no supplier route and cannot be funded by this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidStageKind {
    Classification,
    Generation,
    Edit,
    Inspection,
    Correction,
    Extraction,
    PhotoInspection,
    HeadLocations,
    Segmentation,
    StagingIdentity,
    StagingFacing,
}
impl PaidStageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classification => "classification",
            Self::Generation => "generation",
            Self::Edit => "edit",
            Self::Inspection => "inspection",
            Self::Correction => "correction",
            Self::Extraction => "extraction",
            Self::PhotoInspection => "photo_inspection",
            Self::HeadLocations => "head_locations",
            Self::Segmentation => "segmentation",
            Self::StagingIdentity => "staging_identity",
            Self::StagingFacing => "staging_facing",
        }
    }
    pub fn is_standalone(self) -> bool {
        matches!(
            self,
            Self::PhotoInspection
                | Self::HeadLocations
                | Self::Segmentation
                | Self::StagingIdentity
                | Self::StagingFacing
        )
    }
}

/// Exact approved route identity; maxima may decrease on a particular dispatch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRoute {
    pub key: [u8; 32],
    pub kind: PaidStageKind,
    pub provider: String,
    pub model: String,
    pub route: String,
    pub cause: String,
    pub detail: String,
    pub price_version: [u8; 16],
    pub maximum_units: Vec<(String, u64)>,
}
impl StageRoute {
    pub fn validate(&self) -> Result<(), Refusal> {
        if self.key == [0; 32]
            || self.price_version == [0; 16]
            || [
                &self.provider,
                &self.model,
                &self.route,
                &self.cause,
                &self.detail,
            ]
            .iter()
            .any(|label| label.is_empty() || label.len() > super::MAXIMUM_LABEL_BYTES)
        {
            return Err(Refusal::InvalidBinding);
        }
        if self.maximum_units.is_empty()
            || self.maximum_units.len() > super::MAXIMUM_UNIT_CATEGORIES
            || self.maximum_units.iter().any(|(category, units)| {
                category.is_empty() || category.len() > super::MAXIMUM_LABEL_BYTES || *units == 0
            })
            || self
                .maximum_units
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(Refusal::InvalidUnits);
        }
        Ok(())
    }

    pub fn digest(&self) -> [u8; 32] {
        let mut parts = vec![
            self.key.to_vec(),
            self.kind.label().as_bytes().to_vec(),
            self.provider.as_bytes().to_vec(),
            self.model.as_bytes().to_vec(),
            self.route.as_bytes().to_vec(),
            self.cause.as_bytes().to_vec(),
            self.detail.as_bytes().to_vec(),
            self.price_version.to_vec(),
        ];
        for (category, maximum) in &self.maximum_units {
            parts.push(category.as_bytes().to_vec());
            parts.push(maximum.to_be_bytes().to_vec());
        }
        let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        business_key("df-commerce/image-stage-route-v1", &refs)
    }
    pub fn allows(&self, actual: &Self) -> bool {
        self.key == actual.key
            && self.kind == actual.kind
            && self.provider == actual.provider
            && self.model == actual.model
            && self.route == actual.route
            && self.cause == actual.cause
            && self.detail == actual.detail
            && self.price_version == actual.price_version
            && self.maximum_units.len() == actual.maximum_units.len()
            && self.maximum_units.iter().zip(&actual.maximum_units).all(
                |((category, maximum), (actual_category, units))| {
                    category == actual_category && *units > 0 && units <= maximum
                },
            )
    }
}

/// Authenticated source and configuration pins shared by every child of the job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobScope {
    #[serde(with = "account_serde")]
    pub account: AccountId,
    pub draft: DraftId,
    pub draft_revision: u64,
    pub job: JobId,
    pub source_digest: [u8; 32],
    pub config_revision: u64,
    pub config_digest: [u8; 32],
    pub routes: Vec<StageRoute>,
}
impl JobScope {
    pub fn validate(&self, account: AccountId) -> Result<(), Refusal> {
        if self.config_revision == 0
            || self.routes.is_empty()
            || self.routes.len() > MAXIMUM_STAGE_ROUTES
            || self
                .routes
                .windows(2)
                .any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(Refusal::InvalidBinding);
        }
        for route in &self.routes {
            route.validate()?;
            if route.key == [0; 32]
                || route.cause.is_empty()
                || route.cause.len() > super::MAXIMUM_LABEL_BYTES
                || route.detail.len() > super::MAXIMUM_LABEL_BYTES
            {
                return Err(Refusal::InvalidBinding);
            }
            self.selected_scope(route).validate(account)?;
        }
        Ok(())
    }
    pub fn selected_scope(&self, route: &StageRoute) -> Scope {
        Scope {
            account: self.account,
            draft: self.draft,
            draft_revision: self.draft_revision,
            job: self.job,
            source_digest: self.source_digest,
            config_digest: self.config_digest,
            provider: route.provider.clone(),
            model: route.model.clone(),
            route: route.route.clone(),
            price_version: route.price_version,
            maximum_units: route.maximum_units.clone(),
        }
    }
    pub fn allows(&self, route: &StageRoute) -> bool {
        self.routes.iter().any(|approved| approved.allows(route))
    }
    pub fn digest(&self) -> [u8; 32] {
        // The closed serialized shape has deterministic struct order and no maps.
        let mut parts = vec![
            self.account.as_bytes().to_vec(),
            self.draft.0.to_vec(),
            self.draft_revision.to_be_bytes().to_vec(),
            self.job.0.to_vec(),
            self.source_digest.to_vec(),
            self.config_revision.to_be_bytes().to_vec(),
            self.config_digest.to_vec(),
        ];
        for route in &self.routes {
            parts.push(route.key.to_vec());
            parts.push(route.kind.label().as_bytes().to_vec());
            parts.push(self.selected_scope(route).digest().to_vec());
            parts.push(route.cause.as_bytes().to_vec());
            parts.push(route.detail.as_bytes().to_vec());
        }
        let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        business_key("df-commerce/draft-job-scope-v1", &refs)
    }
}

/// A single spending cap and expiry over all charged and held routes in this job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobConsent {
    pub id: ConsentId,
    pub scope: JobScope,
    pub cap_micro_dollars: u64,
    pub expires_at_ms: i64,
    pub revision: u64,
}
impl JobConsent {
    pub fn validate(&self, account: AccountId, now_ms: i64) -> Result<(), Refusal> {
        self.scope.validate(account)?;
        if self.id.0 == [0; 16]
            || self.revision != 1
            || self.cap_micro_dollars == 0
            || self.cap_micro_dollars > i64::MAX as u64
        {
            return Err(Refusal::InvalidConsent);
        }
        if self.expires_at_ms <= now_ms {
            return Err(Refusal::ConsentExpired);
        }
        Ok(())
    }
}

/// A real planned media identity; no analytical stage invents a pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagePublication {
    pub asset: AssetId,
    pub publication: PublicationId,
}
impl StagePublication {
    pub fn validate(&self) -> Result<(), Refusal> {
        if self.asset.0 == [0; 16] || self.publication.0 == [0; 16] {
            Err(Refusal::InvalidBinding)
        } else {
            Ok(())
        }
    }
}

/// Stage money identity preserves the strict legacy image binding while allowing genuinely no media.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(into = "StageBindingWire", try_from = "StageBindingWire")]
pub struct StageBinding {
    pub scope: Scope,
    pub stage: StageId,
    pub planned_publication: Option<StagePublication>,
    pub operation: [u8; 16],
    pub attempt: [u8; 16],
    pub effect: [u8; 16],
    pub payload_digest: [u8; 32],
}
impl StageBinding {
    pub fn validate(&self, account: AccountId) -> Result<(), Refusal> {
        self.scope.validate(account)?;
        if [self.stage.0, self.operation, self.attempt, self.effect].contains(&[0; 16]) {
            return Err(Refusal::InvalidBinding);
        }
        if let Some(publication) = self.planned_publication {
            publication.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageBindingWire {
    scope: Scope,
    stage: StageId,
    asset: Option<AssetId>,
    publication: Option<PublicationId>,
    operation: [u8; 16],
    attempt: [u8; 16],
    effect: [u8; 16],
    payload_digest: [u8; 32],
}
fn publication_pair(
    asset: Option<AssetId>,
    publication: Option<PublicationId>,
) -> Result<Option<StagePublication>, &'static str> {
    match (asset, publication) {
        (None, None) => Ok(None),
        (Some(asset), Some(publication)) => {
            let identity = StagePublication { asset, publication };
            identity
                .validate()
                .map_err(|_| "invalid publication identity")?;
            Ok(Some(identity))
        }
        _ => Err("partial publication identity"),
    }
}
impl From<StageBinding> for StageBindingWire {
    fn from(value: StageBinding) -> Self {
        Self {
            scope: value.scope,
            stage: value.stage,
            asset: value.planned_publication.map(|p| p.asset),
            publication: value.planned_publication.map(|p| p.publication),
            operation: value.operation,
            attempt: value.attempt,
            effect: value.effect,
            payload_digest: value.payload_digest,
        }
    }
}
impl TryFrom<StageBindingWire> for StageBinding {
    type Error = &'static str;
    fn try_from(value: StageBindingWire) -> Result<Self, Self::Error> {
        Ok(Self {
            scope: value.scope,
            stage: value.stage,
            planned_publication: publication_pair(value.asset, value.publication)?,
            operation: value.operation,
            attempt: value.attempt,
            effect: value.effect,
            payload_digest: value.payload_digest,
        })
    }
}

/// Stable execution basis. Claim leases are checked transactionally and never fingerprinted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(into = "StageCallWire", try_from = "StageCallWire")]
pub struct StageCall {
    pub job: JobId,
    pub stage: StageId,
    pub dispatch: [u8; 16],
    pub parent_dispatch: Option<[u8; 16]>,
    pub kind: PaidStageKind,
    pub input_revision: u32,
    pub input_digest: [u8; 32],
    pub config_revision: u64,
    pub config_digest: [u8; 32],
    pub planned_publication: Option<StagePublication>,
    pub payload_digest: [u8; 32],
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageCallWire {
    job: JobId,
    stage: StageId,
    dispatch: [u8; 16],
    parent_dispatch: Option<[u8; 16]>,
    kind: PaidStageKind,
    input_revision: u32,
    input_digest: [u8; 32],
    config_revision: u64,
    config_digest: [u8; 32],
    asset: Option<AssetId>,
    publication: Option<PublicationId>,
    payload_digest: [u8; 32],
}
impl From<StageCall> for StageCallWire {
    fn from(value: StageCall) -> Self {
        Self {
            job: value.job,
            stage: value.stage,
            dispatch: value.dispatch,
            parent_dispatch: value.parent_dispatch,
            kind: value.kind,
            input_revision: value.input_revision,
            input_digest: value.input_digest,
            config_revision: value.config_revision,
            config_digest: value.config_digest,
            asset: value.planned_publication.map(|p| p.asset),
            publication: value.planned_publication.map(|p| p.publication),
            payload_digest: value.payload_digest,
        }
    }
}
impl TryFrom<StageCallWire> for StageCall {
    type Error = &'static str;
    fn try_from(value: StageCallWire) -> Result<Self, Self::Error> {
        let planned_publication = publication_pair(value.asset, value.publication)?;
        if value.kind.is_standalone() != planned_publication.is_none() {
            return Err("publication kind differs");
        }
        Ok(Self {
            job: value.job,
            stage: value.stage,
            dispatch: value.dispatch,
            parent_dispatch: value.parent_dispatch,
            kind: value.kind,
            input_revision: value.input_revision,
            input_digest: value.input_digest,
            config_revision: value.config_revision,
            config_digest: value.config_digest,
            planned_publication,
            payload_digest: value.payload_digest,
        })
    }
}
impl StageCall {
    pub fn validate_identity(&self) -> Result<(), Refusal> {
        if self.input_revision == 0
            || self.config_revision == 0
            || [self.job.0, self.stage.0, self.dispatch].contains(&[0; 16])
            || self.kind.is_standalone() != self.planned_publication.is_none()
            || self.parent_dispatch == Some([0; 16])
            || self.parent_dispatch == Some(self.dispatch)
        {
            return Err(Refusal::InvalidBinding);
        }
        if let Some(publication) = self.planned_publication {
            publication.validate()?;
        }
        Ok(())
    }
    pub fn validate(&self, scope: &JobScope, route: &StageRoute) -> Result<(), Refusal> {
        self.validate_identity()?;
        if self.job != scope.job
            || self.kind != route.kind
            || self.config_revision != scope.config_revision
            || self.config_digest != scope.config_digest
            || !scope.allows(route)
        {
            return Err(Refusal::InvalidBinding);
        }
        scope.selected_scope(route).validate(scope.account)
    }
    pub fn digest(&self) -> [u8; 32] {
        let publication = self.planned_publication.as_ref();
        business_key(
            if publication.is_some() {
                "df-commerce/draft-stage-call-v1"
            } else {
                "df-commerce/draft-stage-call-v2"
            },
            &[
                &self.job.0,
                &self.stage.0,
                &self.dispatch,
                self.parent_dispatch
                    .as_ref()
                    .map_or(&[], |id| id.as_slice()),
                self.kind.label().as_bytes(),
                &self.input_revision.to_be_bytes(),
                &self.input_digest,
                &self.config_revision.to_be_bytes(),
                &self.config_digest,
                publication.map_or(&[], |p| p.asset.0.as_slice()),
                publication.map_or(&[], |p| p.publication.0.as_slice()),
                &self.payload_digest,
            ],
        )
    }
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;
