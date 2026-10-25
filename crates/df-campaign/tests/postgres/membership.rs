//! Invitations, room codes, join requests, removal, readmission, seats and roles.

use df_campaign::store::{NewInvitation, NewProposal};
use std::sync::Arc;

use df_campaign::{
    config::Tunables,
    errors::{Failure, Outcome, Refusal},
    join_invitation::{JoinFailure, JoinRole},
    records::{CampaignStatus, EventKind, MemberStatus},
    roles::{CampaignRole, Delegation, Delegations},
    store::{CampaignStore, Decision},
};

use crate::fixture::{World, digest_of, operation_id, token_digest};

pub fn refusal_of<T: std::fmt::Debug>(outcome: Outcome<T>) -> Refusal {
    match outcome {
        Err(Failure::Refused(refusal)) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn two_simultaneous_accepts_fill_exactly_the_open_seats() {
    let world = Arc::new(World::new("seats").await);
    let host = world.account(1).await;
    let first = world.account(2).await;
    let second = world.account(3).await;
    // Two seats: the host holds one, one is open.
    let campaign = world.campaign(host, 2).await;
    let tokens = [token_digest(), token_digest()];
    for token in tokens {
        world
            .store
            .create_invitation(
                host,
                campaign.id,
                NewInvitation {
                    joins_as: JoinRole::Player,
                    lifetime_ms: 3_600_000,
                    max_uses: 5,
                    token_digest: token,
                },
                world.now(),
            )
            .await
            .unwrap();
    }
    // Transaction 1 holds the campaign lock after counting; both accepts must wait for it.
    let holder = world.db.raw_as(&world.role).await;
    holder.client.batch_execute("BEGIN").await.unwrap();
    holder
        .client
        .query(
            "SELECT 1 FROM campaigns.campaign WHERE campaign_id = $1 FOR UPDATE",
            &[&campaign.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for (account, token) in [first, second].into_iter().zip(tokens) {
        let world = Arc::clone(&world);
        tasks.push(tokio::spawn(async move {
            world
                .store
                .accept_invitation(account, token, operation_id(), world.now())
                .await
        }));
    }
    world.wait_for_lock_waiters(2).await;
    holder.client.batch_execute("COMMIT").await.unwrap();
    let mut joined = 0;
    let mut full = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => joined += 1,
            Err(Failure::Refused(Refusal::Join(JoinFailure::Full))) => full += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((joined, full), (1, 1));
    let snapshot = world
        .store
        .member_snapshot(host, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.seat_count, 2);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn contributors_are_bounded_by_the_contributor_limit_and_need_no_seat() {
    let tunables = Tunables {
        maximum_contributors: 1,
        ..Tunables::default()
    };
    let world = World::with_tunables("contributors", tunables).await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 1).await;
    let first = world.account(2).await;
    let second = world.account(3).await;
    world
        .join(campaign.id, host, first, JoinRole::Contributor)
        .await;
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Contributor,
                lifetime_ms: 3_600_000,
                max_uses: 5,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    let refused = refusal_of(
        world
            .store
            .accept_invitation(second, token, operation_id(), world.now())
            .await,
    );
    assert_eq!(refused, Refusal::Join(JoinFailure::ContributorsFull));
    // A full table does not stop a contributor link, and a player link is full.
    let player = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 3_600_000,
                max_uses: 5,
                token_digest: player,
            },
            world.now(),
        )
        .await
        .unwrap();
    let full = refusal_of(
        world
            .store
            .accept_invitation(second, player, operation_id(), world.now())
            .await,
    );
    assert_eq!(full, Refusal::Join(JoinFailure::Full));
    world.cleanup().await;
}

#[tokio::test]
async fn invitation_failures_have_their_own_reasons_and_expiry_is_exclusive() {
    let world = World::new("invitefail").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let token = token_digest();
    let row = world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 1_000,
                max_uses: 1,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    // One millisecond before expiry it previews; at expiry it does not.
    assert!(
        world
            .store
            .preview_invitation(token, world.now() + 999)
            .await
            .is_ok()
    );
    let expired = refusal_of(
        world
            .store
            .preview_invitation(token, world.now() + 1_000)
            .await,
    );
    assert_eq!(expired, Refusal::Join(JoinFailure::Expired));
    // Revoked.
    world
        .store
        .revoke_invitation(host, campaign.id, row.id, world.now())
        .await
        .unwrap();
    let revoked = refusal_of(world.store.preview_invitation(token, world.now()).await);
    assert_eq!(revoked, Refusal::Join(JoinFailure::Revoked));
    // Exhausted.
    let single = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 1,
                token_digest: single,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_invitation(guest, single, operation_id(), world.now())
        .await
        .unwrap();
    let other = world.account(3).await;
    let exhausted = refusal_of(
        world
            .store
            .accept_invitation(other, single, operation_id(), world.now())
            .await,
    );
    assert_eq!(exhausted, Refusal::Join(JoinFailure::Exhausted));
    // Unknown.
    let unknown = refusal_of(world.store.preview_invitation([9; 32], world.now()).await);
    assert_eq!(unknown, Refusal::InvitationUnknown);
    // An active member is already joined, even on a spent link.
    let again = refusal_of(
        world
            .store
            .accept_invitation(guest, single, operation_id(), world.now())
            .await,
    );
    assert_eq!(again, Refusal::Join(JoinFailure::AlreadyJoined));
    world.cleanup().await;
}

#[tokio::test]
async fn the_preview_carries_the_campaign_facts_and_the_host_account_and_nothing_else() {
    let world = World::new("preview").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 2,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    let facts = world
        .store
        .preview_invitation(token, world.now())
        .await
        .unwrap();
    assert_eq!(facts.host_account, host);
    assert_eq!(facts.campaign.name, "The Sunken Bell");
    world.cleanup().await;
}

#[tokio::test]
async fn a_repeated_accept_returns_the_first_result_and_a_changed_payload_conflicts() {
    let world = World::new("idempotent").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let token = token_digest();
    let other_token = token_digest();
    for digest in [token, other_token] {
        world
            .store
            .create_invitation(
                host,
                campaign.id,
                NewInvitation {
                    joins_as: JoinRole::Player,
                    lifetime_ms: 60_000,
                    max_uses: 5,
                    token_digest: digest,
                },
                world.now(),
            )
            .await
            .unwrap();
    }
    let operation = operation_id();
    let first = world
        .store
        .accept_invitation(guest, token, operation, world.now())
        .await
        .unwrap();
    // The repeat is resolved before any invitation check, so it works even after revocation.
    let repeat = world
        .store
        .accept_invitation(guest, token, operation, world.now())
        .await
        .unwrap();
    assert_eq!(first, repeat);
    let conflict = refusal_of(
        world
            .store
            .accept_invitation(guest, other_token, operation, world.now())
            .await,
    );
    assert_eq!(conflict, Refusal::OperationConflict);
    let events = world.events(campaign.id).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::Joined)
            .count(),
        2
    );
    // A receipt older than ninety days never executes.
    let late = world.now() + 91 * 24 * 3_600_000;
    let expired = refusal_of(
        world
            .store
            .accept_invitation(guest, token, operation, late)
            .await,
    );
    assert_eq!(expired, Refusal::OperationExpired);
    world.cleanup().await;
}

