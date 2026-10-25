//! Plan section 17, SAVE-01 to SAVE-05: save, resume and checkpoint contracts (plan 13.7).
//!
//! SAVE-01..03 are real tests against the O1 save-store contract (`df_persistence::save_store`)
//! driven through its in-memory faithful store plus fake membership, fence and finance ports;
//! the PostgreSQL writer is covered in `df-persistence` against the scratch server. SAVE-04
//! and SAVE-05 remain blocked on compaction and versioned migration evidence.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;

use df_acceptance::blocked_scenario;
use df_persistence::save_store::{
    BranchId, BranchRequest, CheckpointEnvelope, CreateSavePoint, ManualClock, MembershipChecker,
    MemoryFenceClaimer, MemorySaveStore, ResumeRequest, SaveError, SaveLink, SaveLinkKind,
    SavePointId, SaveStore, resume,
};
use df_types::{CampaignId, MemberId, OperationId, TenantId};

fn tenant() -> TenantId {
    TenantId::from_bytes(&[0xa1; 16]).expect("tenant")
}
fn campaign() -> CampaignId {
    CampaignId::from_bytes(&[0xc1; 16]).expect("campaign")
}
fn member(byte: u8) -> MemberId {
    MemberId::from_bytes(&[byte; 16]).expect("member")
}
fn resume_request() -> ResumeRequest {
    ResumeRequest {
        tenant_id: tenant(),
        campaign_id: campaign(),
        member_id: member(7),
        branch_id: branch(1),
    }
}

fn branch(byte: u8) -> BranchId {
    BranchId::from_bytes(&[byte; 16]).expect("branch")
}
fn save_id(byte: u8) -> SavePointId {
    SavePointId::from_bytes(&[byte; 16]).expect("save point")
}

#[derive(Default)]
struct FakeMembership {
    current: BTreeSet<MemberId>,
}
impl MembershipChecker for FakeMembership {
    fn is_current_member(
        &self,
        _tenant: TenantId,
        _campaign: CampaignId,
        member_id: MemberId,
        _now_seconds: u64,
    ) -> bool {
        self.current.contains(&member_id)
    }
}

/// F's journal, as the branch sees it: it is never an input to a branch, so it cannot move.
struct FakeFinance {
    total_micro_dollars: u64,
    generation_ids: Vec<[u8; 16]>,
}

fn create(seed: u8, branch_id: BranchId) -> CreateSavePoint {
    CreateSavePoint {
        save_point_id: save_id(seed),
        tenant_id: tenant(),
        campaign_id: campaign(),
        branch_id,
        parent_save_point_id: None,
        checkpoint: CheckpointEnvelope {
            bytes: vec![seed; 48],
            codec_version: 3,
            schema_version: 1,
            content_version: 1,
        },
        links: vec![
            SaveLink {
                kind: SaveLinkKind::FinanceAdmission,
                linked_id: [0xf1; 16],
            },
            SaveLink {
                kind: SaveLinkKind::PaidGeneration,
                linked_id: [0x61; 16],
            },
        ],
        operation_id: OperationId::from_bytes(&[seed; 16]).expect("operation"),
        fingerprint: [seed; 32],
        result_bytes: format!("result-{seed}").into_bytes(),
    }
}

#[test]
fn save_01_a_campaign_restarts_after_more_than_an_hour_despite_expired_credentials() {
    let mut store = MemorySaveStore::new();
    store
        .create_save_point(create(6, branch(1)))
        .expect("checkpoint");
    let mut membership = FakeMembership::default();
    membership.current.insert(member(7));
    let mut fences = MemoryFenceClaimer::default();
    let mut clock = ManualClock {
        now_seconds: 10_000,
    };
    let first =
        resume(&store, &membership, &mut fences, &clock, resume_request()).expect("first resume");

    // More than an hour passes; every session credential from the session has expired.
    // Resume takes no credential, so expiry cannot block a current member.
    clock.advance(3 * 60 * 60);
    let resumed = resume(&store, &membership, &mut fences, &clock, resume_request())
        .expect("resume after three hours");
    assert_eq!(resumed.save_point.save_point_id, save_id(6));
    assert!(
        resumed.fence.claim_seq > first.fence.claim_seq,
        "resume takes a NEW owner fence"
    );

    // A member revoked meanwhile cannot resume, and no fence is claimed for them.
    membership.current.remove(&member(7));
    let revoked = resume(&store, &membership, &mut fences, &clock, resume_request());
    assert_eq!(revoked, Err(SaveError::NotMember));
}

