//! Bounded supplied-observation contract, not a production receipt/wire service.
//! Editing or transport recovery never changes the separately admitted view.

#[cfg(any(target_arch = "wasm32", test))]
mod contract {
    use std::future::{Future, ready};

    use df_client::{
        connection::RpcConnection,
        connection_views::{ConnectionViewAcceptance, ConnectionViews},
        receipts::{LookupOutcome, OperationKey, ReceiptLookup, UncertainOperation},
        revisions::ViewAcceptance,
    };
    use df_types::{ClientBindingId, OperationId, RecoveryEpoch, SessionId, SessionRevision};

    pub(super) const COMMITTED: &str = "Supplied committed view: waiting at the gate.";
    pub(super) const NEXT_VIEW: &str = "Supplied committed view: permitted update at revision 2.";
    pub(super) const UNKNOWN: &str =
        "Outcome unknown. Recover the original operation; do not replay.";
    pub(super) const PENDING: &str = "Original operation is pending.";
    #[cfg(target_arch = "wasm32")]
    pub(super) const REFUSAL: &str = "<svg onload=alert(1)> Supplied rejection: edit your draft.";

    // These are private fixture observations, not a proposed wire receipt schema.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum FixtureReceipt {
        AcceptedWorkPending,
        Refused,
    }

    pub(super) struct SuppliedLookup {
        pub(super) outcome: Result<LookupOutcome<FixtureReceipt>, ()>,
        pub(super) requested: Option<OperationKey>,
    }

    impl ReceiptLookup for SuppliedLookup {
        type Receipt = FixtureReceipt;
        type Error = ();

        fn lookup(
            &mut self,
            key: OperationKey,
        ) -> impl Future<Output = Result<LookupOutcome<Self::Receipt>, Self::Error>> {
            self.requested = Some(key);
            ready(self.outcome.clone())
        }
    }

    pub(super) struct Boundary {
        pub(super) binding: ClientBindingId,
        pub(super) connection: RpcConnection<()>,
        pub(super) views: ConnectionViews<&'static str>,
        pub(super) operation: UncertainOperation<String, FixtureReceipt>,
    }

    pub(super) fn revision(sequence: u64) -> Result<SessionRevision, String> {
        let epoch = RecoveryEpoch::new(1).map_err(|error| format!("{error:?}"))?;
        Ok(SessionRevision::new(epoch, sequence))
    }

    pub(super) fn original(
        draft: String,
    ) -> Result<UncertainOperation<String, FixtureReceipt>, String> {
        let session = SessionId::from_bytes(&[1; 16]).map_err(|error| format!("{error:?}"))?;
        let operation = OperationId::from_bytes(&[2; 16]).map_err(|error| format!("{error:?}"))?;
        let key = OperationKey::new(session, operation, revision(1)?.epoch());
        Ok(UncertainOperation::new(key, draft))
    }

    impl Boundary {
        pub(super) fn create(draft: String) -> Result<Self, String> {
            let binding =
                ClientBindingId::from_bytes(&[3; 16]).map_err(|error| format!("{error:?}"))?;
            let connection = RpcConnection::new(());
            let mut views = ConnectionViews::new(binding, connection.generation());
            let accepted = views.accept(&connection.generation(), binding, revision(1)?, COMMITTED);
            if accepted != ConnectionViewAcceptance::View(ViewAcceptance::Applied) {
                return Err("initial supplied view was not admitted".into());
            }
            Ok(Self {
                binding,
                connection,
                views,
                operation: original(draft)?,
            })
        }

        pub(super) fn reconnect(&mut self) -> Result<(), String> {
            let old = self.connection.generation();
            self.connection.close();
            self.connection = RpcConnection::new(());
            if !self.views.reconnect(self.connection.generation()) {
                return Err("new active generation was refused".into());
            }
            if self
                .views
                .accept(&old, self.binding, revision(999)?, "obsolete view")
                != ConnectionViewAcceptance::ObsoleteGeneration
            {
                return Err("obsolete connection replaced the current view".into());
            }
            Ok(())
        }

        pub(super) fn update_view(&mut self) -> Result<(), String> {
            let current = self.connection.generation();
            let result = self
                .views
                .accept(&current, self.binding, revision(2)?, NEXT_VIEW);
            if !matches!(
                result,
                ConnectionViewAcceptance::View(
                    ViewAcceptance::Applied | ViewAcceptance::Duplicate { .. }
                )
            ) {
                return Err("current supplied view was not admitted".into());
            }
            if !matches!(
                self.views
                    .accept(&current, self.binding, revision(1)?, "stale view"),
                ConnectionViewAcceptance::View(ViewAcceptance::Stale { .. })
            ) {
                return Err("stale revision replaced the current view".into());
            }
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::{
        cell::RefCell,
        future::Future,
        task::{Context, Poll, Waker},
    };

    use df_client::receipts::{LookupOutcome, OperationResolution};
    use df_ui::{
        ActionView, ControlError, ControlledAction, ControlledTextInput, DraftError, DraftUpdate,
        FeedbackScope, FeedbackView, InputFeedback, LayoutRole, LayoutRoot, OperationFeedback,
        SessionConnection, TextInputView, panel,
    };
    use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event};

    use super::contract::{
        Boundary, COMMITTED, FixtureReceipt, NEXT_VIEW, PENDING, REFUSAL, SuppliedLookup, UNKNOWN,
        original, revision,
    };

    const LABEL: &str = "Local draft <b>literal</b>";
    const INITIAL_DRAFT: &str = "I would like to enter the gate.";

    struct Surface {
        layout: LayoutRoot,
        committed: Element,
        connection: Element,
        feedback: OperationFeedback,
        scope: FeedbackScope,
    }

    struct Fixture {
        document: Document,
        root: Element,
        output: Element,
        field: ControlledTextInput,
        submit: ControlledAction,
        surfaces: Vec<Surface>,
        controls: Vec<ControlledAction>,
        boundary: Boundary,
        submitted: bool,
    }

    thread_local! {
        static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) };
    }

    fn control(error: ControlError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }

    fn descendant_count(root: &Element) -> Result<u32, JsValue> {
        let mut pending = vec![root.clone()];
        let mut count = 0_u32;
        while let Some(element) = pending.pop() {
            let mut child = element.first_element_child();
            while let Some(current) = child {
                count += 1;
                require(count <= 256, "fixture exceeded bounded node count")?;
                child = current.next_element_sibling();
                pending.push(current);
            }
        }
        Ok(count)
    }

    fn input_view(feedback: InputFeedback<'_>, enabled: bool) -> TextInputView<'_> {
        TextInputView {
            label: LABEL,
            enabled,
            feedback,
        }
    }

    fn action_view(pending: bool, enabled: bool) -> ActionView<'static> {
        ActionView {
            label: "Supply ambiguous submission",
            pending,
            enabled,
        }
    }

    fn run_ready<Output>(future: impl Future<Output = Output>) -> Result<Output, JsValue> {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => Ok(output),
            Poll::Pending => Err(JsValue::from_str("supplied read unexpectedly pending")),
        }
    }

    impl Fixture {
        fn feedback(&self, view: FeedbackView<'_>) -> Result<(), JsValue> {
            for surface in &self.surfaces {
                surface
                    .feedback
                    .update(&surface.scope, view)
                    .map_err(control)?;
            }
            Ok(())
        }

        fn connection(&self, state: SessionConnection) {
            let text = match state {
                SessionConnection::Connecting => "Connecting. Operation outcome is separate.",
                SessionConnection::Connected => "Connected. Operation outcome is separate.",
                SessionConnection::Reconnecting => "Reconnecting. Original outcome stays unknown.",
                SessionConnection::Offline => "Offline. Original outcome stays unknown.",
            };
            for surface in &self.surfaces {
                surface.connection.set_text_content(Some(text));
            }
        }

        fn render_resolution(&self) -> Result<(), JsValue> {
            let resolution = self.boundary.operation.resolution();
            if let Some(view) = FeedbackView::for_resolution(resolution, PENDING, UNKNOWN) {
                self.feedback(view)?;
                let pending = matches!(view, FeedbackView::Pending(_));
                self.field
                    .update(
                        input_view(
                            if pending {
                                InputFeedback::Pending(PENDING)
                            } else {
                                InputFeedback::None
                            },
                            false,
                        ),
                        DraftUpdate::Preserve,
                    )
                    .map_err(control)?;
                self.submit
                    .update(action_view(pending, false))
                    .map_err(control)?;
                return Ok(());
            }
            match resolution {
                OperationResolution::Committed {
                    receipt: FixtureReceipt::Refused,
                    ..
                } => {
                    self.feedback(FeedbackView::Refused(REFUSAL))?;
                    self.field
                        .update(
                            input_view(InputFeedback::Rejected(REFUSAL), true),
                            DraftUpdate::Preserve,
                        )
                        .map_err(control)?;
                    self.submit
                        .update(action_view(false, false))
                        .map_err(control)?;
                }
                OperationResolution::Committed {
                    receipt: FixtureReceipt::AcceptedWorkPending,
                    ..
                } => {
                    self.feedback(FeedbackView::Pending(
                        "Decision accepted; supplied work is still pending.",
                    ))?;
                    self.field
                        .update(
                            input_view(InputFeedback::None, false),
                            DraftUpdate::Preserve,
                        )
                        .map_err(control)?;
                    self.submit
                        .update(action_view(false, false))
                        .map_err(control)?;
                }
                _ => return Err(JsValue::from_str("unmapped supplied operation observation")),
            }
            Ok(())
        }

        fn lookup(
            &mut self,
            outcome: Result<LookupOutcome<FixtureReceipt>, ()>,
        ) -> Result<(), JsValue> {
            require(
                self.submitted,
                "supply the original ambiguous submission first",
            )?;
            require(
                !matches!(
                    self.boundary.operation.resolution(),
                    OperationResolution::Committed { .. }
                        | OperationResolution::NamespaceRetired { .. }
                ),
                "original operation is terminal; start a fresh fixture for another scenario",
            )?;
            let key = self.boundary.operation.key();
            let intent = self.boundary.operation.intent().clone();
            let mut lookup = SuppliedLookup {
                outcome,
                requested: None,
            };
            let result = run_ready(self.boundary.operation.resolve(&mut lookup))?;
            if result.is_err() {
                self.render_resolution()?;
                self.feedback(FeedbackView::Error {
                    uncertainty: UNKNOWN,
                    message: "Supplied lookup unavailable.",
                })?;
            } else {
                self.render_resolution()?;
            }
            require(
                lookup.requested == Some(key),
                "lookup did not retain original key",
            )?;
            require(
                self.boundary.operation.intent() == &intent,
                "lookup changed original intent",
            )
        }

        fn begin(&mut self) -> Result<(), JsValue> {
            require(
                !self.submitted,
                "original operation cannot be replaced or replayed",
            )?;
            self.boundary.operation =
                original(self.field.draft()).map_err(|error| JsValue::from_str(&error))?;
            self.submitted = true;
            for surface in &mut self.surfaces {
                surface.scope = surface
                    .feedback
                    .replace(FeedbackView::Pending(PENDING))
                    .map_err(control)?;
            }
            self.field
                .update(
                    input_view(InputFeedback::Pending(PENDING), true),
                    DraftUpdate::Preserve,
                )
                .map_err(control)?;
            self.submit.update(action_view(true, true)).map_err(control)
        }

        fn render_view(&self) -> Result<(), JsValue> {
            let (_, text) = self
                .boundary
                .views
                .current()
                .ok_or_else(|| JsValue::from_str("no admitted view"))?;
            for surface in &self.surfaces {
                surface.committed.set_text_content(Some(text));
            }
            Ok(())
        }

        fn reconnect(&mut self) -> Result<(), JsValue> {
            self.connection(SessionConnection::Reconnecting);
            self.boundary
                .reconnect()
                .map_err(|error| JsValue::from_str(&error))?;
            self.connection(SessionConnection::Connected);
            self.render_view()
        }

        fn verify(&mut self) -> Result<(), JsValue> {
            require(!self.submitted, "DOM checks require a fresh fixture mount")?;
            self.field
                .update(input_view(InputFeedback::None, true), DraftUpdate::Preserve)
                .map_err(control)?;
            self.field
                .input()
                .set_value("Edited local draft <img src=x>");
            self.field.input().dispatch_event(&Event::new("input")?)?;
            self.field.input().focus()?;
            self.field.input().set_selection_range(1, 4)?;
            let draft = self.field.draft();
            let input = self.field.input().clone();
            let descendants = descendant_count(&self.root)?;
            self.begin()?;
            require(self.field.input().read_only(), "pending draft was editable")?;
            require(
                self.submit.element().disabled(),
                "pending submission remained enabled",
            )?;
            self.lookup(Err(()))?;
            require(
                self.field.input().read_only() && self.submit.element().disabled(),
                "failed lookup released an unresolved control",
            )?;
            require(
                self.field.input().get_attribute("aria-busy").as_deref() == Some("false")
                    && self.submit.element().get_attribute("aria-busy").as_deref() == Some("false"),
                "direct pending-to-error retained busy controls",
            )?;
            require(
                self.field
                    .input()
                    .get_attribute("aria-describedby")
                    .is_none(),
                "failed lookup retained the pending input description",
            )?;
            require(
                self.field.draft() == draft
                    && self.field.input().value() == draft
                    && self.field.input().is_same_node(Some(&input))
                    && self.field.input().selection_start()? == Some(1)
                    && self.field.input().selection_end()? == Some(4)
                    && self
                        .document
                        .active_element()
                        .as_ref()
                        .is_some_and(|active| active.is_same_node(Some(&input))),
                "direct lookup failure changed the draft, input, focus or selection",
            )?;
            require(
                matches!(
                    self.boundary.operation.resolution(),
                    OperationResolution::Uncertain
                ) && self.boundary.operation.intent() == &draft
                    && self
                        .boundary
                        .views
                        .current()
                        .is_some_and(|(_, text)| *text == COMMITTED),
                "failed lookup changed original intent or committed state",
            )?;
            for surface in &self.surfaces {
                require(
                    surface
                        .feedback
                        .root()
                        .get_attribute("data-df-feedback")
                        .as_deref()
                        == Some("unresolved-error")
                        && surface
                            .feedback
                            .root()
                            .get_attribute("aria-busy")
                            .as_deref()
                            == Some("false"),
                    "failed lookup did not render a nonpending unresolved error",
                )?;
                require(
                    surface
                        .feedback
                        .root()
                        .query_selector("[role=status]")?
                        .is_some_and(|status| status.text_content().as_deref() == Some(UNKNOWN))
                        && surface
                            .feedback
                            .root()
                            .query_selector("[role=alert]")?
                            .is_some_and(|alert| {
                                alert.text_content().as_deref()
                                    == Some("Supplied lookup unavailable.")
                            }),
                    "failed lookup omitted uncertainty or its safe error",
                )?;
            }
            self.lookup(Ok(LookupOutcome::InProgress))?;
            self.lookup(Ok(LookupOutcome::NotRecorded))?;
            require(
                self.surfaces.iter().all(|surface| {
                    surface
                        .feedback
                        .root()
                        .get_attribute("data-df-feedback")
                        .as_deref()
                        == Some("uncertain")
                }),
                "absence became a terminal outcome",
            )?;
            self.lookup(Err(()))?;
            self.lookup(Ok(LookupOutcome::ExpiredOrIndeterminate))?;
            self.reconnect()?;
            require(
                matches!(
                    self.boundary.operation.resolution(),
                    OperationResolution::ExpiredOrIndeterminate
                ),
                "reconnect resolved original operation",
            )?;
            self.lookup(Ok(LookupOutcome::Committed {
                revision: revision(1).map_err(|error| JsValue::from_str(&error))?,
                receipt: FixtureReceipt::Refused,
            }))?;
            require(
                !self.field.input().read_only(),
                "refused draft was not editable",
            )?;
            for _ in 0..64 {
                self.render_resolution()?;
                self.render_view()?;
            }
            require(
                self.field.draft() == draft && self.field.input().is_same_node(Some(&input)),
                "draft or input node changed",
            )?;
            require(
                self.field.input().selection_start()? == Some(1)
                    && self.field.input().selection_end()? == Some(4),
                "selection changed",
            )?;
            require(
                self.document
                    .active_element()
                    .as_ref()
                    .is_some_and(|active| active.is_same_node(Some(&input))),
                "input focus was lost",
            )?;
            require(
                self.boundary
                    .views
                    .current()
                    .is_some_and(|(_, text)| *text == COMMITTED),
                "draft or refusal changed committed view",
            )?;
            require(
                descendant_count(&self.root)? == descendants,
                "updates accumulated nodes",
            )?;
            require(
                self.root.query_selector("svg, img, b")?.is_none(),
                "literal text became markup",
            )?;
            for surface in &mut self.surfaces {
                let old = surface.scope.clone();
                surface.scope = surface
                    .feedback
                    .replace(FeedbackView::Uncertain(UNKNOWN))
                    .map_err(control)?;
                require(
                    matches!(
                        surface
                            .feedback
                            .update(&old, FeedbackView::Refused("late refusal")),
                        Err(ControlError::Draft(DraftError::Unavailable))
                    ),
                    "obsolete feedback scope was accepted",
                )?;
            }
            self.boundary
                .update_view()
                .map_err(|error| JsValue::from_str(&error))?;
            self.render_view()?;
            require(
                self.boundary
                    .views
                    .current()
                    .is_some_and(|(_, text)| *text == NEXT_VIEW),
                "new view not admitted",
            )?;
            self.output.set_text_content(Some(
                "DOM contract checks passed. Independent browser review remains pending.",
            ));
            Ok(())
        }

        fn dispose(&self) -> Result<(), JsValue> {
            self.field.dispose().map_err(control)?;
            self.submit.dispose().map_err(control)?;
            for button in &self.controls {
                button.dispose().map_err(control)?;
            }
            for surface in &self.surfaces {
                surface.feedback.dispose().map_err(control)?;
                require(
                    matches!(
                        surface
                            .feedback
                            .update(&surface.scope, FeedbackView::Pending("late update")),
                        Err(ControlError::Draft(DraftError::Disposed))
                    ),
                    "disposed feedback accepted update",
                )?;
                surface.committed.set_text_content(None);
                surface.connection.set_text_content(None);
                surface
                    .layout
                    .unmount()
                    .map_err(|error| JsValue::from_str(&error.to_string()))?;
            }
            self.boundary.connection.close();
            require(self.field.draft().is_empty(), "disposal retained draft")?;
            self.root.remove();
            Ok(())
        }
    }

    #[wasm_bindgen]
    pub fn show_observation(observation: &str) -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture is not mounted"))?;
            match observation {
                "pending" => fixture.begin(),
                "in-progress" => fixture.lookup(Ok(LookupOutcome::InProgress)),
                "not-recorded" => fixture.lookup(Ok(LookupOutcome::NotRecorded)),
                "expired" => fixture.lookup(Ok(LookupOutcome::ExpiredOrIndeterminate)),
                "lookup-error" => fixture.lookup(Err(())),
                "refused" => fixture.lookup(Ok(LookupOutcome::Committed {
                    revision: revision(1).map_err(|error| JsValue::from_str(&error))?,
                    receipt: FixtureReceipt::Refused,
                })),
                "accepted-pending" => fixture.lookup(Ok(LookupOutcome::Committed {
                    revision: revision(1).map_err(|error| JsValue::from_str(&error))?,
                    receipt: FixtureReceipt::AcceptedWorkPending,
                })),
                "reconnect" => fixture.reconnect(),
                "new-view" => {
                    fixture
                        .boundary
                        .update_view()
                        .map_err(|error| JsValue::from_str(&error))?;
                    fixture.render_view()
                }
                "verify" => fixture.verify(),
                _ => Err(JsValue::from_str("unknown bounded fixture observation")),
            }
        })
    }

    #[wasm_bindgen]
    pub fn verify_pending_rejection_contract() -> Result<(), JsValue> {
        show_observation("verify")
    }

    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            if let Some(fixture) = slot.borrow().as_ref() {
                fixture.dispose()?;
            }
            slot.borrow_mut().take();
            Ok(())
        })
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        shutdown()?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("browser document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("document body unavailable"))?;
        let root = document.create_element("main")?;
        root.set_id("pending-rejection-contract");
        let output = document.create_element("p")?;
        output.set_attribute("role", "status")?;
        output.set_text_content(Some("Supplied observations only. Production wire producer and independent browser review remain pending."));
        root.append_child(&output)?;
        let boundary =
            Boundary::create(INITIAL_DRAFT.into()).map_err(|error| JsValue::from_str(&error))?;
        let field = ControlledTextInput::create(
            &document,
            "pending-local-draft",
            input_view(InputFeedback::None, true),
            INITIAL_DRAFT,
        )
        .map_err(control)?;
        let submit =
            ControlledAction::create(&document, action_view(false, true)).map_err(control)?;
        submit
            .on_activate(|| {
                if let Err(error) = show_observation("pending") {
                    FIXTURE.with(|slot| {
                        if let Some(fixture) = slot.borrow().as_ref() {
                            fixture
                                .output
                                .set_text_content(Some(error.as_string().as_deref().unwrap_or("Browser DOM operation failed; no successful presentation is claimed.")));
                        }
                    });
                }
            })
            .map_err(control)?;
        let mut surfaces = Vec::new();
        for (role, title) in [
            (
                LayoutRole::Player,
                "Player: local draft and supplied committed view",
            ),
            (
                LayoutRole::SharedDisplay,
                "Shared display: supplied committed view and operation status",
            ),
        ] {
            let layout = LayoutRoot::create(&document, role)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            let section =
                panel(&document, title).map_err(|error| JsValue::from_str(&error.to_string()))?;
            let committed = document.create_element("p")?;
            committed.set_attribute("data-committed-view", "")?;
            committed.set_text_content(Some(COMMITTED));
            let connection = document.create_element("p")?;
            connection.set_attribute("role", "status")?;
            let (feedback, scope) =
                OperationFeedback::create(&document, FeedbackView::Uncertain(UNKNOWN))
                    .map_err(control)?;
            section.append_child(&committed)?;
            section.append_child(&connection)?;
            section.append_child(feedback.root())?;
            if role == LayoutRole::Player {
                section.append_child(field.root())?;
                section.append_child(submit.element())?;
            }
            layout.content().append_child(&section)?;
            root.append_child(layout.root())?;
            surfaces.push(Surface {
                layout,
                committed,
                connection,
                feedback,
                scope,
            });
        }
        let mut controls = Vec::new();
        for (observation, label) in [
            ("in-progress", "Supply in-progress lookup"),
            ("refused", "Supply known rejection receipt"),
            ("not-recorded", "Supply absent lookup"),
            ("expired", "Supply expired lookup"),
            ("lookup-error", "Supply failed lookup"),
            (
                "accepted-pending",
                "Supply accepted decision with pending work",
            ),
            (
                "reconnect",
                "Recover connection without resolving operation",
            ),
            ("new-view", "Supply a newer permitted committed view"),
            ("verify", "Run bounded DOM contract checks"),
        ] {
            let button = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(control)?;
            let report = output.clone();
            button
                .on_activate(move || match show_observation(observation) {
                    Ok(()) => report.set_text_content(Some(
                        "Supplied observation rendered. Independent review remains pending.",
                    )),
                    Err(error) => {
                        report.set_text_content(Some(error.as_string().as_deref().unwrap_or(
                            "Browser DOM operation failed; no successful presentation is claimed.",
                        )))
                    }
                })
                .map_err(control)?;
            root.append_child(button.element())?;
            controls.push(button);
        }
        body.append_child(&root)?;
        let fixture = Fixture {
            document,
            root,
            output,
            field,
            submit,
            surfaces,
            controls,
            boundary,
            submitted: false,
        };
        fixture.connection(SessionConnection::Connected);
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        future::{Future, pending},
        rc::Rc,
        task::{Context, Poll, Waker},
    };

    use df_client::receipts::{LookupOutcome, OperationKey, OperationResolution, ReceiptLookup};
    use df_ui::FeedbackView;

    use super::contract::{
        Boundary, COMMITTED, FixtureReceipt, PENDING, SuppliedLookup, UNKNOWN, revision,
    };

    struct PendingLookup {
        requested: Rc<Cell<Option<OperationKey>>>,
        calls: u32,
    }

    impl ReceiptLookup for PendingLookup {
        type Receipt = FixtureReceipt;
        type Error = ();

        fn lookup(
            &mut self,
            key: OperationKey,
        ) -> impl Future<Output = Result<LookupOutcome<Self::Receipt>, Self::Error>> {
            self.requested.set(Some(key));
            self.calls += 1;
            pending()
        }
    }

    fn resolve(boundary: &mut Boundary, outcome: Result<LookupOutcome<FixtureReceipt>, ()>) {
        let key = boundary.operation.key();
        let mut lookup = SuppliedLookup {
            outcome,
            requested: None,
        };
        {
            let mut future = std::pin::pin!(boundary.operation.resolve(&mut lookup));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(_)
            ));
        }
        assert_eq!(lookup.requested, Some(key));
    }

    #[test]
    fn uncertain_lookup_and_reconnect_preserve_original_intent_and_committed_view() {
        let mut boundary = Boundary::create("local edited draft".into()).expect("fixture IDs");
        for outcome in [
            Ok(LookupOutcome::InProgress),
            Ok(LookupOutcome::NotRecorded),
            Err(()),
            Ok(LookupOutcome::ExpiredOrIndeterminate),
        ] {
            let expected = if matches!(&outcome, Ok(LookupOutcome::InProgress)) {
                FeedbackView::Pending(PENDING)
            } else {
                FeedbackView::Uncertain(UNKNOWN)
            };
            resolve(&mut boundary, outcome);
            assert_eq!(
                FeedbackView::for_resolution(boundary.operation.resolution(), PENDING, UNKNOWN),
                Some(expected)
            );
            assert_eq!(boundary.operation.intent(), "local edited draft");
            assert_eq!(
                boundary.views.current().map(|(_, text)| *text),
                Some(COMMITTED)
            );
        }
        boundary.reconnect().expect("new supplied generation");
        assert_eq!(
            FeedbackView::for_resolution(boundary.operation.resolution(), PENDING, UNKNOWN),
            Some(FeedbackView::Uncertain(UNKNOWN))
        );
        boundary
            .update_view()
            .expect("newer view admitted; stale view refused");
        assert_eq!(boundary.operation.intent(), "local edited draft");
    }

    #[test]
    fn committed_rejection_requires_receipt_owner_and_never_becomes_game_success() {
        let mut boundary = Boundary::create("retained rejected draft".into()).expect("fixture IDs");
        resolve(
            &mut boundary,
            Ok(LookupOutcome::Committed {
                revision: revision(1).expect("epoch"),
                receipt: FixtureReceipt::Refused,
            }),
        );
        assert!(matches!(
            boundary.operation.resolution(),
            OperationResolution::Committed {
                receipt: FixtureReceipt::Refused,
                ..
            }
        ));
        assert_eq!(
            FeedbackView::for_resolution(boundary.operation.resolution(), PENDING, UNKNOWN),
            None
        );
        assert_eq!(boundary.operation.intent(), "retained rejected draft");
        assert_eq!(
            boundary.views.current().map(|(_, text)| *text),
            Some(COMMITTED)
        );
    }

    #[test]
    fn accepted_decision_with_pending_work_does_not_publish_a_completed_view() {
        let mut boundary = Boundary::create("submitted draft".into()).expect("fixture IDs");
        resolve(
            &mut boundary,
            Ok(LookupOutcome::Committed {
                revision: revision(1).expect("epoch"),
                receipt: FixtureReceipt::AcceptedWorkPending,
            }),
        );
        assert_eq!(
            FeedbackView::for_resolution(boundary.operation.resolution(), PENDING, UNKNOWN),
            None
        );
        assert_eq!(
            boundary.views.current().map(|(_, text)| *text),
            Some(COMMITTED)
        );
    }

    #[test]
    fn dropped_lookup_wait_and_retired_namespace_remain_uncertain_without_replay() {
        let mut boundary = Boundary::create("original intent".into()).expect("fixture IDs");
        let original_key = boundary.operation.key();
        let requested = Rc::new(Cell::new(None));
        let mut lookup = PendingLookup {
            requested: Rc::clone(&requested),
            calls: 0,
        };
        {
            let mut future = std::pin::pin!(boundary.operation.resolve(&mut lookup));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            assert_eq!(requested.get(), Some(original_key));
            // Leaving this scope drops the already-polled pending wait.
        }
        assert_eq!(lookup.calls, 1);
        assert_eq!(requested.get(), Some(original_key));
        assert_eq!(boundary.operation.key(), original_key);
        assert_eq!(boundary.operation.intent(), "original intent");
        assert_eq!(
            boundary.views.current().map(|(_, text)| *text),
            Some(COMMITTED)
        );

        assert!(matches!(
            boundary.operation.resolution(),
            OperationResolution::Uncertain
        ));
        boundary
            .operation
            .retire_namespace(df_types::RecoveryEpoch::new(2).expect("epoch"))
            .expect("newer epoch");
        assert_eq!(
            FeedbackView::for_resolution(boundary.operation.resolution(), PENDING, UNKNOWN),
            Some(FeedbackView::Uncertain(UNKNOWN))
        );
        assert_eq!(boundary.operation.intent(), "original intent");
    }
}