#[tokio::test]
async fn create_campaign_is_idempotent_and_refuses_a_changed_payload() {
    let world = World::new("createop").await;
    let owner = world.account(1).await;
    let operation = operation_id();
    let new = |name: &str| df_campaign::records::NewCampaign {
        tenant: crate::fixture::tenant_id(),
        owner,
        name: name.to_owned(),
        seat_count: 3,
        content_rating: df_campaign::records::ContentRating::Standard,
        room_codes: df_campaign::records::RoomCodes::Off,
        default_co_host_delegations: Delegations::NONE,
    };
    let first = world
        .store
        .create_campaign(new("Alpha"), operation, digest_of("a"), world.now())
        .await
        .unwrap();
    let repeat = world
        .store
        .create_campaign(new("Alpha"), operation, digest_of("a"), world.now())
        .await
        .unwrap();
    assert_eq!(first.id, repeat.id);
    let conflict = refusal_of(
        world
            .store
            .create_campaign(new("Beta"), operation, digest_of("b"), world.now())
            .await,
    );
    assert_eq!(conflict, Refusal::OperationConflict);
    world.cleanup().await;
}

#[tokio::test]
async fn no_member_is_added_to_an_archived_campaign_or_by_a_closing_account() {
    let world = World::new("unavailable").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let closing = world.account(3).await;
    let campaign = world.campaign(host, 4).await;
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 5,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    world.db.set_account_status(closing, "closing").await;
    let by_closing = refusal_of(
        world
            .store
            .accept_invitation(closing, token, operation_id(), world.now())
            .await,
    );
    assert_eq!(by_closing, Refusal::Join(JoinFailure::AccountClosing));
    world
        .store
        .archive_campaign(
            host,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
    let archived = refusal_of(
        world
            .store
            .accept_invitation(guest, token, operation_id(), world.now())
            .await,
    );
    assert_eq!(archived, Refusal::Join(JoinFailure::CampaignUnavailable));
    world.cleanup().await;
}

#[tokio::test]
async fn a_room_code_always_ends_in_approval_with_the_approvers_choice_of_role() {
    let world = World::new("codes").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let code = digest_of("code-1");
    world
        .store
        .rotate_code(host, campaign.id, code, 3_600_000, world.now())
        .await
        .unwrap();
    // The code previews, and requesting creates a pending request, not a member.
    assert!(
        world
            .store
            .lookup_code(guest, code, world.now())
            .await
            .is_ok()
    );
    let request = world
        .store
        .request_join(guest, code, world.now())
        .await
        .unwrap();
    assert!(
        world
            .store
            .member_snapshot(guest, campaign.id)
            .await
            .unwrap()
            .is_none()
    );
    // A repeat returns the same pending request.
    let again = world
        .store
        .request_join(guest, code, world.now())
        .await
        .unwrap();
    assert_eq!(again.id, request.id);
    let member = world
        .store
        .decide_request(
            host,
            campaign.id,
            request.id,
            Decision::Approve(JoinRole::Contributor),
            world.now(),
        )
        .await
        .unwrap()
        .expect("approval creates a member");
    assert_eq!(member.role, CampaignRole::Contributor);
    assert!(!member.holds_seat);
    // A code never resolves to an existing member.
    let existing = refusal_of(world.store.lookup_code(guest, code, world.now()).await);
    assert_eq!(existing, Refusal::Join(JoinFailure::AlreadyJoined));
    // Rotating revokes the old code.
    world
        .store
        .rotate_code(
            host,
            campaign.id,
            digest_of("code-2"),
            3_600_000,
            world.now(),
        )
        .await
        .unwrap();
    let stranger = world.account(3).await;
    let old = refusal_of(world.store.lookup_code(stranger, code, world.now()).await);
    assert_eq!(old, Refusal::CodeUnknown);
    world.cleanup().await;
}

#[tokio::test]
async fn pending_join_requests_are_capped_per_campaign() {
    let tunables = Tunables {
        pending_join_request_cap: 2,
        ..Tunables::default()
    };
    let world = World::with_tunables("requestcap", tunables).await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let code = digest_of("cap");
    world
        .store
        .rotate_code(host, campaign.id, code, 3_600_000, world.now())
        .await
        .unwrap();
    for number in 2..4 {
        let account = world.account(number).await;
        world
            .store
            .request_join(account, code, world.now())
            .await
            .unwrap();
    }
    let third = world.account(4).await;
    let full = refusal_of(world.store.request_join(third, code, world.now()).await);
    assert_eq!(full, Refusal::RequestsFull);
    world.cleanup().await;
}

#[tokio::test]
async fn a_removed_account_is_not_readmitted_by_a_request_until_allowed_back() {
    let world = World::new("allowback").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let member = world.join(campaign.id, host, guest, JoinRole::Player).await;
    world
        .store
        .remove_member(host, campaign.id, member, world.now())
        .await
        .unwrap();
    let code = digest_of("readmit");
    world
        .store
        .rotate_code(host, campaign.id, code, 3_600_000, world.now())
        .await
        .unwrap();
    let request = world
        .store
        .request_join(guest, code, world.now())
        .await
        .unwrap();
    let refused = refusal_of(
        world
            .store
            .decide_request(
                host,
                campaign.id,
                request.id,
                Decision::Approve(JoinRole::Player),
                world.now(),
            )
            .await,
    );
    assert_eq!(refused, Refusal::AllowBackFirst);
    // An old link posted earlier cannot readmit someone who was not allowed back.
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 5,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    let blocked = refusal_of(
        world
            .store
            .accept_invitation(guest, token, operation_id(), world.now())
            .await,
    );
    assert_eq!(blocked, Refusal::Join(JoinFailure::NoLongerValid));
    // Allowing back reactivates the same row and member id.
    world
        .store
        .allow_rejoin(host, campaign.id, member, world.now())
        .await
        .unwrap();
    let accepted = world
        .store
        .accept_invitation(guest, token, operation_id(), world.now())
        .await
        .unwrap();
    assert!(accepted.rejoined);
    assert_eq!(accepted.member, member);
    let snapshot = world
        .store
        .member_snapshot(guest, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.status, MemberStatus::Active);
    assert_eq!(
        snapshot.member_revision, 4,
        "joined, removed, allowed back, rejoined"
    );
    world.cleanup().await;
}

#[tokio::test]
async fn the_rank_table_decides_who_removes_and_readmits_whom() {
    let world = World::new("rank").await;
    let host = world.account(1).await;
    let cohost = world.account(2).await;
    let other_cohost = world.account(3).await;
    let player = world.account(4).await;
    let campaign = world.campaign(host, 6).await;
    let cohost_member = world
        .join(campaign.id, host, cohost, JoinRole::Player)
        .await;
    let other_member = world
        .join(campaign.id, host, other_cohost, JoinRole::Player)
        .await;
    let player_member = world
        .join(campaign.id, host, player, JoinRole::Player)
        .await;
    let invite = Delegations::NONE.with(Delegation::InvitePlayers);
    world
        .store
        .set_co_host(host, campaign.id, cohost_member, invite, world.now())
        .await
        .unwrap();
    world
        .store
        .set_co_host(host, campaign.id, other_member, invite, world.now())
        .await
        .unwrap();
    // Nobody removes the host, a co-host does not remove a co-host, a player removes nobody.
    let host_member = world
        .store
        .member_snapshot(host, campaign.id)
        .await
        .unwrap()
        .unwrap()
        .member;
    assert_eq!(
        refusal_of(
            world
                .store
                .remove_member(cohost, campaign.id, host_member, world.now())
                .await
        ),
        Refusal::CannotRemoveHost
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .remove_member(cohost, campaign.id, other_member, world.now())
                .await
        ),
        Refusal::TargetOutranks
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .remove_member(player, campaign.id, cohost_member, world.now())
                .await
        ),
        Refusal::NotAllowed
    );
    // A co-host removes the player; the host removes the co-host.
    world
        .store
        .remove_member(cohost, campaign.id, player_member, world.now())
        .await
        .unwrap();
    world
        .store
        .remove_member(host, campaign.id, other_member, world.now())
        .await
        .unwrap();
    // Allowing back needs the remover's rank: the player was removed by a co-host, so another
    // co-host may; the co-host removed by the host cannot be allowed back by a co-host.
    world
        .store
        .allow_rejoin(cohost, campaign.id, player_member, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .allow_rejoin(cohost, campaign.id, other_member, world.now())
                .await
        ),
        Refusal::NotAllowed
    );
    world
        .store
        .allow_rejoin(host, campaign.id, other_member, world.now())
        .await
        .unwrap();
    // Nobody removes the owner: the owner is the host here, so use a transfer first.
    world.cleanup().await;
}

