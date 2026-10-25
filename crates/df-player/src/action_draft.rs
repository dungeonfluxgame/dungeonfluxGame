//! Bounded presentation of one original ambiguously submitted action.
use df_client::receipts::{
    LookupOutcome, OperationKey, OperationResolution, ReceiptLookup, ResolutionError,
    RetirementError, UncertainOperation,
};
use df_protocol::common::{self as wire, DecisionReceipt, RejectionCode, SubmitActionRequest};
use df_types::{OperationId, RecoveryEpoch, RunId, SessionId, SessionRevision};

pub const MAX_RETAINED_ACTION_BYTES: usize = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDraftError {
    InvalidOriginalState,
    InvalidRequest,
    RequestTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionReceiptError {
    Identity,
    Revision,
    Outcome,
}

#[derive(PartialEq, Eq)]
pub enum ActionLookupError<E> {
    Lookup(E),
    InvalidReceipt(ActionReceiptError),
}

impl<E> std::fmt::Debug for ActionLookupError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lookup(_) => formatter.write_str("Lookup"),
            Self::InvalidReceipt(error) => formatter
                .debug_tuple("InvalidReceipt")
                .field(error)
                .finish(),
        }
    }
}

/// Status comes exclusively from the canonical operation's current observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDraftStatus {
    Uncertain,
    Pending,
    Accepted { revision: SessionRevision },
    Rejected(RejectionCode),
    Retired { epoch: RecoveryEpoch },
}

/// Owns the existing canonical operation, never another draft or submission queue.
/// Correlation identifiers are not authorization. The caller supplies an authorized
/// read-only lookup, and cancels that borrowed wait before replacing private scope.
pub struct ActionDraft {
    operation: UncertainOperation<SubmitActionRequest, DecisionReceipt>,
    run: RunId,
    observed: SessionRevision,
}

impl ActionDraft {
    pub fn from_uncertain(
        operation: UncertainOperation<SubmitActionRequest, DecisionReceipt>,
    ) -> Result<Self, ActionDraftError> {
        if !matches!(operation.resolution(), OperationResolution::Uncertain) {
            return Err(ActionDraftError::InvalidOriginalState);
        }
        let request = operation.intent();
        if retained_bytes(request).is_none_or(|bytes| bytes > MAX_RETAINED_ACTION_BYTES) {
            return Err(ActionDraftError::RequestTooLarge);
        }
        let operation_key = operation.key();
        let session_id = request
            .session_id
            .as_ref()
            .and_then(|id| SessionId::from_bytes(id.value.as_deref()?).ok());
        let operation_id = request
            .operation_id
            .as_ref()
            .and_then(|id| OperationId::from_bytes(id.value.as_deref()?).ok());
        let run_id = request
            .run_id
            .as_ref()
            .and_then(|id| RunId::from_bytes(id.value.as_deref()?).ok())
            .ok_or(ActionDraftError::InvalidRequest)?;
        let observed_revision =
            revision(request.observed_revision.as_ref()).ok_or(ActionDraftError::InvalidRequest)?;
        if session_id != Some(operation_key.session())
            || operation_id != Some(operation_key.operation())
            || observed_revision.epoch() != operation_key.epoch()
        {
            return Err(ActionDraftError::InvalidRequest);
        }
        Ok(Self {
            operation,
            run: run_id,
            observed: observed_revision,
        })
    }

    pub fn key(&self) -> OperationKey {
        self.operation.key()
    }

    pub fn intent(&self) -> &SubmitActionRequest {
        self.operation.intent()
    }

    pub fn resolution(&self) -> &OperationResolution<DecisionReceipt> {
        self.operation.resolution()
    }

    pub fn status(&self) -> ActionDraftStatus {
        match self.operation.resolution() {
            OperationResolution::InProgress => ActionDraftStatus::Pending,
            OperationResolution::Committed { revision, receipt } => match &receipt.outcome {
                Some(wire::decision_receipt::Outcome::Accepted(_)) => ActionDraftStatus::Accepted {
                    revision: *revision,
                },
                Some(wire::decision_receipt::Outcome::Rejected(rejection)) => {
                    match RejectionCode::try_from(rejection.code) {
                        Ok(code) if code != RejectionCode::Unspecified => {
                            ActionDraftStatus::Rejected(code)
                        }
                        _ => ActionDraftStatus::Uncertain,
                    }
                }
                None => ActionDraftStatus::Uncertain,
            },
            OperationResolution::NamespaceRetired { current_epoch } => ActionDraftStatus::Retired {
                epoch: *current_epoch,
            },
            _ => ActionDraftStatus::Uncertain,
        }
    }

