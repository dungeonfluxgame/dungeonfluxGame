use super::*;

#[tokio::test]
async fn an_analysis_final_committed_after_expiry_stays_private_on_read_watch_and_replay() {
    let journal = FakeJournal::new();
    let observations = AnalysisObservations::default();
    let clock = FakeClock::new(START_MS);
    let service = ImageService::new(
        journal,
        config(),
        clock.clone(),
        FakeIds::new(),
        AnalysisAuth(FakeAuth::permitting(draft_owner())),
    )
    .unwrap()
    .with_observations(observations.clone());
    let mut request = request(operation_id(0xed));
    request.identity.fingerprint = service.canonical_analysis_fingerprint(&request).unwrap();
    let accepted = recorded_view(
        service
            .request_analysis(&TestPrincipal, &request)
            .await
            .unwrap(),
    );
    clock
        .now
        .store(accepted.absolute_deadline_ms + 1, Ordering::SeqCst);
    *observations.0.lock().unwrap() = Some(ImageAnalysisObservation {
        selected_at_ms: accepted.absolute_deadline_ms + 1,
        job_revision: accepted.job_revision,
        result: ImageAnalysisResult::Segmentation {
            source: source_ref(),
            contract_revision: 1,
            masks: vec![crate::AnalysisMaskSet {
                prompt: SegmentationPrompt::Person,
                width_px: 1024,
                height_px: 1024,
                instances: Vec::new(),
            }],
        },
    });
    let read = service
        .read(&TestPrincipal, &accepted.handle)
        .await
        .unwrap();
    assert_eq!(read.handle, accepted.handle);
    assert!(read.analysis_result.is_none());
    assert!(read.result.is_none());
    assert_ne!(read.execution, ImageExecutionState::Completed);
    assert!(matches!(
        service.watch(&TestPrincipal, &accepted.handle, 0).await.unwrap(),
        WatchObservation::Changed(view) if view.analysis_result.is_none(),
    ));
    assert!(matches!(
        service.request_analysis(&TestPrincipal, &request).await.unwrap(),
        RequestOutcome::Replayed(view) if view.analysis_result.is_none(),
    ));
}

#[tokio::test]
async fn an_in_time_analysis_selection_remains_readable_after_the_work_deadline() {
    let clock = FakeClock::new(START_MS);
    let observations = AnalysisObservations::default();
    let service = ImageService::new(
        FakeJournal::new(),
        config(),
        clock.clone(),
        FakeIds::new(),
        AnalysisAuth(FakeAuth::permitting(draft_owner())),
    )
    .unwrap()
    .with_observations(observations.clone());
    let mut request = request(operation_id(0xef));
    request.identity.fingerprint = service.canonical_analysis_fingerprint(&request).unwrap();
    let accepted = recorded_view(
        service
            .request_analysis(&TestPrincipal, &request)
            .await
            .unwrap(),
    );
    let result = ImageAnalysisResult::Segmentation {
        source: source_ref(),
        contract_revision: 1,
        masks: vec![crate::AnalysisMaskSet {
            prompt: SegmentationPrompt::Person,
            width_px: 1024,
            height_px: 1024,
            instances: Vec::new(),
        }],
    };
    *observations.0.lock().unwrap() = Some(ImageAnalysisObservation {
        job_revision: accepted.job_revision,
        selected_at_ms: accepted.absolute_deadline_ms - 1,
        result: result.clone(),
    });
    clock
        .now
        .store(accepted.absolute_deadline_ms + 1, Ordering::SeqCst);
    let read = service
        .read(&TestPrincipal, &accepted.handle)
        .await
        .unwrap();
    assert_eq!(read.analysis_result, Some(result));
    assert_eq!(read.execution, ImageExecutionState::Completed);
}