#[tokio::test]
async fn nobody_removes_the_owner_even_after_a_host_transfer() {
    let world = World::new("owner").await;
    let owner = world.account(1).await;
    let successor = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let successor_member = world
        .join(campaign.id, owner, successor, JoinRole::Player)
        .await;
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: df_campaign::records::TransferKind::Host,
                to_member: successor_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(successor, campaign.id, proposal, world.now())
        .await
        .unwrap();
    let owner_member = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap()
        .member;
    assert_eq!(
        refusal_of(
            world
                .store
                .remove_member(successor, campaign.id, owner_member, world.now())
                .await
        ),
        Refusal::CannotRemoveOwner
    );
    // The owner cannot leave before an ownership transfer, and the host not before a host one.
    assert_eq!(
        refusal_of(
            world
                .store
                .leave_campaign(owner, campaign.id, world.now())
                .await
        ),
        Refusal::OwnerMustTransferFirst
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .leave_campaign(successor, campaign.id, world.now())
                .await
        ),
        Refusal::HostMustTransferFirst
    );
    world.cleanup().await;
}

#[tokio::test]
async fn links_outlive_neither_their_creators_right_nor_their_seat() {
    let world = World::new("links").await;
    let host = world.account(1).await;
    let cohost = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let member = world
        .join(campaign.id, host, cohost, JoinRole::Player)
        .await;
    let invite = Delegations::NONE.with(Delegation::InvitePlayers);
    world
        .store
        .set_co_host(host, campaign.id, member, invite, world.now())
        .await
        .unwrap();
    let token = token_digest();
    world
        .store
        .create_invitation(
            cohost,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 60_000,
                max_uses: 5,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    assert!(
        world
            .store
            .preview_invitation(token, world.now())
            .await
            .is_ok()
    );
    // Taking the bit away revokes the links the co-host made, with one event.
    world
        .store
        .set_co_host(host, campaign.id, member, Delegations::NONE, world.now())
        .await
        .unwrap();
    let revoked = refusal_of(world.store.preview_invitation(token, world.now()).await);
    assert_eq!(revoked, Refusal::Join(JoinFailure::Revoked));
    let events = world.events(campaign.id).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::InvitationsRevoked)
            .count(),
        1
    );
    // And a co-host without the bit cannot create a link any more.
    let refused = refusal_of(
        world
            .store
            .create_invitation(
                cohost,
                campaign.id,
                NewInvitation {
                    joins_as: JoinRole::Player,
                    lifetime_ms: 60_000,
                    max_uses: 5,
                    token_digest: token_digest(),
                },
                world.now(),
            )
            .await,
    );
    assert_eq!(refused, Refusal::NotAllowed);
    world.cleanup().await;
}

