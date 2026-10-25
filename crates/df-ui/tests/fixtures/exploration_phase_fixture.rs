#[cfg(target_arch = "wasm32")]
mod browser {
    use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
    use df_ui::{
        ActionView, CampaignLimits, CampaignMember, CampaignObjective, CampaignView, ConceptScene,
        ControlledAction, ExplorationChoice, ExplorationInput, ExplorationLimits, ExplorationNpc,
        ExplorationPhase, ExplorationPortrait, ExplorationView, ObjectiveState,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlInputElement, Node};

    const MEMBERS: [CampaignMember<'static>; 3] = [
        CampaignMember {
            key: "corin",
            name: "Corin Vale",
            role: "Human · Fighter",
            sigil: "CV",
        },
        CampaignMember {
            key: "lyria",
            name: "Lyria Thalen",
            role: "Elf · Rogue",
            sigil: "LT",
        },
        CampaignMember {
            key: "brom",
            name: "Brom Stonebeard",
            role: "Dwarf · Barbarian",
            sigil: "BS",
        },
    ];
    const OBJECTIVES: [CampaignObjective<'static>; 2] = [
        CampaignObjective {
            label: "Ask after the missing lamplighter",
            state: ObjectiveState::Active,
        },
        CampaignObjective {
            label: "Find the entrance beneath the harbor",
            state: ObjectiveState::Pending,
        },
    ];
    const CHOICES: [ExplorationChoice<'static>; 3] = [
        ExplorationChoice {
            id: "offer:vell/ask-lamplighter?revision=7",
            label: "Ask about the lamplighter",
            detail: "“Who was the last person to see him?”",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offer:vell/ask-harbor?revision=7",
            label: "Ask what lies beneath the harbor",
            detail: "Let Vell tell the story in his own words.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offer:vell/private-unavailable",
            label: "Ask for the sealed ledger",
            detail: "An unavailable option supplied by the fixture view.",
            disabled_reason: Some("Vell has not offered access to the ledger."),
        },
    ];
    const HALL_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "offer:hall/inspect-arch",
            label: "Inspect the carved archway",
            detail: "Send the offered inspection input.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offer:hall/listen",
            label: "Listen beyond the waterline",
            detail: "Send the offered listening input.",
            disabled_reason: None,
        },
    ];
    #[derive(Clone, Copy)]
    enum Mode {
        Conversation,
        Hall,
        Pending,
        Refused,
    }
    struct Fixture {
        phase: Rc<ExplorationPhase>,
        document: Document,
        revision: Cell<SessionRevision>,
        mode: Cell<Mode>,
        controls: Vec<ControlledAction>,
        count: Cell<u32>,
        status: Element,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn error(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn revision(epoch: u64, sequence: u64) -> Result<SessionRevision, JsValue> {
        Ok(SessionRevision::new(
            RecoveryEpoch::new(epoch).map_err(|_| JsValue::from_str("invalid fixture epoch"))?,
            sequence,
        ))
    }
    fn exploration_view(
        revision: SessionRevision,
        mode: Mode,
    ) -> Result<ExplorationView<'static>, JsValue> {
        let hall = matches!(mode, Mode::Hall);
        Ok(ExplorationView {
            binding: ClientBindingId::from_bytes(&[7; 16])
                .map_err(|_| JsValue::from_str("invalid fixture binding"))?,
            revision,
            campaign: CampaignView {
                scene: if hall {
                    ConceptScene::SunkenHall
                } else {
                    ConceptScene::Tavern
                },
                chapter: "Act I · The Lamplighter",
                title: if hall {
                    "The sunken halls"
                } else {
                    "Salt, smoke & secrets"
                },
                description: if hall {
                    "Water remembers what the town has forgotten."
                } else {
                    "A warm hearth. A lowered voice. A story waiting to be heard."
                },
                location: if hall {
                    "Below Greyhaven · The drowned temple"
                } else {
                    "Greyhaven · The Drowned Lantern"
                },
                scene_label: if hall {
                    "Beyond the waterline"
                } else {
                    "At the barkeeper’s table"
                },
                narration: if hall {
                    "Your torch catches the edge of a carved archway. Water trembles around your boots. Far inside the hall, something answers the sound of your footsteps."
                } else {
                    "Rain drums against the warped roof. Vell sets down a glass and looks toward the windows. The lantern above the bar flickers once."
                },
                connection: "Design fixture · Synthetic server view updates · No gameplay session connected",
                notice: "Design fixture · Synthetic server views",
                members: &MEMBERS,
                objectives: &OBJECTIVES,
            },
            npc: if hall {
                None
            } else {
                Some(ExplorationNpc {
                    name: "Vell",
                    context: "Barkeeper · The Drowned Lantern",
                    dialogue: "“You’re asking about the lamplighter? Best keep that question away from the windows.”",
                    portrait: Some(ExplorationPortrait::Vell),
                })
            },
            heading: if hall {
                "What will you do?"
            } else {
                "What will you ask?"
            },
            choices: if hall { &HALL_CHOICES } else { &CHOICES },
            draft_label: "Or write your own words",
            submit_label: "Send your words",
            draft_offer_id: Some(if hall {
                "offer:hall/draft"
            } else {
                "offer:vell/draft"
            }),
            pending: if matches!(mode, Mode::Pending) {
                Some(
                    "Fixture controller received your input. Awaiting a synthetic server view; no outcome is confirmed.",
                )
            } else {
                None
            },
            rejection: if matches!(mode, Mode::Refused) {
                Some(
                    "Synthetic server refusal: that request is unavailable. Your words are retained for editing.",
                )
            } else {
                None
            },
        })
    }
    fn advance(fixture: &Fixture, mode: Mode) -> Result<(), JsValue> {
        let next = fixture
            .revision
            .get()
            .next_sequence()
            .map_err(|_| JsValue::from_str("fixture sequence exhausted"))?;
        fixture
            .phase
            .update(&exploration_view(next, mode)?)
            .map_err(error)?;
        fixture.revision.set(next);
        fixture.mode.set(mode);
        Ok(())
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, JsValue> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(text);
        parent.append_child(&node)?;
        Ok(node)
    }
    struct RetainedAudience {
        root: Element,
        nodes: Vec<Node>,
        input: HtmlInputElement,
        choice: Element,
    }
    impl RetainedAudience {
        fn capture(phase: &ExplorationPhase) -> Result<Self, JsValue> {
            let root = phase.root().clone();
            let input = phase.draft_input().input().clone();
            let choice = root
                .query_selector("[data-exploration-offer]")?
                .ok_or_else(|| JsValue::from_str("privacy fixture choice missing"))?;
            let mut nodes = Vec::new();
            let mut pending: Vec<Node> = vec![root.clone().into()];
            while let Some(node) = pending.pop() {
                let mut child = node.first_child();
                while let Some(current) = child {
                    child = current.next_sibling();
                    pending.push(current);
                }
                nodes.push(node);
            }
            require(
                nodes.iter().any(|node| {
                    node.node_type() == Node::TEXT_NODE
                        && node.node_value().is_some_and(|value| !value.is_empty())
                }),
                "privacy fixture captured no actual Text nodes",
            )?;
            Ok(Self {
                root,
                nodes,
                input,
                choice,
            })
        }
        fn require_erased(&self) -> Result<(), JsValue> {
            require(
                self.root.parent_node().is_none(),
                "retained phase root remains mounted",
            )?;
            require(
                self.root.text_content().is_none_or(|text| text.is_empty()),
                "retained phase root contains audience text",
            )?;
            require(
                self.input.value().is_empty(),
                "retained input contains private draft",
            )?;
            for node in &self.nodes {
                require(
                    node.text_content().is_none_or(|text| text.is_empty()),
                    "retained Element or Text exposes earlier audience content",
                )?;
                if let Some(element) = node.dyn_ref::<Element>() {
                    for name in ["data-exploration-offer", "src", "alt"] {
                        require(
                            element.get_attribute(name).is_none(),
                            "retained element exposes offered identifier or audience asset attribute",
                        )?;
                    }
                }
            }
            require(
                self.choice
                    .get_attribute("data-exploration-offer")
                    .is_none(),
                "retired choice keeps offered identifier",
            )
        }
    }
    fn teardown_privacy(document: &Document, explicit: bool) -> Result<(), JsValue> {
        let first = revision(31, 1)?;
        let holder = document.create_element("div")?;
        holder.set_attribute("hidden", "")?;
        let phase = ExplorationPhase::create(
            document,
            if explicit {
                "exploration-privacy-explicit"
            } else {
                "exploration-privacy-drop"
            },
            &exploration_view(first, Mode::Refused)?,
            ExplorationLimits {
                campaign: CampaignLimits {
                    max_members: 16,
                    max_objectives: 16,
                    max_text_bytes: 4096,
                },
                max_choices: 16,
                max_text_bytes: 4096,
                max_identifier_bytes: 256,
            },
        )
        .map_err(error)?;
        holder.append_child(phase.root())?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("privacy fixture body missing"))?
            .append_child(&holder)?;
        let emitted = Rc::new(Cell::new(0));
        let observed = Rc::clone(&emitted);
        phase
            .on_input(move |_| observed.set(observed.get() + 1))
            .map_err(error)?;
        phase
            .draft_input()
            .input()
            .set_value("Audience-owned draft retained across updates");
        phase
            .draft_input()
            .input()
            .dispatch_event(&Event::new("input")?)?;
        let initial = RetainedAudience::capture(&phase)?;
        let portrait = phase
            .root()
            .query_selector(".exploration-portrait")?
            .ok_or_else(|| JsValue::from_str("privacy portrait missing"))?;
        require(
            portrait.get_attribute("alt").as_deref() == Some("Vell"),
            "fixture portrait lacks NPC identity",
        )?;
        phase
            .update(&exploration_view(revision(31, 2)?, Mode::Conversation)?)
            .map_err(error)?;
        require(
            phase.draft_input().draft() == "Audience-owned draft retained across updates",
            "same-owner update erased draft",
        )?;
        let updated = RetainedAudience::capture(&phase)?;
        let mut no_portrait = exploration_view(revision(31, 3)?, Mode::Conversation)?;
        if let Some(npc) = no_portrait.npc.as_mut() {
            npc.portrait = None;
        }
        phase.update(&no_portrait).map_err(error)?;
        require(
            portrait.get_attribute("src").is_none() && portrait.get_attribute("alt").is_none(),
            "no-portrait update retained NPC image identity",
        )?;
        phase
            .update(&exploration_view(revision(31, 4)?, Mode::Conversation)?)
            .map_err(error)?;
        phase
            .update(&exploration_view(revision(31, 5)?, Mode::Hall)?)
            .map_err(error)?;
        require(
            portrait.get_attribute("src").is_none() && portrait.get_attribute("alt").is_none(),
            "no-NPC update retained portrait identity",
        )?;
        // Both ordinary offer removal and epoch retirement erase their actual
        // detached button and Text references before the whole phase ends.
        require(
            initial
                .choice
                .text_content()
                .is_none_or(|text| text.is_empty())
                && initial
                    .choice
                    .get_attribute("data-exploration-offer")
                    .is_none(),
            "removed offered choice retained audience data",
        )?;
        phase
            .update(&exploration_view(revision(32, 0)?, Mode::Conversation)?)
            .map_err(error)?;
        let epoch_choice = RetainedAudience::capture(&phase)?;
        phase
            .update(&exploration_view(revision(33, 0)?, Mode::Conversation)?)
            .map_err(error)?;
        require(
            epoch_choice
                .choice
                .text_content()
                .is_none_or(|text| text.is_empty())
                && epoch_choice
                    .choice
                    .get_attribute("data-exploration-offer")
                    .is_none(),
            "epoch-retired choice retained audience data",
        )?;
        phase.draft_input().input().set_value("Final private draft");
        phase
            .draft_input()
            .input()
            .dispatch_event(&Event::new("input")?)?;
        let final_nodes = RetainedAudience::capture(&phase)?;
        if explicit {
            phase.dispose().map_err(error)?;
            phase.dispose().map_err(error)?;
            require(
                phase
                    .update(&exploration_view(revision(33, 1)?, Mode::Conversation)?)
                    .is_err(),
                "disposed phase admitted update",
            )?;
            require(
                phase.set_visible(true).is_err(),
                "disposed phase admitted visibility change",
            )?;
            require(
                phase.on_input(|_| {}).is_err(),
                "disposed phase admitted callback",
            )?;
            let before = emitted.get();
            for retained in [&initial, &updated, &epoch_choice, &final_nodes] {
                retained.require_erased()?;
                retained.choice.dispatch_event(&Event::new("click")?)?;
            }
            require(
                emitted.get() == before,
                "retained choice emitted after explicit disposal",
            )?;
        }
        drop(phase);
        let before = emitted.get();
        for retained in [&initial, &updated, &epoch_choice, &final_nodes] {
            retained.require_erased()?;
            retained.choice.dispatch_event(&Event::new("click")?)?;
        }
        require(
            emitted.get() == before,
            "retained choice emitted after teardown",
        )?;
        holder.remove();
        Ok(())
    }
    fn check(fixture: &Fixture) -> Result<(), JsValue> {
        teardown_privacy(&fixture.document, true)?;
        teardown_privacy(&fixture.document, false)?;
        advance(fixture, Mode::Conversation)?;
        let input = fixture.phase.draft_input().input().clone();
        input.set_value("My retained draft <b>literal</b>");
        input.dispatch_event(&Event::new("input")?)?;
        input.focus()?;
        input.set_selection_range(3, 8)?;
        let root = fixture.phase.root().clone();
        let choice = root
            .query_selector("[data-exploration-offer]")?
            .ok_or_else(|| JsValue::from_str("missing choice"))?;
        for _ in 0..64 {
            advance(fixture, Mode::Conversation)?;
        }
        require(
            root.is_same_node(Some(fixture.phase.root())),
            "surface remounted",
        )?;
        require(
            fixture
                .document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&input))),
            "focused draft lost",
        )?;
        require(
            input.selection_start()? == Some(3) && input.selection_end()? == Some(8),
            "draft selection lost",
        )?;
        require(
            fixture.phase.draft_input().draft() == "My retained draft <b>literal</b>",
            "draft lost",
        )?;
        require(
            root.query_selector("[data-exploration-offer]")?
                .is_some_and(|current| current.is_same_node(Some(&choice))),
            "choice remounted",
        )?;
        let count_before = fixture.count.get();
        choice.dispatch_event(&Event::new("click")?)?;
        require(
            fixture.count.get() == count_before + 1,
            "listener emitted zero or multiple inputs",
        )?;
        require(input.read_only(), "pending draft remained editable")?;
        advance(fixture, Mode::Refused)?;
        require(
            !input.read_only()
                && fixture.phase.draft_input().draft() == "My retained draft <b>literal</b>",
            "refusal lost draft",
        )?;
        let current = fixture.revision.get();
        require(
            fixture
                .phase
                .update(&exploration_view(current, Mode::Hall)?)
                .is_err(),
            "duplicate view admitted",
        )?;
        let old_choice = choice.clone();
        let newer = revision(current.epoch().get() + 1, 0)?;
        fixture
            .phase
            .update(&exploration_view(newer, Mode::Conversation)?)
            .map_err(error)?;
        fixture.revision.set(newer);
        fixture.mode.set(Mode::Conversation);
        require(
            fixture.phase.draft_input().draft().is_empty(),
            "generation retained stale draft",
        )?;
        let before = fixture.count.get();
        old_choice.dispatch_event(&Event::new("click")?)?;
        require(
            fixture.count.get() == before,
            "retired generation emitted input",
        )?;
        fixture.phase.set_visible(false).map_err(error)?;
        if let Some(current_choice) = root.query_selector("[data-exploration-offer]")? {
            current_choice.dispatch_event(&Event::new("click")?)?;
        }
        require(fixture.count.get() == before, "hidden scope emitted input")?;
        fixture.phase.set_visible(true).map_err(error)?;
        advance(fixture, Mode::Conversation)?;
        let art = root
            .query_selector(".scene-art")?
            .ok_or_else(|| JsValue::from_str("missing artwork"))?;
        art.dispatch_event(&Event::new("error")?)?;
        require(
            root.class_name().contains("exploration-art-failed"),
            "asset failure fallback missing",
        )?;
        advance(fixture, Mode::Hall)?;
        require(
            !root.class_name().contains("exploration-art-failed"),
            "new scene retained prior failure",
        )?;
        require(
            root.query_selector(".exploration-actions")?.is_some(),
            "fallback removed input",
        )?;
        advance(fixture, Mode::Conversation)?;
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .document()
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("no body"))?;
        let initial = revision(1, 1)?;
        let phase = Rc::new(
            ExplorationPhase::create(
                &document,
                "exploration-action-draft",
                &exploration_view(initial, Mode::Conversation)?,
                ExplorationLimits {
                    campaign: CampaignLimits {
                        max_members: 16,
                        max_objectives: 16,
                        max_text_bytes: 4096,
                    },
                    max_choices: 16,
                    max_text_bytes: 4096,
                    max_identifier_bytes: 256,
                },
            )
            .map_err(error)?,
        );
        let checks = child(
            &document,
            phase.root(),
            "details",
            "exploration-checks",
            None,
        )?;
        child(
            &document,
            &checks,
            "summary",
            "",
            Some("Synthetic view controller & presentation checks"),
        )?;
        let status = child(
            &document,
            &checks,
            "p",
            "exploration-check-status",
            Some(
                "Clicking an offered choice emits input. Only this explicitly synthetic controller applies fixture views.",
            ),
        )?;
        status.set_attribute("role", "status")?;
        let buttons = child(
            &document,
            &checks,
            "div",
            "exploration-fixture-controls",
            None,
        )?;
        let mut controls = Vec::new();
        for (label, operation) in [
            ("Apply conversation view", 0),
            ("Apply exploration view", 1),
            ("Apply pending view", 2),
            ("Apply refusal view", 3),
            ("Replace generation", 4),
            ("Verify mounted lifecycle", 5),
            ("Exercise art fallback", 6),
            ("Dispose fixture", 7),
        ] {
            let action = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            action.on_activate(move || {
                if operation == 7 {
                    if let Err(e) = shutdown()
                        && let Some(document) = web_sys::window().and_then(|window| window.document())
                        && let Some(body) = document.body()
                    {
                        body.set_text_content(Some(&format!(
                            "Fixture disposal failed: {}", e.as_string().unwrap_or_default()
                        )));
                    }
                    return;
                }
                FIXTURE.with(|slot| {
                    if let Some(fixture) = slot.borrow().as_ref() {
                        let result = match operation {
                            0 => advance(fixture, Mode::Conversation), 1 => advance(fixture, Mode::Hall), 2 => advance(fixture, Mode::Pending), 3 => advance(fixture, Mode::Refused),
                            4 => (|| {
                                let next = revision(fixture.revision.get().epoch().get() + 1, 0)?;
                                fixture
                                    .phase
                                    .update(&exploration_view(next, Mode::Conversation)?)
                                    .map_err(error)?;
                                fixture.revision.set(next);
                                fixture.mode.set(Mode::Conversation);
                                Ok(())
                            })(),
                            5 => check(fixture),
                            _ => (|| { if let Some(art) = fixture.phase.root().query_selector(".scene-art")? { art.set_attribute("src", "assets/concept-art/fixture-missing-art.webp")?; } Ok(()) })(),
                        };
                        fixture.status.set_text_content(Some(if result.is_ok() { if operation == 5 { "PASS · 64 updates retained focus, selection, draft and nodes; pending/refusal, stale generation, visibility and asset fallback verified." } else { "Synthetic server view applied by fixture controller." } } else { "FAIL · Fixture presentation check or update failed." }));
                    }
                });
            }).map_err(error)?;
            buttons.append_child(action.element())?;
            controls.push(action);
        }
        phase.on_input(move |input| {
            FIXTURE.with(|slot| {
                if let Some(fixture) = slot.borrow().as_ref() {
                    fixture.count.set(fixture.count.get() + 1);
                    let id = match &input { ExplorationInput::Choice { id, .. } | ExplorationInput::Draft { id, .. } => id };
                    fixture.status.set_text_content(Some(&format!("Input {} emitted exactly once: {id}. Synthetic controller will now apply a pending view.", fixture.count.get())));
                    fixture.phase.root().set_attribute("data-fixture-input-count", &fixture.count.get().to_string()).and_then(|_| advance(fixture, Mode::Pending)).unwrap_or_else(|_| fixture.status.set_text_content(Some("FAIL · Synthetic controller could not apply pending view.")));
                }
            });
        }).map_err(error)?;
        body.append_child(phase.root())?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                phase,
                document,
                revision: Cell::new(initial),
                mode: Cell::new(Mode::Conversation),
                controls,
                count: Cell::new(0),
                status,
            })
        });
        Ok(())
    }
    #[wasm_bindgen]
    pub fn verify() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let owned = slot.borrow();
            let fixture = owned
                .as_ref()
                .ok_or_else(|| JsValue::from_str("disposed fixture"))?;
            check(fixture)
        })
    }
    #[wasm_bindgen]
    pub fn verify_teardown_privacy() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        teardown_privacy(&document, true)?;
        teardown_privacy(&document, false)
    }
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let fixture = slot.borrow_mut().take();
            if let Some(fixture) = fixture {
                let old = fixture.phase.root().query_selector("[data-exploration-offer]")?;
                for control in &fixture.controls { control.dispose().map_err(error)?; }
                fixture.phase.dispose().map_err(error)?; fixture.phase.dispose().map_err(error)?;
                if let Some(old) = old { old.dispatch_event(&Event::new("click")?)?; }
                require(
                    fixture
                        .phase
                        .update(&exploration_view(fixture.revision.get(), Mode::Conversation)?)
                        .is_err(),
                    "disposed phase admitted update",
                )?;
                require(fixture.phase.draft_input().draft().is_empty(), "disposed draft retained text")?;
                if let Some(body) = fixture.document.body() { body.set_text_content(Some("PASS · Exploration fixture disposed twice; draft cleared, listeners retired and late input fenced.")); }
            }
            Ok(())
        })
    }
}
