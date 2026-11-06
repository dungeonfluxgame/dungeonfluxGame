//! Retained standalone responses and GET-only segmentation reconciliation.
use super::*;

impl<J, E, S, P, F, C, I, K, B, T, Q, U> NativeImageRuntime<J, E, S, P, F, C, I, K, B, T, Q, U>
where
    J: ImageJournal,
    E: ImageExecutionJournal,
    S: ImageSources,
    P: ImagePublisher,
    F: ImageDispatchAdmission,
    C: ServiceClock,
    I: RuntimeIds,
    K: ImageCredentials,
    B: ImageCostProjection,
    T: ImageRuntimeTelemetry,
    Q: ImageRuntimeCostPolicy,
    U: ImageUsageJournal,
{
    pub(in crate::runtime) async fn reconcile_standalone(
        &self,
        job: ImageJob,
        lease: &RecoveryLease,
    ) -> Result<Option<crate::publication::PublishedImage>, RuntimeError> {
        let intent = analysis_codec::decode_analysis(job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let config = super::retained_config(&job)?;
        let recovery = &lease.stage;
        let mut stage = self
            .execution
            .read_stage(&job.owner, &job.job, &recovery.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        stage.claim_generation = recovery.claim_generation;
        let context = PublicationContext {
            job,
            stage,
            dispatch: recovery.current.dispatch,
            claim: recovery.claim,
            now_ms: self.clock.now_ms(),
            usage_revision: recovery.current.usage_revision,
            recovery: Some(lease.clone()),
        };
        let mode = recovery
            .current
            .finance
            .map_or(DispatchMode::Local, DispatchMode::Hosted);
        let raw = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?
            .ok_or(RuntimeError::ProviderUnknown)?;
        if recovery.current.state == DispatchState::Failed {
            if raw.evidence == UsageEvidence::Unknown {
                self.admission
                    .outcome(&context.job, context.dispatch, mode, None)
                    .await?;
                self.project_usage(&raw).await?;
            } else {
                self.finish_no_output(&context, &raw, mode).await?;
            }
            return Ok(None);
        }
        let maximum = usize::try_from(
            self.execution
                .read_capacity_bounds(self.settings.capacity.as_str())
                .await
                .map_err(RuntimeError::Execution)?
                .max_decision_payload_bytes,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        if let Some(retained) = &raw.raw_output
            && let Ok((response, dependencies, previous, prompt_index)) = decode_raw(retained)
        {
            let result = match interpret(
                &response,
                &intent,
                &dependencies,
                previous.as_ref(),
                prompt_index,
                maximum,
            ) {
                Ok(result) => result,
                Err(_) if !raw.usable => {
                    // A crash after raw retention still follows the same unusable-output settlement
                    // path.
                    return match self
                        .finish_analysis_response(
                            &context,
                            mode,
                            &raw.route,
                            AnalysisInterpretation {
                                intent: &intent,
                                dependencies: &dependencies,
                                previous,
                                prompt_index,
                                maximum,
                                supplier_elapsed: None,
                            },
                            response,
                            None,
                        )
                        .await
                    {
                        Ok(()) => Ok(None),
                        Err(error)
                            if raw.evidence != UsageEvidence::Unknown
                                && error.failure_cause().is_some() =>
                        {
                            Ok(None)
                        }
                        Err(error) => Err(error),
                    };
                }
                Err(error) => return Err(error),
            };
            let useful = carries_judgement(&result);
            let usage = if raw.usable == useful {
                raw
            } else {
                self.retain_usage(
                    &context,
                    &raw.route,
                    raw.evidence,
                    &raw.units,
                    Some(retained),
                    useful,
                )
                .await?
            };
            self.finish_analysis(
                &context,
                mode,
                &usage,
                AnalysisFinish {
                    intent: &intent,
                    result: &result,
                    dependencies: &dependencies,
                    maximum,
                },
            )
            .await?;
            return Ok(None);
        }
        let ImageAnalysisOperation::Segmentation(_) = &intent.operation else {
            return Err(RuntimeError::ProviderUnknown);
        };
        let remaining = lease
            .lease_until_ms
            .checked_sub(self.clock.now_ms())
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or(RuntimeError::Expired)?;
        let timeout = Duration::from_millis(remaining);
        let inputs = self.analysis_inputs(&intent).await?;
        let consumed = self
            .usage
            .read_sources(&scope(&context))
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        if consumed != inputs.receipts {
            return Err(RuntimeError::SourceDenied);
        }
        let previous = self.analysis_previous(&context.job, &context.stage).await?;
        let prompt_index = previous.as_ref().map_or(0, |result| match result {
            ImageAnalysisResult::Segmentation { masks, .. } => masks.len(),
            _ => 0,
        });
        let (prepared, inputs, _) = self
            .prepare_analysis(
                &intent,
                inputs,
                &config,
                AnalysisPreparation {
                    prompt_index,
                    timeout,
                    maximum,
                    mode: PreparationMode::Resume,
                },
            )
            .await?;
        let Prepared::Segmentation(prepared) = prepared else {
            return Err(RuntimeError::InvalidRecord);
        };
        let stored = self
            .execution
            .read_supplier_checkpoint(
                &context.job.owner,
                &context.job.job,
                &context.stage.stage,
                &context.dispatch,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        let checkpoint = if let Some(stored) = stored {
            if stored.origin != "fal-sam-analysis"
                || stored.checkpoint_version != 1
                || stored.model != raw.route.model
            {
                return Err(RuntimeError::InvalidRecord);
            }
            backend::SegmentationCheckpoint::decode(
                std::str::from_utf8(&stored.checkpoint_bytes)
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            )?
        } else {
            // A crash between POST receipt retention and queue-checkpoint retention resumes from
            // that exact response.
            let (status, body) = analysis::restore_response_bytes(
                raw.raw_output
                    .as_deref()
                    .ok_or(RuntimeError::ProviderUnknown)?,
                maximum,
            )?;
            let checkpoint = prepared.checkpoint_parts(status, &body)?;
            self.execution
                .record_supplier_checkpoint(
                    &SupplierCheckpoint::new(
                        scope(&context),
                        SupplierResume {
                            checkpoint_version: 1,
                            origin: "fal-sam-analysis".to_owned(),
                            model: raw.route.model.clone(),
                            checkpoint_bytes: checkpoint.encode().into_bytes(),
                        },
                        self.clock.now_ms(),
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                )
                .await
                .map_err(RuntimeError::Execution)?;
            checkpoint
        };
        let resume_started = std::time::Instant::now();
        let response = match prepared.resume(&checkpoint).await {
            Ok(response) => response,
            Err(error) => {
                let cause = if error == RuntimeError::Expired {
                    FailureCause::DeadlineExceeded
                } else {
                    FailureCause::TransportLost
                };
                return self
                    .analysis_unknown(
                        &context,
                        &raw.route,
                        mode,
                        Some(cause),
                        Some(resume_started.elapsed()),
                    )
                    .await;
            }
        };
        let supplier_elapsed = Some(resume_started.elapsed());
        self.finish_analysis_response(
            &context,
            mode,
            &raw.route,
            AnalysisInterpretation {
                intent: &intent,
                dependencies: &inputs.dependencies,
                previous,
                prompt_index,
                maximum,
                supplier_elapsed,
            },
            RawAnalysis {
                response: AnalysisResponse::Http {
                    status: response.status,
                    body: response.body,
                },
                segmentation: Some((prepared.width_px, prepared.height_px)),
            },
            None,
        )
        .await?;
        Ok(None)
    }
}