#[tokio::test]
async fn a_contributor_cannot_be_made_co_host_and_only_host_and_cohost_may_be_seatless() {
    let world = World::new("cohostrules").await;
    let host = world.account(1).await;
    let contributor = world.account(2).await;
    let player = world.account(3).await;
    let campaign = world.campaign(host, 4).await;
    let contributor_member = world
        .join(campaign.id, host, contributor, JoinRole::Contributor)
        .await;
    let player_member = world
        .join(campaign.id, host, player, JoinRole::Player)
        .await;
    assert_eq!(
        refusal_of(
            world
                .store
                .set_co_host(
                    host,
                    campaign.id,
                    contributor_member,
                    Delegations::ALL,
                    world.now()
                )
                .await
        ),
        Refusal::ContributorCannotBeCoHost
    );
    // A player cannot use the self seat route; the host can step out and back in.
    assert_eq!(
        refusal_of(
            world
                .store
                .set_seat(player, campaign.id, false, world.now())
                .await
        ),
        Refusal::NotAllowed
    );
    world
        .store
        .set_seat(host, campaign.id, false, world.now())
        .await
        .unwrap();
    let seatless = world
        .store
        .member_snapshot(host, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!seatless.holds_seat);
    assert_eq!(seatless.role, CampaignRole::Host);
    world
        .store
        .set_seat(host, campaign.id, true, world.now())
        .await
        .unwrap();
    // A player becomes a contributor and back, bounded by the limits.
    world
        .store
        .set_player_role(
            host,
            campaign.id,
            player_member,
            JoinRole::Contributor,
            world.now(),
        )
        .await
        .unwrap();
    let changed = world
        .store
        .member_snapshot(player, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.role, CampaignRole::Contributor);
    assert!(!changed.holds_seat);
    world
        .store
        .set_player_role(
            host,
            campaign.id,
            player_member,
            JoinRole::Player,
            world.now(),
        )
        .await
        .unwrap();
    let back = world
        .store
        .member_snapshot(player, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert!(back.holds_seat);
    world.cleanup().await;
}

#[tokio::test]
async fn a_seat_cannot_be_retaken_when_the_table_is_full() {
    let world = World::new("seatfull").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 2).await;
    world.join(campaign.id, host, guest, JoinRole::Player).await;
    world
        .store
        .set_seat(host, campaign.id, false, world.now())
        .await
        .unwrap();
    let other = world.account(3).await;
    world.join(campaign.id, host, other, JoinRole::Player).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .set_seat(host, campaign.id, true, world.now())
                .await
        ),
        Refusal::SeatsFull
    );
    world.cleanup().await;
}

