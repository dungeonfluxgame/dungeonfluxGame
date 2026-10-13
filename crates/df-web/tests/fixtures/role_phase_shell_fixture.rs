#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::ViewAcceptance;
    use df_display::DisplayJoinInput;
    use df_player::PlayerJoinInput;
    use df_types::{ClientBindingId, RecoveryEpoch, RevisionLabel, SessionRevision};
    use df_ui::{
        CampaignLimits, CampaignView, CharacterAction, CharacterActionKind,
        CharacterDisplayConnection, CharacterDisplayHostOffer, CharacterDisplayLimits,
        CharacterDisplayView, CharacterLimits, CharacterPhaseView, CharacterPublicReadiness,
        CharacterSheetView, CharacterStatus, CombatActor, CombatArt, CombatDraft, CombatFeedback,
        CombatLimits, CombatOffer, CombatOfferKind, CombatPhaseView, CombatRoll, ConceptScene,
        ExplorationChoice, ExplorationLimits, ExplorationView, FeedbackView, JoinFeedback,
        JoinPhaseIntent, JoinPhaseView, JoinStage, JoinStamp, SessionConnection,
        SessionOverlayOffer, SessionOverlayView, SheetLabels, SheetOffer, SheetOwnerGeneration,
        SheetRow, SheetSection, SheetTab,
    };
    use df_web::{DisplayPhase, PlayerPhase, RoleInput, RolePhase, RoleShell, RoleShellError};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlInputElement, Node};

    const PRIVATE: &str = "PRIVATE_SHELL_SENTINEL";
    const PUBLIC: &str = "A public party gathers beneath the lanterns.";
    struct Receipt {
        count: Cell<u64>,
        last: RefCell<Option<RoleInput>>,
    }
    struct AppendFailureProbe {
        retained: Vec<Node>,
        button: Element,
        count: u64,
        failed: bool,
    }
    struct Fixture {
        document: Document,
        root: Element,
        player: RoleShell,
        display: RoleShell,
        player_binding: ClientBindingId,
        display_binding: ClientBindingId,
        receipt: Rc<Receipt>,
        sequence: u64,
        epoch: u64,
        phase: u32,
        overlay: bool,
        host_offer: bool,
        append_failure: Option<AppendFailureProbe>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
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
    fn binding(byte: u8) -> Result<ClientBindingId, JsValue> {
        ClientBindingId::from_bytes(&[byte; 16])
            .map_err(|failure| JsValue::from_str(&format!("fixture binding: {failure:?}")))
    }
    fn revision(epoch: u64, sequence: u64) -> Result<SessionRevision, JsValue> {
        Ok(SessionRevision::new(
            RecoveryEpoch::new(epoch)
                .map_err(|failure| JsValue::from_str(&format!("fixture epoch: {failure:?}")))?,
            sequence,
        ))
    }
    fn campaign(public: bool) -> CampaignView<'static> {
        CampaignView {
            scene: ConceptScene::Tavern,
            chapter: "Chapter one · The Drowned Lantern",
            title: "The Drowned Lantern",
            description: "A warm tavern shelters the company from the rain.",
            location: "Greyhaven · Harbor quarter",
            scene_label: "The party gathers",
            narration: if public { PUBLIC } else { PRIVATE },
            connection: "Controlled supplied view",
            notice: "Two persistent role shells · Production RPC pending",
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
    fn join(public: bool, epoch: u64, sequence: u64) -> JoinPhaseView<'static> {
        JoinPhaseView {
            stamp: JoinStamp {
                generation: epoch,
                sequence,
            },
            stage: JoinStage::Invitation,
            campaign: campaign(public),
            panel_heading: "Step into the story",
            panel_description: "The server confirms admission; local input is only a proposal.",
            invitation_label: "Invitation",
            name_label: "Your name",
            join_label: "Request to join",
            join_enabled: !public,
            feedback: JoinFeedback::None,
            room: None,
            roster_heading: "Your company",
            empty_roster: "The company is gathering",
            participants: &[],
            away_heroes: &[],
            lobby_offer: None,
            pairing_fallback: "Use the supplied invitation",
        }
    }
    fn character(epoch: u64, sequence: u64) -> CharacterPhaseView {
        CharacterPhaseView {
            generation: epoch,
            owner_key: "player-one".into(),
            revision: sequence,
            chapter: "Chapter one · Your hero".into(),
            title: "Every story needs its heroes".into(),
            description: PRIVATE.into(),
            connection: "Controlled player projection".into(),
            status: CharacterStatus::Editing,
            status_message: "Draft awaiting server validation".into(),
            editable: true,
            name: "Mara".into(),
            flavor: PRIVATE.into(),
            appearance: Some(df_ui::CharacterAppearanceDraft {
                features: PRIVATE.into(),
                outfit: "A weathered green travel cloak".into(),
            }),
            portrait: None,
            groups: vec![],
            facts: vec![],
            actions: vec![CharacterAction {
                id: "exact-character-submit".into(),
                kind: CharacterActionKind::SubmitDraft,
                label: "Send supplied draft".into(),
                enabled: true,
            }],
        }
    }
    fn display_character(epoch: u64, sequence: u64, host: bool) -> CharacterDisplayView {
        CharacterDisplayView {
            generation: epoch,
            public_scope_key: "public-room".into(),
            revision: sequence,
            chapter: "Chapter one · Shared company".into(),
            title: "Every story needs its heroes".into(),
            description: PUBLIC.into(),
            readiness: CharacterPublicReadiness::Choosing,
            progress_label: "Supplied party readiness".into(),
            public_notice: "Public projection only".into(),
            connection: CharacterDisplayConnection::Connected,
            connection_label: "Controlled display projection".into(),
            members: vec![],
            host_offers: if host {
                vec![CharacterDisplayHostOffer {
                    id: "exact-display-host".into(),
                    label: "Advertised host selection".into(),
                    enabled: true,
                    pending: false,
                }]
            } else {
                vec![]
            },
        }
    }
    fn character_limits() -> CharacterLimits {
        CharacterLimits {
            max_groups: 8,
            max_options: 16,
            max_facts: 16,
            max_actions: 8,
            max_text_bytes: 4096,
        }
    }
    fn display_limits() -> CharacterDisplayLimits {
        CharacterDisplayLimits {
            max_members: 8,
            max_host_offers: 4,
            max_text_bytes: 4096,
        }
    }
    fn exploration(
        public: bool,
        binding: ClientBindingId,
        revision: SessionRevision,
    ) -> ExplorationView<'static> {
        const CHOICES: [ExplorationChoice<'static>; 1] = [ExplorationChoice {
            id: "exact-explore-choice",
            label: "Ask about the harbor",
            detail: "Supplied choice",
            disabled_reason: None,
        }];
        ExplorationView {
            binding,
            revision,
            campaign: campaign(public),
            npc: None,
            heading: "The harbor's stories",
            choices: if public { &[] } else { &CHOICES },
            draft_label: "What would you like to ask?",
            submit_label: "Send supplied question",
            draft_offer_id: if public {
                None
            } else {
                Some("exact-explore-draft")
            },
            pending: None,
            rejection: None,
        }
    }
    fn explore_limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: campaign_limits(),
            max_choices: 8,
            max_text_bytes: 4096,
            max_identifier_bytes: 128,
        }
    }
    fn combat_limits() -> CombatLimits {
        CombatLimits {
            max_actors: 8,
            max_offers: 8,
            max_rolls: 8,
            max_resources_per_actor: 8,
            max_text_bytes: 4096,
            max_total_text_bytes: 32768,
        }
    }
    fn combat<'a>(public: bool, offers: &'a [CombatOffer<'a>]) -> CombatPhaseView<'a> {
        const PRIVATE_ACTORS: [CombatActor<'static>; 1] = [CombatActor {
            key: "mara",
            name: "Mara",
            initiative: "First in supplied order",
            status: PRIVATE,
            resources: &[],
            hit_points: None,
            portrait: None,
        }];
        const PUBLIC_ACTORS: [CombatActor<'static>; 1] = [CombatActor {
            key: "mara",
            name: "Mara",
            initiative: "First in supplied order",
            status: "Party member",
            resources: &[],
            hit_points: None,
            portrait: None,
        }];
        const ROLLS: [CombatRoll<'static>; 1] = [CombatRoll {
            key: "roll-one",
            label: "Committed result",
            value: "17",
            explanation: "13 + 4 · supplied",
            source: "Controlled supplied receipt",
            verdict: None,
        }];
        CombatPhaseView {
            art: CombatArt::Harbor,
            chapter: "Chapter one · The harbor confrontation",
            title: "Lanterns on the rain",
            location: "Greyhaven docks",
            narration: if public { PUBLIC } else { PRIVATE },
            connection: "Controlled supplied combat",
            notice: "Game outcomes remain server-owned",
            turn_label: "Supplied current turn",
            active_actor: Some("mara"),
            actors: if public {
                &PUBLIC_ACTORS
            } else {
                &PRIVATE_ACTORS
            },
            action_heading: "Available proposals",
            action_empty: "Awaiting a permitted offer",
            offers,
            roll_heading: "Committed rolls",
            roll_empty: "No committed rolls",
            rolls: &ROLLS,
            reaction: None,
            draft: if public {
                None
            } else {
                Some(CombatDraft {
                    key: "private-intent",
                    label: "Your intent",
                    enabled: true,
                    feedback: CombatFeedback::None,
                })
            },
            feedback: CombatFeedback::None,
        }
    }
    fn overlay(public: bool, epoch: u64, sequence: u64, host: bool) -> SessionOverlayView<'static> {
        const OFFERS: [SessionOverlayOffer<'static>; 1] = [SessionOverlayOffer {
            key: "exact-host-offer",
            label: "Advertised host selection",
            enabled: true,
            pending: false,
        }];
        SessionOverlayView {
            generation: epoch,
            revision: sequence,
            title: "The Drowned Lantern",
            subtitle: if public { "Shared display" } else { PRIVATE },
            connection: SessionConnection::Connected,
            connection_label: "Connected",
            connection_detail: "Current supplied view",
            operation: None,
            bookend: None,
            host_context: "Only current advertised host offers are presented",
            host_offers: if host { &OFFERS } else { &[] },
            reduced_motion: false,
        }
    }

    impl Fixture {
        fn show(
            &mut self,
            phase: u32,
            epoch: u64,
            with_overlay: bool,
            host: bool,
        ) -> Result<(), JsValue> {
            require(phase <= 4, "unknown fixture phase")?;
            let sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
            let revision = revision(epoch, sequence)?;
            let private_overlay = overlay(false, epoch, sequence, host);
            let public_overlay = overlay(true, epoch, sequence, host);
            let old_player = self.player.root().clone();
            let old_display = self.display.root().clone();
            let (player_result, display_result) = match phase {
                0 => {
                    let player = join(false, epoch, sequence);
                    let display = join(true, epoch, sequence);
                    (
                        self.player.present(
                            self.player_binding,
                            revision,
                            RolePhase::Player(PlayerPhase::Join {
                                view: &player,
                                input: PlayerJoinInput {
                                    identifier: "shell-player-join",
                                    invitation: "",
                                    player_name: "",
                                },
                                limits: campaign_limits(),
                            }),
                            with_overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            revision,
                            RolePhase::Display(DisplayPhase::Join {
                                view: &display,
                                input: DisplayJoinInput {
                                    identifier: "shell-display-join",
                                    invitation: "",
                                    player_name: "",
                                },
                                limits: campaign_limits(),
                            }),
                            with_overlay.then_some(&public_overlay),
                        ),
                    )
                }
                1 => {
                    let player = character(epoch, sequence);
                    let display = display_character(epoch, sequence, host);
                    (
                        self.player.present(
                            self.player_binding,
                            revision,
                            RolePhase::Player(PlayerPhase::Character {
                                view: &player,
                                limits: character_limits(),
                            }),
                            with_overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            revision,
                            RolePhase::Display(DisplayPhase::Character {
                                view: &display,
                                limits: display_limits(),
                            }),
                            with_overlay.then_some(&public_overlay),
                        ),
                    )
                }
                2 => {
                    let player = exploration(false, self.player_binding, revision);
                    let display = exploration(true, self.display_binding, revision);
                    (
                        self.player.present(
                            self.player_binding,
                            revision,
                            RolePhase::Player(PlayerPhase::Exploration {
                                view: &player,
                                limits: explore_limits(),
                            }),
                            with_overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            revision,
                            RolePhase::Display(DisplayPhase::Exploration {
                                view: &display,
                                limits: explore_limits(),
                            }),
                            with_overlay.then_some(&public_overlay),
                        ),
                    )
                }
                3 | 4 => {
                    let offer =
                        RevisionLabel::new(Some("exact-combat-intent")).map_err(|failure| {
                            JsValue::from_str(&format!("fixture label: {failure:?}"))
                        })?;
                    let offers = [CombatOffer {
                        key: "action-one",
                        offer: &offer,
                        option: None,
                        kind: CombatOfferKind::Action,
                        label: "Send supplied intent",
                        explanation: "Server validates the exact proposal",
                        enabled: true,
                        pending: false,
                    }];
                    let private_combat = combat(false, &offers);
                    let public_combat = combat(true, &[]);
                    let player = if phase == 3 {
                        self.player.present(
                            self.player_binding,
                            revision,
                            RolePhase::Player(PlayerPhase::Combat {
                                view: &private_combat,
                                limits: combat_limits(),
                            }),
                            with_overlay.then_some(&private_overlay),
                        )
                    } else {
                        let offers = [SheetOffer {
                            id: "exact-sheet-offer",
                            label: "Advertised equipment selection",
                            enabled: true,
                            pending: false,
                        }];
                        let rows = [SheetRow {
                            key: "lantern",
                            title: "Storm lantern",
                            value: "Supplied inventory",
                            summary: PRIVATE,
                            details: &[],
                            offers: &offers,
                        }];
                        let sections = [SheetSection {
                            key: "equipment",
                            tab: SheetTab::Equipment,
                            title: "Equipment",
                            caption: "Server-provided inventory",
                            rows: &rows,
                        }];
                        let sheet = CharacterSheetView {
                            owner: SheetOwnerGeneration(epoch),
                            revision: sequence,
                            name: "Mara",
                            identity: "Supplied character",
                            subtitle: PRIVATE,
                            connection: "Current supplied view",
                            notice: "No local rules calculations",
                            labels: SheetLabels {
                                tabs: [
                                    "Character",
                                    "Equipment",
                                    "Spells",
                                    "Journal",
                                    "Progression",
                                ],
                                navigation: "Personal character navigation",
                                filter: "Search your sheet",
                                no_matches: "No matching supplied rows",
                                art_fallback: "Portrait unavailable",
                            },
                            sections: &sections,
                        };
                        self.player.present(
                            self.player_binding,
                            revision,
                            RolePhase::Player(PlayerPhase::Sheet(&sheet)),
                            with_overlay.then_some(&private_overlay),
                        )
                    };
                    (
                        player,
                        self.display.present(
                            self.display_binding,
                            revision,
                            RolePhase::Display(DisplayPhase::Combat {
                                view: &public_combat,
                                limits: combat_limits(),
                            }),
                            with_overlay.then_some(&public_overlay),
                        ),
                    )
                }
                _ => return Err(JsValue::from_str("unknown fixture phase")),
            };
            require(
                player_result.map_err(error)? == ViewAcceptance::Applied,
                "player current view not applied",
            )?;
            require(
                display_result.map_err(error)? == ViewAcceptance::Applied,
                "display current view not applied",
            )?;
            require(
                old_player.is_same_node(Some(self.player.root())),
                "player shell was recreated",
            )?;
            require(
                old_display.is_same_node(Some(self.display.root())),
                "display shell was recreated",
            )?;
            require(
                !self
                    .display
                    .root()
                    .text_content()
                    .unwrap_or_default()
                    .contains(PRIVATE),
                "private content reached public shell",
            )?;
            self.sequence = sequence;
            self.epoch = epoch;
            self.phase = phase;
            self.overlay = with_overlay;
            self.host_offer = host;
            self.root
                .set_attribute("data-shell-phase", &phase.to_string())?;
            self.root
                .set_attribute("data-shell-sequence", &sequence.to_string())?;
            Ok(())
        }
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("fixture document missing"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("fixture body missing"))?;
        let root = document.create_element("div")?;
        root.set_attribute("data-role-shell-fixture", "controlled-presentation-only")?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some("DungeonFlux · Two persistent role shells"));
        root.append_child(&heading)?;
        let disclosure = document.create_element("p")?;
        disclosure.set_text_content(Some("Controlled supplied presentation · Production auth, RPC, audio and telemetry remain pending"));
        root.append_child(&disclosure)?;
        let player_slot = document.create_element("section")?;
        player_slot.set_attribute("data-shell-player", "")?;
        let display_slot = document.create_element("section")?;
        display_slot.set_attribute("data-shell-display", "")?;
        root.append_child(&player_slot)?;
        root.append_child(&display_slot)?;
        body.append_child(&root)?;
        let receipt = Rc::new(Receipt {
            count: Cell::new(0),
            last: RefCell::new(None),
        });
        let player_receipt = Rc::clone(&receipt);
        let display_receipt = Rc::clone(&receipt);
        let player_binding = binding(61)?;
        let display_binding = binding(62)?;
        let player = RoleShell::mount(
            &document,
            &player_slot,
            player_binding,
            "shell-player-draft",
            move |input| {
                player_receipt
                    .count
                    .set(player_receipt.count.get().saturating_add(1));
                *player_receipt.last.borrow_mut() = Some(input);
            },
        )
        .map_err(error)?;
        let display = RoleShell::mount(
            &document,
            &display_slot,
            display_binding,
            "shell-display-draft",
            move |input| {
                display_receipt
                    .count
                    .set(display_receipt.count.get().saturating_add(1));
                *display_receipt.last.borrow_mut() = Some(input);
            },
        )
        .map_err(error)?;
        let mut fixture = Fixture {
            document,
            root,
            player,
            display,
            player_binding,
            display_binding,
            receipt,
            sequence: 0,
            epoch: 1,
            phase: 0,
            overlay: false,
            host_offer: false,
            append_failure: None,
        };
        fixture.show(0, 1, false, false)?;
        FIXTURE.with(|owner| *owner.borrow_mut() = Some(fixture));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn shell_show(
        phase: u32,
        epoch: u64,
        with_overlay: bool,
        host: bool,
    ) -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            owner
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?
                .show(phase, epoch, with_overlay, host)
        })
    }
    #[wasm_bindgen]
    pub fn shell_suspend() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            fixture
                .player
                .suspend(SessionConnection::Reconnecting)
                .map_err(error)?;
            fixture
                .display
                .suspend(SessionConnection::Reconnecting)
                .map_err(error)
        })
    }
    #[wasm_bindgen]
    pub fn shell_receipt_count() -> Result<u64, JsValue> {
        FIXTURE.with(|owner| {
            Ok(owner
                .borrow()
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?
                .receipt
                .count
                .get())
        })
    }
    #[wasm_bindgen]
    pub fn shell_last_selection() -> Result<String, JsValue> {
        FIXTURE.with(|owner| {
            let owner = owner.borrow();
            let fixture = owner
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let last = fixture.receipt.last.borrow();
            Ok(match last.as_ref() {
                Some(RoleInput::PlayerJoin(_)) => "player-join".into(),
                Some(RoleInput::DisplayJoin(_)) => "display-join".into(),
                Some(RoleInput::PlayerCharacter(input)) => {
                    format!("player-character:{}", input.action_id)
                }
                Some(RoleInput::DisplayCharacter(input)) => {
                    format!("display-character:{}", input.host_offer_id)
                }
                Some(RoleInput::Sheet(input)) => format!("sheet:{}", input.offer_id),
                Some(RoleInput::PlayerExploration(input)) => {
                    format!("player-exploration:{input:?}")
                }
                Some(RoleInput::DisplayExploration(input)) => {
                    format!("display-exploration:{input:?}")
                }
                Some(RoleInput::PlayerCombat(input)) => {
                    format!("player-combat:{}", input.offer.as_str())
                }
                Some(RoleInput::DisplayCombat(input)) => {
                    format!("display-combat:{}", input.offer.as_str())
                }
                Some(RoleInput::PlayerAftermath(input)) => {
                    format!("player-aftermath:{}", input.key)
                }
                Some(RoleInput::DisplayAftermath(input)) => {
                    format!("display-aftermath:{}", input.key)
                }
                Some(RoleInput::PlayerCampfire(input)) => {
                    format!("player-campfire:{}", input.offer_id)
                }
                Some(RoleInput::DisplayCampfire(input)) => {
                    format!("display-campfire:{}", input.offer_id)
                }
                Some(RoleInput::PlayerTransition(_)) => "player-transition".into(),
                Some(RoleInput::DisplayTransition(_)) => "display-transition".into(),
                Some(RoleInput::PlayerOverlay(input)) => format!("player-overlay:{}", input.key),
                Some(RoleInput::DisplayOverlay(input)) => format!("display-overlay:{}", input.key),
                None => "none".into(),
            })
        })
    }
    fn private_nodes(root: &Element) -> Result<Vec<Node>, JsValue> {
        let mut selected = Vec::new();
        let mut pending = vec![Node::from(root.clone())];
        let mut visited = 0usize;
        while let Some(node) = pending.pop() {
            visited += 1;
            require(visited <= 4096, "fixture traversal bound exceeded")?;
            if node.node_value().is_some_and(|text| text.contains(PRIVATE))
                || node
                    .dyn_ref::<HtmlInputElement>()
                    .is_some_and(|input| input.value().contains(PRIVATE))
            {
                selected.push(node.clone());
            }
            let mut next = node.first_child();
            while let Some(child) = next {
                next = child.next_sibling();
                pending.push(child);
            }
        }
        Ok(selected)
    }
    fn require_erased(nodes: &[Node]) -> Result<(), JsValue> {
        for node in nodes {
            require(
                !node.node_value().is_some_and(|text| text.contains(PRIVATE)),
                "retained private raw Text survived retirement",
            )?;
            if let Some(input) = node.dyn_ref::<HtmlInputElement>() {
                require(
                    !input.value().contains(PRIVATE),
                    "retained private draft survived retirement",
                )?;
            }
        }
        Ok(())
    }
    #[wasm_bindgen]
    pub fn shell_check_ordering() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let current = revision(fixture.epoch, fixture.sequence)?;
            let view = character(fixture.epoch, fixture.sequence);
            let root = fixture.player.root().clone();
            let content = root.text_content();
            let duplicate = fixture
                .player
                .present(
                    fixture.player_binding,
                    current,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            require(
                duplicate == ViewAcceptance::Duplicate { current },
                "duplicate changed shell presentation",
            )?;
            let stale = fixture
                .player
                .present(
                    fixture.player_binding,
                    revision(fixture.epoch, fixture.sequence.saturating_sub(1))?,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            require(
                stale == ViewAcceptance::Stale { current },
                "stale snapshot changed shell presentation",
            )?;
            let wrong = fixture
                .player
                .present(
                    fixture.display_binding,
                    revision(
                        fixture.epoch,
                        fixture
                            .sequence
                            .checked_add(1)
                            .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
                    )?,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            require(
                wrong == ViewAcceptance::WrongBinding,
                "wrong binding entered private shell",
            )?;
            require(
                root.text_content() == content && root.is_same_node(Some(fixture.player.root())),
                "rejected snapshot mutated shell",
            )?;
            fixture.root.set_attribute("data-shell-ordering", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_same_phase() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let selector = match fixture.phase {
                0 => "[data-join-client]",
                1 => "[data-character-client]",
                2 => ".df-exploration",
                3 => "[data-combat-client]",
                4 => "[data-player-client]",
                _ => return Err(JsValue::from_str("unknown fixture phase")),
            };
            let root = fixture
                .player
                .root()
                .query_selector(selector)?
                .ok_or_else(|| JsValue::from_str("fixture phase root missing"))?;
            let input = root
                .query_selector("input")?
                .map(|input| input.dyn_into::<HtmlInputElement>())
                .transpose()
                .map_err(|_| JsValue::from_str("fixture input unavailable"))?;
            if let Some(input) = &input {
                input.set_value(PRIVATE);
                input.dispatch_event(&Event::new("input")?)?;
                input.focus()?;
                input.set_selection_range(3, 7)?;
            }
            let phase = fixture.phase;
            let epoch = fixture.epoch;
            let overlay = fixture.overlay;
            let host = fixture.host_offer;
            for _ in 0..64 {
                fixture.show(phase, epoch, overlay, host)?;
            }
            let current = fixture
                .player
                .root()
                .query_selector(selector)?
                .ok_or_else(|| JsValue::from_str("fixture current phase root missing"))?;
            require(
                root.is_same_node(Some(&current)),
                "ordinary updates remounted current role surface",
            )?;
            if let Some(input) = input {
                require(
                    input.value() == PRIVATE,
                    "same-phase updates lost current draft or filter",
                )?;
                require(
                    fixture
                        .document
                        .active_element()
                        .is_some_and(|active| active.is_same_node(Some(&input))),
                    "same-phase updates lost valid focus",
                )?;
                require(
                    input.selection_start()? == Some(3) && input.selection_end()? == Some(7),
                    "same-phase updates lost current selection",
                )?;
            }
            fixture
                .root
                .set_attribute("data-shell-same-phase", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_scope_cleanup() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let retained = private_nodes(fixture.player.root())?;
            require(!retained.is_empty(), "fixture lacks private retained nodes")?;
            let button = fixture.player.root().query_selector("button")?;
            let count = fixture.receipt.count.get();
            let next_phase = (fixture.phase + 1) % 5;
            fixture.show(next_phase, fixture.epoch, fixture.overlay, false)?;
            require_erased(&retained)?;
            if let Some(button) = button {
                button.dispatch_event(&Event::new("click")?)?;
            }
            require(
                fixture.receipt.count.get() == count,
                "retired phase callback remained active",
            )?;
            fixture
                .root
                .set_attribute("data-shell-scope-cleanup", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_failed_replacement() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let retained = private_nodes(fixture.player.root())?;
            let old_button = fixture.player.root().query_selector("button")?;
            let count = fixture.receipt.count.get();
            let mut invalid = character(
                fixture
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture epoch exhausted"))?,
                fixture
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
            );
            invalid.title.clear();
            let result = fixture.player.present(
                fixture.player_binding,
                revision(
                    fixture.epoch,
                    fixture
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
                )?,
                RolePhase::Player(PlayerPhase::Character {
                    view: &invalid,
                    limits: character_limits(),
                }),
                None,
            );
            require(result.is_err(), "invalid replaced scope mounted")?;
            require_erased(&retained)?;
            if let Some(button) = old_button {
                button.dispatch_event(&Event::new("click")?)?;
            }
            require(
                fixture.receipt.count.get() == count,
                "failed replacement revived old callback",
            )?;
            require(
                fixture
                    .player
                    .root()
                    .text_content()
                    .is_some_and(|text| text.contains("Presentation unavailable")),
                "failed replacement lacked visible error",
            )?;
            fixture.show(
                fixture.phase,
                fixture.epoch,
                fixture.overlay,
                fixture.host_offer,
            )?;
            fixture
                .root
                .set_attribute("data-shell-failed-replacement", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_epoch_cleanup() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let retained = private_nodes(fixture.player.root())?;
            fixture.show(
                fixture.phase,
                fixture
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture epoch exhausted"))?,
                fixture.overlay,
                fixture.host_offer,
            )?;
            require_erased(&retained)?;
            fixture
                .root
                .set_attribute("data-shell-epoch-cleanup", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_reentrant_dispose() -> Result<(), JsValue> {
        FIXTURE.with(|fixture_owner| {
            let (document, parent) = {
                let owner = fixture_owner.borrow();
                let fixture = owner
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
                (fixture.document.clone(), fixture.root.clone())
            };
            let slot = document.create_element("section")?;
            parent.append_child(&slot)?;
            let holder: Rc<RefCell<Option<RoleShell>>> = Rc::new(RefCell::new(None));
            let weak = Rc::downgrade(&holder);
            let observed = Rc::new(Cell::new(false));
            let callback_observed = Rc::clone(&observed);
            let scope_binding = binding(63)?;
            let shell = RoleShell::mount(
                &document,
                &slot,
                scope_binding,
                "callback-dispose-draft",
                move |_| {
                    if let Some(holder) = weak.upgrade()
                        && let Some(shell) = holder.borrow_mut().as_mut()
                    {
                        callback_observed.set(shell.dispose().is_ok());
                    }
                },
            )
            .map_err(error)?;
            *holder.borrow_mut() = Some(shell);
            let view = character(1, 1);
            holder
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| JsValue::from_str("probe owner missing"))?
                .present(
                    scope_binding,
                    revision(1, 1)?,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            let (button, retained) = {
                let owner = holder.borrow();
                let shell = owner
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("probe owner missing"))?;
                (
                    shell
                        .root()
                        .query_selector("[data-action-id='exact-character-submit']")?
                        .ok_or_else(|| JsValue::from_str("probe advertised action missing"))?,
                    private_nodes(shell.root())?,
                )
            };
            button.dispatch_event(&Event::new("click")?)?;
            require(
                observed.get(),
                "current callback synchronous dispose failed",
            )?;
            require_erased(&retained)?;
            let mut owner = holder.borrow_mut();
            let shell = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("probe owner missing"))?;
            require(
                shell
                    .present(
                        scope_binding,
                        revision(1, 2)?,
                        RolePhase::Player(PlayerPhase::Character {
                            view: &view,
                            limits: character_limits(),
                        }),
                        None,
                    )
                    .is_err(),
                "disposed shell remounted",
            )?;
            shell.dispose().map_err(error)?;
            slot.remove();
            parent.set_attribute("data-shell-reentrant-dispose", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_suspended_sheet_invalid() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            fixture.show(4, fixture.epoch, false, false)?;
            fixture
                .player
                .suspend(SessionConnection::Offline)
                .map_err(error)?;
            let filter = fixture
                .player
                .root()
                .query_selector("input")?
                .ok_or_else(|| JsValue::from_str("sheet filter missing"))?;
            let tab = fixture
                .player
                .root()
                .query_selector(".sheet-tabs button")?
                .ok_or_else(|| JsValue::from_str("sheet tab missing"))?;
            require(
                !filter.matches(":disabled")? && !tab.matches(":disabled")?,
                "suspended sheet local inspection unavailable",
            )?;
            let invalid = CharacterSheetView {
                owner: SheetOwnerGeneration(fixture.epoch),
                revision: fixture
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
                name: "",
                identity: "Supplied character",
                subtitle: PRIVATE,
                connection: "Offline",
                notice: "Current supplied inventory",
                labels: SheetLabels {
                    tabs: ["Character", "Equipment", "Spells", "Journal", "Progression"],
                    navigation: "Personal character navigation",
                    filter: "Search your sheet",
                    no_matches: "No matching supplied rows",
                    art_fallback: "Portrait unavailable",
                },
                sections: &[],
            };
            let result = fixture.player.present(
                fixture.player_binding,
                revision(
                    fixture.epoch,
                    fixture
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
                )?,
                RolePhase::Player(PlayerPhase::Sheet(&invalid)),
                None,
            );
            require(result.is_err(), "invalid ordinary sheet view applied")?;
            require(
                !filter.matches(":disabled")? && !tab.matches(":disabled")?,
                "invalid view disabled suspended sheet inspection",
            )?;
            require(
                fixture
                    .player
                    .root()
                    .text_content()
                    .is_some_and(|text| text.contains("Offline")),
                "explicit Offline was labelled Reconnecting",
            )?;
            fixture
                .root
                .set_attribute("data-shell-suspended-sheet-invalid", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_implicit_drop() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let (document, parent) = {
                let owner = owner.borrow();
                let fixture = owner
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
                (fixture.document.clone(), fixture.root.clone())
            };
            let slot = document.create_element("section")?;
            parent.append_child(&slot)?;
            let calls = Rc::new(Cell::new(0u64));
            let callback_calls = Rc::clone(&calls);
            let scope_binding = binding(64)?;
            let mut shell = RoleShell::mount(
                &document,
                &slot,
                scope_binding,
                "implicit-drop-draft",
                move |_| {
                    callback_calls.set(callback_calls.get().saturating_add(1));
                },
            )
            .map_err(error)?;
            let view = character(1, 1);
            shell
                .present(
                    scope_binding,
                    revision(1, 1)?,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            let root = shell.root().clone();
            let retained = private_nodes(&root)?;
            require(
                !retained.is_empty(),
                "implicit-Drop probe lacks private content",
            )?;
            let button = root
                .query_selector("[data-action-id='exact-character-submit']")?
                .ok_or_else(|| JsValue::from_str("implicit-Drop action missing"))?;
            drop(shell);
            require(!root.is_connected(), "implicit Drop left shell mounted")?;
            require_erased(&retained)?;
            button.dispatch_event(&Event::new("click")?)?;
            require(calls.get() == 0, "implicit Drop retained active callback")?;
            slot.remove();
            parent.set_attribute("data-shell-implicit-drop", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_private_owner_replacement() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            fixture.show(1, fixture.epoch, false, false)?;
            let retained = private_nodes(fixture.player.root())?;
            require(
                !retained.is_empty(),
                "private-owner probe lacks private content",
            )?;
            let button = fixture
                .player
                .root()
                .query_selector("[data-action-id='exact-character-submit']")?
                .ok_or_else(|| JsValue::from_str("private-owner action missing"))?;
            let count = fixture.receipt.count.get();
            let sequence = fixture
                .sequence
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
            let mut view = character(fixture.epoch, sequence);
            view.owner_key = "player-two".into();
            let result = fixture
                .player
                .present(
                    fixture.player_binding,
                    revision(fixture.epoch, sequence)?,
                    RolePhase::Player(PlayerPhase::Character {
                        view: &view,
                        limits: character_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            require(
                result == ViewAcceptance::Applied,
                "current same-epoch owner replacement refused",
            )?;
            require_erased(&retained)?;
            button.dispatch_event(&Event::new("click")?)?;
            require(
                fixture.receipt.count.get() == count,
                "replaced private owner retained callback",
            )?;
            fixture.sequence = sequence;
            fixture.phase = 1;
            fixture.overlay = false;
            fixture.host_offer = false;
            fixture
                .root
                .set_attribute("data-shell-private-owner-replacement", "pass")?;
            Ok(())
        })
    }
    /// Prepare retained obsolete handles. The independent browser evaluator must
    /// inject a one-shot appendChild failure ONLY on the marked player slot when
    /// it receives a new data-combat-client=player root, retaining that new root.
    /// The fixture contains no authored JS or production fault policy.
    #[wasm_bindgen]
    pub fn shell_prepare_append_failure() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            require(
                fixture.append_failure.is_none(),
                "append-failure probe already prepared",
            )?;
            fixture.show(1, fixture.epoch, false, false)?;
            let retained = private_nodes(fixture.player.root())?;
            require(
                !retained.is_empty(),
                "append-failure probe lacks private content",
            )?;
            let button = fixture
                .player
                .root()
                .query_selector("[data-action-id='exact-character-submit']")?
                .ok_or_else(|| JsValue::from_str("append-failure old action missing"))?;
            fixture
                .player
                .root()
                .query_selector("fieldset[aria-label='Current game presentation']")?
                .ok_or_else(|| JsValue::from_str("append-failure slot missing"))?
                .set_attribute("data-shell-append-fault", "player")?;
            fixture.append_failure = Some(AppendFailureProbe {
                retained,
                button,
                count: fixture.receipt.count.get(),
                failed: false,
            });
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_try_append_failure() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            require(
                fixture.append_failure.is_some(),
                "append-failure probe not prepared",
            )?;
            let sequence = fixture
                .sequence
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
            let offer = RevisionLabel::new(Some("exact-combat-intent"))
                .map_err(|failure| JsValue::from_str(&format!("fixture label: {failure:?}")))?;
            let offers = [CombatOffer {
                key: "action-one",
                offer: &offer,
                option: None,
                kind: CombatOfferKind::Action,
                label: "Send supplied intent",
                explanation: "Server validates the exact proposal",
                enabled: true,
                pending: false,
            }];
            let view = combat(false, &offers);
            let result = fixture.player.present(
                fixture.player_binding,
                revision(fixture.epoch, sequence)?,
                RolePhase::Player(PlayerPhase::Combat {
                    view: &view,
                    limits: combat_limits(),
                }),
                None,
            );
            require(
                matches!(
                    result,
                    Err(RoleShellError::PlayerCombat(
                        df_player::CombatMountError::Surface(df_ui::CombatError::Dom(_))
                    ))
                ),
                "expected owned-slot combat append DOM failure did not occur",
            )?;
            fixture.sequence = sequence;
            fixture.phase = 3;
            fixture.overlay = false;
            fixture.host_offer = false;
            fixture
                .append_failure
                .as_mut()
                .ok_or_else(|| JsValue::from_str("append-failure probe missing"))?
                .failed = true;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_append_failure() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let probe = fixture
                .append_failure
                .take()
                .ok_or_else(|| JsValue::from_str("append-failure probe missing"))?;
            require(probe.failed, "owned-slot DOM failure was not executed")?;
            require_erased(&probe.retained)?;
            probe.button.dispatch_event(&Event::new("click")?)?;
            require(
                fixture.receipt.count.get() == probe.count,
                "failed append retained old active callback",
            )?;
            require(
                fixture
                    .player
                    .root()
                    .text_content()
                    .is_some_and(|text| text.contains("Presentation unavailable")),
                "failed append lacked visible error",
            )?;
            fixture
                .root
                .set_attribute("data-shell-append-failure", "pass")?;
            Ok(())
        })
    }
    const SERVER_PENDING: &str = "SERVER_SUPPLIED_PENDING";
    const SERVER_REJECTION: &str = "SERVER_SUPPLIED_REJECTION";

    #[wasm_bindgen]
    pub fn shell_last_join_invitation() -> Result<String, JsValue> {
        FIXTURE.with(|owner| {
            let owner = owner.borrow();
            let fixture = owner
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let last = fixture.receipt.last.borrow();
            match last.as_ref() {
                Some(RoleInput::PlayerJoin(JoinPhaseIntent::Join { invitation, .. })) => {
                    Ok(invitation.clone())
                }
                _ => Err(JsValue::from_str("last actual receipt is not player Join")),
            }
        })
    }
    #[wasm_bindgen]
    pub fn shell_last_join_player_name() -> Result<String, JsValue> {
        FIXTURE.with(|owner| {
            let owner = owner.borrow();
            let fixture = owner
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let last = fixture.receipt.last.borrow();
            match last.as_ref() {
                Some(RoleInput::PlayerJoin(JoinPhaseIntent::Join { player_name, .. })) => {
                    Ok(player_name.clone())
                }
                _ => Err(JsValue::from_str("last actual receipt is not player Join")),
            }
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_join_payload(
        expected_invitation: &str,
        expected_name: &str,
    ) -> Result<(), JsValue> {
        require(
            expected_invitation.len() <= 8192 && expected_name.len() <= 8192,
            "Join payload assertion input exceeds fixture bound",
        )?;
        FIXTURE.with(|owner| {
            let owner = owner.borrow();
            let fixture = owner
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let last = fixture.receipt.last.borrow();
            match last.as_ref() {
                Some(RoleInput::PlayerJoin(JoinPhaseIntent::Join {
                    invitation,
                    player_name,
                    stamp,
                })) => {
                    require(
                        invitation == expected_invitation && player_name == expected_name,
                        "Join forwarded payload differs from actual supplied input",
                    )?;
                    require(
                        stamp.generation == fixture.epoch && stamp.sequence == fixture.sequence,
                        "Join forwarded stamp is not current supplied view",
                    )?;
                    fixture
                        .root
                        .set_attribute("data-shell-join-payload", "pass")?;
                    Ok(())
                }
                _ => Err(JsValue::from_str("last actual receipt is not player Join")),
            }
        })
    }

    // Only the fixture's explicit incoming-view control changes presentation.
    // An actual selection callback records its typed input and does not invoke
    // this producer or decide pending/rejection/readiness locally.
    impl Fixture {
        fn supplied_state(&mut self, phase: u32, state: u32) -> Result<(), JsValue> {
            require(
                phase <= 4 && state <= 2 && !(phase == 4 && state == 2),
                "unsupported supplied presentation fixture state",
            )?;
            if state == 0 {
                self.show(phase, self.epoch, self.overlay, self.host_offer)?;
                self.root.set_attribute("data-shell-supplied-state", "0")?;
                return Ok(());
            }
            if self.phase != phase {
                self.show(phase, self.epoch, self.overlay, self.host_offer)?;
            }
            let sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
            let current_revision = revision(self.epoch, sequence)?;
            let pending = state == 1;
            let mut private_overlay = overlay(false, self.epoch, sequence, self.host_offer);
            let mut public_overlay = overlay(true, self.epoch, sequence, self.host_offer);
            let overlay_offers = [SessionOverlayOffer {
                key: "exact-host-offer",
                label: "Advertised host selection",
                enabled: true,
                pending,
            }];
            private_overlay.operation = Some(if pending {
                FeedbackView::Pending(SERVER_PENDING)
            } else {
                FeedbackView::Refused(SERVER_REJECTION)
            });
            private_overlay.host_offers = if self.host_offer {
                &overlay_offers
            } else {
                &[]
            };
            public_overlay.host_offers = if self.host_offer {
                &overlay_offers
            } else {
                &[]
            };
            let player_root = self.player.root().clone();
            let display_root = self.display.root().clone();
            let (player_result, display_result) = match phase {
                0 => {
                    let mut player = join(false, self.epoch, sequence);
                    player.feedback = if pending {
                        JoinFeedback::Pending(SERVER_PENDING)
                    } else {
                        JoinFeedback::Rejected(SERVER_REJECTION)
                    };
                    player.join_enabled = !pending;
                    let display = join(true, self.epoch, sequence);
                    (
                        self.player.present(
                            self.player_binding,
                            current_revision,
                            RolePhase::Player(PlayerPhase::Join {
                                view: &player,
                                input: PlayerJoinInput {
                                    identifier: "shell-player-join",
                                    invitation: "",
                                    player_name: "",
                                },
                                limits: campaign_limits(),
                            }),
                            self.overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            current_revision,
                            RolePhase::Display(DisplayPhase::Join {
                                view: &display,
                                input: DisplayJoinInput {
                                    identifier: "shell-display-join",
                                    invitation: "",
                                    player_name: "",
                                },
                                limits: campaign_limits(),
                            }),
                            self.overlay.then_some(&public_overlay),
                        ),
                    )
                }
                1 => {
                    let mut player = character(self.epoch, sequence);
                    player.status = if pending {
                        CharacterStatus::Pending
                    } else {
                        CharacterStatus::Rejected
                    };
                    player.status_message = if pending {
                        SERVER_PENDING
                    } else {
                        SERVER_REJECTION
                    }
                    .into();
                    player.editable = !pending;
                    for action in &mut player.actions {
                        action.enabled = !pending;
                    }
                    let mut display = display_character(self.epoch, sequence, self.host_offer);
                    for offer in &mut display.host_offers {
                        offer.pending = pending;
                    }
                    (
                        self.player.present(
                            self.player_binding,
                            current_revision,
                            RolePhase::Player(PlayerPhase::Character {
                                view: &player,
                                limits: character_limits(),
                            }),
                            self.overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            current_revision,
                            RolePhase::Display(DisplayPhase::Character {
                                view: &display,
                                limits: display_limits(),
                            }),
                            self.overlay.then_some(&public_overlay),
                        ),
                    )
                }
                2 => {
                    let choices = [ExplorationChoice {
                        id: "exact-explore-choice",
                        label: "Ask about the harbor",
                        detail: "Supplied choice",
                        disabled_reason: if pending { Some(SERVER_PENDING) } else { None },
                    }];
                    let mut player = exploration(false, self.player_binding, current_revision);
                    player.choices = &choices;
                    player.pending = pending.then_some(SERVER_PENDING);
                    player.rejection = (!pending).then_some(SERVER_REJECTION);
                    let display = exploration(true, self.display_binding, current_revision);
                    (
                        self.player.present(
                            self.player_binding,
                            current_revision,
                            RolePhase::Player(PlayerPhase::Exploration {
                                view: &player,
                                limits: explore_limits(),
                            }),
                            self.overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            current_revision,
                            RolePhase::Display(DisplayPhase::Exploration {
                                view: &display,
                                limits: explore_limits(),
                            }),
                            self.overlay.then_some(&public_overlay),
                        ),
                    )
                }
                3 => {
                    let offer =
                        RevisionLabel::new(Some("exact-combat-intent")).map_err(|failure| {
                            JsValue::from_str(&format!("fixture label: {failure:?}"))
                        })?;
                    let offers = [CombatOffer {
                        key: "action-one",
                        offer: &offer,
                        option: None,
                        kind: CombatOfferKind::Action,
                        label: "Send supplied intent",
                        explanation: "Server validates the exact proposal",
                        enabled: true,
                        pending,
                    }];
                    let mut player = combat(false, &offers);
                    player.feedback = if pending {
                        CombatFeedback::Pending(SERVER_PENDING)
                    } else {
                        CombatFeedback::Refused(SERVER_REJECTION)
                    };
                    player.draft = Some(CombatDraft {
                        key: "private-intent",
                        label: "Your intent",
                        enabled: !pending,
                        feedback: player.feedback,
                    });
                    let display = combat(true, &[]);
                    (
                        self.player.present(
                            self.player_binding,
                            current_revision,
                            RolePhase::Player(PlayerPhase::Combat {
                                view: &player,
                                limits: combat_limits(),
                            }),
                            self.overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            current_revision,
                            RolePhase::Display(DisplayPhase::Combat {
                                view: &display,
                                limits: combat_limits(),
                            }),
                            self.overlay.then_some(&public_overlay),
                        ),
                    )
                }
                4 => {
                    let offers = [SheetOffer {
                        id: "exact-sheet-offer",
                        label: "Advertised equipment selection",
                        enabled: true,
                        pending: true,
                    }];
                    let rows = [SheetRow {
                        key: "lantern",
                        title: "Storm lantern",
                        value: "Supplied inventory",
                        summary: PRIVATE,
                        details: &[],
                        offers: &offers,
                    }];
                    let sections = [SheetSection {
                        key: "equipment",
                        tab: SheetTab::Equipment,
                        title: "Equipment",
                        caption: "Server-provided inventory",
                        rows: &rows,
                    }];
                    let player = CharacterSheetView {
                        owner: SheetOwnerGeneration(self.epoch),
                        revision: sequence,
                        name: "Mara",
                        identity: "Supplied character",
                        subtitle: PRIVATE,
                        connection: "Current supplied view",
                        notice: SERVER_PENDING,
                        labels: SheetLabels {
                            tabs: ["Character", "Equipment", "Spells", "Journal", "Progression"],
                            navigation: "Personal character navigation",
                            filter: "Search your sheet",
                            no_matches: "No matching supplied rows",
                            art_fallback: "Portrait unavailable",
                        },
                        sections: &sections,
                    };
                    let display = combat(true, &[]);
                    (
                        self.player.present(
                            self.player_binding,
                            current_revision,
                            RolePhase::Player(PlayerPhase::Sheet(&player)),
                            self.overlay.then_some(&private_overlay),
                        ),
                        self.display.present(
                            self.display_binding,
                            current_revision,
                            RolePhase::Display(DisplayPhase::Combat {
                                view: &display,
                                limits: combat_limits(),
                            }),
                            self.overlay.then_some(&public_overlay),
                        ),
                    )
                }
                _ => {
                    return Err(JsValue::from_str(
                        "unsupported supplied presentation fixture phase",
                    ));
                }
            };
            require(
                player_result.map_err(error)? == ViewAcceptance::Applied
                    && display_result.map_err(error)? == ViewAcceptance::Applied,
                "supplied current state was not applied",
            )?;
            require(
                player_root.is_same_node(Some(self.player.root()))
                    && display_root.is_same_node(Some(self.display.root())),
                "supplied state recreated persistent role shell",
            )?;
            require(
                !self
                    .display
                    .root()
                    .text_content()
                    .unwrap_or_default()
                    .contains(PRIVATE),
                "private content reached public shell",
            )?;
            self.sequence = sequence;
            self.root
                .set_attribute("data-shell-supplied-state", &state.to_string())?;
            self.root
                .set_attribute("data-shell-sequence", &sequence.to_string())?;
            Ok(())
        }
    }
    #[wasm_bindgen]
    pub fn shell_show_supplied_state(phase: u32, state: u32) -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            owner
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?
                .supplied_state(phase, state)
        })
    }
    /// An explicitly supplied public projection replaces the private fixture's
    /// local role surface. The synthetic caller supplies that authorized input;
    /// the route/RolePhase itself never grants a real audience permission.
    #[wasm_bindgen]
    pub fn shell_check_role_replacement() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            fixture.show(1, fixture.epoch, false, false)?;
            let root = fixture.player.root().clone();
            let retained = private_nodes(&root)?;
            require(
                !retained.is_empty(),
                "role-replacement probe lacks private content",
            )?;
            let button = root
                .query_selector("[data-action-id='exact-character-submit']")?
                .ok_or_else(|| JsValue::from_str("role-replacement action missing"))?;
            let count = fixture.receipt.count.get();
            let sequence = fixture
                .sequence
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
            let public_view = display_character(fixture.epoch, sequence, false);
            let result = fixture
                .player
                .present(
                    fixture.player_binding,
                    revision(fixture.epoch, sequence)?,
                    RolePhase::Display(DisplayPhase::Character {
                        view: &public_view,
                        limits: display_limits(),
                    }),
                    None,
                )
                .map_err(error)?;
            require(
                result == ViewAcceptance::Applied,
                "same-binding current role replacement refused",
            )?;
            require(
                root.is_same_node(Some(fixture.player.root())),
                "role replacement recreated shell",
            )?;
            require_erased(&retained)?;
            button.dispatch_event(&Event::new("click")?)?;
            require(
                fixture.receipt.count.get() == count,
                "old private-role callback remained active",
            )?;
            require(
                root.query_selector("[data-character-client='display']")?
                    .is_some(),
                "actual display mount missing from replaced role",
            )?;
            require(
                !root.text_content().unwrap_or_default().contains(PRIVATE),
                "private content survived supplied public role replacement",
            )?;
            fixture.sequence = sequence;
            fixture.show(1, fixture.epoch, false, false)?;
            fixture
                .root
                .set_attribute("data-shell-role-replacement", "pass")?;
            Ok(())
        })
    }
    struct ProbeFocusTarget(HtmlInputElement);
    impl Drop for ProbeFocusTarget {
        fn drop(&mut self) {
            self.0.remove();
        }
    }

    #[wasm_bindgen]
    pub fn shell_check_unrelated_focus() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            for phase in [2, 3] {
                fixture.show(phase, fixture.epoch, false, false)?;
                let draft = fixture
                    .player
                    .root()
                    .query_selector("input")?
                    .ok_or_else(|| JsValue::from_str("reconnect draft missing"))?
                    .dyn_into::<HtmlInputElement>()
                    .map_err(|_| JsValue::from_str("reconnect draft is not input"))?;
                draft.set_value(PRIVATE);
                draft.dispatch_event(&Event::new("input")?)?;
                draft.focus()?;
                draft.set_selection_range(3, 7)?;
                fixture
                    .player
                    .suspend(SessionConnection::Reconnecting)
                    .map_err(error)?;
                let unrelated = ProbeFocusTarget(
                    fixture
                        .document
                        .create_element("input")?
                        .dyn_into::<HtmlInputElement>()
                        .map_err(|_| JsValue::from_str("probe focus target is not input"))?,
                );
                unrelated
                    .0
                    .set_attribute("aria-label", "Owned unrelated focus probe")?;
                unrelated.0.set_attribute(
                    "style",
                    "position:fixed;right:8px;bottom:8px;width:80px;height:30px;z-index:10000",
                )?;
                fixture.root.append_child(&unrelated.0)?;
                unrelated.0.focus()?;
                require(
                    fixture
                        .document
                        .active_element()
                        .is_some_and(|active| active.is_same_node(Some(&unrelated.0))),
                    "owned unrelated focus was not established",
                )?;
                fixture.show(phase, fixture.epoch, false, false)?;
                require(
                    fixture
                        .document
                        .active_element()
                        .is_some_and(|active| active.is_same_node(Some(&unrelated.0))),
                    "newer reconnect stole competing user focus",
                )?;
                require(
                    draft.value() == PRIVATE,
                    "unrelated focus reconnect lost private draft",
                )?;
            }
            fixture
                .root
                .set_attribute("data-shell-unrelated-focus", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_check_new_scope_focus() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut owner = owner.borrow_mut();
            let fixture = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            for phase in [2, 3] {
                fixture.show(phase, fixture.epoch, false, false)?;
                let draft = fixture
                    .player
                    .root()
                    .query_selector("input")?
                    .ok_or_else(|| JsValue::from_str("epoch focus draft missing"))?
                    .dyn_into::<HtmlInputElement>()
                    .map_err(|_| JsValue::from_str("epoch focus draft is not input"))?;
                draft.set_value(PRIVATE);
                draft.dispatch_event(&Event::new("input")?)?;
                draft.focus()?;
                draft.set_selection_range(3, 7)?;
                let retained = private_nodes(fixture.player.root())?;
                fixture
                    .player
                    .suspend(SessionConnection::Reconnecting)
                    .map_err(error)?;
                let epoch = fixture
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture epoch exhausted"))?;
                fixture.show(phase, epoch, false, false)?;
                require_erased(&retained)?;
                require(
                    !draft.is_connected() && draft.value().is_empty(),
                    "replaced epoch retained old draft node",
                )?;
                require(
                    !fixture
                        .document
                        .active_element()
                        .is_some_and(|active| active.is_same_node(Some(&draft))),
                    "replaced epoch resurrected old focus",
                )?;
            }
            fixture
                .root
                .set_attribute("data-shell-new-scope-focus", "pass")?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shell_shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|owner| {
            let mut fixture = owner
                .borrow_mut()
                .take()
                .ok_or_else(|| JsValue::from_str("fixture disposed"))?;
            let player = fixture.player.dispose();
            let display = fixture.display.dispose();
            fixture.root.remove();
            player.map_err(error)?;
            display.map_err(error)
        })
    }
}
