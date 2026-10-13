#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        ActionView, CampaignLimits, CampaignMember, CampaignView, ConceptScene, ControlledAction,
        JoinFeedback, JoinPhaseIntent, JoinPhaseSurface, JoinPhaseView, JoinRoom, JoinStage,
        JoinStamp, JoinUpdate, LobbyOffer, LobbyParticipant,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, HtmlButtonElement};

    const PARTY: [LobbyParticipant<'static>; 5] = [
        LobbyParticipant {
            key: "mira",
            name: "Mira",
            sigil: "M",
            readiness: "Ready",
            presence: "Connected · Remote",
        },
        LobbyParticipant {
            key: "aster",
            name: "Aster",
            sigil: "A",
            readiness: "Choosing",
            presence: "Connected · At the table",
        },
        LobbyParticipant {
            key: "rowan",
            name: "Rowan",
            sigil: "R",
            readiness: "Ready",
            presence: "Sleeping · Membership retained",
        },
        LobbyParticipant {
            key: "fen",
            name: "Fen",
            sigil: "F",
            readiness: "Not ready",
            presence: "Reconnecting · Remote",
        },
        LobbyParticipant {
            key: "orin",
            name: "Orin",
            sigil: "O",
            readiness: "Ready",
            presence: "Connected · Remote",
        },
    ];

    #[derive(Clone, Copy)]
    enum Preview {
        Title,
        Invitation,
        Pending,
        Rejected,
        Lobby,
        Reconnecting,
        Offline,
        Empty,
    }
    struct Fixture {
        surface: Rc<JoinPhaseSurface>,
        controls: Vec<Rc<ControlledAction>>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }

    fn error(value: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&value.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn view<'a>(stamp: JoinStamp, mode: Preview) -> JoinPhaseView<'a> {
        let lobby = matches!(
            mode,
            Preview::Lobby | Preview::Reconnecting | Preview::Offline | Preview::Empty
        );
        JoinPhaseView {
            stamp,
            stage: if lobby {
                JoinStage::Lobby
            } else if matches!(mode, Preview::Title) {
                JoinStage::Title
            } else {
                JoinStage::Invitation
            },
            campaign: CampaignView {
                scene: ConceptScene::Harbor,
                chapter: "A tale of salt, smoke & secrets",
                title: "The Drowned Lantern",
                description: "The tide has taken the streets of Greyhaven. Tonight, a light in the harbor calls your party home.",
                location: "Greyhaven · The old harbor",
                scene_label: "Gather your party",
                narration: "Every adventure begins with an invitation. Beyond the rain, a door opens, a lantern burns, and a story waits to be told together.",
                connection: match mode {
                    Preview::Reconnecting => {
                        "Design preview · Synthetic connection: reconnecting · No live session"
                    }
                    Preview::Offline => {
                        "Design preview · Synthetic connection: offline · No live session"
                    }
                    _ => {
                        "Design preview · Synthetic room and party · No gameplay session connected"
                    }
                },
                notice: "Join & lobby · Design preview",
                members: &[],
                objectives: &[],
            },
            panel_heading: if lobby {
                "Your party awaits"
            } else {
                "Step into the story"
            },
            panel_description: if lobby {
                "Readiness and presence below are supplied preview facts. Your seat stays distinct from your connection."
            } else {
                "Bring an invitation and the name your friends know you by. Your next great story begins together."
            },
            invitation_label: "Room code or invitation",
            name_label: "Your player name",
            join_label: if matches!(mode, Preview::Pending) {
                "Waiting for response…"
            } else {
                "Join the adventure"
            },
            join_enabled: true,
            feedback: match mode {
                Preview::Pending => JoinFeedback::Pending(
                    "Preview: the join request is awaiting a server response.",
                ),
                Preview::Rejected => JoinFeedback::Rejected(
                    "Preview: this invitation was rejected. Your drafts are retained; edit them and try again.",
                ),
                Preview::Reconnecting => JoinFeedback::Pending(
                    "Preview: reconnecting. Existing membership is shown; no new action is enabled.",
                ),
                Preview::Offline => JoinFeedback::Rejected(
                    "Preview: connection unavailable. Reconnection belongs to the client shell.",
                ),
                _ => JoinFeedback::None,
            },
            room: if lobby {
                Some(JoinRoom {
                    label: "Room · Preview",
                    code: "LANTERN",
                    detail: "A shared table, wherever you are",
                })
            } else {
                None
            },
            roster_heading: "Adventurers at the table",
            empty_roster: "The lantern is lit. Your adventurers will appear here when the server admits them.",
            participants: if lobby && !matches!(mode, Preview::Empty) {
                &PARTY
            } else {
                &[]
            },
            away_heroes: &[],
            lobby_offer: if lobby {
                Some(LobbyOffer {
                    key: "preview-ready-offer",
                    label: "Mark yourself ready",
                    enabled: !matches!(mode, Preview::Reconnecting | Preview::Offline),
                    pending: false,
                })
            } else {
                None
            },
            pairing_fallback: "Public invitation / QR panel: awaiting the established display join component. No credential or QR is generated by this preview.",
        }
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: &str,
    ) -> Result<Element, JsValue> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(Some(text));
        parent.append_child(&node)?;
        Ok(node)
    }

    fn remove_child(document_root: &Element, selector: &str) -> Result<(), JsValue> {
        let node = document_root
            .query_selector(selector)?
            .ok_or_else(|| JsValue::from_str("regression node missing"))?;
        let parent = node
            .parent_node()
            .ok_or_else(|| JsValue::from_str("regression parent missing"))?;
        parent.remove_child(&node)?;
        Ok(())
    }
    fn click_join(surface: &JoinPhaseSurface) -> Result<(), JsValue> {
        surface
            .root()
            .query_selector(".join-fields button")?
            .ok_or_else(|| JsValue::from_str("join button missing"))?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("join button type mismatch"))?
            .click();
        Ok(())
    }
    fn failed_ordinary_update_check(document: &Document) -> Result<(), JsValue> {
        let committed = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        let surface = JoinPhaseSurface::create(
            document,
            "join-ordinary-failure-regression",
            &view(committed, Preview::Lobby),
            "retained invitation",
            "retained name",
            CampaignLimits {
                max_members: 128,
                max_objectives: 128,
                max_text_bytes: 4096,
            },
        )
        .map_err(error)?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("ordinary regression body missing"))?;
        body.append_child(surface.root())?;
        let emitted = Rc::new(Cell::new(0_u64));
        let observed = Rc::new(Cell::new(None::<JoinStamp>));
        let emitted_intents = Rc::clone(&emitted);
        let observed_stamp = Rc::clone(&observed);
        surface
            .on_intent(move |intent| {
                emitted_intents.set(emitted_intents.get().saturating_add(1));
                observed_stamp.set(Some(match intent {
                    JoinPhaseIntent::Join { stamp, .. }
                    | JoinPhaseIntent::LobbyAction { stamp, .. } => stamp,
                }));
            })
            .map_err(error)?;
        let result = (|| -> Result<(), JsValue> {
            let invitation_node = surface.invitation().input().clone();
            let name_node = surface.player_name().input().clone();
            // Mounted reproduction of the reviewer's same-owner seq2 failure.
            remove_child(surface.root(), "[data-participant-key=mira]")?;
            let attempted = JoinStamp {
                generation: committed.generation,
                sequence: 2,
            };
            let mut attempted_view = view(attempted, Preview::Empty);
            attempted_view.lobby_offer = Some(LobbyOffer {
                key: "current-attempt-offer",
                label: "Current attempt",
                enabled: true,
                pending: false,
            });
            require(
                surface.update(&attempted_view).is_err(),
                "ordinary reconciliation failure not triggered",
            )?;
            require(
                surface
                    .update(&view(committed, Preview::Empty))
                    .map_err(error)?
                    == JoinUpdate::StaleSequence,
                "committed seq1 replay revived failed seq2",
            )?;
            let action = surface
                .root()
                .query_selector(".join-lobby button")?
                .ok_or_else(|| JsValue::from_str("ordinary regression action missing"))?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("ordinary regression action type mismatch"))?;
            action.click();
            require(
                emitted.get() == 0,
                "failed attempt or old replay emitted an intent",
            )?;
            require(
                surface.invitation().draft() == "retained invitation"
                    && surface.player_name().draft() == "retained name"
                    && invitation_node.is_same_node(Some(surface.invitation().input()))
                    && name_node.is_same_node(Some(surface.player_name().input())),
                "same-owner failure changed valid draft ownership or input nodes",
            )?;
            require(
                surface.update(&attempted_view).map_err(error)? == JoinUpdate::Applied,
                "exact failed seq2 attempt did not recover",
            )?;
            action.click();
            require(
                emitted.get() == 1 && observed.get() == Some(attempted),
                "recovered intent was not bound to exact current attempt",
            )?;
            require(
                surface.update(&attempted_view).map_err(error)? == JoinUpdate::StaleSequence,
                "successfully recovered sequence replay was accepted",
            )?;
            require(
                surface
                    .update(&view(committed, Preview::Empty))
                    .map_err(error)?
                    == JoinUpdate::StaleSequence,
                "old committed sequence returned after recovery",
            )?;
            Ok(())
        })();
        surface.dispose().map_err(error)?;
        surface.dispose().map_err(error)?;
        result
    }

    fn failed_replacement_check(document: &Document) -> Result<(), JsValue> {
        let old_members = [CampaignMember {
            key: "old-member",
            name: "Old owner",
            role: "Member",
            sigil: "O",
        }];
        let prior = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        let mut initial = view(prior, Preview::Invitation);
        initial.campaign.members = &old_members;
        let surface = JoinPhaseSurface::create(
            document,
            "join-failure-regression",
            &initial,
            "obsolete invitation",
            "obsolete name",
            CampaignLimits {
                max_members: 128,
                max_objectives: 128,
                max_text_bytes: 4096,
            },
        )
        .map_err(error)?;
        let emitted = Rc::new(Cell::new(0_u64));
        let emitted_intents = Rc::clone(&emitted);
        surface
            .on_intent(move |_| emitted_intents.set(emitted_intents.get().saturating_add(1)))
            .map_err(error)?;
        let result = (|| -> Result<(), JsValue> {
            // Exercise the reviewer's exact early campaign reconciliation failure:
            // an existing keyed node is detached before removal reconciliation.
            remove_child(surface.root(), ".party-card")?;
            let replacement = JoinStamp {
                generation: 2,
                sequence: 1,
            };
            require(
                surface
                    .replace_owner(&view(replacement, Preview::Invitation))
                    .is_err(),
                "campaign failure not triggered",
            )?;
            require(
                surface.invitation().draft().is_empty()
                    && surface.player_name().draft().is_empty()
                    && surface.invitation().input().value().is_empty()
                    && surface.player_name().input().value().is_empty(),
                "early failure retained obsolete drafts",
            )?;
            let stale = JoinStamp {
                generation: prior.generation,
                sequence: u64::MAX,
            };
            require(
                surface
                    .update(&view(stale, Preview::Invitation))
                    .map_err(error)?
                    == JoinUpdate::StaleGeneration,
                "old owner revived after campaign failure",
            )?;
            click_join(&surface)?;
            require(emitted.get() == 0, "suspended replacement emitted intent")?;
            require(
                surface
                    .update(&view(replacement, Preview::Invitation))
                    .map_err(error)?
                    == JoinUpdate::Applied,
                "same-stamp replacement recovery failed",
            )?;
            click_join(&surface)?;
            require(
                emitted.get() == 1,
                "recovered owner did not emit permitted intent",
            )?;
            let lobby_stamp = JoinStamp {
                generation: 2,
                sequence: 2,
            };
            surface
                .update(&view(lobby_stamp, Preview::Lobby))
                .map_err(error)?;
            // Exercise the later keyed lobby failure after the old owner retires.
            remove_child(surface.root(), "[data-participant-key=mira]")?;
            let next_owner = JoinStamp {
                generation: 3,
                sequence: 1,
            };
            require(
                surface
                    .replace_owner(&view(next_owner, Preview::Invitation))
                    .is_err(),
                "lobby failure not triggered",
            )?;
            require(
                surface
                    .update(&view(
                        JoinStamp {
                            generation: 2,
                            sequence: u64::MAX,
                        },
                        Preview::Invitation,
                    ))
                    .map_err(error)?
                    == JoinUpdate::StaleGeneration,
                "old lobby owner revived after failed replacement",
            )?;
            click_join(&surface)?;
            require(
                emitted.get() == 1,
                "failed lobby replacement emitted an old intent",
            )?;
            require(
                surface
                    .update(&view(next_owner, Preview::Invitation))
                    .map_err(error)?
                    == JoinUpdate::Applied,
                "lobby replacement recovery failed",
            )?;
            click_join(&surface)?;
            require(
                emitted.get() == 2,
                "recovered lobby replacement did not emit",
            )?;
            Ok(())
        })();
        surface.dispose().map_err(error)?;
        surface.dispose().map_err(error)?;
        result
    }

    fn focus_transition_check(
        document: &Document,
        surface: &JoinPhaseSurface,
        revision: &mut JoinStamp,
    ) -> Result<(), JsValue> {
        let heading = surface
            .root()
            .query_selector(".join-heading")?
            .ok_or_else(|| JsValue::from_str("heading missing"))?;
        let lobby_action = surface
            .root()
            .query_selector(".join-lobby button")?
            .ok_or_else(|| JsValue::from_str("lobby action missing"))?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("lobby action type mismatch"))?;
        let focused_heading = || {
            document
                .active_element()
                .as_ref()
                .is_some_and(|active| heading.is_same_node(Some(active)))
        };
        revision.sequence = revision.sequence.saturating_add(1);
        surface
            .update(&view(*revision, Preview::Lobby))
            .map_err(error)?;
        lobby_action.focus()?;
        revision.sequence = revision.sequence.saturating_add(1);
        surface
            .update(&view(*revision, Preview::Invitation))
            .map_err(error)?;
        require(
            focused_heading(),
            "lobby-to-invitation focus was not restored",
        )?;
        revision.sequence = revision.sequence.saturating_add(1);
        surface
            .update(&view(*revision, Preview::Lobby))
            .map_err(error)?;
        lobby_action.focus()?;
        revision.sequence = revision.sequence.saturating_add(1);
        surface
            .update(&view(*revision, Preview::Offline))
            .map_err(error)?;
        require(focused_heading(), "disabled lobby offer stranded focus")?;
        revision.sequence = revision.sequence.saturating_add(1);
        surface
            .update(&view(*revision, Preview::Lobby))
            .map_err(error)?;
        lobby_action.focus()?;
        revision.sequence = revision.sequence.saturating_add(1);
        let mut no_offer = view(*revision, Preview::Lobby);
        no_offer.lobby_offer = None;
        surface.update(&no_offer).map_err(error)?;
        require(focused_heading(), "removed lobby offer stranded focus")?;
        Ok(())
    }

    fn replacement_focus_check(document: &Document) -> Result<(), JsValue> {
        let surface = JoinPhaseSurface::create(
            document,
            "join-focus-regression",
            &view(
                JoinStamp {
                    generation: 1,
                    sequence: 1,
                },
                Preview::Lobby,
            ),
            "",
            "",
            CampaignLimits {
                max_members: 128,
                max_objectives: 128,
                max_text_bytes: 4096,
            },
        )
        .map_err(error)?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("focus regression body missing"))?;
        body.append_child(surface.root())?;
        let result = (|| -> Result<(), JsValue> {
            let action = surface
                .root()
                .query_selector(".join-lobby button")?
                .ok_or_else(|| JsValue::from_str("replacement lobby action missing"))?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("replacement action type mismatch"))?;
            action.focus()?;
            surface
                .replace_owner(&view(
                    JoinStamp {
                        generation: 2,
                        sequence: 1,
                    },
                    Preview::Invitation,
                ))
                .map_err(error)?;
            let heading = surface
                .root()
                .query_selector(".join-heading")?
                .ok_or_else(|| JsValue::from_str("replacement heading missing"))?;
            require(
                document
                    .active_element()
                    .as_ref()
                    .is_some_and(|active| heading.is_same_node(Some(active))),
                "owner replacement hid the focused lobby action",
            )
        })();
        surface.dispose().map_err(error)?;
        result
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        require(
            !FIXTURE.with(|slot| slot.borrow().is_some()),
            "preview already mounted",
        )?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · Join & lobby · Design preview");
        let stamp = Rc::new(Cell::new(JoinStamp {
            generation: 1,
            sequence: 1,
        }));
        let surface = Rc::new(
            JoinPhaseSurface::create(
                &document,
                "join-preview",
                &view(stamp.get(), Preview::Title),
                "",
                "",
                CampaignLimits {
                    max_members: 128,
                    max_objectives: 128,
                    max_text_bytes: 4096,
                },
            )
            .map_err(error)?,
        );
        let intent_status = child(
            &document,
            surface.root(),
            "p",
            "connection",
            "Input preview: no intent emitted. Join and readiness controls emit only; they cannot confirm a session.",
        )?;
        intent_status.set_attribute("data-preview-intents", "0")?;
        intent_status.set_attribute("role", "status")?;
        let intents = Rc::new(Cell::new(0_u64));
        let emitted = Rc::clone(&intents);
        surface.on_intent(move |intent| {
            emitted.set(emitted.get().saturating_add(1));
            let operation = match intent { JoinPhaseIntent::Join { .. } => "Join", JoinPhaseIntent::LobbyAction { .. } => "Lobby action" };
            // Drafts/offer keys are deliberately absent from the visible diagnostic.
            intent_status.set_text_content(Some(&format!("{operation} intent emitted · {} total · Awaiting real server wiring; no admission or readiness is confirmed.", emitted.get())));
            if intent_status.set_attribute("data-preview-intents", &emitted.get().to_string()).is_err() {
                intent_status.set_text_content(Some("Intent emitted; preview diagnostic could not update."));
            }
        }).map_err(error)?;
        let mut controls = Vec::new();
        for (mode, label) in [
            (Preview::Title, "Title"),
            (Preview::Invitation, "Invitation"),
            (Preview::Pending, "Pending"),
            (Preview::Rejected, "Rejected"),
            (Preview::Lobby, "Lobby"),
            (Preview::Reconnecting, "Reconnect"),
            (Preview::Offline, "Offline"),
            (Preview::Empty, "Empty party"),
        ] {
            let button = Rc::new(
                ControlledAction::create(
                    &document,
                    ActionView {
                        label,
                        enabled: true,
                        pending: false,
                    },
                )
                .map_err(error)?,
            );
            let owner = Rc::clone(&surface);
            let next = Rc::clone(&stamp);
            button
                .on_activate(move || {
                    let current = next.get();
                    let revision = JoinStamp {
                        generation: current.generation,
                        sequence: current.sequence.saturating_add(1),
                    };
                    match owner.update(&view(revision, mode)) {
                        Ok(JoinUpdate::Applied) => next.set(revision),
                        Ok(_) | Err(_) => {
                            if let Ok(Some(notice)) = owner.root().query_selector(".preview-badge")
                            {
                                notice.set_text_content(Some(
                                    "Preview update failed; controls retained",
                                ));
                            }
                        }
                    }
                })
                .map_err(error)?;
            surface.navigation().append_child(button.element())?;
            controls.push(button);
        }
        let checks = child(&document, surface.root(), "details", "art-gallery", "")?;
        child(&document, &checks, "summary", "", "Presentation checks")?;
        let check_status = child(
            &document,
            &checks,
            "p",
            "connection",
            "Use keyboard or mouse to enter real drafts, then exercise updates and owner replacement. These checks do not run gameplay.",
        )?;
        check_status.set_attribute("data-lifecycle-check", "pending")?;
        let verify = Rc::new(
            ControlledAction::create(
                &document,
                ActionView {
                    label: "Verify focus, drafts & stale updates",
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        let owner = Rc::clone(&surface);
        let next = Rc::clone(&stamp);
        let check_document = document.clone();
        let verification = check_status.clone();
        verify.on_activate(move || {
            let result = (|| -> Result<(), JsValue> {
                let invitation = owner.invitation().draft();
                let name = owner.player_name().draft();
                let input = owner.player_name().input().clone();
                let root = owner.root().clone();
                let mut revision = next.get();
                revision.sequence = revision.sequence.saturating_add(1);
                owner.update(&view(revision, Preview::Invitation)).map_err(error)?;
                input.focus()?;
                input.set_selection_range(0, 1)?;
                let start = input.selection_start()?;
                let end = input.selection_end()?;
                for _ in 0..64 {
                    revision.sequence = revision.sequence.saturating_add(1);
                    require(owner.update(&view(revision, Preview::Invitation)).map_err(error)? == JoinUpdate::Applied, "update was not admitted")?;
                }
                require(owner.invitation().draft() == invitation && owner.player_name().draft() == name, "draft changed")?;
                require(root.is_same_node(Some(owner.root())) && input.is_same_node(Some(owner.player_name().input())), "mounted node changed")?;
                require(check_document.active_element().as_ref().is_some_and(|active| input.is_same_node(Some(active))), "input focus changed")?;
                require(start == input.selection_start()? && end == input.selection_end()?, "selection changed")?;
                let old = JoinStamp { generation: revision.generation.saturating_sub(1), sequence: u64::MAX };
                require(owner.update(&view(old, Preview::Lobby)).map_err(error)? == JoinUpdate::StaleGeneration, "stale generation admitted")?;
                require(owner.update(&view(revision, Preview::Lobby)).map_err(error)? == JoinUpdate::StaleSequence, "repeated sequence admitted")?;
                require(owner.root().get_attribute("data-join-stage").as_deref() == Some("invitation"), "stale view changed stage")?;
                revision.sequence = revision.sequence.saturating_add(1);
                owner.update(&view(revision, Preview::Pending)).map_err(error)?;
                require(input.read_only(), "pending draft is editable")?;
                revision.sequence = revision.sequence.saturating_add(1);
                owner.update(&view(revision, Preview::Rejected)).map_err(error)?;
                require(!input.read_only() && owner.player_name().draft() == name && owner.invitation().draft() == invitation, "rejection lost drafts")?;
                require(check_document.active_element().as_ref().is_some_and(|active| input.is_same_node(Some(active))), "pending/rejection changed focus")?;
                focus_transition_check(&check_document, &owner, &mut revision)?;
                replacement_focus_check(&check_document)?;
                failed_replacement_check(&check_document)?;
                failed_ordinary_update_check(&check_document)?;
                next.set(revision);
                Ok(())
            })();
            verification.set_text_content(Some(if result.is_ok() { "PASS · 64 updates retained nodes, drafts, focus and selection; stale/pending/rejected checks passed. Lobby transitions and removed/disabled offers restored focus. Campaign/roster DOM failures cleared obsolete drafts, retired old owners, blocked intents and recovered the replacement. Ordinary failure rejected old sequence replay and recovered only the exact attempted stamp." } else { "FAIL · Focus/draft/generation or reconciliation recovery checks failed." }));
            if verification.set_attribute("data-lifecycle-check", if result.is_ok() { "pass" } else { "fail" }).is_err() {
                verification.set_text_content(Some("Lifecycle result could not be recorded."));
            }
        }).map_err(error)?;
        checks.append_child(verify.element())?;
        controls.push(verify);
        let replace = Rc::new(
            ControlledAction::create(
                &document,
                ActionView {
                    label: "Replace owner & clear drafts",
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        let owner = Rc::clone(&surface);
        let next = Rc::clone(&stamp);
        let replacement_status = check_status.clone();
        replace.on_activate(move || {
            let prior = next.get();
            let replacement = JoinStamp { generation: prior.generation.saturating_add(1), sequence: 1 };
            let result = (|| -> Result<(), JsValue> {
                owner.replace_owner(&view(replacement, Preview::Invitation)).map_err(error)?;
                require(owner.invitation().draft().is_empty() && owner.player_name().draft().is_empty(), "obsolete drafts retained")?;
                require(owner.update(&view(JoinStamp { generation: prior.generation, sequence: u64::MAX }, Preview::Lobby)).map_err(error)? == JoinUpdate::StaleGeneration, "old owner update admitted")?;
                next.set(replacement);
                Ok(())
            })();
            replacement_status.set_text_content(Some(if result.is_ok() { "PASS · Ownership replacement cleared both drafts; old generation remained fenced." } else { "FAIL · Ownership replacement check failed." }));
        }).map_err(error)?;
        checks.append_child(replace.element())?;
        controls.push(replace);
        let terminal = document.create_element("p")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("data-preview-disposal", "pending")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute(
            "style",
            "color:#f5eee0;padding:32px;font:18px Georgia,serif",
        )?;
        let dispose = Rc::new(
            ControlledAction::create(
                &document,
                ActionView {
                    label: "Dispose preview twice",
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        let terminal_status = terminal.clone();
        let terminal_document = document.clone();
        dispose.on_activate(move || {
            let result = (|| -> Result<(), JsValue> {
                shutdown()?;
                shutdown()?;
                require(terminal_document.query_selector("[data-join-phase]")?.is_none(), "join surface remains")?;
                require(!FIXTURE.with(|slot| slot.borrow().is_some()), "fixture owner remains")?;
                terminal_status.set_attribute("data-preview-disposal", "pass")?;
                terminal_status.remove_attribute("hidden")?;
                terminal_status.set_text_content(Some("PASS · Preview disposed twice. Owned controls and cinematic surface were released."));
                Ok(())
            })();
            if result.is_err() {
                terminal_status.set_text_content(Some("FAIL · Preview disposal check failed."));
                if terminal_status.remove_attribute("hidden").is_err() { terminal_status.set_text_content(Some("FAIL · Disposal diagnostic unavailable.")); }
            }
        }).map_err(error)?;
        checks.append_child(dispose.element())?;
        controls.push(dispose);
        body.append_child(surface.root())?;
        body.append_child(&terminal)?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(Fixture { surface, controls }));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        let fixture = FIXTURE.with(|slot| slot.borrow_mut().take());
        if let Some(fixture) = fixture {
            let mut failure = None;
            for control in fixture.controls {
                if let Err(value) = control.dispose() {
                    failure.get_or_insert(error(value));
                }
            }
            if let Err(value) = fixture.surface.dispose() {
                failure.get_or_insert(error(value));
            }
            if let Some(failure) = failure {
                return Err(failure);
            }
        }
        Ok(())
    }
}
