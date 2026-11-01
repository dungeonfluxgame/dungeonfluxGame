use super::*;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, atomic::AtomicUsize};

#[derive(Default)]
struct StartBarrier {
    entered: tokio::sync::Notify,
    released: Mutex<bool>,
    changed: Condvar,
    starts: AtomicUsize,
}
impl StartBarrier {
    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.changed.notify_all();
    }
}
impl port::MeterObserver for StartBarrier {
    fn started(&self, _: &port::MeterStart) {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        let released = self.released.lock().unwrap();
        let (released, timed_out) = self
            .changed
            .wait_timeout_while(released, Duration::from_secs(30), |released| !*released)
            .unwrap();
        let passed = *released && !timed_out.timed_out();
        drop(released);
        assert!(passed, "bounded actual Book start barrier expired");
    }
    fn settled(&self, _: &port::MeteredCall) {}
}
struct ReleaseStart(Arc<StartBarrier>);
impl Drop for ReleaseStart {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn real_unsent_book_opening_is_sealed_and_acknowledged_after_public_cancellation() {
    use df_imagery::*;
    let ledger = Arc::new(SpendLedger::default());
    let port = port::MeteringPort::from_ledger(ledger.clone());
    let barrier = Arc::new(StartBarrier::default());
    port.observe(barrier.clone());
    let endpoint = CostImageEndpoint::new(1024, 640, CostImageReply::Silent);
    let patch = cost_image_patch(
        endpoint.address,
        BackendId::OpenAi,
        1024,
        640,
        GenerationQuality::Low,
        false,
    );
    let Some(mut fixture) =
        crate::gameplay::image_test_support::ImageHostFixture::open(endpoint.address, patch, port)
            .await
    else {
        return;
    };
    let log = fixture.root().join("actual-unsent-cost.log");
    let opened = ledger.clone();
    let opening_log = log.clone();
    tokio::task::spawn_blocking(move || {
        opened
            .costs()
            .open(
                "df_gameplay_demo_test",
                Some(&opening_log),
                table(),
                PricingStatus::NotLoaded,
                1_000,
            )
            .unwrap();
    })
    .await
    .unwrap();
    let release = ReleaseStart(barrier.clone());
    let context = fixture.imagery().consumer_context().await.unwrap();
    let request = cost_image_request(&context, 47, Purpose::ItemCloseUp, vec![]);
    let view = match context
        .service
        .request(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("actual unsent demand refused: {other:?}"),
    };
    tokio::time::timeout(Duration::from_secs(15), barrier.entered.notified())
        .await
        .unwrap();
    let (mut database, connection) = fixture
        .database()
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    let driver = tokio::spawn(connection);
    let before = database.query_one(
        "SELECT d.dispatch_id,d.stage_id,d.book_call_id,d.dispatch_state,d.settlement, \
         d.local_accounting,d.finance_attempt,d.cost_unsent_ack, \
         EXISTS(SELECT 1 FROM imagery.image_publication_allocation a WHERE a.job_id=d.job_id) AS admitted, \
         (SELECT count(*) FROM imagery.image_dispatch_usage u WHERE u.dispatch_id=d.dispatch_id) AS raw_count \
         FROM imagery.image_dispatch_attempt d WHERE d.job_id=$1",
        &[&view.job.as_bytes().as_slice()],
    ).await.unwrap();
    assert_eq!(before.get::<_, String>("dispatch_state"), "prepared");
    assert_eq!(before.get::<_, String>("settlement"), "held");
    assert!(before.get::<_, bool>("local_accounting"));
    assert!(
        before
            .get::<_, Option<Vec<u8>>>("finance_attempt")
            .is_none()
    );
    assert!(!before.get::<_, bool>("cost_unsent_ack"));
    assert!(before.get::<_, bool>("admitted"));
    assert_eq!(before.get::<_, i64>("raw_count"), 0);
    let dispatch: [u8; 16] = before.get::<_, Vec<u8>>("dispatch_id").try_into().unwrap();
    let stage: [u8; 16] = before.get::<_, Vec<u8>>("stage_id").try_into().unwrap();
    let call_id: String = before.get("book_call_id");
    assert_eq!(call_id, df_imagery::cost::pinned_call_id(&dispatch));
    let spec = {
        let state = state_of(ledger.costs());
        let Some(PinnedSlot::Open(opening, durable)) = state.pinned.get(&call_id) else {
            panic!("actual prepared dispatch must own a real Book opening");
        };
        assert!(
            durable.load(Ordering::Acquire),
            "opening is durably persisted before cancellation"
        );
        opening.spec.clone()
    };
    assert!(endpoint.posts().is_empty());
    assert!(matches!(
        context
            .service
            .cancel(&context.principal, &view.handle)
            .await
            .unwrap(),
        CancelOutcome::Withdrawn(_)
    ));
    assert!(matches!(
        context
            .service
            .cancel(&context.principal, &view.handle)
            .await
            .unwrap(),
        CancelOutcome::AlreadyWithdrawn(_)
    ));
    let cancelled = database
        .query_one(
            "SELECT j.state,d.dispatch_state,d.cost_unsent_ack FROM imagery.image_job j \
             JOIN imagery.image_dispatch_attempt d USING(job_id) WHERE d.dispatch_id=$1",
            &[&dispatch.as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(cancelled.get::<_, String>("state"), "retired");
    assert_eq!(cancelled.get::<_, String>("dispatch_state"), "prepared");
    assert!(!cancelled.get::<_, bool>("cost_unsent_ack"));
    drop(release);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let row = database.query_one("SELECT dispatch_state,settlement,cost_unsent_ack FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1", &[&dispatch.as_slice()]).await.unwrap();
        if row.get::<_, bool>("cost_unsent_ack") {
            assert_eq!(row.get::<_, String>("dispatch_state"), "released");
            assert_eq!(row.get::<_, String>("settlement"), "released");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "actual native cleanup did not acknowledge the durable Book seal"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let tx = database.transaction().await.unwrap();
    let receipt =
        df_persistence::imagery_publication::read_no_delivery_in(&tx, &context.owner, &view.job)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        receipt.reason,
        df_imagery::runtime::NoDeliveryReason::Retired
    );
    assert_eq!(
        receipt.tail,
        df_imagery::publication::NoDeliveryStage::Dispatch {
            scope: df_imagery::execution::ExecutionScope {
                owner: context.owner.clone(),
                job: view.job,
                stage: df_imagery::execution::ExecutionStageId::from_bytes(&stage).unwrap(),
                dispatch: df_imagery::execution::DispatchId::from_bytes(&dispatch).unwrap(),
            },
            usage_revision: 0,
        }
    );
    assert_eq!(receipt.actor, receipt.tail);
    tx.commit().await.unwrap();
    let owner = context.owner.clone();
    drop(context);
    assert!(ledger.costs().pinned_durable(&call_id, None).unwrap());
    assert_eq!(
        ledger.costs().start_pinned(&call_id, spec.clone()),
        Ok(PinnedStart::AlreadySettled)
    );
    assert_eq!(ledger.costs().totals().calls, 0);
    assert_eq!(state_of(ledger.costs()).session_totals.events, 0);
    assert!(endpoint.posts().is_empty());
    fixture.restart().await;
    let tx = database.transaction().await.unwrap();
    assert_eq!(
        df_persistence::imagery_publication::read_no_delivery_in(&tx, &owner, &view.job)
            .await
            .unwrap()
            .unwrap(),
        receipt
    );
    assert!(
        df_persistence::imagery_execution::scan_unsent_prepared_in(
            &tx,
            "gameplay-images",
            None,
            1,
            i64::try_from(now_ms()).unwrap()
        )
        .await
        .unwrap()
        .is_empty()
    );
    let row = tx.query_one("SELECT cost_unsent_ack,usage_revision,(SELECT count(*) FROM imagery.image_dispatch_usage u WHERE u.dispatch_id=d.dispatch_id) AS raw_count FROM imagery.image_dispatch_attempt d WHERE dispatch_id=$1", &[&dispatch.as_slice()]).await.unwrap();
    assert!(row.get::<_, bool>("cost_unsent_ack"));
    assert_eq!(row.get::<_, i32>("usage_revision"), 0);
    assert_eq!(row.get::<_, i64>("raw_count"), 0);
    tx.commit().await.unwrap();
    assert_eq!(barrier.starts.load(Ordering::SeqCst), 1);
    assert!(endpoint.posts().is_empty());
    assert_eq!(ledger.costs().totals().calls, 0);
    drop(database);
    driver.await.unwrap().unwrap();
    let closed = ledger.clone();
    tokio::task::spawn_blocking(move || closed.costs().close())
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || {
        let recovered = opened_book("actual-unsent-replay", Some(&log));
        assert!(recovered.pinned_durable(&call_id, None).unwrap());
        assert_eq!(
            recovered.start_pinned(&call_id, spec),
            Ok(PinnedStart::AlreadySettled)
        );
        assert_eq!(recovered.totals().calls, 0);
        recovered.close();
    })
    .await
    .unwrap();
    fixture.close().await;
}
