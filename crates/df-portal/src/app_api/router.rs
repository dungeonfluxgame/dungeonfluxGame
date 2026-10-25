//! The routes of the account app's API under `/app/api/` and the bundle under `/app/`. Paths are
//! exactly those `df_account_app::api::ApiCall::request` produces.

use std::path::PathBuf;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post, put},
};
use df_customer::{AccountStore, EmailSender};

use super::{
    admin, auth, campaign, customer, display,
    guard::{MAXIMUM_BODY_BYTES, api_headers},
    ports::Ports,
    session,
    state::AppApiState,
    static_files::router_for_bundle,
};

fn account_routes<S: AccountStore, M: EmailSender, P: Ports>() -> Router<AppApiState<S, M, P>> {
    Router::new()
        .route("/app/api/session", get(session::get_session::<S, M, P>))
        .route("/app/api/operation", get(session::get_operation::<S, M, P>))
        .route("/app/api/sign-up", post(auth::sign_up::<S, M, P>))
        .route("/app/api/sign-in", post(auth::sign_in::<S, M, P>))
        .route("/app/api/sign-out", post(auth::sign_out::<S, M, P>))
        .route("/app/api/verify-email", post(auth::verify_email::<S, M, P>))
        .route(
            "/app/api/verify-email/resend",
            post(auth::resend_verification::<S, M, P>),
        )
        .route(
            "/app/api/password/forgot",
            post(auth::forgot_password::<S, M, P>),
        )
        .route(
            "/app/api/password/reset",
            post(auth::reset_password::<S, M, P>),
        )
        .route("/app/api/overview", get(customer::overview::<S, M, P>))
        .route(
            "/app/api/credit/offers",
            get(customer::credit_offers::<S, M, P>),
        )
        .route(
            "/app/api/credit/checkouts",
            post(customer::create_checkout::<S, M, P>),
        )
        .route(
            "/app/api/credit/checkouts/{checkout_id}",
            get(customer::checkout_status::<S, M, P>),
        )
        .route("/app/api/funding", get(customer::funding::<S, M, P>))
        .route("/app/api/funding/pace", post(customer::set_pace::<S, M, P>))
        .route(
            "/app/api/funding/allocate",
            post(customer::allocate::<S, M, P>),
        )
        .route("/app/api/statements", get(customer::statements::<S, M, P>))
        .route(
            "/app/api/account-stats",
            get(customer::account_stats::<S, M, P>),
        )
        .route(
            "/app/api/play-sessions",
            get(customer::play_sessions::<S, M, P>),
        )
        .route("/app/api/assets", get(customer::assets::<S, M, P>))
        .route("/app/api/settings", get(customer::settings::<S, M, P>))
        .route(
            "/app/api/settings/profile",
            post(customer::save_profile::<S, M, P>),
        )
        .route(
            "/app/api/settings/email",
            post(customer::change_email::<S, M, P>),
        )
        .route(
            "/app/api/settings/password",
            post(customer::change_password::<S, M, P>),
        )
        .route(
            "/app/api/settings/preferences",
            post(customer::save_preferences::<S, M, P>),
        )
        .route(
            "/app/api/settings/export",
            post(customer::request_export::<S, M, P>),
        )
        .route(
            "/app/api/settings/sessions/revoke",
            post(customer::revoke_session::<S, M, P>),
        )
        .route(
            "/app/api/settings/sessions/revoke-others",
            post(customer::revoke_other_sessions::<S, M, P>),
        )
        .route(
            "/app/api/settings/sessions/revoke-all",
            post(customer::revoke_all_sessions::<S, M, P>),
        )
        .route(
            "/app/api/settings/close",
            post(customer::close_account::<S, M, P>),
        )
}

fn admin_routes<S: AccountStore, M: EmailSender, P: Ports>() -> Router<AppApiState<S, M, P>> {
    Router::new()
        .route("/app/api/admin/summary", get(admin::summary::<S, M, P>))
        .route("/app/api/admin/accounts", get(admin::accounts::<S, M, P>))
        .route(
            "/app/api/admin/accounts/{account_id}",
            get(admin::account::<S, M, P>),
        )
        .route(
            "/app/api/admin/accounts/{account_id}/sessions/revoke",
            post(admin::revoke_account_session::<S, M, P>),
        )
        .route(
            "/app/api/admin/accounts/{account_id}/assets/{asset_id}/hold",
            post(admin::hold_asset::<S, M, P>),
        )
        .route(
            "/app/api/admin/accounts/{account_id}/assets/{asset_id}/release",
            post(admin::release_asset::<S, M, P>),
        )
        .route("/app/api/admin/refunds", post(admin::refund::<S, M, P>))
        .route(
            "/app/api/admin/credits",
            post(admin::goodwill_credit::<S, M, P>),
        )
        .route("/app/api/admin/disputes", get(admin::disputes::<S, M, P>))
        .route(
            "/app/api/admin/disputes/{dispute_id}/evidence",
            post(admin::dispute_evidence::<S, M, P>),
        )
        .route(
            "/app/api/admin/reconciliation",
            get(admin::reconciliation::<S, M, P>),
        )
        .route("/app/api/admin/providers", get(admin::providers::<S, M, P>))
        .route(
            "/app/api/admin/providers/{provider_id}/pause",
            post(admin::pause_provider::<S, M, P>),
        )
        .route(
            "/app/api/admin/providers/{provider_id}/resume",
            post(admin::resume_provider::<S, M, P>),
        )
        .route("/app/api/admin/review", get(admin::review_queue::<S, M, P>))
        .route(
            "/app/api/admin/review/{item_id}/decision",
            post(admin::decide_review::<S, M, P>),
        )
        .route("/app/api/admin/audit", get(admin::audit::<S, M, P>))
}

