use super::*;

fn late_image(view: &ImageDemandView) -> crate::publication::PublishedImage {
    let intent =
        df_assets::ImageIntent::new("image/png", 1, 1, df_assets::ImageAlpha::Opaque).unwrap();
    crate::publication::PublishedImage {
        job: view.job,
        stage: crate::execution::ExecutionStageId::from_bytes(&[1; 16]).unwrap(),
        dispatch: crate::execution::DispatchId::from_bytes(&[2; 16]).unwrap(),
        asset: AssetId::from_bytes(&[3; 16]).unwrap(),
        attempt: df_assets::AttemptId::from_bytes(&[4; 16]).unwrap(),
        recipe_digest: [5; 32],
        job_revision: view.job_revision,
        published_at_ms: view.absolute_deadline_ms + 1,
        private_reconciliation: true,
        variants: vec![crate::publication::PublishedVariant {
            variant: df_assets::AssetVariant::Master,
            write: df_assets::WriteId::from_bytes(&[6; 16]).unwrap(),
            object_version: df_assets::ObjectVersion::new("late-immutable-version".to_owned())
                .unwrap(),
            byte_len: 1,
            sha256: [7; 32],
            intent,
            alpha: df_assets::ImageAlpha::Opaque,
        }],
    }
}

#[tokio::test]
async fn a_late_private_publication_receipt_keeps_history_readable_and_final_output_private() {
    let (service, handles) = setup(draft_owner(), live_config());
    let results = FakeResults {
        result: Arc::new(Mutex::new(None)),
        reads: Arc::new(AtomicU64::new(0)),
    };
    let service = service.with_results(results.clone());
    let request = stamp(generate_request(draft_owner(), operation_id(0xeb)));
    let accepted = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    {
        let mut stored = handles.journal.locked();
        stored.jobs.get_mut(&accepted.job).unwrap().state = JobState::RecipeFrozen;
    }
    handles
        .clock
        .now
        .store(accepted.absolute_deadline_ms + 1, Ordering::SeqCst);
    *results.result.lock().unwrap() = Some(late_image(&accepted));
    let reads_before = results.reads.load(Ordering::SeqCst);

    let read = service
        .read(&handles.principal, &accepted.handle)
        .await
        .unwrap();
    assert_eq!(read.handle, accepted.handle);
    assert_eq!(read.job, accepted.job);
    assert_eq!(read.interest, accepted.interest);
    assert!(read.result.is_none());
    assert!(read.analysis_result.is_none());
    assert!(read.inspection.is_none());
    assert!(read.staging.is_empty());
    assert_ne!(read.execution, crate::ImageExecutionState::Ready);
    assert!(matches!(
        service.watch(&handles.principal, &accepted.handle, 0).await.unwrap(),
        WatchObservation::Changed(view) if view.result.is_none(),
    ));
    assert!(matches!(
        service.request(&handles.principal, &request).await.unwrap(),
        RequestOutcome::Replayed(view) if view.result.is_none(),
    ));
    assert!(results.reads.load(Ordering::SeqCst) > reads_before);
}

#[derive(Clone)]
struct ExpiringResults {
    result: Arc<Mutex<Option<crate::publication::PublishedImage>>>,
    clock: FakeClock,
    expires_at_ms: Arc<AtomicI64>,
}
impl crate::publication::ImageResults for ExpiringResults {
    async fn read_result(
        &self,
        _: &ImageOwner,
        _: &ImageJobId,
    ) -> Result<Option<crate::publication::PublishedImage>, crate::publication::PublicationError>
    {
        let result = self.result.lock().unwrap().clone();
        let deadline_ms = self.expires_at_ms.load(Ordering::SeqCst);
        if deadline_ms > 0 {
            self.clock.now.store(deadline_ms, Ordering::SeqCst);
        }
        Ok(result)
    }
}

#[tokio::test]
async fn deadline_expiry_during_a_publication_read_never_attaches_the_returned_image() {
    let (service, handles) = setup(draft_owner(), live_config());
    let results = ExpiringResults {
        result: Arc::new(Mutex::new(None)),
        clock: handles.clock.clone(),
        expires_at_ms: Arc::new(AtomicI64::new(0)),
    };
    let service = service.with_results(results.clone());
    let request = stamp(generate_request(draft_owner(), operation_id(0xec)));
    let accepted = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    {
        let mut stored = handles.journal.locked();
        stored.jobs.get_mut(&accepted.job).unwrap().state = JobState::RecipeFrozen;
    }
    let mut image = late_image(&accepted);
    image.private_reconciliation = false;
    *results.result.lock().unwrap() = Some(image);
    results
        .expires_at_ms
        .store(accepted.absolute_deadline_ms, Ordering::SeqCst);
    let read = service
        .read(&handles.principal, &accepted.handle)
        .await
        .unwrap();
    assert!(read.result.is_none());
    assert_ne!(read.execution, crate::ImageExecutionState::Ready);
    assert_eq!(read.handle, accepted.handle);
}

#[tokio::test]
async fn an_in_time_public_image_remains_readable_after_the_work_deadline() {
    let (service, handles) = setup(draft_owner(), live_config());
    let results = FakeResults {
        result: Arc::new(Mutex::new(None)),
        reads: Arc::new(AtomicU64::new(0)),
    };
    let service = service.with_results(results.clone());
    let request = stamp(generate_request(draft_owner(), operation_id(0xee)));
    let accepted = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    {
        let mut stored = handles.journal.locked();
        stored.jobs.get_mut(&accepted.job).unwrap().state = JobState::RecipeFrozen;
    }
    let mut timely = late_image(&accepted);
    timely.published_at_ms = accepted.absolute_deadline_ms - 1;
    timely.private_reconciliation = false;
    *results.result.lock().unwrap() = Some(timely.clone());
    handles
        .clock
        .now
        .store(accepted.absolute_deadline_ms + 1, Ordering::SeqCst);
    let read = service
        .read(&handles.principal, &accepted.handle)
        .await
        .unwrap();
    assert_eq!(read.result, Some(timely.clone()));
    assert_eq!(read.execution, crate::ImageExecutionState::Ready);
    assert!(matches!(
        service.request(&handles.principal, &request).await.unwrap(),
        RequestOutcome::Replayed(view) if view.result == Some(timely),
    ));
}
