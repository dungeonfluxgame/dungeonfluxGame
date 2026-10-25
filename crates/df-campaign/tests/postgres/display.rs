//! Display pairing and read-only tickets.

use df_campaign::store::NewProposal;
use df_campaign::{
    config::Tunables,
    display_pairing::{PairingRefusal, PairingState, TicketRefusal},
    errors::Refusal,
    ids::PairingId,
    join_invitation::JoinRole,
    records::{DeviceLabel, EventKind, EventScope, PairingRow, RevokeReason, TicketRow},
    roles::{Delegation, Delegations},
    store::{CampaignStore, NewPairing, OwnerFence},
};
use df_types::{AccountId, CampaignId, SessionId};

use crate::{
    fixture::{World, digest_of, session_id},
    membership::refusal_of,
};

fn new_pairing(number: u32) -> NewPairing {
    NewPairing {
        device_digest: digest_of(&format!("device{number}")),
        device_label: DeviceLabel::Desktop,
        ip_key: digest_of(&format!("ip{number}")),
        ip_key_loose: digest_of(&format!("loose{number}")),
        user_code: format!("CODE{number:04}"),
    }
}

const LIFETIME: u64 = 600_000;

async fn begin(world: &World, number: u32) -> PairingRow {
    world
        .store
        .begin_pairing(new_pairing(number), LIFETIME, world.now())
        .await
        .unwrap()
}

async fn approve(
    world: &World,
    host: AccountId,
    campaign: CampaignId,
    session: SessionId,
    pairing: PairingId,
) {
    world
        .store
        .approve_pairing(host, campaign, session, pairing, world.now())
        .await
        .unwrap();
}

async fn pair(
    world: &World,
    host: AccountId,
    campaign: CampaignId,
    session: SessionId,
    number: u32,
) -> TicketRow {
    let pairing = begin(world, number).await;
    approve(world, host, campaign, session, pairing.id).await;
    world
        .store
        .redeem_pairing(
            digest_of(&format!("device{number}")),
            None,
            digest_of(&format!("ticket{number}")),
            true,
            world.now(),
        )
        .await
        .unwrap()
        .ticket
}

async fn table() -> (World, AccountId, CampaignId, SessionId) {
    let world = World::new("display").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    (world, host, campaign.id, session_id(1))
}

