//! Plan section 17, AUTH-01 to AUTH-03: invitations, room codes, display tokens and private
//! drafts (plan 3.2, 9.6 and 14.1).
//!
//! Real tests exercise the one-shot invitation policy of `df-session` with a controlled clock.
//! The invitation owner (S1) answers every failure with one byte-identical response, so the
//! distinct policy outcomes tested here are internal and never shown to a visitor. Room-code
//! seat recovery, display tickets and draft-portrait access need S1, O1, O2 and F and are blocked
//! scenarios.

use df_acceptance::{ControlledClock, blocked_scenario, operation};
use df_session::invitation::{InvitationFailure, InvitationUse, OneShotInvitation};
use df_types::SessionId;

const INVITATION_LIFETIME_SECONDS: u64 = 600;

type Invitation = OneShotInvitation<&'static str, u8>;

fn session() -> SessionId {
    SessionId::from_bytes(&[7; 16]).expect("a nonzero identity")
}

fn fresh_invitation(clock: &ControlledClock) -> Invitation {
    OneShotInvitation::new(clock.now_seconds() + INVITATION_LIFETIME_SECONDS)
}

#[test]
fn auth_01_an_invitation_admits_exactly_one_principal() {
    let clock = ControlledClock::starting_at(1_000);
    let mut invitation = fresh_invitation(&clock);

    let first = invitation.consume("alice", session(), operation(1), 1, clock.now_seconds());
    let second = invitation.consume("mallory", session(), operation(2), 1, clock.now_seconds());

    assert_eq!(first, Ok(InvitationUse::ConsumedNow));
    assert_eq!(second, Err(InvitationFailure::Consumed));
}

#[test]
fn auth_01_reusing_the_first_redeemers_operation_id_does_not_take_over_their_seat() {
    let clock = ControlledClock::starting_at(1_000);
    let mut invitation = fresh_invitation(&clock);
    invitation
        .consume("alice", session(), operation(1), 1, clock.now_seconds())
        .expect("alice redeems");

    let takeover = invitation.consume("mallory", session(), operation(1), 1, clock.now_seconds());

    assert_eq!(takeover, Err(InvitationFailure::Consumed));
}

#[test]
fn auth_01_the_accepted_redemption_can_be_retried_after_the_invitation_expires() {
    let clock = ControlledClock::starting_at(1_000);
    let mut invitation = fresh_invitation(&clock);
    invitation
        .consume("alice", session(), operation(1), 1, clock.now_seconds())
        .expect("alice redeems");
    clock.advance(INVITATION_LIFETIME_SECONDS * 10);

    let lost_response_retry =
        invitation.consume("alice", session(), operation(1), 1, clock.now_seconds());

    assert_eq!(lost_response_retry, Ok(InvitationUse::ExactReplay));
}

#[test]
fn auth_01_a_changed_request_under_the_same_operation_is_a_conflict_not_a_replay() {
    let clock = ControlledClock::starting_at(1_000);
    let mut invitation = fresh_invitation(&clock);
    invitation
        .consume("alice", session(), operation(1), 1, clock.now_seconds())
        .expect("alice redeems");

    let altered = invitation.consume("alice", session(), operation(1), 2, clock.now_seconds());

    assert_eq!(altered, Err(InvitationFailure::OperationConflict));
}

#[test]
fn auth_01_an_unused_invitation_expires_exactly_at_its_deadline() {
    let clock = ControlledClock::starting_at(1_000);
    let mut invitation = fresh_invitation(&clock);
    clock.advance(INVITATION_LIFETIME_SECONDS - 1);
    let one_second_early =
        invitation
            .clone()
            .consume("alice", session(), operation(1), 1, clock.now_seconds());
    clock.advance(1);

    let at_the_deadline =
        invitation.consume("alice", session(), operation(1), 1, clock.now_seconds());

    assert_eq!(one_second_early, Ok(InvitationUse::ConsumedNow));
    assert_eq!(at_the_deadline, Err(InvitationFailure::Expired));
}

#[test]
fn auth_01_a_revoked_invitation_cannot_be_redeemed_and_a_redeemed_one_cannot_be_revoked() {
    let clock = ControlledClock::starting_at(1_000);
    let mut revoked = fresh_invitation(&clock);
    let mut redeemed = fresh_invitation(&clock);
    revoked
        .revoke()
        .expect("an unused invitation can be revoked");
    redeemed
        .consume("alice", session(), operation(1), 1, clock.now_seconds())
        .expect("alice redeems");

    let after_revoke = revoked.consume("alice", session(), operation(1), 1, clock.now_seconds());
    let revoke_after_use = redeemed.revoke();

    assert_eq!(after_revoke, Err(InvitationFailure::Revoked));
    assert_eq!(revoke_after_use, Err(InvitationFailure::Consumed));
}

