use super::*;

#[tokio::test]
async fn resolved_owned_reference_preserves_real_revision_and_actual_empty_usage() {
    let config = ImageConfig::defaults().unwrap();
    let request = public_request(87);
    let artifact = artifact(&request, &config);
    let view = stored_view(&request, &config, 90);
    let handle = OperationHandle {
        owner: request.identity.owner.clone(),
        operation: view.receipt.operation,
    };
    let mut demand = crate::service::project_demand(&handle, &view).unwrap();
    demand.execution = ImageExecutionState::Queued;
    demand.usage = Some(crate::runtime::ImageUsageSummary {
        job: view.job.job,
        stages: Vec::new(),
    });
    let mut service = service(config, vec![view.clone()], None);
    service.results.artifact = Some(artifact.clone());
    service
        .project_owned_artifact(&handle, &view, &mut demand)
        .await
        .unwrap();
    assert_eq!(demand.artifact, Some(artifact));
    assert_eq!(demand.job, view.job.job);
    assert_eq!(demand.job_revision, view.job.revision);
    assert_eq!(demand.interest, view.receipt.interest);
    assert_eq!(demand.execution, ImageExecutionState::Ready);
    assert!(demand.usage.unwrap().stages.is_empty());
    assert!(demand.result.is_none());
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn uncertain_demand_and_changed_dependency_revision_never_adopt_an_artifact() {
    let config = ImageConfig::defaults().unwrap();
    let request = public_request(87);
    let artifact = artifact(&request, &config);
    let view = stored_view(&request, &config, 90);
    let handle = OperationHandle {
        owner: request.identity.owner.clone(),
        operation: view.receipt.operation,
    };
    for state in [ImageExecutionState::Unknown, ImageExecutionState::Uncertain] {
        let mut service = service(config.clone(), vec![view.clone()], None);
        service.results.artifact = Some(artifact.clone());
        let mut demand = crate::service::project_demand(&handle, &view).unwrap();
        demand.execution = state;
        service
            .project_owned_artifact(&handle, &view, &mut demand)
            .await
            .unwrap();
        assert!(demand.artifact.is_none());
        assert_eq!(demand.execution, state);
        assert_eq!(service.results.reads.load(Ordering::SeqCst), 0);
    }
    let mut newer = view.clone();
    newer.job.revision += 1;
    let mut service = service(config, vec![newer], None);
    service.results.artifact = Some(artifact);
    let mut demand = crate::service::project_demand(&handle, &view).unwrap();
    demand.execution = ImageExecutionState::Queued;
    assert_eq!(
        service
            .project_owned_artifact(&handle, &view, &mut demand)
            .await
            .unwrap_err(),
        ImageServiceError::Unknown { handle },
    );
    assert!(demand.artifact.is_none());
    assert_eq!(demand.job_revision, view.job.revision);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_actual_recorded_stage_keeps_its_own_usage_and_never_adopts_an_artifact() {
    let config = ImageConfig::defaults().unwrap();
    let request = public_request(87);
    let artifact = artifact(&request, &config);
    let view = stored_view(&request, &config, 90);
    let handle = OperationHandle {
        owner: request.identity.owner.clone(),
        operation: view.receipt.operation,
    };
    let mut demand = crate::service::project_demand(&handle, &view).unwrap();
    demand.execution = ImageExecutionState::Queued;
    demand.usage = Some(crate::runtime::ImageUsageSummary {
        job: view.job.job,
        stages: vec![crate::runtime::ImageStageUsage {
            stage: crate::execution::ExecutionStageId::from_bytes(&[91; 16]).unwrap(),
            dispatch: crate::execution::DispatchId::from_bytes(&[92; 16]).unwrap(),
            kind: crate::execution::StageKind::Generation,
            observation: crate::runtime::StageUsageObservation::Pending,
            amount: crate::runtime::ImageUsageAmount::Unknown,
        }],
    });
    let original = demand.clone();
    let mut service = service(config, vec![view.clone()], None);
    service.results.artifact = Some(artifact);
    service
        .project_owned_artifact(&handle, &view, &mut demand)
        .await
        .unwrap();
    assert_eq!(demand, original);
    assert_eq!(service.results.reads.load(Ordering::SeqCst), 0);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}
