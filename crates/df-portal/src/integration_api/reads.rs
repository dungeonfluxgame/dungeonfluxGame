//! Typed audience-safe projections. HTTP never queries owner tables.
use super::*;
use axum::extract::{Query, State};
use df_types::AccountId;
use serde::Serialize;
use std::future::Future;

#[derive(Serialize)]
pub struct CampaignSummary {
    pub campaign_id: String,
    pub name: String,
    pub role: df_account_app::wire_campaign::CampaignRole,
}
#[derive(Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub campaign_id: String,
    pub campaign_name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_seconds: u64,
    pub players: u32,
    pub role: df_account_app::wire_campaign::CampaignRole,
}
#[derive(Serialize)]
pub struct FundingSummary {
    pub purchased_available_micro_dollars: u64,
    pub promotional_available_micro_dollars: u64,
    pub allocated_micro_dollars: u64,
    pub reserved_micro_dollars: u64,
    pub trial_granted_micro_dollars: Option<u64>,
    pub trial_remaining_micro_dollars: Option<u64>,
}

pub struct Page<T> {
    pub rows: Vec<T>,
    pub next: Option<TokenId>,
}
#[derive(Clone, Copy, Debug)]
pub struct ReadFailure;

pub trait IntegrationReads: Send + Sync + 'static {
    fn campaigns(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> impl Future<Output = Result<Page<CampaignSummary>, ReadFailure>> + Send;
    fn sessions(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> impl Future<Output = Result<Page<SessionSummary>, ReadFailure>> + Send;
    fn funding(
        &self,
        account: AccountId,
    ) -> impl Future<Output = Result<FundingSummary, ReadFailure>> + Send;
}

pub(super) async fn campaigns<
    S: AccountStore + PersonalTokenStore,
    M: EmailSender,
    R: IntegrationReads,
>(
    State(state): State<ApiState<S, M, R>>,
    h: HeaderMap,
    query: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Failure> {
    let request = request_id();
    let principal = authorize(&state, &h, Scope::PlaySessions, &request).await?;
    let Query(paging) = query.map_err(|_| Failure::invalid(&request))?;
    let now = state.service.now_ms();
    let (limit, after) = paging.checked(
        &state.peppers,
        principal.account.id.as_bytes(),
        "campaigns",
        now,
        &request,
    )?;
    let page = state
        .reads
        .campaigns(principal.account.id, after, limit)
        .await
        .map_err(|_| Failure::unavailable(&request))?;
    let next = page.next.map(|id| {
        state
            .peppers
            .cursor(principal.account.id.as_bytes(), "campaigns", id, now)
    });
    Ok(respond(json!(page.rows), &request, Some(next), &h, false))
}
pub(super) async fn sessions<
    S: AccountStore + PersonalTokenStore,
    M: EmailSender,
    R: IntegrationReads,
>(
    State(state): State<ApiState<S, M, R>>,
    h: HeaderMap,
    query: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Failure> {
    let request = request_id();
    let principal = authorize(&state, &h, Scope::PlaySessions, &request).await?;
    let Query(paging) = query.map_err(|_| Failure::invalid(&request))?;
    let now = state.service.now_ms();
    let (limit, after) = paging.checked(
        &state.peppers,
        principal.account.id.as_bytes(),
        "play-sessions",
        now,
        &request,
    )?;
    let page = state
        .reads
        .sessions(principal.account.id, after, limit)
        .await
        .map_err(|_| Failure::unavailable(&request))?;
    let next = page.next.map(|id| {
        state
            .peppers
            .cursor(principal.account.id.as_bytes(), "play-sessions", id, now)
    });
    Ok(respond(json!(page.rows), &request, Some(next), &h, false))
}
pub(super) async fn funding<
    S: AccountStore + PersonalTokenStore,
    M: EmailSender,
    R: IntegrationReads,
>(
    State(state): State<ApiState<S, M, R>>,
    h: HeaderMap,
) -> Result<Response, Failure> {
    let request = request_id();
    let principal = authorize(&state, &h, Scope::Funding, &request).await?;
    let data = state
        .reads
        .funding(principal.account.id)
        .await
        .map_err(|_| Failure::unavailable(&request))?;
    Ok(respond(json!(data), &request, None, &h, true))
}
