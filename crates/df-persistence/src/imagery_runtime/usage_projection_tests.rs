use super::*;
use df_imagery::cost::*;
use df_imagery::runtime::usage_projection::ImageUsageAmount;
use df_types::AccountId;
use std::sync::atomic::{AtomicUsize, Ordering};

fn limits() -> UsageProjectionLimits {
    UsageProjectionLimits {
        max_attempts: 4,
        max_observations: 4,
        max_unit_lines: 8,
        max_word_bytes: 128,
    }
}

#[test]
fn database_envelope_is_positive_checked_and_reads_one_extra_attempt() {
    let bounds = read_bounds(limits()).unwrap();
    assert_eq!(bounds.attempts_plus_one, 5);
    assert_eq!(bounds.unit_lines, 8);
    assert_eq!(bounds.word_bytes, 128);
    assert_eq!(bounds.unit_json_bytes, 8 * (6 * 128 + 96) + 2);
    for limits in [
        UsageProjectionLimits {
            max_attempts: usize::MAX,
            ..limits()
        },
        UsageProjectionLimits {
            max_word_bytes: usize::MAX,
            ..limits()
        },
        UsageProjectionLimits {
            max_unit_lines: 0,
            ..limits()
        },
    ] {
        assert!(matches!(
            read_bounds(limits),
            Err(JournalError::InvalidRecord)
        ));
    }
}

#[test]
fn query_selects_only_bounded_latest_metadata_for_exact_scoped_job() {
    let sql = query();
    for forbidden in [
        "raw_output",
        "intent_bytes",
        "config_bytes",
        "recipe_bytes",
        "u.*",
        "SELECT *",
    ] {
        assert!(!sql.contains(forbidden), "forbidden projection column");
    }
    assert!(sql.contains("WHERE j.job_id=$1 AND j.owner_scope=$2"));
    assert!(sql.contains("owner_scope=j.owner_scope AND job_id=j.job_id"));
    assert!(sql.contains("ORDER BY revision DESC LIMIT 1"));
    assert!(sql.contains("CASE WHEN octet_length(u.provider)"));
}

struct Reader {
    reads: AtomicUsize,
    fail: bool,
}
impl ImageCostProjection for Reader {
    fn start_pinned<'a>(
        &'a self,
        _: &'a str,
        _: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        panic!("read projection must not write")
    }
    fn correct<'a>(
        &'a self,
        _: &'a str,
        _: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        panic!("read projection must not write")
    }
    fn record_unsent<'a>(&'a self, _: &'a str) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        panic!("read projection must not write")
    }
    fn is_durable<'a>(
        &'a self,
        _: &'a str,
        _: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        panic!("receipt must be coherent")
    }
    fn read_receipt<'a>(
        &'a self,
        _: &'a str,
    ) -> CostFuture<'a, Result<Option<PinnedCostReceipt>, CostError>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            if self.fail {
                Err(CostError::BookNotOpen)
            } else {
                Ok(None)
            }
        })
    }
}

fn fixture() -> (ImageOwner, ImageJobId, UsageProjectionRows) {
    let owner = ImageOwner::Draft {
        account: AccountId::from_bytes(&[1; 16]).unwrap(),
    };
    let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
    let stage = ExecutionStageId::from_bytes(&[3; 16]).unwrap();
    let dispatch = df_imagery::execution::DispatchId::from_bytes(&[4; 16]).unwrap();
    let attempt = UsageAttempt {
        scope: ExecutionScope {
            owner: owner.clone(),
            job,
            stage,
            dispatch,
        },
        kind: StageKind::Generation,
        mode: DispatchMode::Local,
    };
    let usage = RuntimeUsage {
        owner: owner.clone(),
        job,
        stage,
        dispatch,
        revision: UsageRevision(1),
        route: CostRoute {
            provider: "openai".to_owned(),
            model: "image".to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "test".to_owned(),
            maxima: vec![CostUnits {
                category: "image_output".to_owned(),
                quantity: 5,
            }],
        },
        evidence: UsageEvidence::Reported,
        units: vec![CostUnits {
            category: "image_output".to_owned(),
            quantity: 2,
        }],
        usable: true,
        at_ms: 1,
        raw_output: None,
    };
    (
        owner,
        job,
        UsageProjectionRows {
            job_revision: 7,
            attempts: vec![attempt],
            observations: vec![usage],
        },
    )
}

#[tokio::test]
async fn book_read_runs_only_for_observed_attempt_and_never_fabricates_money() {
    let reader = Reader {
        reads: AtomicUsize::new(0),
        fail: false,
    };
    let (owner, job, mut rows) = fixture();
    rows.observations.clear();
    assert_eq!(
        project_rows(rows, &owner, job, &reader, limits())
            .await
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::Pending
    );
    assert_eq!(reader.reads.load(Ordering::Relaxed), 0);
    let (_, _, rows) = fixture();
    assert_eq!(
        project_rows(rows, &owner, job, &reader, limits())
            .await
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::Pending
    );
    assert_eq!(reader.reads.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn invalid_snapshot_is_rejected_before_book_and_book_failure_is_explicit() {
    let reader = Reader {
        reads: AtomicUsize::new(0),
        fail: true,
    };
    let (owner, job, mut rows) = fixture();
    rows.observations[0].job = ImageJobId::from_bytes(&[9; 16]).unwrap();
    assert!(matches!(
        project_rows(rows, &owner, job, &reader, limits()).await,
        Err(JournalError::InvalidRecord)
    ));
    assert_eq!(reader.reads.load(Ordering::Relaxed), 0);
    let (_, _, rows) = fixture();
    assert!(matches!(
        project_rows(rows, &owner, job, &reader, limits()).await,
        Err(JournalError::Unavailable)
    ));
    assert_eq!(reader.reads.load(Ordering::Relaxed), 1);
}

#[test]
fn stored_accounting_metadata_requires_exact_valid_local_or_hosted_tuple() {
    assert_eq!(
        decode_mode(true, None, None, None).unwrap(),
        DispatchMode::Local
    );
    assert!(matches!(
        decode_mode(
            false,
            Some(vec![1; 16]),
            Some(vec![2; 16]),
            Some(vec![3; 16])
        )
        .unwrap(),
        DispatchMode::Hosted(_)
    ));
    for (local, attempt, operation, effect) in [
        (false, None, None, None),
        (
            true,
            Some(vec![1; 16]),
            Some(vec![2; 16]),
            Some(vec![3; 16]),
        ),
        (false, Some(vec![1; 16]), None, Some(vec![3; 16])),
        (
            false,
            Some(vec![0; 16]),
            Some(vec![2; 16]),
            Some(vec![3; 16]),
        ),
        (
            false,
            Some(vec![1; 15]),
            Some(vec![2; 16]),
            Some(vec![3; 16]),
        ),
    ] {
        assert!(matches!(
            decode_mode(local, attempt, operation, effect),
            Err(JournalError::InvalidRecord)
        ));
    }
}

#[tokio::test]
async fn verified_local_transform_has_no_book_read_and_no_monetary_zero() {
    let reader = Reader {
        reads: AtomicUsize::new(0),
        fail: true,
    };
    let (owner, job, mut rows) = fixture();
    rows.attempts[0].kind = StageKind::LocalTransform;
    rows.observations.clear();
    assert_eq!(
        project_rows(rows, &owner, job, &reader, limits())
            .await
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::NotApplicable
    );
    assert_eq!(reader.reads.load(Ordering::Relaxed), 0);
}
