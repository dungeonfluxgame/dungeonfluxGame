#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::ViewAcceptance;
    use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
    use df_ui::{
        CampaignLimits, CampaignView, ConceptScene, ExplorationChoice, ExplorationLimits,
        ExplorationView, SessionConnection, SessionOverlayView,
    };
    use df_web::{CampaignPanelError, CampaignPanelShell, PanelLease, PlayerPhase, RolePhase};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlInputElement};

    type EventListener = Closure<dyn FnMut(Event)>;

    const DRAFT: &str = "Ask the keeper about the harbor";
    const GROUPS: [&str; 8] = [
        "Panel action retains campaign and draft",
        "Campaign update retains open panel",
        "Repeated close and reopen disposes once",
        "Replacement fences stale controls",
        "Obsolete completion cannot mutate",
        "Invalid ownership and mount refusal",
        "Cleanup failure fences before reopen",
        "Explicit disposal and Drop",
    ];
    fn error(failure: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&failure.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn document() -> Result<Document, JsValue> {
        web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("fixture document missing"))
    }
    fn binding() -> Result<ClientBindingId, JsValue> {
        ClientBindingId::from_bytes(&[93; 16]).map_err(|failure| error(format!("{failure:?}")))
    }
    fn revision(sequence: u64) -> Result<SessionRevision, JsValue> {
        Ok(SessionRevision::new(
            RecoveryEpoch::new(1).map_err(|failure| error(format!("{failure:?}")))?,
            sequence,
        ))
    }
    fn campaign() -> CampaignView<'static> {
        CampaignView {
            scene: ConceptScene::Tavern,
            chapter: "Controlled coexistence fixture",
            title: "The Drowned Lantern",
            description: "A supplied campaign remains mounted.",
            location: "Greyhaven",
            scene_label: "Independent local panel",
            narration: "Game authority remains with the existing connection owner.",
            connection: "Connected supplied presentation",
            notice: "No account or payment RPC",
            members: &[],
            objectives: &[],
        }
    }
    fn campaign_limits() -> CampaignLimits {
        CampaignLimits {
            max_members: 8,
            max_objectives: 8,
            max_text_bytes: 4096,
        }
    }
    struct Case {
        document: Document,
        parent: Element,
        shell: CampaignPanelShell,
        role_calls: Rc<Cell<u32>>,
        sequence: u64,
    }
    impl Case {
        fn new() -> Result<Self, JsValue> {
            let document = document()?;
            let parent = document.create_element("section")?;
            parent.set_attribute("data-coexistence-case", "")?;
            document
                .body()
                .ok_or_else(|| error("fixture body missing"))?
                .append_child(&parent)?;
            let role_calls = Rc::new(Cell::new(0_u32));
            let count = Rc::clone(&role_calls);
            let shell = CampaignPanelShell::mount(
                &document,
                &parent,
                binding()?,
                "coexistence-draft",
                move |_| {
                    count.set(count.get().saturating_add(1));
                },
            )
            .map_err(error)?;
            let mut case = Self {
                document,
                parent,
                shell,
                role_calls,
                sequence: 0,
            };
            case.present()?;
            let draft = case.draft()?;
            draft.set_value(DRAFT);
            draft.focus()?;
            draft.set_selection_range(3, 7)?;
            Ok(case)
        }
        fn present(&mut self) -> Result<(), JsValue> {
            let sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| error("fixture revision exhausted"))?;
            let revision = revision(sequence)?;
            let binding = binding()?;
            let choices = [ExplorationChoice {
                id: "supplied-question",
                label: "Ask the keeper",
                detail: "Controlled selection",
                disabled_reason: None,
            }];
            let view = ExplorationView {
                binding,
                revision,
                campaign: campaign(),
                npc: None,
                heading: "A current supplied campaign",
                choices: &choices,
                draft_label: "Your ongoing question",
                submit_label: "Send supplied question",
                draft_offer_id: Some("supplied-draft"),
                pending: None,
                rejection: None,
            };
            let overlay = SessionOverlayView {
                generation: 1,
                revision: sequence,
                title: "Controlled campaign",
                subtitle: "Local panel coexistence",
                connection: SessionConnection::Connected,
                connection_label: "Connected",
                connection_detail: "Supplied current presentation",
                operation: None,
                bookend: None,
                host_context: "No host authority",
                host_offers: &[],
                reduced_motion: false,
            };
            let accepted = self
                .shell
                .campaign_mut()
                .map_err(error)?
                .present(
                    binding,
                    revision,
                    RolePhase::Player(PlayerPhase::Exploration {
                        view: &view,
                        limits: ExplorationLimits {
                            campaign: campaign_limits(),
                            max_choices: 8,
                            max_text_bytes: 4096,
                            max_identifier_bytes: 128,
                        },
                    }),
                    Some(&overlay),
                )
                .map_err(error)?;
            require(
                accepted == ViewAcceptance::Applied,
                "current campaign was not applied",
            )?;
            self.sequence = sequence;
            Ok(())
        }
        fn draft(&self) -> Result<HtmlInputElement, JsValue> {
            self.shell
                .campaign()
                .root()
                .query_selector("input")?
                .ok_or_else(|| error("campaign draft missing"))?
                .dyn_into::<HtmlInputElement>()
                .map_err(|_| error("campaign draft has wrong type"))
        }
        fn unchanged(
            &self,
            campaign: &Element,
            draft: &HtmlInputElement,
            sequence: u64,
        ) -> Result<(), JsValue> {
            require(
                campaign.is_same_node(Some(self.shell.campaign().root())),
                "campaign remounted",
            )?;
            require(campaign.is_connected(), "campaign disconnected")?;
            let current_draft = self.draft()?;
            require(
                draft.is_same_node(Some(&current_draft)),
                "draft node replaced",
            )?;
            require(draft.value() == DRAFT, "ongoing campaign draft lost")?;
            require(
                draft.selection_start()? == Some(3) && draft.selection_end()? == Some(7),
                "ongoing draft cursor lost",
            )?;
            require(
                self.shell.campaign().revision() == Some(revision(sequence)?),
                "panel action changed campaign revision",
            )?;
            require(
                self.document
                    .active_element()
                    .is_some_and(|active| active.is_same_node(Some(draft))),
                "panel action stole campaign focus",
            )
        }
    }
    impl Drop for Case {
        fn drop(&mut self) {
            let _result = self.shell.dispose();
            self.parent.remove();
        }
    }
    struct Panel {
        lease: PanelLease,
        root: Element,
        button: Element,
        calls: Rc<Cell<u32>>,
        cleanups: Rc<Cell<u32>>,
    }
    fn open(case: &mut Case, fail_cleanup: bool) -> Result<Panel, JsValue> {
        let root = case.document.create_element("section")?;
        root.set_attribute("data-local-customer-panel", "")?;
        let heading = case.document.create_element("h2")?;
        heading.set_text_content(Some("Caller-owned local customer panel"));
        let button = case.document.create_element("button")?;
        button.set_text_content(Some("Local panel action · no RPC"));
        root.append_child(&heading)?;
        root.append_child(&button)?;
        let calls = Rc::new(Cell::new(0_u32));
        let cleanups = Rc::new(Cell::new(0_u32));
        let listener: Rc<RefCell<Option<EventListener>>> = Rc::new(RefCell::new(None));
        let cleanup_listener = Rc::clone(&listener);
        let cleanup_button = button.clone();
        let cleanup_count = Rc::clone(&cleanups);
        let lease = case
            .shell
            .open_panel(root.clone(), move || {
                cleanup_count.set(cleanup_count.get().saturating_add(1));
                if let Some(listener) = cleanup_listener.borrow_mut().take() {
                    cleanup_button.remove_event_listener_with_callback(
                        "click",
                        listener.as_ref().unchecked_ref(),
                    )?;
                }
                if fail_cleanup {
                    Err(error("controlled caller cleanup failure"))
                } else {
                    Ok(())
                }
            })
            .map_err(error)?;
        let action_lease = lease.clone();
        let action_calls = Rc::clone(&calls);
        let callback = Closure::<dyn FnMut(Event)>::new(move |_| {
            let next = action_calls.get().saturating_add(1);
            if next <= 64
                && action_lease
                    .apply(|root| root.set_attribute("data-local-actions", &next.to_string()))
                    .is_ok()
            {
                action_calls.set(next);
            }
        });
        if let Err(failure) =
            button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())
        {
            let _cleanup = case.shell.close_panel();
            return Err(failure);
        }
        *listener.borrow_mut() = Some(callback);
        Ok(Panel {
            lease,
            root,
            button,
            calls,
            cleanups,
        })
    }
    fn click(button: &Element) -> Result<(), JsValue> {
        button.dispatch_event(&Event::new("click")?)?;
        Ok(())
    }
    fn obsolete(lease: &PanelLease) -> Result<(), JsValue> {
        let entered = Cell::new(false);
        let result = lease.apply(|root| {
            entered.set(true);
            root.set_text_content(Some("OBSOLETE COMPLETION"));
            Ok(())
        });
        require(
            matches!(
                result,
                Err(CampaignPanelError::StaleLease | CampaignPanelError::Disposed)
            ),
            "obsolete lease accepted",
        )?;
        require(!entered.get(), "obsolete caller callback entered")
    }

    #[wasm_bindgen]
    pub fn panel_check_group(group: u32) -> Result<(), JsValue> {
        require(group < 8, "unknown case group")?;
        let mut case = Case::new()?;
        let campaign = case.shell.campaign().root().clone();
        let draft = case.draft()?;
        match group {
            0 => {
                let game_button = campaign
                    .query_selector("button")?
                    .ok_or_else(|| error("game control missing"))?;
                click(&game_button)?;
                require(
                    case.role_calls.get() == 1,
                    "current game callback did not dispatch",
                )?;
                let panel = open(&mut case, false)?;
                require(
                    panel.root.parent_element().is_some_and(|host| {
                        host.get_attribute("data-customer-panel-slot").is_some()
                    }),
                    "panel not mounted in its slot",
                )?;
                click(&panel.button)?;
                require(
                    panel.calls.get() == 1 && case.role_calls.get() == 1,
                    "panel/game callback ownership crossed",
                )?;
                case.unchanged(&campaign, &draft, 1)?;
                case.shell.close_panel().map_err(error)?;
                case.unchanged(&campaign, &draft, 1)?;
                require(panel.cleanups.get() == 1, "panel cleanup not exactly once")?;
            }
            1 => {
                let panel = open(&mut case, false)?;
                case.present()?;
                case.unchanged(&campaign, &draft, 2)?;
                require(panel.root.is_connected(), "campaign update retired panel")?;
                panel
                    .lease
                    .apply(|root| root.set_attribute("data-new-view-panel", "retained"))
                    .map_err(error)?;
                click(&panel.button)?;
                require(
                    panel.calls.get() == 1,
                    "current panel listener lost on game update",
                )?;
            }
            2 => {
                for _ in 0..32 {
                    let panel = open(&mut case, false)?;
                    click(&panel.button)?;
                    case.shell.close_panel().map_err(error)?;
                    case.shell.close_panel().map_err(error)?;
                    click(&panel.button)?;
                    require(
                        panel.cleanups.get() == 1 && panel.calls.get() == 1,
                        "close leaked or repeated panel listener",
                    )?;
                    require(!panel.root.is_connected(), "closed panel retained DOM")?;
                    obsolete(&panel.lease)?;
                    case.unchanged(&campaign, &draft, 1)?;
                }
            }
            3 => {
                let old = open(&mut case, false)?;
                let current = open(&mut case, false)?;
                obsolete(&old.lease)?;
                click(&old.button)?;
                click(&current.button)?;
                require(
                    old.cleanups.get() == 1 && old.calls.get() == 0 && current.calls.get() == 1,
                    "replacement did not fence old control",
                )?;
                require(
                    !old.root.is_connected() && current.root.is_connected(),
                    "replacement DOM ownership crossed",
                )?;
                case.unchanged(&campaign, &draft, 1)?;
            }
            4 => {
                let old = open(&mut case, false)?;
                let before = old.root.text_content();
                case.shell.close_panel().map_err(error)?;
                let current = open(&mut case, false)?;
                let current_text = current.root.text_content();
                obsolete(&old.lease)?;
                require(
                    old.root.text_content() == before
                        && current.root.text_content() == current_text,
                    "obsolete completion mutated panel content",
                )?;
                case.unchanged(&campaign, &draft, 1)?;
            }
            5 => {
                let panel = open(&mut case, false)?;
                let cleanup = Rc::new(Cell::new(0_u32));
                let count = Rc::clone(&cleanup);
                let result = case.shell.open_panel(campaign.clone(), move || {
                    count.set(count.get() + 1);
                    Ok(())
                });
                require(
                    matches!(result, Err(CampaignPanelError::InvalidPanelOwnership)),
                    "attached campaign accepted as panel",
                )?;
                require(cleanup.get() == 1, "refused transfer skipped cleanup")?;
                panel
                    .lease
                    .apply(|root| root.set_attribute("data-admission-refused", "preserved"))
                    .map_err(error)?;
                let detached = case.document.create_element("section")?;
                let mount = CampaignPanelShell::mount(
                    &case.document,
                    &detached,
                    binding()?,
                    "invalid identifier",
                    |_| {},
                );
                require(
                    matches!(mount, Err(CampaignPanelError::Campaign(_))),
                    "invalid role mount accepted",
                )?;
                require(
                    detached.child_element_count() == 0,
                    "failed mount left partial shell",
                )?;
                case.unchanged(&campaign, &draft, 1)?;
            }
            6 => {
                let old = open(&mut case, true)?;
                require(
                    matches!(
                        case.shell.close_panel(),
                        Err(CampaignPanelError::Cleanup { .. })
                    ),
                    "cleanup failure hidden",
                )?;
                obsolete(&old.lease)?;
                click(&old.button)?;
                require(
                    old.cleanups.get() == 1 && old.calls.get() == 0 && !old.root.is_connected(),
                    "failed cleanup revived old callback",
                )?;
                let current = open(&mut case, false)?;
                click(&current.button)?;
                require(
                    current.calls.get() == 1,
                    "cleanup failure prevented valid reopen",
                )?;
                case.unchanged(&campaign, &draft, 1)?;
            }
            7 => {
                let panel = open(&mut case, false)?;
                let game_button = campaign
                    .query_selector("button")?
                    .ok_or_else(|| error("game control missing"))?;
                case.shell.dispose().map_err(error)?;
                case.shell.dispose().map_err(error)?;
                obsolete(&panel.lease)?;
                click(&panel.button)?;
                click(&game_button)?;
                require(
                    panel.cleanups.get() == 1
                        && panel.calls.get() == 0
                        && case.role_calls.get() == 0,
                    "explicit disposal repeated cleanup or retained callbacks",
                )?;
                require(
                    !campaign.is_connected(),
                    "disposed campaign remained mounted",
                )?;
                let mut dropped = Case::new()?;
                let dropped_campaign = dropped.shell.campaign().root().clone();
                let dropped_panel = open(&mut dropped, false)?;
                drop(dropped);
                obsolete(&dropped_panel.lease)?;
                click(&dropped_panel.button)?;
                require(
                    dropped_panel.cleanups.get() == 1
                        && dropped_panel.calls.get() == 0
                        && !dropped_campaign.is_connected(),
                    "Drop did not dispose both owners once",
                )?;
            }
            _ => return Err(error("unknown case group")),
        }
        Ok(())
    }

    struct Demo {
        case: Case,
        panel: Option<Panel>,
    }
    thread_local! {
        static DEMO: RefCell<Option<Demo>> = const { RefCell::new(None) };
        static CONTROLS: RefCell<Vec<EventListener>> = const { RefCell::new(Vec::new()) };
    }
    #[wasm_bindgen]
    pub fn panel_open() -> Result<(), JsValue> {
        DEMO.with(|owner| {
            let mut owner = owner.borrow_mut();
            let demo = owner.as_mut().ok_or_else(|| error("demo disposed"))?;
            demo.panel = Some(open(&mut demo.case, false)?);
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn panel_close() -> Result<(), JsValue> {
        DEMO.with(|owner| {
            let mut owner = owner.borrow_mut();
            let demo = owner.as_mut().ok_or_else(|| error("demo disposed"))?;
            demo.case.shell.close_panel().map_err(error)?;
            demo.panel = None;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn panel_campaign_update() -> Result<(), JsValue> {
        DEMO.with(|owner| {
            owner
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| error("demo disposed"))?
                .case
                .present()
        })
    }
    #[wasm_bindgen]
    pub fn panel_dispose_demo() -> Result<(), JsValue> {
        DEMO.with(|owner| {
            let demo = owner
                .borrow_mut()
                .take()
                .ok_or_else(|| error("demo already disposed"))?;
            drop(demo);
            Ok(())
        })
    }
    fn control(
        document: &Document,
        parent: &Element,
        label: &str,
        action: impl Fn() -> Result<(), JsValue> + 'static,
        status: &Element,
    ) -> Result<(), JsValue> {
        let button = document.create_element("button")?;
        button.set_text_content(Some(label));
        let status = status.clone();
        let callback = Closure::<dyn FnMut(Event)>::new(move |_| match action() {
            Ok(()) => {
                status.set_text_content(Some("PASS · controlled local operation"));
            }
            Err(failure) => {
                status.set_text_content(Some(&format!("FAIL · {failure:?}")));
            }
        });
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        parent.append_child(&button)?;
        CONTROLS.with(|controls| controls.borrow_mut().push(callback));
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = document()?;
        let body = document
            .body()
            .ok_or_else(|| error("fixture body missing"))?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some(
            "DungeonFlux · Campaign and customer panel coexistence",
        ));
        body.append_child(&heading)?;
        let disclosure = document.create_element("p")?;
        disclosure.set_text_content(Some("Controlled Rust-mounted presentation. No production account, payment, auth or CustomerService transport. Panel actions leave the campaign mounted."));
        body.append_child(&disclosure)?;
        let controls = document.create_element("nav")?;
        controls.set_attribute("aria-label", "Controlled coexistence checks")?;
        let status = document.create_element("p")?;
        status.set_attribute("aria-live", "polite")?;
        status.set_text_content(Some("Ready · checks have not run"));
        body.append_child(&controls)?;
        body.append_child(&status)?;
        for (index, label) in GROUPS.iter().enumerate() {
            control(
                &document,
                &controls,
                label,
                move || panel_check_group(index as u32),
                &status,
            )?;
        }
        control(
            &document,
            &controls,
            "Open or replace local panel",
            panel_open,
            &status,
        )?;
        control(
            &document,
            &controls,
            "Close local panel",
            panel_close,
            &status,
        )?;
        control(
            &document,
            &controls,
            "Supply newer campaign view",
            panel_campaign_update,
            &status,
        )?;
        control(
            &document,
            &controls,
            "Dispose demo",
            panel_dispose_demo,
            &status,
        )?;
        DEMO.with(|owner| {
            *owner.borrow_mut() = Some(Demo {
                case: Case::new()?,
                panel: None,
            });
            Ok::<(), JsValue>(())
        })?;
        panel_open()
    }
}