#[tokio::test]
async fn a_display_pairs_redeems_once_and_the_ticket_verifies_with_the_device_secret() {
    let (world, host, campaign, session) = table().await;
    let pairing = begin(&world, 1).await;
    assert_eq!(pairing.state, PairingState::Pending);
    let status = world
        .store
        .pairing_status(digest_of("device1"), world.now())
        .await
        .unwrap();
    assert_eq!(status.user_code.as_deref(), Some("CODE0001"));
    // Lookup shows what is waiting and approves nothing.
    let found = world
        .store
        .lookup_pairing(host, campaign, "CODE0001".into(), world.now())
        .await
        .unwrap();
    assert_eq!(found.id, pairing.id);
    assert_eq!(
        world
            .store
            .pairing_status(digest_of("device1"), world.now())
            .await
            .unwrap()
            .state,
        PairingState::Pending
    );
    approve(&world, host, campaign, session, pairing.id).await;
    let ticket = world
        .store
        .redeem_pairing(
            digest_of("device1"),
            None,
            digest_of("ticket1"),
            true,
            world.now(),
        )
        .await
        .unwrap()
        .ticket;
    let verified = world
        .store
        .verify_ticket(
            digest_of("ticket1"),
            digest_of("device1"),
            true,
            world.now(),
        )
        .await
        .unwrap();
    assert_eq!(verified.ticket.id, ticket.id);
    // The ticket needs the device secret: another device is refused.
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(
                    digest_of("ticket1"),
                    digest_of("device2"),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::WrongDevice)
    );
    // An ended session is not usable.
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(
                    digest_of("ticket1"),
                    digest_of("device1"),
                    false,
                    world.now()
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::SessionEnded)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn lookup_and_approval_need_the_start_or_pause_delegation() {
    let (world, host, campaign, session) = table().await;
    let guest = world.account(2).await;
    let outsider = world.account(3).await;
    let member = world.join(campaign, host, guest, JoinRole::Player).await;
    let pairing = begin(&world, 1).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .lookup_pairing(guest, campaign, "CODE0001".into(), world.now())
                .await
        ),
        Refusal::NotAllowed
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(guest, campaign, session, pairing.id, world.now())
                .await
        ),
        Refusal::NotAllowed
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(outsider, campaign, session, pairing.id, world.now())
                .await
        ),
        Refusal::NotFound
    );
    // A co-host with the bit may.
    let start = Delegations::NONE.with(Delegation::StartOrPauseSession);
    world
        .store
        .set_co_host(host, campaign, member, start, world.now())
        .await
        .unwrap();
    world
        .store
        .approve_pairing(guest, campaign, session, pairing.id, world.now())
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn pairing_expiry_is_exclusive_and_a_pairing_is_approved_once() {
    let (world, host, campaign, session) = table().await;
    let pairing = begin(&world, 1).await;
    let at_expiry = world.now() + LIFETIME;
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(host, campaign, session, pairing.id, at_expiry)
                .await
        ),
        Refusal::Pairing(PairingRefusal::Expired)
    );
    assert_eq!(
        world
            .store
            .pairing_status(digest_of("device1"), at_expiry)
            .await
            .unwrap()
            .state,
        PairingState::Expired
    );
    let fresh = begin(&world, 2).await;
    world
        .store
        .approve_pairing(
            host,
            campaign,
            session,
            fresh.id,
            world.now() + LIFETIME - 1,
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(host, campaign, session, fresh.id, world.now())
                .await
        ),
        Refusal::Pairing(PairingRefusal::NotPending)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn the_display_cap_counts_approved_unredeemed_pairings_and_live_tickets() {
    let tunables = Tunables {
        maximum_displays_per_session: 2,
        ..Tunables::default()
    };
    let world = World::with_tunables("displaycap", tunables).await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await.id;
    let session = session_id(1);
    let first = begin(&world, 1).await;
    let second = begin(&world, 2).await;
    let third = begin(&world, 3).await;
    // Pending pairings do not count; approved-unredeemed ones do.
    approve(&world, host, campaign, session, first.id).await;
    approve(&world, host, campaign, session, second.id).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(host, campaign, session, third.id, world.now())
                .await
        ),
        Refusal::DisplayLimit
    );
    // Redeeming turns an approval into a live ticket without changing the count.
    for number in [1, 2] {
        world
            .store
            .redeem_pairing(
                digest_of(&format!("device{number}")),
                None,
                digest_of(&format!("ticket{number}")),
                true,
                world.now(),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        refusal_of(
            world
                .store
                .approve_pairing(host, campaign, session, third.id, world.now())
                .await
        ),
        Refusal::DisplayLimit
    );
    // Unpairing one frees a place.
    let tickets = world
        .store
        .unpair(host, campaign, session, None, world.now())
        .await
        .unwrap();
    assert_eq!(tickets, 2);
    approve(&world, host, campaign, session, third.id).await;
    world.cleanup().await;
}