blocked_scenario! {
    fn auth_01_a_guessed_room_code_or_an_old_invitation_cannot_take_over_an_authenticated_paid_seat,
    blocked_on: "blocked on S1 and O1 API: df_auth::short_code join requests with host approval, the keyed throttle layers, byte-identical invitation failures, and authenticated seat recovery with local-room reclaim off when hosted (campaigns design 4.2, 8.3, 11)",
    setup: [
        "campaign C with host H and player P holding a paid seat (P funded an allocation); a live room code RC and an invitation link already used by P",
        "attacker X is authenticated as a different verified account; Y is unauthenticated; throttle ceilings read from configuration",
    ],
    steps: [
        "X presents RC, then guesses codes up to the process ceiling and per-account ceiling on a controlled clock",
        "assert: a hit alone yields a join request for host approval and never a seat (a_room_code_always_ends_in_approval), never resolves to an existing member (a_room_code_never_resolves_to_an_existing_member), and the pending request cap refuses the twenty-first request (the_pending_request_cap_refuses_the_twenty_first_request)",
        "assert: guessing hits the typed ceiling (guessing_codes_hits_the_process_ceiling) and flooding one throttle layer does not lock out another",
        "X and Y present P's used invitation token, an unknown token and a malformed token",
        "assert: the three failure responses are byte-identical and carry no campaign details (an_unknown_token_and_a_malformed_token_return_the_same_response, failure_responses_show_no_campaign_details), and a reused exhausted invitation never adds a second member",
        "assert: X and Y never obtain P's seat, characters, allocation or private notes, and the audit log records the attempts without the code or token values",
        "assert: P reconnects from a new device after authenticating and reclaims the same seat; a host transfer needs the host's explicit action",
    ],
    outcome: "No takeover of authenticated paid seat",
}

blocked_scenario! {
    fn auth_02_a_display_token_cannot_invoke_payment_or_a_secret_view,
    blocked_on: "blocked on S1, P and F API: the display ticket role enforced on every non-public route of the merged router, including the finance commands (campaigns design 8.5, 13.2 request S4-1)",
    setup: [
        "campaign C with a paired TV display (one-time expiring challenge redeemed) and player P with a private hero sheet and secret notes carrying seeded markers",
        "the merged router: S1's campaign routes, P's /app/api routes, F's finance commands and the edge routes; the display's ticket and device cookie are the only credentials sent",
    ],
    steps: [
        "iterate every non-public route, GET as well as POST, filling path parameters from a seeded fixture (a_display_ticket_is_refused_by_every_non_public_route)",
        "call every money command: create purchase, allocate, withdraw, approve quote, refund, start and pause session (a_display_principal_is_refused_by_every_finance_command)",
        "assert: every call is refused with the same typed denial, no seeded marker appears in any body, and the store is unchanged",
        "assert: a reused or expired pairing challenge cannot mint a display ticket, and unregistered paths and methods are not served",
        "assert: the display still receives the shared scene view and the low-funds status without payer details",
    ],
    outcome: "Denied",
}

blocked_scenario! {
    fn auth_03_another_member_requesting_a_draft_portrait_is_denied_and_bearer_grants_expire,
    blocked_on: "blocked on O2 and S1 API: the delivery gateway's principal-bound grants with audience labels and the draft owner check (asset design 8.1, 8.3)",
    setup: [
        "player P has a draft portrait (OwnerPrivate) and a published portrait (CampaignShared) in campaign C; member Q is in C",
        "a controlled clock; grant_lifetime (default 15 minutes) read from configuration",
    ],
    steps: [
        "Q requests P's draft portrait with Q's own session, with a valid draft capability copied from P, and with no session; also by content key",
        "assert: every refusal is the same concealed 404; the owner gets 200",
        "P uses a grant before and after grant_lifetime",
        "assert: the grant works before expiry, fails at expiry, and renewal does a fresh load rather than extending the old grant",
        "assert: Q can fetch the published portrait; sharing was an explicit audience change recorded in the audit log",
    ],
    outcome: "Denied; bearer grants expire according to policy",
}