fn campaign_routes<S: AccountStore, M: EmailSender, P: Ports>() -> Router<AppApiState<S, M, P>> {
    Router::new()
        .route("/app/api/campaigns", get(campaign::dashboard::<S, M, P>))
        .route(
            "/app/api/campaigns/new",
            get(campaign::wizard_catalog::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/drafts",
            post(campaign::start_draft::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/drafts/{draft_id}/steps/{step}",
            put(campaign::save_draft::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/drafts/{draft_id}/finish",
            post(campaign::finish_draft::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}",
            get(campaign::campaign::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/invitations",
            post(campaign::create_invitation::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/invitations/{invitation_id}/revoke",
            post(campaign::revoke_invitation::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/code/rotate",
            post(campaign::rotate_code::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/requests/{request_id}/approve",
            post(campaign::approve_request::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/requests/{request_id}/deny",
            post(campaign::deny_request::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/leave",
            post(campaign::leave::<S, M, P>),
        )
        .route(
            "/app/api/campaigns/{campaign_id}/lobby",
            post(campaign::open_lobby::<S, M, P>),
        )
}

fn session_routes<S: AccountStore, M: EmailSender, P: Ports>() -> Router<AppApiState<S, M, P>> {
    Router::new()
        .route(
            "/app/api/sessions/{session_id}/lobby",
            get(campaign::lobby::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/character",
            post(campaign::choose_character::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/ready",
            post(campaign::ready::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/unready",
            post(campaign::unready::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/microphone",
            post(campaign::microphone::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/start",
            post(campaign::start_session::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/display/lookup",
            post(campaign::lookup_display::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/display/approve",
            post(campaign::approve_display::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/display/unpair-all",
            post(campaign::unpair_all_displays::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session_id}/lobby/display/{ticket_id}/unpair",
            post(campaign::unpair_display::<S, M, P>),
        )
}

fn join_and_display_routes<S: AccountStore, M: EmailSender, P: Ports>()
-> Router<AppApiState<S, M, P>> {
    Router::new()
        .route(
            "/app/api/join/pending",
            get(campaign::join_pending::<S, M, P>),
        )
        .route(
            "/app/api/join/preview",
            post(campaign::join_preview::<S, M, P>),
        )
        .route("/app/api/join/hold", post(campaign::join_hold::<S, M, P>))
        .route(
            "/app/api/join/accept",
            post(campaign::join_accept::<S, M, P>),
        )
        .route(
            "/app/api/join/code/lookup",
            post(campaign::code_lookup::<S, M, P>),
        )
        .route(
            "/app/api/join/code/request",
            post(campaign::code_request::<S, M, P>),
        )
        .route(
            "/app/api/display/challenge",
            post(display::challenge::<S, M, P>),
        )
        .route("/app/api/display/status", get(display::status::<S, M, P>))
        .route("/app/api/display/redeem", post(display::redeem::<S, M, P>))
        .route("/app/api/display/renew", post(display::renew::<S, M, P>))
}

/// The JSON API alone, for composition that serves the bundle itself.
pub fn api_router<S: AccountStore, M: EmailSender, P: Ports>(
    state: AppApiState<S, M, P>,
) -> Router {
    account_routes::<S, M, P>()
        .merge(admin_routes::<S, M, P>())
        .merge(campaign_routes::<S, M, P>())
        .merge(session_routes::<S, M, P>())
        .merge(join_and_display_routes::<S, M, P>())
        .layer(DefaultBodyLimit::max(MAXIMUM_BODY_BYTES))
        .layer(middleware::from_fn(api_headers))
        .with_state(state)
}

/// The API under `/app/api/` and the bundle in `bundle_directory` under `/app/`.
pub fn router<S: AccountStore, M: EmailSender, P: Ports>(
    state: AppApiState<S, M, P>,
    bundle_directory: impl Into<PathBuf>,
) -> Router {
    api_router(state).merge(router_for_bundle(bundle_directory))
}