#[tokio::test]
async fn a_lost_redemption_response_is_recovered_by_one_reissue() {
    let (world, host, campaign, session) = table().await;
    let pairing = begin(&world, 1).await;
    approve(&world, host, campaign, session, pairing.id).await;
    let device = digest_of("device1");
    let first = world
        .store
        .redeem_pairing(device, None, digest_of("ticket-a"), true, world.now())
        .await
        .unwrap();
    assert!(!first.reissued);
    // The response was lost: the display repeats with the device cookie only, inside the window.
    let later = world.now() + 30_000;
    let second = world
        .store
        .redeem_pairing(device, None, digest_of("ticket-b"), true, later)
        .await
        .unwrap();
    assert!(second.reissued);
    assert_ne!(second.ticket.id, first.ticket.id);
    assert_eq!(
        second.ticket.expires_at_ms, first.ticket.expires_at_ms,
        "a reissue extends nothing"
    );
    // The old ticket is revoked as reissued, with its own event.
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(digest_of("ticket-a"), device, true, later)
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    let events = world.events(campaign).await;
    let revoked: Vec<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::DisplayTicketRevoked)
        .collect();
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0].scope, EventScope::Ticket);
    assert_eq!(revoked[0].ticket, Some(first.ticket.id));
    // Repeating with the new cookie present issues nothing; a second reissue is refused.
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(
                    device,
                    Some(digest_of("ticket-b")),
                    digest_of("ticket-c"),
                    true,
                    later
                )
                .await
        ),
        Refusal::Pairing(PairingRefusal::AlreadyRedeemed)
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(device, None, digest_of("ticket-d"), true, later)
                .await
        ),
        Refusal::Pairing(PairingRefusal::ReissueUsed)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_repeat_after_the_reissue_window_issues_nothing() {
    let (world, host, campaign, session) = table().await;
    let pairing = begin(&world, 1).await;
    approve(&world, host, campaign, session, pairing.id).await;
    let device = digest_of("device1");
    world
        .store
        .redeem_pairing(device, None, digest_of("ticket-a"), true, world.now())
        .await
        .unwrap();
    let late = world.now() + world.store.tunables().display_reissue_window_ms();
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(device, None, digest_of("ticket-b"), true, late)
                .await
        ),
        Refusal::Pairing(PairingRefusal::ReissueWindowClosed)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_ticket_is_valid_one_millisecond_before_expiry_and_refused_at_expiry() {
    let (world, host, campaign, session) = table().await;
    let ticket = pair(&world, host, campaign, session, 1).await;
    let device = digest_of("device1");
    let token = digest_of("ticket1");
    assert!(
        world
            .store
            .verify_ticket(token, device, true, ticket.expires_at_ms - 1)
            .await
            .is_ok()
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(token, device, true, ticket.expires_at_ms)
                .await
        ),
        Refusal::Ticket(TicketRefusal::Expired)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn renewal_extends_the_same_ticket_only_inside_the_window_and_never_at_expiry() {
    let (world, host, campaign, session) = table().await;
    let ticket = pair(&world, host, campaign, session, 1).await;
    let device = digest_of("device1");
    let token = digest_of("ticket1");
    let tunables = world.store.tunables().clone();
    // Too early.
    let early = ticket.expires_at_ms - tunables.display_renewal_window_ms() - 1;
    assert_eq!(
        refusal_of(world.store.extend_ticket(token, device, true, early).await),
        Refusal::Ticket(TicketRefusal::OutsideRenewalWindow)
    );
    // Inside the window: the same ticket id, a later expiry, and the old token still works.
    let inside = ticket.expires_at_ms - tunables.display_renewal_window_ms();
    let renewed = world
        .store
        .extend_ticket(token, device, true, inside)
        .await
        .unwrap();
    assert_eq!(renewed.id, ticket.id);
    assert!(renewed.expires_at_ms > ticket.expires_at_ms);
    assert!(
        world
            .store
            .verify_ticket(token, device, true, inside)
            .await
            .is_ok()
    );
    world.cleanup().await;
}

#[tokio::test]
async fn renewal_at_exactly_the_expiry_instant_is_refused() {
    let (world, host, campaign, session) = table().await;
    let lapsed = pair(&world, host, campaign, session, 1).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .extend_ticket(
                    digest_of("ticket1"),
                    digest_of("device1"),
                    true,
                    lapsed.expires_at_ms
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Expired)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn renewal_past_the_pairing_cap_or_the_end_of_the_session_is_refused() {
    let (world, host, campaign, session) = table().await;
    let ticket = pair(&world, host, campaign, session, 1).await;
    let device = digest_of("device1");
    let token = digest_of("ticket1");
    // Walk renewals forward until the pairing's cap stops them.
    let tunables = world.store.tunables().clone();
    let mut now = ticket.expires_at_ms - tunables.display_renewal_window_ms();
    let mut last = ticket.expires_at_ms;
    let mut refused = None;
    for _ in 0..40 {
        match world.store.extend_ticket(token, device, true, now).await {
            Ok(row) => {
                last = row.expires_at_ms;
                now = last - tunables.display_renewal_window_ms();
            }
            Err(failure) => {
                refused = Some(refusal_of::<()>(Err(failure)));
                break;
            }
        }
    }
    assert_eq!(
        refused,
        Some(Refusal::Ticket(TicketRefusal::RenewalCapReached))
    );
    assert!(last <= world.now() + tunables.display_pairing_maximum_ms());
    world.cleanup().await;
}

#[tokio::test]
async fn the_end_of_the_session_refuses_renewal_regardless() {
    let (world, host, campaign, session) = table().await;
    let fresh = pair(&world, host, campaign, session, 1).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .extend_ticket(
                    digest_of("ticket1"),
                    digest_of("device1"),
                    false,
                    fresh.expires_at_ms - 1
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::SessionEnded)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn unpair_revokes_one_ticket_with_its_own_event_and_looks_up_the_ticket_by_session_and_campaign()
 {
    let (world, host, campaign, session) = table().await;
    let first = pair(&world, host, campaign, session, 1).await;
    let second = pair(&world, host, campaign, session, 2).await;
    // A ticket looked up under another session answers as missing.
    assert_eq!(
        refusal_of(
            world
                .store
                .unpair(host, campaign, session_id(9), Some(first.id), world.now())
                .await
        ),
        Refusal::NotFound
    );
    assert_eq!(
        world
            .store
            .unpair(host, campaign, session, Some(first.id), world.now())
            .await
            .unwrap(),
        1
    );
    assert!(
        world
            .store
            .verify_ticket(
                digest_of("ticket2"),
                digest_of("device2"),
                true,
                world.now()
            )
            .await
            .is_ok()
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(
                    digest_of("ticket1"),
                    digest_of("device1"),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    let events = world.events(campaign).await;
    assert!(
        events
            .iter()
            .any(|e| e.kind == EventKind::DisplayTicketRevoked && e.ticket == Some(first.id))
    );
    assert!(!events.iter().any(|e| e.ticket == Some(second.id)));
    world.cleanup().await;
}

#[tokio::test]
async fn a_host_transfer_revokes_every_display_ticket_with_one_row_each() {
    let (world, host, campaign, session) = table().await;
    let guest = world.account(2).await;
    let guest_member = world.join(campaign, host, guest, JoinRole::Player).await;
    pair(&world, host, campaign, session, 1).await;
    pair(&world, host, campaign, session, 2).await;
    let proposal = world
        .store
        .propose_transfer(
            host,
            campaign,
            NewProposal {
                kind: df_campaign::records::TransferKind::Host,
                to_member: guest_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(guest, campaign, proposal, world.now())
        .await
        .unwrap();
    let events = world.events(campaign).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == EventKind::DisplayTicketRevoked)
            .count(),
        2
    );
    for number in [1, 2] {
        assert_eq!(
            refusal_of(
                world
                    .store
                    .verify_ticket(
                        digest_of(&format!("ticket{number}")),
                        digest_of(&format!("device{number}")),
                        true,
                        world.now()
                    )
                    .await
            ),
            Refusal::Ticket(TicketRefusal::Revoked)
        );
    }
    world.cleanup().await;
}

#[tokio::test]
async fn an_approver_who_loses_the_delegation_takes_their_tickets_with_them() {
    let (world, host, campaign, session) = table().await;
    let cohost = world.account(2).await;
    let member = world.join(campaign, host, cohost, JoinRole::Player).await;
    let start = Delegations::NONE.with(Delegation::StartOrPauseSession);
    world
        .store
        .set_co_host(host, campaign, member, start, world.now())
        .await
        .unwrap();
    let pairing = begin(&world, 1).await;
    world
        .store
        .approve_pairing(cohost, campaign, session, pairing.id, world.now())
        .await
        .unwrap();
    world
        .store
        .redeem_pairing(
            digest_of("device1"),
            None,
            digest_of("ticket1"),
            true,
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .set_co_host(host, campaign, member, Delegations::NONE, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(
                    digest_of("ticket1"),
                    digest_of("device1"),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn archive_and_session_end_revoke_tickets_and_record_the_fence_for_audit() {
    let (world, host, campaign, session) = table().await;
    pair(&world, host, campaign, session, 1).await;
    pair(&world, host, campaign, session, 2).await;
    let fence = OwnerFence {
        recovery_epoch: 3,
        claim_seq: 7,
        placement_id: [5; 16],
    };
    let revoked = world
        .store
        .revoke_session_tickets(
            session,
            campaign,
            RevokeReason::SessionEnded,
            Some(fence),
            world.now(),
        )
        .await
        .unwrap();
    assert_eq!(revoked, 2);
    let events = world.events(campaign).await;
    let rows: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::DisplayTicketRevoked)
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.writer_fence.as_deref() == Some(&fence.audit_bytes()[..]))
    );
    // A second call revokes nothing and writes nothing.
    assert_eq!(
        world
            .store
            .revoke_session_tickets(
                session,
                campaign,
                RevokeReason::SessionEnded,
                None,
                world.now()
            )
            .await
            .unwrap(),
        0
    );
    // Archive revokes the tickets of a later pairing too.
    pair(&world, host, campaign, session, 3).await;
    world
        .store
        .archive_campaign(host, campaign, "The Sunken Bell".into(), false, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .verify_ticket(
                    digest_of("ticket3"),
                    digest_of("device3"),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn challenges_are_capped_per_address_key_and_a_code_collision_is_retryable() {
    let tunables = Tunables {
        display_pending_per_ip_maximum: 2,
        ..Tunables::default()
    };
    let world = World::with_tunables("challengecap", tunables).await;
    let same_ip = |number: u32| NewPairing {
        ip_key: digest_of("same"),
        ..new_pairing(number)
    };
    world
        .store
        .begin_pairing(same_ip(1), LIFETIME, world.now())
        .await
        .unwrap();
    world
        .store
        .begin_pairing(same_ip(2), LIFETIME, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .begin_pairing(same_ip(3), LIFETIME, world.now())
                .await
        ),
        Refusal::Throttled
    );
    // A live pending code is not handed out twice.
    let collision = world
        .store
        .begin_pairing(
            NewPairing {
                user_code: "CODE0001".into(),
                ..new_pairing(9)
            },
            LIFETIME,
            world.now(),
        )
        .await;
    assert!(matches!(
        collision,
        Err(df_campaign::errors::Failure::Store(
            df_campaign::errors::StoreError::Retryable
        ))
    ));
    world.cleanup().await;
}

#[tokio::test]
async fn a_repeat_redeem_after_an_unpair_inside_the_window_issues_nothing() {
    let (world, host, campaign, session) = table().await;
    let pairing = begin(&world, 1).await;
    approve(&world, host, campaign, session, pairing.id).await;
    let device = digest_of("device1");
    world
        .store
        .redeem_pairing(device, None, digest_of("ticket-a"), true, world.now())
        .await
        .unwrap();
    // The host unpairs; the display, which lost the response or was sent back, repeats the redeem
    // with the device cookie only, well inside the reissue window.
    world
        .store
        .unpair(host, campaign, session, None, world.now())
        .await
        .unwrap();
    let later = world.now() + 1_000;
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(device, None, digest_of("ticket-b"), true, later)
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked),
        "a revocation is never turned back into a ticket"
    );
    let live: i64 = world
        .db
        .admin
        .client
        .query_one(
            "SELECT count(*) FROM campaigns.display_ticket WHERE revoked_at_ms IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(live, 0);
    world.cleanup().await;
}

#[tokio::test]
async fn a_host_transfer_followed_by_a_repeat_redeem_issues_nothing_either() {
    let (world, host, campaign, session) = table().await;
    let guest = world.account(2).await;
    let member = world.join(campaign, host, guest, JoinRole::Player).await;
    let pairing = begin(&world, 1).await;
    approve(&world, host, campaign, session, pairing.id).await;
    let device = digest_of("device1");
    world
        .store
        .redeem_pairing(device, None, digest_of("ticket-a"), true, world.now())
        .await
        .unwrap();
    let proposal = world
        .store
        .propose_transfer(
            host,
            campaign,
            NewProposal {
                kind: df_campaign::records::TransferKind::Host,
                to_member: member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(guest, campaign, proposal, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(
                    device,
                    None,
                    digest_of("ticket-b"),
                    true,
                    world.now() + 1_000
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn the_approvers_authority_is_checked_at_the_first_redemption_and_again_at_a_reissue() {
    let (world, host, campaign, session) = table().await;
    let cohost = world.account(2).await;
    let member = world.join(campaign, host, cohost, JoinRole::Player).await;
    let start = Delegations::NONE.with(Delegation::StartOrPauseSession);
    world
        .store
        .set_co_host(host, campaign, member, start, world.now())
        .await
        .unwrap();
    // Approved by the co-host, who loses the delegation before the display redeems.
    let pairing = begin(&world, 1).await;
    world
        .store
        .approve_pairing(cohost, campaign, session, pairing.id, world.now())
        .await
        .unwrap();
    world
        .store
        .set_co_host(host, campaign, member, Delegations::NONE, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(
                    digest_of("device1"),
                    None,
                    digest_of("t1"),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::Pairing(PairingRefusal::ApproverNotAllowed)
    );
    // A second co-host approves, redeems, and loses the right; a lost-response repeat is refused.
    world
        .store
        .set_co_host(host, campaign, member, start, world.now())
        .await
        .unwrap();
    let second = begin(&world, 2).await;
    world
        .store
        .approve_pairing(cohost, campaign, session, second.id, world.now())
        .await
        .unwrap();
    world
        .store
        .redeem_pairing(
            digest_of("device2"),
            None,
            digest_of("t2"),
            true,
            world.now(),
        )
        .await
        .unwrap();
    // Losing the right revokes the ticket (so the repeat finds none) ...
    world
        .store
        .set_co_host(host, campaign, member, Delegations::NONE, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .redeem_pairing(
                    digest_of("device2"),
                    None,
                    digest_of("t3"),
                    true,
                    world.now() + 1_000
                )
                .await
        ),
        Refusal::Ticket(TicketRefusal::Revoked)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn challenges_are_capped_per_loose_address_key_as_well_as_per_tight_key() {
    let tunables = Tunables {
        display_pending_per_ip_maximum: 2,
        throttle_loose_multiplier: 2,
        ..Tunables::default()
    };
    let world = World::with_tunables("loosecap", tunables).await;
    // Four different tight keys inside one loose key (one /48): the loose cap is 2 * 2 = 4.
    let pairing = |number: u32| NewPairing {
        ip_key: digest_of(&format!("tight{number}")),
        ip_key_loose: digest_of("one network"),
        ..new_pairing(number)
    };
    for number in 1..=4 {
        world
            .store
            .begin_pairing(pairing(number), LIFETIME, world.now())
            .await
            .unwrap();
    }
    assert_eq!(
        refusal_of(
            world
                .store
                .begin_pairing(pairing(5), LIFETIME, world.now())
                .await
        ),
        Refusal::Throttled,
        "one network cannot hold more pending pairings than the loose cap"
    );
    // Another network is unaffected.
    let other = NewPairing {
        ip_key_loose: digest_of("another network"),
        ..pairing(6)
    };
    world
        .store
        .begin_pairing(other, LIFETIME, world.now())
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn the_hard_limit_on_pending_pairings_refuses_a_flood_without_scanning_it() {
    let tunables = Tunables {
        display_pending_hard_limit: 3,
        ..Tunables::default()
    };
    let world = World::with_tunables("hardlimit", tunables).await;
    for number in 1..=3 {
        world
            .store
            .begin_pairing(new_pairing(number), LIFETIME, world.now())
            .await
            .unwrap();
    }
    assert_eq!(
        refusal_of(
            world
                .store
                .begin_pairing(new_pairing(4), LIFETIME, world.now())
                .await
        ),
        Refusal::Throttled
    );
    world.cleanup().await;
}
