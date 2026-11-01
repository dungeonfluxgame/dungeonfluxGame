//! Trusted Fal segmentation route edits keep queue provider semantics separate from text analysis.
use super::{OverlayError, OverlaySource, Provenance, shared};
use crate::config::{
    AccountHandle, ApiEndpoint, ModelName, SegmentationProvider, SegmentationRoute,
};

/// Partial segmentation route settings; absent fields inherit the trusted lower route.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SegmentationRouteFields {
    pub provider: Option<SegmentationProvider>,
    pub account: Option<AccountHandle>,
    pub model: Option<ModelName>,
    pub endpoint: Option<ApiEndpoint>,
}
/// Clears the entire segmentation route or atomically applies its supplied fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SegmentationRoutePatch {
    Clear,
    Fields(SegmentationRouteFields),
}
impl From<SegmentationRoute> for SegmentationRoutePatch {
    fn from(route: SegmentationRoute) -> Self {
        Self::Fields(SegmentationRouteFields {
            provider: Some(route.provider),
            account: Some(route.account),
            model: Some(route.model),
            endpoint: Some(route.endpoint),
        })
    }
}
impl SegmentationRoutePatch {
    pub(crate) fn merge(&mut self, next: &Self) {
        match (self, next) {
            (Self::Fields(prior), Self::Fields(next)) => {
                if next.provider.is_some() {
                    prior.provider = next.provider;
                }
                if let Some(value) = &next.account {
                    prior.account = Some(value.clone());
                }
                if let Some(value) = &next.model {
                    prior.model = Some(value.clone());
                }
                if let Some(value) = &next.endpoint {
                    prior.endpoint = Some(value.clone());
                }
            }
            (prior, next) => *prior = next.clone(),
        }
    }
}
pub(super) fn parse_slot(
    slot: &mut Option<SegmentationRoutePatch>,
    field: Option<&str>,
    path: &str,
    value: &str,
) -> Result<(), OverlayError> {
    let Some(field) = field else {
        if value != "none" {
            return Err(shared::invalid(
                path,
                "a route value is none; configure named segmentation fields",
            ));
        }
        *slot = Some(SegmentationRoutePatch::Clear);
        return Ok(());
    };
    let mut fields = match slot {
        Some(SegmentationRoutePatch::Fields(fields)) => fields.clone(),
        _ => SegmentationRouteFields::default(),
    };
    match field {
        "provider" => {
            fields.provider = Some(match value {
                "fal" => SegmentationProvider::Fal,
                _ => {
                    return Err(shared::invalid(
                        path,
                        "the configured segmentation provider is fal",
                    ));
                }
            })
        }
        "account" => fields.account = Some(super::need_account(path, value)?),
        "model" => fields.model = Some(super::need_model(path, value)?),
        "endpoint" => fields.endpoint = Some(super::need_endpoint(path, value)?),
        _ => {
            return Err(OverlayError::UnknownField {
                field: path.to_owned(),
            });
        }
    }
    *slot = Some(SegmentationRoutePatch::Fields(fields));
    Ok(())
}
pub(super) fn apply_route(
    slot: &mut Option<SegmentationRoute>,
    patch: &SegmentationRoutePatch,
    provenance: &mut Provenance,
    source: impl Fn(&str) -> OverlaySource,
) -> Result<(), OverlayError> {
    let path = |field: &str| {
        if field.is_empty() {
            "analysis.segmentation".to_owned()
        } else {
            format!("analysis.segmentation.{field}")
        }
    };
    let SegmentationRoutePatch::Fields(fields) = patch else {
        *slot = None;
        for field in ["", "provider", "account", "model", "endpoint"] {
            shared::note(provenance, &path(field), source(field));
        }
        return Ok(());
    };
    let prior = slot.as_ref();
    let provider = fields
        .provider
        .or_else(|| prior.map(|route| route.provider))
        .ok_or_else(|| {
            shared::invalid(
                &path("provider"),
                "a configured segmentation route requires a provider",
            )
        })?;
    let account = fields
        .account
        .clone()
        .or_else(|| prior.map(|route| route.account.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("account"),
                "a configured segmentation route requires an account",
            )
        })?;
    let model = fields
        .model
        .clone()
        .or_else(|| prior.map(|route| route.model.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("model"),
                "a configured segmentation route requires a model",
            )
        })?;
    let endpoint = fields
        .endpoint
        .clone()
        .or_else(|| prior.map(|route| route.endpoint.clone()))
        .ok_or_else(|| {
            shared::invalid(
                &path("endpoint"),
                "a configured segmentation route requires an endpoint",
            )
        })?;
    *slot = Some(SegmentationRoute {
        provider,
        account,
        model,
        endpoint,
    });
    shared::note(provenance, &path(""), source(""));
    for (field, edited) in [
        ("provider", fields.provider.is_some()),
        ("account", fields.account.is_some()),
        ("model", fields.model.is_some()),
        ("endpoint", fields.endpoint.is_some()),
    ] {
        if edited {
            shared::note(provenance, &path(field), source(field));
        }
    }
    Ok(())
}