    pub async fn resolve<P>(
        &mut self,
        port: &mut P,
    ) -> Result<&OperationResolution<DecisionReceipt>, ResolutionError<ActionLookupError<P::Error>>>
    where
        P: ReceiptLookup<Receipt = DecisionReceipt>,
    {
        let mut validated = ValidatedLookup {
            port,
            key: self.key(),
            run: self.run,
            observed: self.observed,
        };
        self.operation.resolve(&mut validated).await
    }

    pub fn retire_namespace(&mut self, epoch: RecoveryEpoch) -> Result<(), RetirementError> {
        self.operation.retire_namespace(epoch)
    }
}

fn retained_bytes(request: &SubmitActionRequest) -> Option<usize> {
    let mut bytes = std::mem::size_of::<SubmitActionRequest>();
    for capacity in [
        request
            .session_id
            .as_ref()
            .and_then(|id| id.value.as_ref())
            .map_or(0, Vec::capacity),
        request
            .run_id
            .as_ref()
            .and_then(|id| id.value.as_ref())
            .map_or(0, Vec::capacity),
        request
            .operation_id
            .as_ref()
            .and_then(|id| id.value.as_ref())
            .map_or(0, Vec::capacity),
        request.offer_id.capacity(),
    ] {
        bytes = bytes.checked_add(capacity)?;
    }
    if let Some(character) = &request.character {
        bytes = bytes.checked_add(character.name.capacity())?;
        bytes = bytes.checked_add(
            character
                .choices
                .capacity()
                .checked_mul(std::mem::size_of::<wire::JourneyChoice>())?,
        )?;
        for choice in &character.choices {
            bytes = bytes.checked_add(choice.group_id.capacity())?;
            bytes = bytes.checked_add(choice.option_id.capacity())?;
        }
    }
    Some(bytes)
}

fn revision(value: Option<&wire::SessionRevision>) -> Option<SessionRevision> {
    let value = value?;
    let epoch = RecoveryEpoch::new(value.epoch.as_ref()?.value?).ok()?;
    Some(SessionRevision::new(epoch, value.sequence?))
}

struct ValidatedLookup<'a, P> {
    port: &'a mut P,
    key: OperationKey,
    run: RunId,
    observed: SessionRevision,
}

impl<P: ReceiptLookup<Receipt = DecisionReceipt>> ReceiptLookup for ValidatedLookup<'_, P> {
    type Receipt = DecisionReceipt;
    type Error = ActionLookupError<P::Error>;

    async fn lookup(
        &mut self,
        key: OperationKey,
    ) -> Result<LookupOutcome<DecisionReceipt>, Self::Error> {
        if key != self.key {
            return Err(ActionLookupError::InvalidReceipt(
                ActionReceiptError::Identity,
            ));
        }
        let outcome = self
            .port
            .lookup(key)
            .await
            .map_err(ActionLookupError::Lookup)?;
        if let LookupOutcome::Committed { revision, receipt } = &outcome {
            validate_receipt(self.key, self.run, self.observed, *revision, receipt)
                .map_err(ActionLookupError::InvalidReceipt)?;
        }
        Ok(outcome)
    }
}

