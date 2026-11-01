//! Trusted analysis route edits share one parser, cumulative merge and effective-route builder.
use super::{OverlayError, OverlaySource, Provenance, shared};
use crate::config::{AccountHandle, ApiEndpoint, ModelName};
use crate::request::AnalysisRoute;

/// Partial trusted route settings; absent fields inherit the lower configured route.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnalysisRouteFields {
    pub account: Option<AccountHandle>,
    pub model: Option<ModelName>,
    pub endpoint: Option<ApiEndpoint>,
    pub maximum_output_tokens: Option<u64>,
}
impl AnalysisRouteFields {
    fn merge(&mut self, next: &Self) {
        if let Some(value) = &next.account {
            self.account = Some(value.clone());
        }
        if let Some(value) = &next.model {
            self.model = Some(value.clone());
        }
        if let Some(value) = &next.endpoint {
            self.endpoint = Some(value.clone());
        }
        if let Some(value) = next.maximum_output_tokens {
            self.maximum_output_tokens = Some(value);
        }
    }
}
/// Clears a trusted route, or changes its supplied nonsecret fields atomically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisRoutePatch {
    Clear,
    Fields(AnalysisRouteFields),
}
impl From<AnalysisRoute> for AnalysisRoutePatch {
    fn from(route: AnalysisRoute) -> Self {
        Self::Fields(AnalysisRouteFields {
            account: Some(route.account),
            model: Some(route.model),
            endpoint: Some(route.endpoint),
            maximum_output_tokens: Some(route.maximum_output_tokens),
        })
    }
}
impl AnalysisRoutePatch {
    pub(crate) fn merge(&mut self, next: &Self) {
        match (self, next) {
            (Self::Fields(prior), Self::Fields(next)) => prior.merge(next),
            (prior, next) => *prior = next.clone(),
        }
    }
}

pub(super) fn route_slot<'a>(
    patch: &'a mut super::RuntimePatch,
    stage: &str,
    path: &str,
) -> Result<&'a mut Option<AnalysisRoutePatch>, OverlayError> {
    match stage {
        "classification" => Ok(&mut patch.analysis_classification),
        "inspection" => Ok(&mut patch.analysis_inspection),
        "head_locations" => Ok(&mut patch.analysis_head_locations),
        "staging_identity" => Ok(&mut patch.analysis_staging_identity),
        "staging_facing" => Ok(&mut patch.analysis_staging_facing),
        _ => Err(OverlayError::UnknownField {
            field: path.to_owned(),
        }),
    }
}
pub(super) fn parse_slot(
    slot: &mut Option<AnalysisRoutePatch>,
    field: Option<&str>,
    path: &str,
    value: &str,
) -> Result<(), OverlayError> {
    let Some(field) = field else {
        if value != "none" {
            return Err(shared::invalid(
                path,
                "a route value is none; set named route fields to configure it",
            ));
        }
        *slot = Some(AnalysisRoutePatch::Clear);
        return Ok(());
    };
    let mut fields = match slot {
        Some(AnalysisRoutePatch::Fields(fields)) => fields.clone(),
        _ => AnalysisRouteFields::default(),
    };
    match field {
        "account" => fields.account = Some(super::need_account(path, value)?),
        "model" => fields.model = Some(super::need_model(path, value)?),
        "endpoint" => fields.endpoint = Some(super::need_endpoint(path, value)?),
        "maximum_output_tokens" => {
            fields.maximum_output_tokens = Some(super::need_u64(path, value)?)
        }
        _ => {
            return Err(OverlayError::UnknownField {
                field: path.to_owned(),
            });
        }
    }
    *slot = Some(AnalysisRoutePatch::Fields(fields));
    Ok(())
}

pub(super) fn apply_route(
    slot: &mut Option<AnalysisRoute>,
    patch: &AnalysisRoutePatch,
    stage: &str,
    provenance: &mut Provenance,
    source: impl Fn(&str) -> OverlaySource,
) -> Result<(), OverlayError> {
    let path = |field: &str| {
        if field.is_empty() {
            format!("analysis.{stage}")
        } else {
            format!("analysis.{stage}.{field}")
        }
    };
    let AnalysisRoutePatch::Fields(fields) = patch else {
        *slot = None;
        for field in ["", "account", "model", "endpoint", "maximum_output_tokens"] {
            shared::note(provenance, &path(field), source(field));
        }
        return Ok(());
    };
    let prior = slot.as_ref();
    let account = fields
        .account
        .clone()
        .or_else(|| prior.map(|route| route.account.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("account"),
                "a configured analysis route requires an account",
            )
        })?;
    let model = fields
        .model
        .clone()
        .or_else(|| prior.map(|route| route.model.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("model"),
                "a configured analysis route requires a model",
            )
        })?;
    let endpoint = fields
        .endpoint
        .clone()
        .or_else(|| prior.map(|route| route.endpoint.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("endpoint"),
                "a configured analysis route requires an endpoint",
            )
        })?;
    let maximum_output_tokens = fields
        .maximum_output_tokens
        .or_else(|| prior.map(|route| route.maximum_output_tokens))
        .ok_or_else(|| {
            shared::invalid(
                &path("maximum_output_tokens"),
                "a configured analysis route requires an output token limit",
            )
        })?;
    *slot = Some(AnalysisRoute {
        account,
        model,
        endpoint,
        maximum_output_tokens,
    });
    shared::note(provenance, &path(""), source(""));
    for (field, edited) in [
        ("account", fields.account.is_some()),
        ("model", fields.model.is_some()),
        ("endpoint", fields.endpoint.is_some()),
        (
            "maximum_output_tokens",
            fields.maximum_output_tokens.is_some(),
        ),
    ] {
        if edited {
            shared::note(provenance, &path(field), source(field));
        }
    }
    Ok(())
}
