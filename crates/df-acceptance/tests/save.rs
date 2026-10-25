//! Plan section 17, SAVE-01 to SAVE-05: save, resume and checkpoint contracts (plan 13.7).
//!
//! The production save and resume contract, owner fencing and long-inactivity recovery belong to
//! O1. The nearest existing evidence (the `df-session` submission and host-pause contract tests
//! and the `df-persistence` owned-PostgreSQL suites) is listed in the acceptance matrix; these
//! scenarios state the hosted-product gate and are blocked until the production root exists.

use df_acceptance::blocked_scenario;

blocked_scenario! {
    fn save_01_a_campaign_restarts_after_more_than_an_hour_despite_expired_credentials,
    blocked_on: "blocked on O1 API: resume that authenticates membership afresh and claims a new owner fence without any stored short-lived token",
    setup: [
        "campaign C with a committed checkpoint; all access tokens, connection tickets and display tickets issued during the session have expired (controlled clock +3 hours)",
        "the runner process is gone and the old owner lease has lapsed",
    ],
    steps: [
        "member P signs in afresh and presses Resume",
        "assert: membership is re-authenticated, a new owner fence greater than the old one is claimed, and C resumes from the latest durable revision",
        "assert: grants that were stored in the saved game and have expired are re-issued or recovered by policy; the campaign is never unrecoverable because a credential expired",
        "assert: a former member who was revoked meanwhile cannot resume",
    ],
    outcome: "Campaign resumes despite old credential expiry",
}

blocked_scenario! {
    fn save_02_a_crash_after_the_state_commit_but_before_the_response_returns_the_original_result,
    blocked_on: "blocked on O1 and F API: idempotent decision submission with a durable operation receipt on the production composition, and F's retried admission returning the original reservation (finance design 26.1)",
    setup: [
        "session on the production composition with a fault hook that kills the connection after the database commit and before the reply",
        "player P submits a decision with idempotency key K that triggers a dice roll and a paid narration",
    ],
    steps: [
        "submit with K, lose the reply, reconnect and retry with K",
        "assert: the retry returns the original outcome with the same dice results, the game state advanced once, and the paid narration was dispatched once (a_retried_admission_with_the_same_operation_returns_the_original_reservation)",
        "retry with K and a different payload",
        "assert: the response is a conflict, never a second decision",
    ],
    outcome: "Retried operation returns original result",
}

blocked_scenario! {
    fn save_03_an_older_save_branch_keeps_the_financial_journal_and_paid_generations_forward_only,
    blocked_on: "blocked on O1, F and O2 API: branch-from-save-point and restore operations that take no financial input and copy asset references (finance design 26.1, asset design 18)",
    setup: [
        "campaign C at checkpoint 20; payer A spent $3.00 across checkpoints 6 to 20 and four paid images were generated and stored",
    ],
    steps: [
        "branch C from checkpoint 6 as C2 and play to the same point again; also restore C itself to checkpoint 6",
        "assert: A's journal still shows the $3.00 and nothing is refunded or re-opened (no_finance_operation_accepts_a_game_revision_and_a_branch_creates_no_funding)",
        "assert: replay of the branch uses the four stored images and makes no provider request; a regenerated image is a new explicit request with a new quote",
        "assert: the branch has a new identity and does not overwrite C's later checkpoints",
    ],
    outcome: "Financial journal and paid generations remain forward-only",
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
