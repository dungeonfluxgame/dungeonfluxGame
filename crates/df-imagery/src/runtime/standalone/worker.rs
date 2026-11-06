//! Durably fenced standalone supplier decisions and their metered stages.
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
    pub(in crate::runtime) async fn run_standalone(
        &self,
        job: ImageJob,
        stage: StageView,
    ) -> Result<Option<crate::publication::PublishedImage>, RuntimeError> {
        let intent = analysis_codec::decode_analysis(job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let config = super::retained_config(&job)?;
        if stage.kind != kind(&intent.operation) {
            return Err(RuntimeError::InvalidRecord);
        }
        if crate::service::analysis::analysis_policy(
            &self.settings.config,
            intent.purpose.purpose,
            intent.execution,
            false,
        ) != crate::PolicyDecision::Admit
        {
            return Err(RuntimeError::Disabled);
        }
        let now = self.clock.now_ms();
        let lease_ms = i64::try_from(config.scheduler.stage_deadline_secs)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .checked_mul(1000)
            .ok_or(RuntimeError::InvalidConfiguration)?
            .min(
                job.absolute_deadline_ms
                    .checked_sub(now)
                    .ok_or(RuntimeError::Expired)?,
            );
        let timeout = Duration::from_millis(
            u64::try_from(lease_ms)
                .ok()
                .filter(|value| *value > 0)
                .ok_or(RuntimeError::Expired)?,
        );
        let maximum = usize::try_from(
            self.execution
                .read_capacity_bounds(self.settings.capacity.as_str())
                .await
                .map_err(RuntimeError::Execution)?
                .max_decision_payload_bytes,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        let inputs = self.analysis_inputs(&intent).await?;
        let previous = self.analysis_previous(&job, &stage).await?;
        let prompt_index = previous.as_ref().map_or(0, |result| match result {
            ImageAnalysisResult::Segmentation { masks, .. } => masks.len(),
            _ => 0,
        });
        let (prepared, inputs, quota) = self
            .prepare_analysis(
                &intent,
                inputs,
                &config,
                AnalysisPreparation {
                    prompt_index,
                    timeout,
                    maximum,
                    mode: PreparationMode::Fresh,
                },
            )
            .await?;
        let claim = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let stage = self
            .execution
            .claim(
                &ClaimCommand::new(
                    job.owner.clone(),
                    job.job,
                    stage.stage,
                    self.settings.active_config_revision,
                    claim,
                    lease_ms,
                    now,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(crate::telemetry::EventKind::Claimed, &job, &stage, None);
        if let Prepared::Local(result) = prepared {
            return self
                .complete_local_analysis(job, stage, claim, &inputs, result, maximum)
                .await;
        }
        let mut quota = super::super::quota_gate::claim(quota)?;
        let dispatch = DispatchId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let (request_bytes, pictures) = prepared.request_facts()?;
        let mut basis = cost_basis(&job, &config)?;
        basis.prompt_bytes = request_bytes;
        basis.reference_count = pictures;
        let route = self.cost_policy.stage_route(&job, &config, &basis)?;
        let admission = DispatchAdmission {
            job: &job,
            stage: &stage,
            dispatch,
            claim,
            config: &config,
            quote: Some(&route),
            sources: &inputs.receipts,
        };
        let mode = self.admission.admit(&admission).await?;
        if mode.is_local() != stage.local_accounting {
            return Err(RuntimeError::InvalidRecord);
        }
        self.execution
            .prepare(
                &PrepareCommand::new(
                    StageClaim {
                        scope: StageScope {
                            owner: job.owner.clone(),
                            job: job.job,
                            stage: stage.stage,
                        },
                        token: claim,
                        generation: stage.claim_generation,
                    },
                    dispatch,
                    mode,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        let context = PublicationContext {
            job,
            stage,
            dispatch,
            claim,
            now_ms: self.clock.now_ms(),
            usage_revision: 0,
            recovery: None,
        };
        self.usage
            .record_sources(&scope(&context), &inputs.receipts)
            .await?;
        self.costs
            .start_pinned(&crate::cost::pinned_call_id(dispatch.as_bytes()), &route)
            .await
            .map_err(RuntimeError::Cost)?;
        self.emit(
            crate::telemetry::EventKind::Admitted,
            &context.job,
            &context.stage,
            Some(dispatch),
        );
        if self
            .admission
            .mark_sent(
                &DispatchAdmission {
                    job: &context.job,
                    stage: &context.stage,
                    dispatch,
                    claim,
                    config: &config,
                    quote: Some(&route),
                    sources: &inputs.receipts,
                },
                mode,
            )
            .await?
            != SendAuthorization::New
        {
            return self
                .analysis_unknown(&context, &route, mode, None, None)
                .await;
        }
        self.execution
            .mark_sent(
                &MarkSentCommand::new(
                    context.job.owner.clone(),
                    context.job.job,
                    context.stage.stage,
                    dispatch,
                    claim,
                    context.stage.claim_generation,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(
            crate::telemetry::EventKind::Sent,
            &context.job,
            &context.stage,
            Some(dispatch),
        );
        let execution_started = std::time::Instant::now();
        let response = self
            .execute_analysis(&context, &route, mode, prepared, &mut quota)
            .await?;
        let supplier_elapsed = Some(execution_started.elapsed());
        self.finish_analysis_response(
            &context,
            mode,
            &route,
            AnalysisInterpretation {
                intent: &intent,
                dependencies: &inputs.dependencies,
                previous,
                prompt_index,
                maximum,
                supplier_elapsed,
            },
            response,
            quota,
        )
        .await?;
        Ok(None)
    }

    pub(super) async fn analysis_previous(
        &self,
        job: &ImageJob,
        stage: &StageView,
    ) -> Result<Option<ImageAnalysisResult>, RuntimeError> {
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let Some(parent) = basis.parent_dispatch else {
            return Ok(None);
        };
        let dispatch = self
            .execution
            .read_dispatch(&job.owner, &parent)
            .await
            .map_err(RuntimeError::Execution)?;
        if dispatch.job != job.job {
            return Err(RuntimeError::InvalidRecord);
        }
        let checkpoint = self
            .execution
            .read_decision_checkpoint(&job.owner, &job.job, &dispatch.stage, &parent)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)?;
        Ok(Some(decode_payload(checkpoint.payload())?.0))
    }

    pub(super) async fn execute_analysis(
        &self,
        context: &PublicationContext,
        route: &crate::cost::CostRoute,
        mode: DispatchMode,
        prepared: Prepared,
        quota: &mut Option<super::super::quota::QuotaPermit>,
    ) -> Result<RawAnalysis, RuntimeError> {
        let execution_started = std::time::Instant::now();
        match prepared {
            Prepared::Photo(photo) => match photo.execute().await {
                Ok(response) => Ok(RawAnalysis {
                    response: AnalysisResponse::Http {
                        status: response.status,
                        body: response.body,
                    },
                    segmentation: None,
                }),
                Err(error) => Ok(RawAnalysis {
                    response: AnalysisResponse::Transport(if error == RuntimeError::Expired {
                        AnalysisTransportFailure::Deadline
                    } else {
                        AnalysisTransportFailure::Transport
                    }),
                    segmentation: None,
                }),
            },
            Prepared::Chat(chat) => match chat.execute().await {
                Ok(response) => Ok(RawAnalysis {
                    response: AnalysisResponse::Http {
                        status: response.status,
                        body: response.body,
                    },
                    segmentation: None,
                }),
                Err(error) => {
                    let cause = if error == RuntimeError::Expired {
                        FailureCause::DeadlineExceeded
                    } else {
                        FailureCause::TransportLost
                    };
                    self.analysis_unknown(
                        context,
                        route,
                        mode,
                        Some(cause),
                        Some(execution_started.elapsed()),
                    )
                    .await?;
                    Err(super::codec::supplier_failure(cause))
                }
            },
            Prepared::Segmentation(mut segmentation) => {
                let submitted = match segmentation.submit().await {
                    Ok(response) => response,
                    Err(error) => {
                        let cause = if error == RuntimeError::Expired {
                            FailureCause::DeadlineExceeded
                        } else {
                            FailureCause::TransportLost
                        };
                        self.analysis_unknown(
                            context,
                            route,
                            mode,
                            Some(cause),
                            Some(execution_started.elapsed()),
                        )
                        .await?;
                        return Err(super::codec::supplier_failure(cause));
                    }
                };
                // The POST receipt is durable before parsing the GET-only continuation.
                self.retain_usage(
                    context,
                    route,
                    UsageEvidence::Unknown,
                    &[],
                    Some(&analysis::retain_response(
                        submitted.status,
                        &submitted.body,
                    )),
                    false,
                )
                .await?;
                let checkpoint = match segmentation.checkpoint(&submitted) {
                    Ok(checkpoint) => checkpoint,
                    Err(_) => {
                        let answer = super::super::quota_gate::raw_answer(
                            submitted.status,
                            &submitted.body,
                            false,
                        );
                        super::super::quota_gate::observe(
                            quota.take(),
                            answer,
                            &self.telemetry,
                            self.event_context(
                                &context.job,
                                &context.stage,
                                Some(context.dispatch),
                            ),
                        );
                        return match self
                            .analysis_unknown(
                                context,
                                route,
                                mode,
                                Some(super::codec::http_failure(
                                    submitted.status,
                                    &submitted.body,
                                )),
                                Some(execution_started.elapsed()),
                            )
                            .await
                        {
                            Err(RuntimeError::ProviderUnknown)
                                if answer == super::super::quota::QuotaAnswer::Exhausted =>
                            {
                                Err(RuntimeError::SupplierQuotaExhausted)
                            }
                            Err(error) => Err(error),
                            Ok(_) => Err(RuntimeError::ProviderUnknown),
                        };
                    }
                };
                self.execution
                    .record_supplier_checkpoint(
                        &SupplierCheckpoint::new(
                            scope(context),
                            SupplierResume {
                                checkpoint_version: 1,
                                origin: "fal-sam-analysis".to_owned(),
                                model: route.model.clone(),
                                checkpoint_bytes: checkpoint.encode().into_bytes(),
                            },
                            self.clock.now_ms(),
                        )
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?;
                self.emit(
                    crate::telemetry::EventKind::SupplierCheckpointed,
                    &context.job,
                    &context.stage,
                    Some(context.dispatch),
                );
                let response = match segmentation.resume(&checkpoint).await {
                    Ok(response) => response,
                    Err(error) => {
                        let cause = if error == RuntimeError::Expired {
                            FailureCause::DeadlineExceeded
                        } else {
                            FailureCause::TransportLost
                        };
                        self.analysis_unknown(
                            context,
                            route,
                            mode,
                            Some(cause),
                            Some(execution_started.elapsed()),
                        )
                        .await?;
                        return Err(super::codec::supplier_failure(cause));
                    }
                };
                Ok(RawAnalysis {
                    response: AnalysisResponse::Http {
                        status: response.status,
                        body: response.body,
                    },
                    segmentation: Some((segmentation.width_px, segmentation.height_px)),
                })
            }
            Prepared::Local(_) => Err(RuntimeError::InvalidRecord),
        }
    }

    pub(super) async fn analysis_unknown(
        &self,
        context: &PublicationContext,
        route: &crate::cost::CostRoute,
        mode: DispatchMode,
        cause: Option<FailureCause>,
        supplier_elapsed: Option<Duration>,
    ) -> Result<Option<crate::publication::PublishedImage>, RuntimeError> {
        let previous = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?;
        let retained = previous
            .as_ref()
            .and_then(|usage| usage.raw_output.as_deref());
        let raw = match previous.as_ref() {
            Some(raw) if raw.evidence != UsageEvidence::Unknown => raw.clone(),
            _ => {
                self.retain_usage(context, route, UsageEvidence::Unknown, &[], retained, false)
                    .await?
            }
        };
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Unknown,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        if let Some(elapsed) = supplier_elapsed {
            event = event.elapsed(elapsed);
        }
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.raw_checkpointed = Some(raw.raw_output.is_some());
        self.telemetry.observe(event);
        let cause = context
            .recovery
            .as_ref()
            .and_then(|lease| lease.stage.current.cause)
            .or(cause);
        self.complete(
            context,
            cause.map_or(CompletionOutcome::Unknown, |cause| {
                CompletionOutcome::UnknownWithCause { cause }
            }),
        )
        .await?;
        self.project_usage(&raw).await?;
        self.admission
            .outcome(&context.job, context.dispatch, mode, None)
            .await?;
        Err(cause.map_or(
            RuntimeError::ProviderUnknown,
            super::codec::supplier_failure,
        ))
    }

    pub(super) async fn finish_analysis_response(
        &self,
        context: &PublicationContext,
        mode: DispatchMode,
        route: &crate::cost::CostRoute,
        interpretation: AnalysisInterpretation<'_>,
        response: RawAnalysis,
        quota: Option<super::super::quota::QuotaPermit>,
    ) -> Result<(), RuntimeError> {
        let AnalysisInterpretation {
            intent,
            dependencies,
            previous,
            prompt_index,
            maximum,
            supplier_elapsed,
        } = interpretation;
        let parsed = analysis::parse_response(response.body(), maximum);
        let (evidence, units) = if response.segmentation.is_some() {
            (UsageEvidence::Estimated, route.maxima.clone())
        } else {
            parsed
                .as_ref()
                .map(analysis::reported_usage)
                .unwrap_or((UsageEvidence::Unknown, Vec::new()))
        };
        let retained = encode_raw(
            &response,
            dependencies,
            previous.as_ref(),
            prompt_index,
            maximum,
        )?;
        // Raw bytes precede schema interpretation and economic qualification.
        let mut raw = self
            .retain_usage(context, route, evidence, &units, Some(&retained), false)
            .await?;
        let basis = self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event_context =
            self.event_context(&context.job, &context.stage, Some(context.dispatch));
        event_context.purpose = Some(intent.purpose.purpose);
        event_context.analysis_kind = Some(telemetry_kind(&intent.operation));
        event_context.config_revision = Some(basis.config_revision);
        event_context.parent_attempt = basis.parent_dispatch;
        let mut event = crate::telemetry::ImageEvent::new(
            if response.status().is_some() {
                crate::telemetry::EventKind::SupplierCompleted
            } else {
                crate::telemetry::EventKind::Unknown
            },
            event_context.clone(),
        );
        if let Some(elapsed) = supplier_elapsed {
            event = event.elapsed(elapsed);
        }
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.supplier_dispatched = Some(true);
        self.telemetry.observe(event);
        let result = interpret(
            &response,
            intent,
            dependencies,
            previous.as_ref(),
            prompt_index,
            maximum,
        );
        let useful = result.as_ref().is_ok_and(carries_judgement);
        let quota_answer = response
            .status()
            .map_or(super::super::quota::QuotaAnswer::Inconclusive, |status| {
                super::super::quota_gate::raw_answer(status, response.body(), useful)
            });
        super::super::quota_gate::observe(quota, quota_answer, &self.telemetry, event_context);
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let cause = super::codec::response_failure(&response);
                self.complete(
                    context,
                    if evidence == UsageEvidence::Unknown {
                        CompletionOutcome::UnknownWithCause {
                            cause: context
                                .recovery
                                .as_ref()
                                .and_then(|lease| lease.stage.current.cause)
                                .unwrap_or(cause),
                        }
                    } else {
                        CompletionOutcome::Unusable {
                            cause,
                            usage_revision: u32::try_from(raw.revision.0)
                                .map_err(|_| RuntimeError::InvalidRecord)?,
                        }
                    },
                )
                .await?;
                if evidence == UsageEvidence::Unknown {
                    self.admission
                        .outcome(&context.job, context.dispatch, mode, None)
                        .await?;
                    self.project_usage(&raw).await?;
                } else {
                    self.finish_no_output(context, &raw, mode).await?;
                }
                let mut context_event =
                    self.event_context(&context.job, &context.stage, Some(context.dispatch));
                context_event.purpose = Some(intent.purpose.purpose);
                context_event.analysis_kind = Some(telemetry_kind(&intent.operation));
                context_event.config_revision = Some(basis.config_revision);
                context_event.parent_attempt = basis.parent_dispatch;
                self.telemetry.observe(
                    crate::telemetry::ImageEvent::new(
                        if evidence == UsageEvidence::Unknown {
                            crate::telemetry::EventKind::Unknown
                        } else {
                            crate::telemetry::EventKind::Failed
                        },
                        context_event,
                    )
                    .failed(if error == RuntimeError::ProviderRefused {
                        crate::telemetry::FailureKind::ProviderRefused
                    } else {
                        crate::telemetry::FailureKind::Decode
                    }),
                );
                return Err(super::codec::supplier_failure(cause));
            }
        };
        let usable = carries_judgement(&result);
        raw = self
            .retain_usage(context, route, evidence, &units, Some(&retained), usable)
            .await?;
        self.finish_analysis(
            context,
            mode,
            &raw,
            AnalysisFinish {
                intent,
                result: &result,
                dependencies,
                maximum,
            },
        )
        .await
    }
}
