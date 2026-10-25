use super::*;
use crate::wire::Refusal;
use crate::wire_campaign::{
    CampaignDraft, FundingModel, HeroChoice, JoinMode, PictureDefault, Reason,
};

fn draft() -> CampaignDraft {
    CampaignDraft {
        name: "The Drowned Lantern".to_owned(),
        adventure_id: Some("a1".to_owned()),
        content_rating: None,
        seat_count: 4,
        hero_choice: HeroChoice::InLobby,
        has_voice: true,
        pictures: PictureDefault::Automatic,
        pace: crate::wire::PacePreset::Immersive,
        custom_ceiling_cents_per_hour: None,
        co_hosts_can_invite: true,
        co_hosts_can_start: false,
        co_hosts_can_export: false,
        join_mode: JoinMode::LinkOnly,
        funding: FundingModel::Sponsor,
    }
}

#[test]
fn a_typed_refusal_body_is_kept_whatever_the_status() {
    let call = ApiCall::SignIn(SignInRequest {
        email: "a@example.test".to_owned(),
        password: "x".to_owned(),
    });
    let body = r#"{"code":"invalid","refusal":"wrong_password"}"#;
    assert_eq!(
        call.decode(422, body),
        Reply::SignedIn(Err(ApiError::refused(Refusal::WrongPassword)))
    );
}

#[test]
fn a_campaign_reason_rides_beside_the_envelope_code() {
    let call = ApiCall::JoinPreview(TokenBody {
        token: "ab".to_owned(),
    });
    let body = r#"{"code":"not_found","reason":"revoked"}"#;
    assert_eq!(
        call.decode(404, body),
        Reply::JoinPreview(Err(ApiError::because(ErrorCode::NotFound, Reason::Revoked)))
    );
}

#[test]
fn an_unreadable_failure_maps_by_status_and_a_lost_mutation_is_unknown() {
    assert_eq!(
        ApiCall::Overview.decode(401, "<html>"),
        Reply::Overview(Err(ApiError::of(ErrorCode::SignedOut)))
    );
    assert_eq!(
        ApiCall::Overview.lost(),
        Reply::Overview(Err(ApiError::of(ErrorCode::Unavailable)))
    );
    let checkout = ApiCall::CreateCheckout(CreateCheckoutRequest {
        amount_cents: 2000,
        operation: "op".to_owned(),
    });
    assert_eq!(
        checkout.lost(),
        Reply::CheckoutCreated(Err(ApiError::of(ErrorCode::UnknownOutcome)))
    );
    assert_eq!(
        checkout.decode(200, "not json"),
        Reply::CheckoutCreated(Err(ApiError::of(ErrorCode::UnknownOutcome)))
    );
}

#[test]
fn a_search_query_is_percent_encoded_into_the_path() {
    let call = ApiCall::AdminAccounts {
        query: "a b&c=d".to_owned(),
        standing: None,
        verified: None,
        has_balance: None,
        sort: None,
    };
    assert_eq!(
        call.request().path,
        "/app/api/admin/accounts?q=a%20b%26c%3Dd"
    );
    assert_eq!(call.request().method, Method::Get);
}

#[test]
fn every_post_has_a_json_body_and_the_csrf_guard() {
    for call in [
        ApiCall::SignOut,
        ApiCall::ResendVerification,
        ApiCall::RevokeOtherSessions,
        ApiCall::StartDraft,
        ApiCall::DisplayChallenge,
    ] {
        let request = call.request();
        assert_eq!(request.method, Method::Post);
        assert_eq!(request.body.as_deref(), Some("{}"));
        assert_eq!(request.guard, Guard::Csrf, "{call:?}");
    }
}

#[test]
fn reads_carry_no_guard_and_no_body() {
    for call in [
        ApiCall::Session,
        ApiCall::Overview,
        ApiCall::Dashboard,
        ApiCall::JoinPending,
        ApiCall::DisplayStatus,
        ApiCall::Lobby {
            session_id: "5e".to_owned(),
        },
    ] {
        let request = call.request();
        assert_eq!(request.method, Method::Get, "{call:?}");
        assert_eq!(request.guard, Guard::None, "{call:?}");
        assert_eq!(request.body, None, "{call:?}");
    }
}

#[test]
fn display_redeem_uses_the_device_guard_not_a_csrf_token() {
    let request = ApiCall::DisplayRedeem.request();
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.guard, Guard::Device);
}

#[test]
fn saving_a_wizard_step_is_a_put_to_that_step() {
    let call = ApiCall::SaveDraft {
        draft_id: "d1".to_owned(),
        step: 3,
        request: SaveDraftRequest { draft: draft() },
    };
    let request = call.request();
    assert_eq!(request.method, Method::Put);
    assert_eq!(request.path, "/app/api/campaigns/drafts/d1/steps/3");
    assert!(call.is_mutation());
}

#[test]
fn an_invitation_token_travels_in_the_body_and_never_in_the_path() {
    for call in [
        ApiCall::JoinPreview(TokenBody {
            token: "secrettoken".to_owned(),
        }),
        ApiCall::JoinHold(TokenBody {
            token: "secrettoken".to_owned(),
        }),
        ApiCall::JoinAccept(JoinAcceptRequest {
            token: Some("secrettoken".to_owned()),
            operation: "op".to_owned(),
        }),
    ] {
        let request = call.request();
        assert!(!request.path.contains("secrettoken"), "{call:?}");
        assert!(
            request
                .body
                .as_deref()
                .is_some_and(|body| body.contains("secrettoken"))
        );
    }
}

#[test]
fn ready_and_unready_are_separate_endpoints() {
    let request = |is_ready| {
        ApiCall::SetReady {
            session_id: "5e".to_owned(),
            is_ready,
            request: ReadyRequest {
                operation: "op".to_owned(),
            },
        }
        .request()
        .path
    };
    assert_eq!(request(true), "/app/api/sessions/5e/lobby/ready");
    assert_eq!(request(false), "/app/api/sessions/5e/lobby/unready");
}

#[test]
fn every_reply_variant_decodes_through_its_call() {
    let call = ApiCall::DisplayStatus;
    assert_eq!(
        call.decode(200, r#"{"state":"approved"}"#),
        Reply::DisplayStatus(Ok(crate::wire_campaign::DisplayStatus {
            state: crate::wire_campaign::DisplayState::Approved
        }))
    );
    assert_eq!(
        ApiCall::LeaveCampaign {
            campaign_id: "c".to_owned(),
            request: OperationRequest {
                operation: "o".to_owned()
            }
        }
        .decode(200, r#"{"accepted":true}"#),
        Reply::Left(Ok(Accepted { accepted: true }))
    );
}

#[test]
fn choosing_a_hero_posts_the_typed_choice_and_reads_the_authoritative_lobby() {
    let call = ApiCall::ChooseCharacter {
        session_id: "session".to_owned(),
        request: crate::wire_campaign::CharacterRequest {
            choice: crate::wire_campaign::CharacterChoice::Template,
            reference: Some("11".repeat(16)),
            operation: "operation".to_owned(),
        },
    };
    let request = call.request();
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.path, "/app/api/sessions/session/lobby/character");
    assert!(matches!(call.decode(503, "{}"), Reply::Lobby(Err(_))));
}
