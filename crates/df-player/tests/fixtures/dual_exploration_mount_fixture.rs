#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::ViewAcceptance;
    use df_display::{DisplayExploration, ExplorationMountError as DisplayError};
    use df_player::{ExplorationMountError as PlayerError, PlayerExploration};
    use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
    use df_ui::{
        ActionView, CampaignLimits, CampaignMember, CampaignView, ConceptScene, ControlledAction,
        ExplorationChoice, ExplorationInput, ExplorationLimits, ExplorationNpc,
        ExplorationPortrait, ExplorationView,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlButtonElement, HtmlInputElement, Node};

    const PRIVATE_TEXT: &str = "PRIVATE_NPC_STORY_SENTINEL";
    const PRIVATE_DRAFT: &str = "PRIVATE_DRAFT_SENTINEL";
    const PRIVATE_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "offered:player/ask?opaque=1",
            label: "Ask Vell about the lantern",
            detail: "Send the supplied question.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offered:player/listen?opaque=2",
            label: "Listen to the room",
            detail: "Send the supplied listening offer.",
            disabled_reason: None,
        },
    ];
    const REORDERED_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "offered:player/listen?opaque=2",
            label: "Listen to the room",
            detail: "Send the supplied listening offer.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offered:player/ask?opaque=1",
            label: "Ask Vell about the lantern",
            detail: "Send the supplied question.",
            disabled_reason: None,
        },
    ];
    const HOST_CHOICES: [ExplorationChoice<'static>; 1] = [ExplorationChoice {
        id: "offered:public-host/continue?opaque=3",
        label: "Continue the public scene",
        detail: "Fixture-supplied public host offer.",
        disabled_reason: None,
    }];
    const MEMBERS: [CampaignMember<'static>; 1] = [CampaignMember {
        key: "corin",
        name: "Corin Vale",
        role: "Adventurer",
        sigil: "CV",
    }];

    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn binding(byte: u8) -> Result<ClientBindingId, JsValue> {
        ClientBindingId::from_bytes(&[byte; 16])
            .map_err(|failure| JsValue::from_str(&format!("fixture binding rejected: {failure:?}")))
    }
    fn revision(epoch: u64, sequence: u64) -> Result<SessionRevision, JsValue> {
        Ok(SessionRevision::new(
            RecoveryEpoch::new(epoch).map_err(|failure| {
                JsValue::from_str(&format!("fixture epoch rejected: {failure:?}"))
            })?,
            sequence,
        ))
    }
    fn limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: CampaignLimits {
                max_members: 8,
                max_objectives: 8,
                max_text_bytes: 1024,
            },
            max_choices: 8,
            max_text_bytes: 1024,
            max_identifier_bytes: 256,
        }
    }
    fn public_campaign() -> CampaignView<'static> {
        CampaignView {
            scene: ConceptScene::Tavern,
            chapter: "Act I · The Lamplighter",
            title: "The Drowned Lantern",
            description: "A hearth, a lowered voice, a story waiting to be heard.",
            location: "Greyhaven · The Drowned Lantern",
            scene_label: "At the barkeeper’s table",
            narration: "Rain drums against the roof. Vell sets down a glass beside the lantern.",
            connection: "Synthetic role fixture · No session or provider connected",
            notice: "Caller-filtered presentation boundary · Not an RPC projection",
            members: &MEMBERS,
            objectives: &[],
        }
    }
    // This separately constructed public snapshot never reads a private view.
    fn public_view(
        binding: ClientBindingId,
        revision: SessionRevision,
        host: bool,
        pending: bool,
    ) -> ExplorationView<'static> {
        ExplorationView {
            binding,
            revision,
            campaign: public_campaign(),
            npc: Some(ExplorationNpc {
                name: "Vell",
                context: "Barkeeper · The Drowned Lantern",
                dialogue: "Welcome, travelers. The storm will pass.",
                portrait: Some(ExplorationPortrait::Vell),
            }),
            heading: "The party’s conversation",
            choices: if host { &HOST_CHOICES } else { &[] },
            draft_label: "No public free-text input offered",
            submit_label: "No public draft offered",
            draft_offer_id: None,
            pending: if pending {
                Some("Waiting for a supplied public scene update.")
            } else {
                None
            },
            rejection: None,
        }
    }
    fn private_view(
        binding: ClientBindingId,
        revision: SessionRevision,
        reordered: bool,
        pending: bool,
        refused: bool,
    ) -> ExplorationView<'static> {
        let mut campaign = public_campaign();
        campaign.narration =
            "PRIVATE_NPC_STORY_SENTINEL · You alone recognize the lamplighter’s crest.";
        ExplorationView {
            binding,
            revision,
            campaign,
            npc: Some(ExplorationNpc {
                name: "Vell",
                context: PRIVATE_TEXT,
                dialogue: "PRIVATE_NPC_STORY_SENTINEL · Vell whispers a name only you heard.",
                portrait: Some(ExplorationPortrait::Vell),
            }),
            heading: "Your private conversation",
            choices: if reordered {
                &REORDERED_CHOICES
            } else {
                &PRIVATE_CHOICES
            },
            draft_label: "Your words",
            submit_label: "Send your words",
            draft_offer_id: Some("offered:player/draft?opaque=4"),
            pending: if pending {
                Some("Your input is pending; no outcome is confirmed.")
            } else {
                None
            },
            rejection: if refused {
                Some("The supplied refusal leaves your words editable.")
            } else {
                None
            },
        }
    }

    struct Fixture {
        document: Document,
        player_parent: Element,
        display_parent: Element,
        player: Option<PlayerExploration>,
        display: Option<DisplayExploration>,
        player_binding: ClientBindingId,
        display_binding: ClientBindingId,
        revision: SessionRevision,
        host: bool,
        inputs: Rc<RefCell<Vec<ExplorationInput>>>,
        public_inputs: Rc<RefCell<Vec<ExplorationInput>>>,
        status: Element,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn player(fixture: &Fixture) -> Result<&PlayerExploration, JsValue> {
        fixture
            .player
            .as_ref()
            .ok_or_else(|| JsValue::from_str("player mount is absent"))
    }
    fn display(fixture: &Fixture) -> Result<&DisplayExploration, JsValue> {
        fixture
            .display
            .as_ref()
            .ok_or_else(|| JsValue::from_str("display mount is absent"))
    }
    fn record_status(fixture: &Fixture, value: &str) -> Result<(), JsValue> {
        fixture.status.set_text_content(Some(value));
        fixture.status.set_attribute("data-result", value)
    }
    fn input(root: &Element) -> Result<HtmlInputElement, JsValue> {
        root.query_selector("input")?
            .ok_or_else(|| JsValue::from_str("draft input missing"))?
            .dyn_into()
            .map_err(|_| JsValue::from_str("wrong draft node type"))
    }
    fn first_choice(root: &Element) -> Result<HtmlButtonElement, JsValue> {
        root.query_selector(".exploration-choice button")?
            .ok_or_else(|| JsValue::from_str("choice missing"))?
            .dyn_into()
            .map_err(|_| JsValue::from_str("wrong choice node type"))
    }
    fn next_revision(fixture: &Fixture) -> Result<SessionRevision, JsValue> {
        fixture.revision.next_sequence().map_err(|failure| {
            JsValue::from_str(&format!("fixture revision rejected: {failure:?}"))
        })
    }
    fn update(
        fixture: &mut Fixture,
        reordered: bool,
        pending: bool,
        refused: bool,
    ) -> Result<(), JsValue> {
        let current_revision = next_revision(fixture)?;
        player(fixture)?
            .update(&private_view(
                fixture.player_binding,
                current_revision,
                reordered,
                pending,
                refused,
            ))
            .map_err(error)?;
        display(fixture)?
            .update(&public_view(
                fixture.display_binding,
                current_revision,
                fixture.host,
                pending,
            ))
            .map_err(error)?;
        fixture.revision = current_revision;
        Ok(())
    }
    fn mount_pair(fixture: &mut Fixture) -> Result<(), JsValue> {
        let inputs = Rc::clone(&fixture.inputs);
        fixture.player = Some(
            PlayerExploration::mount(
                &fixture.document,
                &fixture.player_parent,
                "player-exploration-draft",
                &private_view(
                    fixture.player_binding,
                    fixture.revision,
                    false,
                    false,
                    false,
                ),
                limits(),
                move |input| inputs.borrow_mut().push(input),
            )
            .map_err(error)?,
        );
        let public_inputs = Rc::clone(&fixture.public_inputs);
        fixture.display = Some(
            DisplayExploration::mount(
                &fixture.document,
                &fixture.display_parent,
                "public-exploration-draft",
                &public_view(
                    fixture.display_binding,
                    fixture.revision,
                    fixture.host,
                    false,
                ),
                limits(),
                move |input| public_inputs.borrow_mut().push(input),
            )
            .map_err(error)?,
        );
        Ok(())
    }
    fn audience_safe(fixture: &Fixture) -> Result<(), JsValue> {
        let root = display(fixture)?.root();
        require(
            !root.outer_html().contains(PRIVATE_TEXT) && !root.outer_html().contains(PRIVATE_DRAFT),
            "public DOM contains private fixture sentinels",
        )?;
        require(
            input(root)?.value().is_empty(),
            "public draft contains private input",
        )
    }

    #[wasm_bindgen]
    pub fn exercise_exploration_mounts() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot.as_mut().ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let root = player(fixture)?.root().clone();
            let draft = input(&root)?;
            draft.set_value(PRIVATE_DRAFT);
            draft.dispatch_event(&Event::new("input")?)?;
            draft.focus()?;
            let raw_private_text = root.query_selector(".exploration-dialogue")?.and_then(|element| element.first_child()).ok_or_else(|| JsValue::from_str("private Text missing"))?;
            update(fixture, true, false, false)?;
            require(player(fixture)?.root().is_same_node(Some(&root)), "ordinary update remounted player")?;
            require(input(&root)?.is_same_node(Some(&draft)), "ordinary update replaced draft node")?;
            require(draft.value() == PRIVATE_DRAFT, "ordinary update lost valid draft")?;
            require(fixture.document.active_element().is_some_and(|active| active.is_same_node(Some(&draft))), "ordinary update lost draft focus")?;
            require(raw_private_text.node_value().unwrap_or_default().is_empty(), "replaced private Text remains readable")?;
            audience_safe(fixture)?;
            let count = fixture.inputs.borrow().len();
            let choice = first_choice(&root)?;
            choice.click();
            let inputs = fixture.inputs.borrow();
            require(inputs.len() == count + 1, "offered choice did not emit exactly once")?;
            require(matches!(inputs.last(), Some(ExplorationInput::Choice { id, revision }) if id == "offered:player/listen?opaque=2" && *revision == fixture.revision), "choice changed supplied opaque ID or revision")?;
            drop(inputs);
            player(fixture)?.suspend_input().map_err(error)?;
            display(fixture)?.suspend_input().map_err(error)?;
            let count = fixture.inputs.borrow().len();
            choice.dispatch_event(&Event::new("click")?)?;
            require(fixture.inputs.borrow().len() == count, "suspended input emitted")?;
            let duplicate = player(fixture)?.update(&private_view(fixture.player_binding, fixture.revision, false, false, false));
            require(matches!(duplicate, Err(PlayerError::Admission(ViewAcceptance::Duplicate { .. }))), "duplicate snapshot resumed input")?;
            choice.dispatch_event(&Event::new("click")?)?;
            require(fixture.inputs.borrow().len() == count, "duplicate snapshot reactivated choice")?;
            update(fixture, true, false, true)?;
            require(draft.value() == PRIVATE_DRAFT && !draft.read_only(), "reconnect/refusal lost editable draft")?;
            update(fixture, true, true, false)?;
            let count = fixture.inputs.borrow().len();
            choice.dispatch_event(&Event::new("click")?)?;
            require(fixture.inputs.borrow().len() == count, "pending view emitted input")?;
            update(fixture, false, false, false)?;
            let mut wrong = private_view(binding(99)?, next_revision(fixture)?, false, false, false);
            require(matches!(player(fixture)?.update(&wrong), Err(PlayerError::Admission(ViewAcceptance::WrongBinding))), "wrong player binding accepted")?;
            wrong = public_view(binding(99)?, next_revision(fixture)?, false, false);
            require(matches!(display(fixture)?.update(&wrong), Err(DisplayError::Admission(ViewAcceptance::WrongBinding))), "wrong public binding accepted")?;
            require(display(fixture)?.root().query_selector(".exploration-choice")?.is_none(), "public view manufactured host offers")?;
            fixture.host = true;
            update(fixture, false, false, false)?;
            let public_count = fixture.public_inputs.borrow().len();
            first_choice(display(fixture)?.root())?.click();
            let public_inputs = fixture.public_inputs.borrow();
            require(public_inputs.len() == public_count + 1, "explicit public host offer did not emit once")?;
            require(matches!(public_inputs.last(), Some(ExplorationInput::Choice { id, revision }) if id == "offered:public-host/continue?opaque=3" && *revision == fixture.revision), "public host input changed its offered ID or revision")?;
            drop(public_inputs);
            fixture.host = false;
            let old_host = first_choice(display(fixture)?.root())?;
            update(fixture, false, false, false)?;
            old_host.dispatch_event(&Event::new("click")?)?;
            require(fixture.public_inputs.borrow().len() == public_count + 1, "retired public host offer emitted")?;
            let epoch = fixture.revision.epoch().get().checked_add(1).ok_or_else(|| JsValue::from_str("fixture epoch exhausted"))?;
            fixture.revision = revision(epoch, 0)?;
            player(fixture)?.update(&private_view(fixture.player_binding, fixture.revision, false, false, false)).map_err(error)?;
            display(fixture)?.update(&public_view(fixture.display_binding, fixture.revision, false, false)).map_err(error)?;
            require(draft.value().is_empty(), "new epoch retained obsolete draft")?;
            audience_safe(fixture)?;
            record_status(fixture, "exercise-pass")
        })
    }

    #[wasm_bindgen]
    pub fn replace_exploration_binding() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let retained = player(fixture)?.root().clone();
            let draft = input(&retained)?;
            draft.set_value(PRIVATE_DRAFT);
            draft.dispatch_event(&Event::new("input")?)?;
            let raw: Node = retained
                .query_selector(".exploration-dialogue")?
                .and_then(|node| node.first_child())
                .ok_or_else(|| JsValue::from_str("private Text missing"))?;
            player(fixture)?.revoke().map_err(error)?;
            display(fixture)?.revoke().map_err(error)?;
            fixture.player.take();
            fixture.display.take();
            require(
                raw.node_value().unwrap_or_default().is_empty()
                    && draft.value().is_empty()
                    && !retained.outer_html().contains(PRIVATE_TEXT),
                "binding revoke retained private DOM",
            )?;
            fixture.inputs.borrow_mut().clear();
            fixture.public_inputs.borrow_mut().clear();
            fixture.player_binding = binding(9)?;
            fixture.display_binding = binding(10)?;
            fixture.revision = revision(1, 0)?;
            fixture.host = false;
            mount_pair(fixture)?;
            audience_safe(fixture)?;
            record_status(fixture, "replacement-pass")
        })
    }

    #[wasm_bindgen]
    pub fn drop_exploration_mounts() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let retained = player(fixture)?.root().clone();
            let draft = input(&retained)?;
            draft.set_value(PRIVATE_DRAFT);
            draft.dispatch_event(&Event::new("input")?)?;
            let raw = retained
                .query_selector(".exploration-dialogue")?
                .and_then(|node| node.first_child())
                .ok_or_else(|| JsValue::from_str("private Text missing"))?;
            let old_choice = first_choice(&retained)?;
            let count = fixture.inputs.borrow().len();
            fixture.player.take();
            fixture.display.take();
            old_choice.dispatch_event(&Event::new("click")?)?;
            require(
                fixture.inputs.borrow().len() == count,
                "Drop retained active input callback",
            )?;
            require(
                raw.node_value().unwrap_or_default().is_empty()
                    && draft.value().is_empty()
                    && !retained.outer_html().contains(PRIVATE_TEXT),
                "Drop retained private DOM",
            )?;
            fixture.inputs.borrow_mut().clear();
            fixture.public_inputs.borrow_mut().clear();
            record_status(fixture, "drop-pass")
        })
    }

    #[wasm_bindgen]
    pub fn advance_exploration_mounts() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            update(fixture, false, false, false)?;
            audience_safe(fixture)?;
            record_status(fixture, "advance-pass")
        })
    }
    fn section(document: &Document, parent: &Element, role: &str) -> Result<Element, JsValue> {
        let section = document.create_element("section")?;
        section.set_attribute("data-client", role)?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some(role));
        section.append_child(&heading)?;
        parent.append_child(&section)?;
        Ok(section)
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("window missing"))?
            .document()
            .ok_or_else(|| JsValue::from_str("document missing"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body missing"))?;
        let presentation_style = document.create_element("style")?;
        presentation_style.set_text_content(Some(
            r#"
body{margin:0;background:#141310;color:#f4eddf}
body > p{width:92%;margin:0 auto;padding:20px 0 14px;font:11px/1.7 system-ui,sans-serif;color:#b4aca0;overflow-wrap:anywhere}
body > nav[aria-label="Fixture controls"]{width:92%;margin:0 auto 12px;display:flex;flex-wrap:wrap;gap:8px}
body > nav[aria-label="Fixture controls"] button{min-height:44px;padding:10px 13px;border:1px solid #a98b574d;border-radius:6px;background:#29251e;color:#ede2cb;font:12px/1.5 system-ui,sans-serif;touch-action:manipulation;cursor:pointer;overflow-wrap:anywhere}
body > nav[aria-label="Fixture controls"] button:hover:enabled{background:#42382a;border-color:#e5bd77}
body > nav[aria-label="Fixture controls"] button:focus-visible{outline:3px solid #f1d69b;outline-offset:4px}
#dual-exploration-status{display:block;width:92%;box-sizing:border-box;margin:0 auto 18px;padding:8px 12px;border-left:2px solid #a98b574d;background:#1d1c17;font:11px/1.7 system-ui,sans-serif;color:#b4aca0;overflow-wrap:anywhere}
body > section[data-client] > h1{width:92%;margin:0 auto;padding:18px 0 12px;border-top:1px solid #d9b77a24;font:10px/1.7 system-ui,sans-serif;letter-spacing:2px;text-transform:uppercase;color:#d9b77a}
@media(max-width:700px){body > nav[aria-label="Fixture controls"]{display:grid;grid-template-columns:1fr 1fr}body > nav[aria-label="Fixture controls"] button{min-width:0}}
@media(prefers-reduced-motion:reduce){body > nav[aria-label="Fixture controls"] button{animation:none!important;transition:none!important}}
"#,
        ));
        body.append_child(&presentation_style)?;
        let notice = document.create_element("p")?;
        notice.set_text_content(Some("Synthetic dual-client exploration mounts. Public and private snapshots are supplied separately; no RPC/session authority is claimed."));
        body.append_child(&notice)?;
        let controls_root = document.create_element("nav")?;
        controls_root.set_attribute("aria-label", "Fixture controls")?;
        body.append_child(&controls_root)?;
        let status = document.create_element("output")?;
        status.set_id("dual-exploration-status");
        status.set_attribute("aria-live", "polite")?;
        body.append_child(&status)?;
        let player_parent = section(&document, &body, "player")?;
        let display_parent = section(&document, &body, "shared-display")?;
        let mut fixture = Fixture {
            document: document.clone(),
            player_parent,
            display_parent,
            player: None,
            display: None,
            player_binding: binding(7)?,
            display_binding: binding(8)?,
            revision: revision(1, 0)?,
            host: false,
            inputs: Rc::new(RefCell::new(Vec::new())),
            public_inputs: Rc::new(RefCell::new(Vec::new())),
            status,
            controls: Vec::new(),
        };
        mount_pair(&mut fixture)?;
        for (id, label, operation) in [
            (
                "exercise-exploration",
                "Exercise updates, offers and reconnect",
                exercise_exploration_mounts as fn() -> Result<(), JsValue>,
            ),
            (
                "advance-exploration",
                "Apply newer supplied snapshots",
                advance_exploration_mounts,
            ),
            (
                "replace-exploration",
                "Replace both bindings",
                replace_exploration_binding,
            ),
            (
                "drop-exploration",
                "Drop both mounts",
                drop_exploration_mounts,
            ),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            control.element().set_id(id);
            control.on_activate(move || {
                if let Err(error) = operation() {
                    FIXTURE.with(|slot| { if let Some(fixture) = slot.borrow().as_ref() { fixture.status.set_text_content(Some(&format!("fixture-failure: {error:?}"))); if let Err(status_error) = fixture.status.set_attribute("data-result", "failure") { fixture.status.set_text_content(Some(&format!("fixture-failure: {error:?}; status-failure: {status_error:?}"))); } } });
                }
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            fixture.controls.push(control);
        }
        audience_safe(&fixture)?;
        record_status(&fixture, "ready")?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
}
