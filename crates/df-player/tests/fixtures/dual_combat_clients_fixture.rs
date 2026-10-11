#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::ViewAcceptance;
    use df_display::{CombatMountError as DisplayError, DisplayCombat};
    use df_player::{CombatMountError as PlayerError, PlayerCombat};
    use df_types::{ClientBindingId, RecoveryEpoch, RevisionLabel, SessionRevision};
    use df_ui::{
        ActionView, CombatActor, CombatArt, CombatDraft, CombatFeedback, CombatIntent,
        CombatLimits, CombatOffer, CombatOfferKind, CombatPhaseView, CombatReaction,
        CombatResource, CombatRoll, ControlledAction,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlButtonElement, HtmlInputElement};

    const PRIVATE_TEXT: &str = "PRIVATE_COMBAT_SENTINEL";
    const PRIVATE_DRAFT: &str = "PRIVATE_COMBAT_DRAFT_SENTINEL";
    const RESOURCES: [CombatResource<'static>; 1] = [CombatResource {
        label: "Movement",
        value: "30 ft available",
    }];
    const PRIVATE_ACTORS: [CombatActor<'static>; 1] = [CombatActor {
        key: "corin",
        name: "Corin Vale",
        initiative: "18",
        status: PRIVATE_TEXT,
        resources: &RESOURCES,
        hit_points: None,
        portrait: None,
    }];
    const PLAYER_ACTORS: [CombatActor<'static>; 1] = [CombatActor {
        key: "corin",
        name: "Corin Vale",
        initiative: "18",
        status: "Your turn · ranger",
        resources: &RESOURCES,
        hit_points: None,
        portrait: None,
    }];
    const PARTY_ACTORS: [CombatActor<'static>; 3] = [
        CombatActor {
            key: "corin",
            name: "Corin Vale",
            initiative: "18",
            status: "Taking aim",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "mira",
            name: "Mira Ashford",
            initiative: "14",
            status: "Waiting in the shadows",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "dockwarden",
            name: "Dockwarden",
            initiative: "11",
            status: "At the broken pier",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
    ];
    const ROLLS: [CombatRoll<'static>; 1] = [CombatRoll {
        key: "roll-1",
        label: "Longbow · attack roll",
        value: "17",
        explanation: "13 on the die + 4 attack bonus",
        source: "Resolved · Corin Vale",
        verdict: None,
    }];

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
    fn label(value: &str) -> Result<RevisionLabel, JsValue> {
        RevisionLabel::new(Some(value))
            .map_err(|failure| JsValue::from_str(&format!("fixture label: {failure:?}")))
    }
    fn limits() -> CombatLimits {
        CombatLimits {
            max_actors: 8,
            max_offers: 8,
            max_rolls: 8,
            max_resources_per_actor: 8,
            max_text_bytes: 1024,
            max_total_text_bytes: 8192,
        }
    }

    // Public props are constructed independently. No private view is filtered in
    // the display client and no secret input reaches that role's mount.
    fn supplied<R>(
        public: bool,
        mode: Mode,
        reordered: bool,
        operation: impl FnOnce(&CombatPhaseView<'_>) -> R,
    ) -> Result<R, JsValue> {
        let action = label(if public {
            "public-host-continue"
        } else {
            "private-action"
        })?;
        let reaction = label(if public {
            "public-host-observe"
        } else {
            "private-reaction"
        })?;
        let option = label("exact-option")?;
        let roll = label(if public {
            "public-host-roll"
        } else {
            "private-roll"
        })?;
        let pending = mode == Mode::Pending;
        let enabled = mode != Mode::Locked;
        let mut offers = vec![
            CombatOffer {
                key: "action",
                offer: &action,
                option: Some(&option),
                kind: CombatOfferKind::Action,
                label: if public {
                    "Continue the encounter"
                } else {
                    "Loose an arrow"
                },
                explanation: if public {
                    "Bring the next moment into view"
                } else {
                    "Take aim at the dockwarden"
                },
                enabled,
                pending,
            },
            CombatOffer {
                key: "reaction",
                offer: &reaction,
                option: None,
                kind: CombatOfferKind::Reaction,
                label: "Take a reaction",
                explanation: "When an opening appears",
                enabled: enabled && mode == Mode::Reaction,
                pending,
            },
            CombatOffer {
                key: "roll",
                offer: &roll,
                option: None,
                kind: CombatOfferKind::Roll,
                label: "Roll the dice",
                explanation: "Await the result",
                enabled: enabled && mode == Mode::Ready,
                pending,
            },
        ];
        if reordered {
            offers.reverse();
        }
        let view = CombatPhaseView {
            art: CombatArt::Harbor,
            chapter: "The drowned lantern · encounter",
            title: "Lanterns at the docks",
            location: "Greyhaven Harbor",
            narration: if public {
                "A bowstring breaks the silence. Beyond the lanterns, the dockwarden steps into the light."
            } else if reordered {
                "PRIVATE_COMBAT_SENTINEL · Updated private observation"
            } else if mode == Mode::Ready {
                PRIVATE_TEXT
            } else {
                "The dockwarden is in the open. This is your moment."
            },
            connection: "Greyhaven Harbor",
            notice: "Synthetic demo · offline",
            turn_label: "Round 2 · initiative",
            active_actor: Some("corin"),
            actors: if public {
                &PARTY_ACTORS
            } else if reordered || mode == Mode::Ready {
                &PRIVATE_ACTORS
            } else {
                &PLAYER_ACTORS
            },
            action_heading: if public { "The next move" } else { "Your move" },
            action_empty: "No current offers",
            offers: &offers,
            roll_heading: "The dice have spoken",
            roll_empty: "Your next roll will appear here.",
            rolls: if mode == Mode::Resolved { &ROLLS } else { &[] },
            reaction: if mode == Mode::Reaction {
                Some(CombatReaction {
                    heading: "An opening appears",
                    timing: "Reaction available",
                    explanation: "Choose your response before the moment passes.",
                })
            } else {
                None
            },
            draft: if public {
                None
            } else {
                Some(CombatDraft {
                    key: "same-draft-owner",
                    label: "Describe your approach",
                    enabled: enabled && !pending,
                    feedback: if mode == Mode::Refused {
                        CombatFeedback::Refused("Choose another approach. Your words are saved.")
                    } else {
                        CombatFeedback::None
                    },
                })
            },
            feedback: if pending {
                CombatFeedback::Pending("Your move is being considered…")
            } else if mode == Mode::Refused {
                CombatFeedback::Refused("That move is no longer available.")
            } else {
                CombatFeedback::None
            },
        };
        Ok(operation(&view))
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Ready,
        Pending,
        Refused,
        Reaction,
        Locked,
        Resolved,
    }
    #[derive(Default)]
    struct Recorded {
        count: u64,
        last: Option<CombatIntent>,
        overflow: bool,
    }
    fn record(recorded: &Rc<RefCell<Recorded>>, intent: CombatIntent) {
        let mut recorded = recorded.borrow_mut();
        if let Some(count) = recorded.count.checked_add(1) {
            recorded.count = count;
        } else {
            recorded.overflow = true;
        }
        recorded.last = Some(intent);
    }
    struct Fixture {
        document: Document,
        player_parent: Element,
        display_parent: Element,
        player: Option<PlayerCombat>,
        display: Option<DisplayCombat>,
        player_binding: ClientBindingId,
        display_binding: ClientBindingId,
        revision: SessionRevision,
        private: Rc<RefCell<Recorded>>,
        public: Rc<RefCell<Recorded>>,
        status: Element,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn player(fixture: &Fixture) -> Result<&PlayerCombat, JsValue> {
        fixture
            .player
            .as_ref()
            .ok_or_else(|| JsValue::from_str("player absent"))
    }
    fn display(fixture: &Fixture) -> Result<&DisplayCombat, JsValue> {
        fixture
            .display
            .as_ref()
            .ok_or_else(|| JsValue::from_str("display absent"))
    }
    fn button(root: &Element, key: &str) -> Result<HtmlButtonElement, JsValue> {
        root.query_selector(&format!("[data-combat-offer='{key}']"))?
            .ok_or_else(|| JsValue::from_str("offer button absent"))?
            .dyn_into()
            .map_err(|_| JsValue::from_str("wrong offer type"))
    }
    fn input(root: &Element) -> Result<HtmlInputElement, JsValue> {
        root.query_selector("input")?
            .ok_or_else(|| JsValue::from_str("draft absent"))?
            .dyn_into()
            .map_err(|_| JsValue::from_str("wrong draft type"))
    }
    fn status(fixture: &Fixture, value: &str) -> Result<(), JsValue> {
        fixture.status.set_text_content(Some(value));
        fixture.status.set_attribute("data-result", value)
    }
    fn safe(fixture: &Fixture) -> Result<(), JsValue> {
        let public = display(fixture)?.root();
        require(
            !public.outer_html().contains(PRIVATE_TEXT)
                && !public.outer_html().contains(PRIVATE_DRAFT),
            "public DOM leaked private sentinel",
        )?;
        require(
            public.query_selector("input")?.is_none(),
            "public role manufactured private draft",
        )
    }
    fn mount_pair(fixture: &mut Fixture) -> Result<(), JsValue> {
        let private = Rc::clone(&fixture.private);
        fixture.player = Some(
            supplied(false, Mode::Resolved, false, |view| {
                PlayerCombat::mount(
                    &fixture.document,
                    &fixture.player_parent,
                    "combat-private-draft",
                    (fixture.player_binding, fixture.revision),
                    view,
                    limits(),
                    move |intent| record(&private, intent),
                )
            })?
            .map_err(error)?,
        );
        let public = Rc::clone(&fixture.public);
        fixture.display = Some(
            supplied(true, Mode::Resolved, false, |view| {
                DisplayCombat::mount(
                    &fixture.document,
                    &fixture.display_parent,
                    "combat-public-draft",
                    (fixture.display_binding, fixture.revision),
                    view,
                    limits(),
                    move |intent| record(&public, intent),
                )
            })?
            .map_err(error)?,
        );
        safe(fixture)
    }
    fn update(fixture: &mut Fixture, mode: Mode, reordered: bool) -> Result<(), JsValue> {
        let next = fixture
            .revision
            .next_sequence()
            .map_err(|failure| JsValue::from_str(&format!("fixture sequence: {failure:?}")))?;
        supplied(false, mode, reordered, |view| {
            player(fixture)?
                .update(fixture.player_binding, next, view)
                .map_err(error)
        })??;
        supplied(true, mode, reordered, |view| {
            display(fixture)?
                .update(fixture.display_binding, next, view)
                .map_err(error)
        })??;
        fixture.revision = next;
        safe(fixture)
    }
    fn exact(
        recorded: &Recorded,
        binding: ClientBindingId,
        revision: SessionRevision,
        public: bool,
        reaction: bool,
        draft: Option<&str>,
    ) -> Result<(), JsValue> {
        let last = recorded
            .last
            .as_ref()
            .ok_or_else(|| JsValue::from_str("intent absent"))?;
        let offer = if public {
            if reaction {
                "public-host-observe"
            } else {
                "public-host-continue"
            }
        } else if reaction {
            "private-reaction"
        } else {
            "private-action"
        };
        let expected = CombatIntent {
            binding,
            revision,
            kind: if reaction {
                CombatOfferKind::Reaction
            } else {
                CombatOfferKind::Action
            },
            offer: label(offer)?,
            option: if reaction {
                None
            } else {
                Some(label("exact-option")?)
            },
            draft: draft.map(str::to_owned),
        };
        require(
            !recorded.overflow && *last == expected,
            "callback changed advertised intent",
        )
    }

    fn exact_roll(
        recorded: &Recorded,
        binding: ClientBindingId,
        revision: SessionRevision,
        public: bool,
        draft: Option<&str>,
    ) -> Result<(), JsValue> {
        let expected = CombatIntent {
            binding,
            revision,
            kind: CombatOfferKind::Roll,
            offer: label(if public {
                "public-host-roll"
            } else {
                "private-roll"
            })?,
            option: None,
            draft: draft.map(str::to_owned),
        };
        require(
            !recorded.overflow && recorded.last.as_ref() == Some(&expected),
            "roll callback changed advertised intent",
        )
    }

    #[wasm_bindgen]
    pub fn exercise_combat_mounts() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture absent"))?;
            let outer = player(f)?.root().clone();
            let public_outer = display(f)?.root().clone();
            let draft = input(&outer)?;
            draft.set_value(PRIVATE_DRAFT);
            draft.dispatch_event(&Event::new("input")?)?;
            let action = button(&outer, "action")?;
            action.focus()?;
            let raw = outer
                .query_selector(".combat-narration")?
                .and_then(|node| node.first_child())
                .ok_or_else(|| JsValue::from_str("private Text absent"))?;
            update(f, Mode::Ready, true)?;
            require(
                player(f)?.root().is_same_node(Some(&outer))
                    && display(f)?.root().is_same_node(Some(&public_outer)),
                "ordinary update replaced mount",
            )?;
            require(
                input(&outer)?.is_same_node(Some(&draft)) && draft.value() == PRIVATE_DRAFT,
                "ordinary update lost draft identity/value",
            )?;
            require(
                button(&outer, "action")?.is_same_node(Some(&action))
                    && f.document
                        .active_element()
                        .is_some_and(|active| active.is_same_node(Some(&action))),
                "keyed reorder lost button focus/identity",
            )?;
            require(
                raw.node_value().unwrap_or_default().is_empty(),
                "superseded private Text remains readable",
            )?;
            let count = f.private.borrow().count;
            action.click();
            require(
                f.private.borrow().count == count + 1,
                "action did not emit once",
            )?;
            exact(
                &f.private.borrow(),
                f.player_binding,
                f.revision,
                false,
                false,
                Some(PRIVATE_DRAFT),
            )?;
            let public_count = f.public.borrow().count;
            button(&public_outer, "action")?.click();
            require(
                f.public.borrow().count == public_count + 1,
                "public advertised offer did not emit once",
            )?;
            exact(
                &f.public.borrow(),
                f.display_binding,
                f.revision,
                true,
                false,
                None,
            )?;
            let count = f.private.borrow().count;
            button(&outer, "roll")?.click();
            require(
                f.private.borrow().count == count + 1,
                "roll did not emit once",
            )?;
            exact_roll(
                &f.private.borrow(),
                f.player_binding,
                f.revision,
                false,
                Some(PRIVATE_DRAFT),
            )?;
            let public_count = f.public.borrow().count;
            button(&public_outer, "roll")?.click();
            require(
                f.public.borrow().count == public_count + 1,
                "public roll did not emit once",
            )?;
            exact_roll(
                &f.public.borrow(),
                f.display_binding,
                f.revision,
                true,
                None,
            )?;
            player(f)?.suspend_input().map_err(error)?;
            display(f)?.suspend_input().map_err(error)?;
            let count = f.private.borrow().count;
            let public_count = f.public.borrow().count;
            action.dispatch_event(&Event::new("click")?)?;
            button(&public_outer, "action")?.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == count && f.public.borrow().count == public_count,
                "offline synthetic event emitted",
            )?;
            let mounted_player = player(f)?;
            let duplicate = supplied(false, Mode::Ready, false, |view| {
                mounted_player.update(f.player_binding, f.revision, view)
            })?;
            require(
                matches!(
                    duplicate,
                    Err(PlayerError::Admission(ViewAcceptance::Duplicate { .. }))
                ),
                "duplicate resumed offline mount",
            )?;
            action.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == count,
                "duplicate reactivated callback",
            )?;
            update(f, Mode::Refused, false)?;
            require(
                draft.value() == PRIVATE_DRAFT && !draft.read_only(),
                "reconnect/refusal lost valid draft",
            )?;
            update(f, Mode::Pending, false)?;
            action.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == count && action.disabled(),
                "pending offer emitted or stayed interactive",
            )?;
            update(f, Mode::Locked, false)?;
            action.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == count && action.disabled(),
                "locked offer emitted",
            )?;
            update(f, Mode::Reaction, false)?;
            let old_reaction = button(&outer, "reaction")?;
            old_reaction.click();
            exact(
                &f.private.borrow(),
                f.player_binding,
                f.revision,
                false,
                true,
                Some(PRIVATE_DRAFT),
            )?;
            let reaction_count = f.private.borrow().count;
            update(f, Mode::Resolved, false)?;
            old_reaction.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == reaction_count,
                "expired reaction emitted",
            )?;
            require(
                outer
                    .query_selector(".combat-roll-value")?
                    .and_then(|node| node.text_content())
                    .as_deref()
                    == Some("17"),
                "committed supplied roll not displayed",
            )?;
            let wrong = binding(99)?;
            let next = f
                .revision
                .next_sequence()
                .map_err(|failure| JsValue::from_str(&format!("sequence: {failure:?}")))?;
            let mounted_player = player(f)?;
            let wrong_player = supplied(false, Mode::Ready, false, |view| {
                mounted_player.update(wrong, next, view)
            })?;
            require(
                matches!(
                    wrong_player,
                    Err(PlayerError::Admission(ViewAcceptance::WrongBinding))
                ),
                "wrong player binding admitted",
            )?;
            let mounted_display = display(f)?;
            let wrong_display = supplied(true, Mode::Ready, false, |view| {
                mounted_display.update(wrong, next, view)
            })?;
            require(
                matches!(
                    wrong_display,
                    Err(DisplayError::Admission(ViewAcceptance::WrongBinding))
                ),
                "wrong public binding admitted",
            )?;
            let old_epoch = f.revision;
            let epoch = old_epoch
                .epoch()
                .get()
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("epoch exhausted"))?;
            f.revision = revision(epoch, 0)?;
            let before_epoch = outer
                .query_selector(".combat-narration")?
                .and_then(|node| node.first_child())
                .ok_or_else(|| JsValue::from_str("private Text absent"))?;
            supplied(false, Mode::Ready, false, |view| {
                player(f)?
                    .update(f.player_binding, f.revision, view)
                    .map_err(error)
            })??;
            supplied(true, Mode::Ready, false, |view| {
                display(f)?
                    .update(f.display_binding, f.revision, view)
                    .map_err(error)
            })??;
            require(
                player(f)?.root().is_same_node(Some(&outer))
                    && draft.value().is_empty()
                    && before_epoch.node_value().unwrap_or_default().is_empty(),
                "epoch recovery retained old draft/prose or replaced outer mount",
            )?;
            let count = f.private.borrow().count;
            action.dispatch_event(&Event::new("click")?)?;
            require(
                f.private.borrow().count == count,
                "old epoch action emitted",
            )?;
            let mounted_player = player(f)?;
            let stale = supplied(false, Mode::Ready, false, |view| {
                mounted_player.update(f.player_binding, old_epoch, view)
            })?;
            require(
                matches!(
                    stale,
                    Err(PlayerError::Admission(ViewAcceptance::Stale { .. }))
                ),
                "old epoch snapshot admitted",
            )?;
            safe(f)?;
            status(f, "exercise-pass")
        })
    }
    #[wasm_bindgen]
    pub fn show_demo_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Resolved, "demo")
    }
    #[wasm_bindgen]
    pub fn show_ready_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Ready, "ready")
    }
    #[wasm_bindgen]
    pub fn show_refused_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Refused, "refused")
    }
    #[wasm_bindgen]
    pub fn show_pending_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Pending, "pending")
    }
    #[wasm_bindgen]
    pub fn show_reaction_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Reaction, "reaction")
    }
    #[wasm_bindgen]
    pub fn show_locked_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Locked, "locked")
    }
    #[wasm_bindgen]
    pub fn show_resolved_combat() -> Result<(), JsValue> {
        apply_mode(Mode::Resolved, "resolved")
    }
    fn apply_mode(mode: Mode, label: &str) -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture absent"))?;
            update(f, mode, false)?;
            status(f, label)
        })
    }
    fn terminate(fixture: &mut Fixture, revoke: bool) -> Result<(), JsValue> {
        let root = player(fixture)?.root().clone();
        let public_root = display(fixture)?.root().clone();
        let draft = input(&root)?;
        draft.set_value(PRIVATE_DRAFT);
        draft.dispatch_event(&Event::new("input")?)?;
        let raw = root
            .query_selector(".combat-narration")?
            .and_then(|node| node.first_child())
            .ok_or_else(|| JsValue::from_str("private Text absent"))?;
        let actor = root
            .query_selector("[data-actor-key]")?
            .ok_or_else(|| JsValue::from_str("actor absent"))?;
        let action = button(&root, "action")?;
        let public_action = button(&public_root, "action")?;
        let count = fixture.private.borrow().count;
        let public_count = fixture.public.borrow().count;
        if revoke {
            player(fixture)?.revoke().map_err(error)?;
            display(fixture)?.revoke().map_err(error)?;
        }
        fixture.player.take();
        fixture.display.take();
        action.dispatch_event(&Event::new("click")?)?;
        public_action.dispatch_event(&Event::new("click")?)?;
        require(
            fixture.private.borrow().count == count
                && fixture.public.borrow().count == public_count,
            "terminal mount emitted retained event",
        )?;
        require(
            draft.value().is_empty()
                && raw.node_value().unwrap_or_default().is_empty()
                && actor.get_attribute("data-actor-key").is_none()
                && actor.text_content().unwrap_or_default().is_empty()
                && root.text_content().unwrap_or_default().is_empty()
                && public_root.text_content().unwrap_or_default().is_empty(),
            "terminal mount retained private DOM/draft/attributes",
        )
    }
    #[wasm_bindgen]
    pub fn replace_combat_bindings() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture absent"))?;
            terminate(f, true)?;
            f.player_binding = binding(41)?;
            f.display_binding = binding(42)?;
            f.revision = revision(1, 0)?;
            mount_pair(f)?;
            status(f, "replacement-pass")
        })
    }
    #[wasm_bindgen]
    pub fn drop_combat_mounts() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture absent"))?;
            terminate(f, false)?;
            status(f, "drop-pass")
        })
    }
    fn parent(document: &Document, body: &Element, role: &str) -> Result<Element, JsValue> {
        let root = document.create_element("section")?;
        root.set_attribute("data-client", role)?;
        let heading = document.create_element("h1")?;
        heading.set_text_content(Some(role));
        root.append_child(&heading)?;
        body.append_child(&root)?;
        Ok(root)
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("window absent"))?
            .document()
            .ok_or_else(|| JsValue::from_str("document absent"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body absent"))?;
        let notice = document.create_element("p")?;
        notice.set_text_content(Some("DungeonFlux browser showcase"));
        body.append_child(&notice)?;
        let controls_root = document.create_element("nav")?;
        controls_root.set_attribute("aria-label", "Fixture controls")?;
        body.append_child(&controls_root)?;
        let status_node = document.create_element("output")?;
        status_node.set_id("dual-combat-status");
        status_node.set_attribute("aria-live", "polite")?;
        body.append_child(&status_node)?;
        let mut fixture = Fixture {
            document: document.clone(),
            player_parent: parent(&document, &body, "player")?,
            display_parent: parent(&document, &body, "shared-display")?,
            player: None,
            display: None,
            player_binding: binding(31)?,
            display_binding: binding(32)?,
            revision: revision(1, 0)?,
            private: Rc::new(RefCell::new(Recorded::default())),
            public: Rc::new(RefCell::new(Recorded::default())),
            status: status_node,
            controls: Vec::new(),
        };
        mount_pair(&mut fixture)?;
        for (id, label, operation) in [
            (
                "exercise-combat",
                "Exercise both combat mounts",
                exercise_combat_mounts as fn() -> Result<(), JsValue>,
            ),
            (
                "pending-combat",
                "Show supplied pending proposal",
                show_pending_combat,
            ),
            (
                "reaction-combat",
                "Show advertised reaction window",
                show_reaction_combat,
            ),
            (
                "locked-combat",
                "Show supplied locked offers",
                show_locked_combat,
            ),
            (
                "resolved-combat",
                "Show committed supplied roll",
                show_resolved_combat,
            ),
            (
                "replace-combat",
                "Revoke and replace both bindings",
                replace_combat_bindings,
            ),
            ("drop-combat", "Drop both mounts", drop_combat_mounts),
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
                if let Err(failure) = operation() {
                    FIXTURE.with(|slot| {
                        if let Some(fixture) = slot.borrow().as_ref() {
                            fixture.status.set_text_content(Some(&format!("fixture-failure: {failure:?}")));
                            if let Err(status_failure) = fixture.status.set_attribute("data-result", "failure") {
                                fixture.status.set_text_content(Some(&format!("fixture-failure: {failure:?}; status-failure: {status_failure:?}")));
                            }
                        }
                    });
                }
            }).map_err(error)?;
            controls_root.append_child(control.element())?;
            fixture.controls.push(control);
        }
        status(&fixture, "ready")?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
}
