//! Synthetic authorized lookup port: no live submit, credentials or lookup RPC.
#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::receipts::{LookupOutcome, OperationKey, ReceiptLookup, UncertainOperation};
    use df_player::{ActionDraft, ActionDraftLabels, ActionDraftStatus, MountedActionDraft};
    use df_protocol::common::{self as wire, DecisionReceipt};
    use df_types::{OperationId, RecoveryEpoch, SessionId, SessionRevision};
    use df_ui::{
        ActionView, ControlledAction, ControlledTextInput, FeedbackView, InputFeedback,
        OperationFeedback, TextInputView,
    };
    use std::{
        cell::RefCell,
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    use wasm_bindgen::prelude::*;
    use web_sys::{Element, Event, HtmlInputElement};

    type FixtureCommand = fn() -> Result<(), JsValue>;
    struct Fixture {
        root: Element,
        input: ControlledTextInput,
        action: ControlledAction,
        toolbar: Vec<ControlledAction>,
        draft: Option<ActionDraft>,
        mounted: Option<MountedActionDraft>,
        submissions: usize,
        lookups: usize,
        old_scope: Option<df_ui::FeedbackScope>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn error(_: impl std::fmt::Debug) -> JsValue {
        JsValue::from_str("draft fixture boundary refused update")
    }
    fn check(value: bool, message: &str) -> Result<(), JsValue> {
        if value {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn epoch() -> RecoveryEpoch {
        RecoveryEpoch::new(1).expect("fixture epoch")
    }
    fn key() -> OperationKey {
        OperationKey::new(
            SessionId::from_bytes(&[1; 16]).expect("fixture session"),
            OperationId::from_bytes(&[3; 16]).expect("fixture operation"),
            epoch(),
        )
    }
    fn revision(sequence: u64) -> wire::SessionRevision {
        wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
            sequence: Some(sequence),
        }
    }
    fn request(name: String) -> wire::SubmitActionRequest {
        wire::SubmitActionRequest {
            session_id: Some(wire::SessionId {
                value: Some(vec![1; 16]),
            }),
            run_id: Some(wire::RunId {
                value: Some(vec![2; 16]),
            }),
            operation_id: Some(wire::OperationId {
                value: Some(vec![3; 16]),
            }),
            observed_revision: Some(revision(7)),
            offer_id: "synthetic-current-character-offer".into(),
            action_kind: 3,
            character: Some(wire::CharacterSelection {
                portrait_scenery: String::new(),
                name,
                choices: Vec::new(),
            }),
            ..Default::default()
        }
    }
    struct Port {
        outcome: Option<LookupOutcome<DecisionReceipt>>,
        calls: usize,
    }
    impl ReceiptLookup for Port {
        type Receipt = DecisionReceipt;
        type Error = ();
        async fn lookup(
            &mut self,
            supplied: OperationKey,
        ) -> Result<LookupOutcome<DecisionReceipt>, ()> {
            if supplied != key() {
                return Err(());
            }
            self.calls += 1;
            self.outcome.take().ok_or(())
        }
    }
    fn ready<F: Future>(future: F) -> Result<F::Output, JsValue> {
        let mut future = pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => Ok(output),
            Poll::Pending => Err(JsValue::from_str("fixture unexpectedly pending")),
        }
    }
    fn labels() -> ActionDraftLabels<'static> {
        ActionDraftLabels {
            input: "Character name · private local draft",
            action: "Submit current offered intention",
            pending: "Awaiting a server decision · draft retained",
            uncertain: "Outcome unknown · reconnect does not confirm acceptance",
            refused: "Server rejected the original offer · edit your retained draft",
            error: "Receipt could not be validated · outcome remains unknown",
            accepted: "Server receipt confirms acceptance",
        }
    }
    fn with_fixture<T>(f: impl FnOnce(&mut Fixture) -> Result<T, JsValue>) -> Result<T, JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            f(slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture not mounted"))?)
        })
    }
    fn command(operation: fn() -> Result<(), JsValue>) {
        if operation().is_err() {
            FIXTURE.with(|slot| {
                if let Some(fixture) = slot.borrow().as_ref() {
                    let _ = fixture.root.set_attribute("data-fixture-error", "true");
                }
            });
        }
    }
    #[wasm_bindgen]
    pub fn fixture_start() -> Result<(), JsValue> {
        fixture_dispose()?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document missing"))?;
        let root = document.create_element("main")?;
        root.set_class_name("df-ui-root");
        root.set_attribute("style", "max-width:48rem;margin:2rem auto;padding:1.5rem")?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some("Player action draft · receipt lifecycle"));
        root.append_child(&heading)?;
        let note = document.create_element("p")?;
        note.set_text_content(Some("Synthetic read-only lookup fixture. Local input emits an intention; only a validated server receipt supplies a decision."));
        root.append_child(&note)?;
        let input = ControlledTextInput::create(
            &document,
            "action-draft-lifecycle",
            TextInputView {
                label: labels().input,
                enabled: true,
                feedback: InputFeedback::None,
            },
            "PRIVATE-DRAFT <img src=x onerror=alert(1)>",
        )
        .map_err(error)?;
        let action = ControlledAction::create(
            &document,
            ActionView {
                label: labels().action,
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        action
            .on_activate(|| command(fixture_submit))
            .map_err(error)?;
        root.append_child(input.root())?;
        root.append_child(action.element())?;
        let mut toolbar = Vec::new();
        let commands: [(&str, FixtureCommand); 9] = [
            ("Lookup in progress", fixture_pending),
            ("Lookup not recorded", fixture_unknown),
            ("Reconnect · same operation", fixture_reconnect),
            ("Cancel borrowed lookup wait", fixture_cancel_wait),
            ("Malformed receipt", fixture_malformed),
            ("Server rejection", fixture_rejected),
            ("Server acceptance", fixture_accepted),
            ("Retire namespace", fixture_retire),
            ("Dispose private scope", fixture_dispose),
        ];
        for (label, operation) in commands {
            let button = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            button
                .on_activate(move || command(operation))
                .map_err(error)?;
            root.append_child(button.element())?;
            toolbar.push(button);
        }
        document
            .body()
            .ok_or_else(|| JsValue::from_str("body missing"))?
            .append_child(&root)?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                root,
                input,
                action,
                toolbar,
                draft: None,
                mounted: None,
                submissions: 0,
                lookups: 0,
                old_scope: None,
            })
        });
        Ok(())
    }
    #[wasm_bindgen]
    pub fn fixture_submit() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            check(
                fixture.draft.is_none(),
                "fixture already has an original operation",
            )?;
            let draft = ActionDraft::from_uncertain(UncertainOperation::new(
                key(),
                request(fixture.input.draft()),
            ))
            .map_err(error)?;
            let document = fixture
                .root
                .owner_document()
                .ok_or_else(|| JsValue::from_str("document missing"))?;
            let mounted = MountedActionDraft::mount(
                &document,
                &fixture.root,
                &draft,
                &fixture.input,
                &fixture.action,
                labels(),
            )
            .map_err(error)?;
            fixture.old_scope = Some(mounted.scope());
            fixture.draft = Some(draft);
            fixture.mounted = Some(mounted);
            fixture.submissions += 1;
            Ok(())
        })?;
        fixture_pending()
    }
    fn observe(
        outcome: Option<LookupOutcome<DecisionReceipt>>,
        disconnected: bool,
    ) -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let draft = fixture
                .draft
                .as_mut()
                .ok_or_else(|| JsValue::from_str("submit first"))?;
            let mut port = Port { outcome, calls: 0 };
            let failed = ready(draft.resolve(&mut port))?.is_err();
            fixture.lookups += port.calls;
            let mounted = fixture
                .mounted
                .as_ref()
                .ok_or_else(|| JsValue::from_str("mount missing"))?;
            mounted
                .sync(
                    &mounted.scope(),
                    draft,
                    &fixture.input,
                    &fixture.action,
                    disconnected,
                    failed,
                )
                .map_err(error)
        })
    }
    #[wasm_bindgen]
    pub fn fixture_pending() -> Result<(), JsValue> {
        observe(Some(LookupOutcome::InProgress), false)
    }
    #[wasm_bindgen]
    pub fn fixture_unknown() -> Result<(), JsValue> {
        observe(Some(LookupOutcome::NotRecorded), false)
    }
    #[wasm_bindgen]
    pub fn fixture_reconnect() -> Result<(), JsValue> {
        observe(Some(LookupOutcome::NotRecorded), true)
    }
    fn decision(accepted: bool, malformed: bool) -> LookupOutcome<DecisionReceipt> {
        let request = request(String::new());
        LookupOutcome::Committed {
            revision: SessionRevision::new(epoch(), 8),
            receipt: DecisionReceipt {
                session_id: request.session_id,
                run_id: request.run_id,
                operation_id: if malformed {
                    None
                } else {
                    request.operation_id
                },
                revision: Some(revision(8)),
                replayed: true,
                outcome: Some(if accepted {
                    wire::decision_receipt::Outcome::Accepted(Box::default())
                } else {
                    wire::decision_receipt::Outcome::Rejected(wire::Rejection {
                        code: wire::RejectionCode::StaleOffer as i32,
                    })
                }),
            },
        }
    }
    struct PendingPort {
        calls: usize,
    }
    impl ReceiptLookup for PendingPort {
        type Receipt = DecisionReceipt;
        type Error = ();
        async fn lookup(
            &mut self,
            supplied: OperationKey,
        ) -> Result<LookupOutcome<DecisionReceipt>, ()> {
            if supplied != key() {
                return Err(());
            }
            self.calls += 1;
            std::future::pending().await
        }
    }
    #[wasm_bindgen]
    pub fn fixture_cancel_wait() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let draft = fixture
                .draft
                .as_mut()
                .ok_or_else(|| JsValue::from_str("submit first"))?;
            let mut port = PendingPort { calls: 0 };
            {
                let mut wait = pin!(draft.resolve(&mut port));
                check(
                    wait.as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending(),
                    "borrowed fixture lookup did not remain pending",
                )?;
            }
            fixture.lookups += port.calls;
            let mounted = fixture
                .mounted
                .as_ref()
                .ok_or_else(|| JsValue::from_str("mount missing"))?;
            mounted
                .sync(
                    &mounted.scope(),
                    draft,
                    &fixture.input,
                    &fixture.action,
                    true,
                    false,
                )
                .map_err(error)
        })
    }
    #[wasm_bindgen]
    pub fn fixture_malformed() -> Result<(), JsValue> {
        observe(Some(decision(true, true)), false)
    }
    #[wasm_bindgen]
    pub fn fixture_rejected() -> Result<(), JsValue> {
        observe(Some(decision(false, false)), false)
    }
    #[wasm_bindgen]
    pub fn fixture_accepted() -> Result<(), JsValue> {
        observe(Some(decision(true, false)), false)
    }
    #[wasm_bindgen]
    pub fn fixture_retire() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let draft = fixture
                .draft
                .as_mut()
                .ok_or_else(|| JsValue::from_str("submit first"))?;
            draft
                .retire_namespace(RecoveryEpoch::new(2).map_err(error)?)
                .map_err(error)?;
            let mounted = fixture
                .mounted
                .as_ref()
                .ok_or_else(|| JsValue::from_str("mount missing"))?;
            mounted
                .sync(
                    &mounted.scope(),
                    draft,
                    &fixture.input,
                    &fixture.action,
                    true,
                    false,
                )
                .map_err(error)
        })
    }
    #[wasm_bindgen]
    pub fn fixture_dispose() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            if let Some(mut fixture) = slot.borrow_mut().take() {
                let result = if let Some(mounted) = fixture.mounted.as_mut() {
                    mounted
                        .dispose(&fixture.input, &fixture.action)
                        .map_err(error)
                } else {
                    let input = fixture.input.dispose();
                    let action = fixture.action.dispose();
                    input.and(action).map_err(error)
                };
                for button in &fixture.toolbar {
                    button.dispose().map_err(error)?;
                }
                fixture.root.set_text_content(None);
                fixture.root.remove();
                result?;
            }
            Ok(())
        })
    }
    fn field() -> Result<HtmlInputElement, JsValue> {
        with_fixture(|fixture| Ok(fixture.input.input().clone()))
    }
    fn write(input: &HtmlInputElement, text: &str) -> Result<(), JsValue> {
        input.set_value(text);
        input.dispatch_event(&Event::new("input")?)?;
        Ok(())
    }
    #[wasm_bindgen]
    pub fn fixture_verify() -> Result<u32, JsValue> {
        fixture_start()?;
        let input = field()?;
        let action = with_fixture(|fixture| Ok(fixture.action.element().clone()))?;
        write(&input, "PRIVATE-DRAFT <b>literal</b>")?;
        input.focus()?;
        input.set_selection_range(2, 9)?;
        let before = input.value();
        action.click();
        with_fixture(|fixture| {
            check(
                fixture.submissions == 1,
                "native click did not emit one intention",
            )?;
            check(
                input.read_only() && action.disabled(),
                "pending controls not gated",
            )?;
            check(
                input.is_same_node(Some(fixture.input.input().as_ref())),
                "input remounted",
            )?;
            check(
                input.selection_start()? == Some(2) && input.selection_end()? == Some(9),
                "pending changed selection",
            )?;
            check(
                fixture.root.query_selector("b")?.is_none(),
                "draft parsed as markup",
            )
        })?;
        action.click();
        fixture_unknown()?;
        fixture_reconnect()?;
        fixture_cancel_wait()?;
        fixture_malformed()?;
        with_fixture(|fixture| {
            check(fixture.submissions == 1, "disabled action submitted again")?;
            check(
                input.value() == before && input.read_only(),
                "unknown/reconnect lost draft",
            )?;
            check(
                matches!(
                    fixture.draft.as_ref().map(ActionDraft::status),
                    Some(ActionDraftStatus::Uncertain)
                ),
                "malformed receipt settled",
            )?;
            check(
                fixture
                    .root
                    .text_content()
                    .is_none_or(|text| !text.contains(labels().accepted)),
                "malformed receipt showed success",
            )?;
            let document = fixture
                .root
                .owner_document()
                .ok_or_else(|| JsValue::from_str("document missing"))?;
            let (_other, foreign_scope) =
                OperationFeedback::create(&document, FeedbackView::Pending("Other operation"))
                    .map_err(error)?;
            let mounted = fixture
                .mounted
                .as_ref()
                .ok_or_else(|| JsValue::from_str("mount missing"))?;
            check(
                mounted
                    .sync(
                        &foreign_scope,
                        fixture.draft.as_ref().expect("submitted"),
                        &fixture.input,
                        &fixture.action,
                        false,
                        false,
                    )
                    .is_err(),
                "foreign scope released busy state",
            )
        })?;
        fixture_rejected()?;
        check(
            !input.read_only() && action.disabled(),
            "rejection editability/retry gate wrong",
        )?;
        write(&input, "EDITED-PRIVATE-DRAFT")?;
        check(
            input.value() == "EDITED-PRIVATE-DRAFT",
            "rejection discarded edit",
        )?;
        fixture_dispose()?;
        check(
            input.value().is_empty() && action.disabled(),
            "disposal retained private input",
        )?;
        action.click();
        fixture_start()?;
        let accepted_input = field()?;
        let accepted_action = with_fixture(|fixture| Ok(fixture.action.element().clone()))?;
        accepted_action.click();
        fixture_accepted()?;
        with_fixture(|fixture| {
            check(
                matches!(
                    fixture.draft.as_ref().map(ActionDraft::status),
                    Some(ActionDraftStatus::Accepted { .. })
                ),
                "validated accepted receipt missing",
            )?;
            check(
                fixture
                    .root
                    .text_content()
                    .is_some_and(|text| text.contains(labels().accepted)),
                "confirmed literal status missing",
            )?;
            let calls = fixture.lookups;
            check(calls == 2, "unexpected lookup count")
        })?;
        fixture_reconnect()?;
        with_fixture(|fixture| check(fixture.lookups == 2, "terminal decision repeated lookup"))?;
        fixture_dispose()?;
        check(
            accepted_input.value().is_empty(),
            "accepted private draft retained after teardown",
        )?;
        fixture_start()?;
        with_fixture(|fixture| Ok(fixture.action.element().clone()))?.click();
        fixture_retire()?;
        with_fixture(|fixture| {
            let mounted = fixture
                .mounted
                .as_ref()
                .ok_or_else(|| JsValue::from_str("mount missing"))?;
            let old = fixture
                .old_scope
                .as_ref()
                .ok_or_else(|| JsValue::from_str("old scope missing"))?;
            check(
                mounted
                    .sync(
                        old,
                        fixture.draft.as_ref().expect("submitted"),
                        &fixture.input,
                        &fixture.action,
                        false,
                        false,
                    )
                    .is_err(),
                "retired scope revived",
            )
        })?;
        fixture_dispose()?;
        // Leave a usable initial view for independent native browser interaction.
        fixture_start()?;
        Ok(4095)
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        fixture_start()
    }
}