fn validate_receipt(
    key: OperationKey,
    run: RunId,
    observed: SessionRevision,
    supplied: SessionRevision,
    receipt: &DecisionReceipt,
) -> Result<(), ActionReceiptError> {
    let session = receipt
        .session_id
        .as_ref()
        .and_then(|id| SessionId::from_bytes(id.value.as_deref()?).ok());
    let operation = receipt
        .operation_id
        .as_ref()
        .and_then(|id| OperationId::from_bytes(id.value.as_deref()?).ok());
    let actual_run = receipt
        .run_id
        .as_ref()
        .and_then(|id| RunId::from_bytes(id.value.as_deref()?).ok());
    if session != Some(key.session())
        || operation != Some(key.operation())
        || actual_run != Some(run)
    {
        return Err(ActionReceiptError::Identity);
    }
    let actual = revision(receipt.revision.as_ref()).ok_or(ActionReceiptError::Revision)?;
    if actual != supplied || actual.epoch() != key.epoch() || actual < observed {
        return Err(ActionReceiptError::Revision);
    }
    match &receipt.outcome {
        Some(wire::decision_receipt::Outcome::Accepted(_)) => Ok(()),
        Some(wire::decision_receipt::Outcome::Rejected(rejection)) => {
            match RejectionCode::try_from(rejection.code) {
                Ok(code) if code != RejectionCode::Unspecified => Ok(()),
                _ => Err(ActionReceiptError::Outcome),
            }
        }
        None => Err(ActionReceiptError::Outcome),
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{ActionDraft, ActionDraftStatus, OperationKey};
    use df_ui::{
        ActionView, ControlError, ControlledAction, ControlledTextInput, DraftError, DraftUpdate,
        FeedbackScope, FeedbackView, InputFeedback, OperationFeedback, TextInputView,
    };
    use web_sys::{Document, Element};

    /// Caller-localized literal text, bounded before any mounted state changes.
    #[derive(Clone, Copy)]
    pub struct ActionDraftLabels<'a> {
        pub input: &'a str,
        pub action: &'a str,
        pub pending: &'a str,
        pub uncertain: &'a str,
        pub refused: &'a str,
        pub error: &'a str,
        pub accepted: &'a str,
    }

    #[derive(Debug)]
    pub enum ActionDraftMountError {
        Labels,
        WrongOperation,
        WrongControls,
        Control(ControlError),
    }

    impl From<ControlError> for ActionDraftMountError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }

    impl From<wasm_bindgen::JsValue> for ActionDraftMountError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Control(error.into())
        }
    }

    /// Borrows the existing canonical controls on each update. Native node identity
    /// binds those borrows; no draft buffer, input listener, or receipt state is copied.
    /// The web owner must dispose this mount on member/run/access/binding replacement.
    pub struct MountedActionDraft {
        key: OperationKey,
        root: Element,
        input_node: Element,
        action_node: Element,
        feedback: OperationFeedback,
        scope: FeedbackScope,
        confirmed: Element,
        labels: [String; 7],
        disposed: bool,
    }

    impl MountedActionDraft {
        pub fn mount(
            document: &Document,
            parent: &Element,
            draft: &ActionDraft,
            input: &ControlledTextInput,
            action: &ControlledAction,
            labels: ActionDraftLabels<'_>,
        ) -> Result<Self, ActionDraftMountError> {
            let values = [
                labels.input,
                labels.action,
                labels.pending,
                labels.uncertain,
                labels.refused,
                labels.error,
                labels.accepted,
            ];
            if values
                .iter()
                .any(|text| text.trim().is_empty() || text.len() > 4096)
            {
                return Err(ActionDraftMountError::Labels);
            }
            let root = document.create_element("section")?;
            root.set_class_name("df-ui-panel");
            let (feedback, scope) =
                OperationFeedback::create(document, FeedbackView::Uncertain(labels.uncertain))?;
            let confirmed = document.create_element("p")?;
            confirmed.set_attribute("role", "status")?;
            confirmed.set_attribute("aria-live", "polite")?;
            root.append_child(feedback.root())?;
            root.append_child(&confirmed)?;
            let mounted = Self {
                key: draft.key(),
                root,
                input_node: input.input().clone().into(),
                action_node: action.element().clone().into(),
                feedback,
                scope,
                confirmed,
                labels: values.map(str::to_owned),
                disposed: false,
            };
            mounted.sync(&mounted.scope, draft, input, action, false, false)?;
            parent.append_child(&mounted.root)?;
            Ok(mounted)
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        /// Capture before awaiting. Other widgets and stale owners cannot update controls.
        pub fn scope(&self) -> FeedbackScope {
            self.scope.clone()
        }

        fn require_controls(
            &self,
            input: &ControlledTextInput,
            action: &ControlledAction,
        ) -> Result<(), ActionDraftMountError> {
            if !self.input_node.is_same_node(Some(input.input().as_ref()))
                || !self
                    .action_node
                    .is_same_node(Some(action.element().as_ref()))
            {
                return Err(ActionDraftMountError::WrongControls);
            }
            Ok(())
        }

        pub fn sync(
            &self,
            scope: &FeedbackScope,
            draft: &ActionDraft,
            input: &ControlledTextInput,
            action: &ControlledAction,
            connection_read_only: bool,
            lookup_failed: bool,
        ) -> Result<(), ActionDraftMountError> {
            self.require_controls(input, action)?;
            if self.disposed {
                return Err(ControlError::from(DraftError::Disposed).into());
            }
            if draft.key() != self.key {
                return Err(ActionDraftMountError::WrongOperation);
            }
            let draft_status = draft.status();
            if matches!(draft_status, ActionDraftStatus::Retired { .. }) {
                // Check presentation scope before any destructive private-control change.
                self.feedback
                    .update(scope, FeedbackView::Uncertain(&self.labels[3]))?;
                let input_result = input.dispose();
                let action_result = action.dispose();
                let feedback_result = self.feedback.dispose();
                self.confirmed.set_text_content(None);
                self.root.remove();
                input_result?;
                action_result?;
                feedback_result?;
                return Ok(());
            }
            let view = match draft_status {
                ActionDraftStatus::Pending => FeedbackView::Pending(&self.labels[2]),
                ActionDraftStatus::Rejected(_) => FeedbackView::Refused(&self.labels[4]),
                _ if lookup_failed => FeedbackView::Error {
                    uncertainty: &self.labels[3],
                    message: &self.labels[5],
                },
                _ => FeedbackView::Uncertain(&self.labels[3]),
            };
            // FeedbackScope is checked before busy/editability or confirmed text changes.
            self.feedback.update(scope, view)?;
            let accepted = matches!(draft_status, ActionDraftStatus::Accepted { .. });
            if accepted {
                self.feedback.root().set_attribute("hidden", "")?;
                self.confirmed.set_text_content(Some(&self.labels[6]));
            } else {
                self.feedback.root().remove_attribute("hidden")?;
                self.confirmed.set_text_content(None);
            }
            let rejected = matches!(draft_status, ActionDraftStatus::Rejected(_));
            let is_pending = matches!(draft_status, ActionDraftStatus::Pending);
            input.update(
                TextInputView {
                    label: &self.labels[0],
                    enabled: rejected && !connection_read_only,
                    feedback: if rejected {
                        InputFeedback::Rejected(&self.labels[4])
                    } else if is_pending {
                        InputFeedback::Pending(&self.labels[2])
                    } else {
                        InputFeedback::None
                    },
                },
                DraftUpdate::Preserve,
            )?;
            // A rejected draft is editable, but another send requires a current offer owner.
            action.update(ActionView {
                label: &self.labels[1],
                enabled: false,
                pending: is_pending,
            })?;
            self.root.set_attribute(
                "data-action-draft",
                match draft_status {
                    ActionDraftStatus::Pending => "pending",
                    ActionDraftStatus::Accepted { .. } => "accepted",
                    ActionDraftStatus::Rejected(_) => "rejected",
                    _ => "uncertain",
                },
            )?;
            Ok(())
        }

        /// Terminal private-scope teardown; attempts every cleanup even after a DOM error.
        pub fn dispose(
            &mut self,
            input: &ControlledTextInput,
            action: &ControlledAction,
        ) -> Result<(), ActionDraftMountError> {
            self.require_controls(input, action)?;
            self.disposed = true;
            let input_result = input.dispose();
            let action_result = action.dispose();
            let feedback_result = self.feedback.dispose();
            self.confirmed.set_text_content(None);
            self.root.remove();
            input_result?;
            action_result?;
            feedback_result?;
            Ok(())
        }
    }

    impl Drop for MountedActionDraft {
        fn drop(&mut self) {
            self.confirmed.set_text_content(None);
            self.root.remove();
            // Borrowed canonical controls remain caller-owned; its teardown must dispose them.
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{ActionDraftLabels, ActionDraftMountError, MountedActionDraft};