#[test]
fn save_02_a_crash_after_the_state_commit_but_before_the_response_returns_the_original_result() {
    let mut store = MemorySaveStore::new();
    let request = create(9, branch(1));
    // The commit lands; the reply is lost (the injected failure is after commit).
    let _lost_reply = store.create_save_point(request.clone()).expect("commit");

    let retried = store.create_save_point(request.clone()).expect("retry");
    assert!(
        !retried.committed_now,
        "the retry returns the stored result"
    );
    assert_eq!(retried.save_point_id, save_id(9));
    assert_eq!(retried.result_bytes, b"result-9".to_vec());
    assert_eq!(store.save_point_count(), 1, "the state advanced once");

    let mut changed = request.clone();
    changed.fingerprint = [42; 32];
    assert_eq!(
        store.create_save_point(changed),
        Err(SaveError::OperationConflict),
        "the same operation with a different payload is a conflict, never a second decision"
    );
    assert_eq!(store.save_point_count(), 1);
}

#[test]
fn save_03_an_older_save_branch_keeps_the_financial_journal_and_paid_generations_forward_only() {
    let mut store = MemorySaveStore::new();
    // Checkpoint 6, then checkpoint 20 on the original branch.
    store
        .create_save_point(create(6, branch(1)))
        .expect("checkpoint 6");
    let mut twenty = create(20, branch(1));
    twenty.parent_save_point_id = Some(save_id(6));
    store.create_save_point(twenty).expect("checkpoint 20");
    let finance = FakeFinance {
        total_micro_dollars: 3_000_000,
        generation_ids: vec![[0x61; 16]],
    };

    // Branch from checkpoint 6 and restore the campaign itself to checkpoint 6: each is a
    // new branch head; neither takes any finance input.
    let branched = store
        .branch_from(BranchRequest {
            tenant_id: tenant(),
            campaign_id: campaign(),
            source_save_point_id: save_id(6),
            new_save_point_id: save_id(26),
            new_branch_id: branch(2),
            operation_id: OperationId::from_bytes(&[26; 16]).expect("operation"),
            fingerprint: [26; 32],
            result_bytes: b"branch-result".to_vec(),
        })
        .expect("branch");
    assert!(branched.committed_now);
    let restored = store
        .branch_from(BranchRequest {
            tenant_id: tenant(),
            campaign_id: campaign(),
            source_save_point_id: save_id(6),
            new_save_point_id: save_id(27),
            new_branch_id: branch(3),
            operation_id: OperationId::from_bytes(&[27; 16]).expect("operation"),
            fingerprint: [27; 32],
            result_bytes: b"restore-result".to_vec(),
        })
        .expect("restore");
    assert!(restored.committed_now);

    assert_eq!(
        finance.total_micro_dollars, 3_000_000,
        "nothing is refunded or re-opened"
    );
    assert_eq!(finance.generation_ids.len(), 1, "paid generations persist");
    for id in [save_id(26), save_id(27)] {
        let point = store.save_point(id).expect("branch save");
        assert_eq!(point.parent_save_point_id, Some(save_id(6)));
        assert_eq!(
            point.links.len(),
            2,
            "finance and generation links stay forward"
        );
        assert_eq!(point.checkpoint.bytes, vec![6; 48]);
    }
    // The original branch still heads at checkpoint 20; the branch did not overwrite it.
    let head = store
        .latest_save_point(tenant(), campaign(), branch(1))
        .expect("original head");
    assert_eq!(head.save_point_id, save_id(20));
}

blocked_scenario! {
    fn save_04_a_large_six_player_campaign_keeps_checkpoint_compaction_and_recovery_within_bounds,
    blocked_on: "blocked on O1 API: checkpoint compaction that keeps semantic and billing links, with measured size and recovery-time bounds",
    setup: [
        "synthetic six-player campaign with 5,000 accepted decisions, 600 assets and 300 paid operations on PostgreSQL",
        "bounds read from configuration: maximum checkpoint bytes, maximum events replayed on recovery, maximum recovery seconds",
    ],
    steps: [
        "play to 5,000 decisions with compaction at the configured interval, then kill and recover",
        "assert: each checkpoint stays under the byte bound, recovery replays no more than the event bound and finishes inside the time bound",
        "assert: compaction preserved the links from decisions to billing records, and an old save still validates against the compacted history",
    ],
    outcome: "Checkpoint compaction and recovery stay within bounds",
}

blocked_scenario! {
    fn save_05_a_save_from_an_old_schema_or_content_version_migrates_or_refuses_read_only,
    blocked_on: "blocked on O1 API: versioned checkpoint reader with migration evidence and a read-only refusal outcome",
    setup: [
        "fixture saves: one at the previous schema version, one at the previous content version, one from an unknown future version, one corrupted",
    ],
    steps: [
        "open each save through the production resume path",
        "assert: the previous schema save migrates with recorded evidence and the old bytes remain intact until the new one is verified",
        "assert: the unsupported and corrupted saves open read-only with a typed explanation and can still be exported; the original is never modified",
        "assert: no migration runs on promotion without evidence, and a failed migration leaves the save resumable by the previous build",
    ],
    outcome: "Tested migration or safe read-only refusal",
}