#[tokio::test]
async fn archive_needs_the_owner_and_the_typed_name_and_refuses_a_session_past_the_lobby() {
    let world = World::new("archive").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    world.join(campaign.id, host, guest, JoinRole::Player).await;
    assert_eq!(
        refusal_of(
            world
                .store
                .archive_campaign(
                    guest,
                    campaign.id,
                    "The Sunken Bell".into(),
                    false,
                    world.now()
                )
                .await
        ),
        Refusal::OwnerOnly
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .archive_campaign(host, campaign.id, "wrong".into(), false, world.now())
                .await
        ),
        Refusal::ConfirmationRequired
    );
    assert_eq!(
        refusal_of(
            world
                .store
                .archive_campaign(
                    host,
                    campaign.id,
                    "The Sunken Bell".into(),
                    true,
                    world.now()
                )
                .await
        ),
        Refusal::SessionInProgress
    );
    world
        .store
        .archive_campaign(
            host,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
    let snapshot = world
        .store
        .member_snapshot(guest, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.campaign_status, CampaignStatus::Archived);
    assert!(
        snapshot.authorized,
        "members still read an archived campaign"
    );
    // Revoking means re-check, not deny: the event is campaign scope and revoking.
    let events = world.events(campaign.id).await;
    let archived = events
        .iter()
        .find(|event| event.kind == EventKind::Archived)
        .unwrap();
    assert!(archived.revoking);
    world
        .store
        .unarchive_campaign(host, campaign.id, world.now())
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn an_operation_id_older_than_ninety_days_never_executes_even_after_its_receipt_was_purged() {
    let world = World::new("opage").await;
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 200 * 24 * 3_600_000,
                max_uses: 5,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    let operation = operation_id();
    world
        .store
        .accept_invitation(guest, token, operation, world.now())
        .await
        .unwrap();
    // Retention purged the receipt; the id still carries its issue time.
    world
        .db
        .admin
        .client
        .execute("DELETE FROM campaigns.campaign_operation", &[])
        .await
        .unwrap();
    let late = world.now() + 91 * 24 * 3_600_000;
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_invitation(guest, token, operation, late)
                .await
        ),
        Refusal::OperationExpired,
        "without the age check this would run again"
    );
    // A fresh id issued now is a new operation, however late.
    let fresh = df_campaign::ids::new_operation_id(late).unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_invitation(guest, token, fresh, late)
                .await
        ),
        Refusal::Join(JoinFailure::AlreadyJoined)
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_co_host_without_the_invite_bit_is_refused_before_the_requesters_account_is_locked() {
    let mut world = World::new("deciderorder").await;
    let host = world.account(1).await;
    let cohost = world.account(2).await;
    let requester = world.account(3).await;
    let campaign = world.campaign(host, 4).await;
    let member = world
        .join(campaign.id, host, cohost, JoinRole::Player)
        .await;
    world
        .store
        .set_co_host(host, campaign.id, member, Delegations::NONE, world.now())
        .await
        .unwrap();
    let code = digest_of("order-code");
    world
        .store
        .rotate_code(host, campaign.id, code, 3_600_000, world.now())
        .await
        .unwrap();
    let request = world
        .store
        .request_join(requester, code, world.now())
        .await
        .unwrap();
    // B holds the requester's account row, as a close in progress would.
    let b_role = world.db.accounts_login_role(Some("10s")).await;
    let mut b = world.db.raw_as(&b_role).await;
    let tx = b.client.transaction().await.unwrap();
    tx.execute(
        "UPDATE accounts.account SET status = 'active' WHERE account_id = $1",
        &[&requester.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    let refused = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        world.store.decide_request(
            cohost,
            campaign.id,
            request.id,
            Decision::Approve(JoinRole::Player),
            world.now(),
        ),
    )
    .await
    .expect("the refusal did not wait for another account's lock");
    assert_eq!(refusal_of(refused), Refusal::NotAllowed);
    tx.rollback().await.unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn only_the_host_moves_people_between_seats_and_contributions() {
    let world = World::new("hostonlyrole").await;
    let host = world.account(1).await;
    let cohost = world.account(2).await;
    let player = world.account(3).await;
    let campaign = world.campaign(host, 4).await;
    let cohost_member = world
        .join(campaign.id, host, cohost, JoinRole::Player)
        .await;
    let player_member = world
        .join(campaign.id, host, player, JoinRole::Player)
        .await;
    let invite = Delegations::NONE.with(Delegation::InvitePlayers);
    world
        .store
        .set_co_host(host, campaign.id, cohost_member, invite, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .set_player_role(
                    cohost,
                    campaign.id,
                    player_member,
                    JoinRole::Contributor,
                    world.now()
                )
                .await
        ),
        Refusal::NotAllowed,
        "a co-host holding the invite bit still cannot change roles"
    );
    world
        .store
        .set_player_role(
            host,
            campaign.id,
            player_member,
            JoinRole::Contributor,
            world.now(),
        )
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn an_account_cannot_own_more_campaigns_than_the_configured_bound() {
    let tunables = Tunables {
        campaigns_per_account: 2,
        ..Tunables::default()
    };
    let world = World::with_tunables("campaigncap", tunables).await;
    let owner = world.account(1).await;
    world.campaign(owner, 3).await;
    world.campaign(owner, 3).await;
    let third = world
        .store
        .create_campaign(
            df_campaign::records::NewCampaign {
                tenant: crate::fixture::tenant_id(),
                owner,
                name: "Third".into(),
                seat_count: 3,
                content_rating: df_campaign::records::ContentRating::Standard,
                room_codes: df_campaign::records::RoomCodes::Off,
                default_co_host_delegations: Delegations::NONE,
            },
            operation_id(),
            digest_of("third"),
            world.now(),
        )
        .await;
    assert_eq!(refusal_of(third), Refusal::TooManyCampaigns);
    world.cleanup().await;
}
