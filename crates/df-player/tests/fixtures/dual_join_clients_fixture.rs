//! Reachable synthetic two-role join/lobby presentation. No credentials, live
//! transport, admission, readiness mutations or provider calls are connected.

#[cfg(target_arch = "wasm32")]
mod browser {
    use df_display::{DisplayJoinConnection, DisplayJoinInput, DisplayJoinScreen};
    use df_player::{PlayerJoinConnection, PlayerJoinInput, PlayerJoinScreen};
    use df_ui::{
        ActionView, CampaignLimits, CampaignView, ConceptScene, ControlledAction, JoinFeedback,
        JoinPhaseIntent, JoinPhaseView, JoinRoom, JoinStage, JoinStamp, JoinUpdate, LobbyOffer,
        LobbyParticipant,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, prelude::*};
    use web_sys::{Document, Element, Event, HtmlButtonElement, HtmlInputElement, Node};

    const PARTY: [LobbyParticipant<'static>; 3] = [
        LobbyParticipant {
            key: "mira",
            name: "Mira",
            sigil: "M",
            readiness: "Ready",
            presence: "Connected · remote",
        },
        LobbyParticipant {
            key: "rowan",
            name: "Rowan",
            sigil: "R",
            readiness: "Choosing",
            presence: "Reconnecting · seat retained",
        },
        LobbyParticipant {
            key: "fen",
            name: "Fen",
            sigil: "F",
            readiness: "Not ready",
            presence: "Sleeping · seat retained",
        },
    ];
    const PRIVATE: &str = "PRIVATE-JOIN-SENTINEL · Your invitation awaits a server response.";
    #[derive(Clone, Copy)]
    enum Mode {
        Invitation,
        Pending,
        Rejected,
        Lobby,
        Host,
        Empty,
    }
    #[derive(Default)]
    struct IntentRecord {
        count: usize,
        last: Option<JoinPhaseIntent>,
    }
    struct Fixture {
        player: PlayerJoinScreen,
        display: DisplayJoinScreen,
        player_stamp: JoinStamp,
        display_stamp: JoinStamp,
        player_mode: Mode,
        display_mode: Mode,
        player_intents: Rc<RefCell<IntentRecord>>,
        display_intents: Rc<RefCell<IntentRecord>>,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn error(value: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&value.to_string())
    }
    fn check(value: bool, message: &str) -> Result<(), JsValue> {
        if value {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn required(root: &Element, selector: &str) -> Result<Element, JsValue> {
        root.query_selector(selector)?
            .ok_or_else(|| JsValue::from_str("join fixture node missing"))
    }
    fn field(root: &Element, suffix: &str) -> Result<HtmlInputElement, JsValue> {
        let feedback = required(
            root,
            &format!(".join-fields .df-ui-feedback[id$='-{suffix}-feedback']"),
        )?;
        let container = feedback
            .parent_element()
            .ok_or_else(|| JsValue::from_str("join fixture field container missing"))?;
        required(&container, "label input")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("join fixture input has wrong type"))
    }
    fn click(root: &Element, selector: &str) -> Result<(), JsValue> {
        required(root, selector)?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("join fixture action has wrong type"))?
            .click();
        Ok(())
    }
    fn write_draft(input: &HtmlInputElement, value: &str) -> Result<(), JsValue> {
        input.set_value(value);
        input.dispatch_event(&Event::new("input")?)?;
        Ok(())
    }
    fn limits() -> CampaignLimits {
        CampaignLimits {
            max_members: 64,
            max_objectives: 64,
            max_text_bytes: 4096,
        }
    }
    fn view(player: bool, stamp: JoinStamp, mode: Mode) -> JoinPhaseView<'static> {
        let lobby = matches!(mode, Mode::Lobby | Mode::Host | Mode::Empty);
        let offer = if player && lobby || !player && matches!(mode, Mode::Host) {
            Some(LobbyOffer {
                key: if player {
                    "synthetic-player-ready-current"
                } else {
                    "synthetic-display-host-start-current"
                },
                label: if player {
                    "Mark yourself ready"
                } else {
                    "Begin the story · host offer"
                },
                enabled: true,
                pending: false,
            })
        } else {
            None
        };
        JoinPhaseView {
            stamp,
            stage: if lobby {
                JoinStage::Lobby
            } else {
                JoinStage::Invitation
            },
            campaign: CampaignView {
                scene: ConceptScene::Harbor,
                chapter: "Prologue · The party gathers",
                title: "The Drowned Lantern",
                description: "Rain slips between the rooftops of Greyhaven. Across the harbor, an old lantern is waiting for your company.",
                location: "Greyhaven · The old harbor",
                scene_label: "Gather your party",
                narration: if player {
                    PRIVATE
                } else {
                    "PUBLIC · Beyond the rain, a door opens and a story waits to be told together."
                },
                connection: if player {
                    "Synthetic permitted player projection · Production RPC pending"
                } else {
                    "Synthetic public display projection · Production RPC pending"
                },
                notice: "Two-role join fixture · Input emits intentions only",
                members: &[],
                objectives: &[],
            },
            panel_heading: if lobby {
                "Your party awaits"
            } else {
                "Step into the story"
            },
            panel_description: if player {
                "Your name and invitation stay on this player client. Sending input cannot confirm a seat or readiness."
            } else {
                "Public presence and readiness are supplied facts. Host controls appear only when this projection explicitly includes an offer."
            },
            invitation_label: "Room code or invitation",
            name_label: "Your player name",
            join_label: "Join the adventure",
            join_enabled: player,
            feedback: match mode {
                Mode::Pending => {
                    JoinFeedback::Pending("Synthetic response pending · no membership confirmed")
                }
                Mode::Rejected => JoinFeedback::Rejected(
                    "Synthetic invitation rejected · edit your retained drafts",
                ),
                _ => JoinFeedback::None,
            },
            room: if lobby {
                Some(JoinRoom {
                    label: "Public room · synthetic",
                    code: "LANTERN",
                    detail: "At one table, wherever you are",
                })
            } else {
                None
            },
            roster_heading: "Adventurers at the table",
            empty_roster: "The lantern is lit. Admitted adventurers appear when the server supplies them.",
            participants: if lobby && !matches!(mode, Mode::Empty) {
                &PARTY
            } else {
                &[]
            },
            away_heroes: &[],
            lobby_offer: offer,
            pairing_fallback: "Public invitation and QR component remain caller-owned. This fixture generates no credential, invitation URL or QR code.",
        }
    }
    fn advance(stamp: &mut JoinStamp) {
        stamp.sequence = stamp.sequence.saturating_add(1);
    }
    fn update(fixture: &mut Fixture, player: Mode, display: Mode) -> Result<(), JsValue> {
        advance(&mut fixture.player_stamp);
        advance(&mut fixture.display_stamp);
        check(
            fixture
                .player
                .update(&view(true, fixture.player_stamp, player))
                .map_err(error)?
                == JoinUpdate::Applied,
            "player update was not applied",
        )?;
        check(
            fixture
                .display
                .update(&view(false, fixture.display_stamp, display))
                .map_err(error)?
                == JoinUpdate::Applied,
            "display update was not applied",
        )?;
        fixture.player_mode = player;
        fixture.display_mode = display;
        Ok(())
    }
    fn collect(node: &Node, nodes: &mut Vec<Node>) {
        nodes.push(node.clone());
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            collect(&current, nodes);
        }
    }
    fn privacy(display: &Element) -> Result<(), JsValue> {
        check(
            !display.outer_html().contains("PRIVATE-JOIN-SENTINEL"),
            "private projection reached display DOM",
        )?;
        check(
            !display.outer_html().contains("PRIVATE-INVITATION"),
            "private invitation reached display DOM",
        )?;
        check(
            !display.outer_html().contains("PRIVATE-NAME"),
            "private name reached display DOM",
        )
    }
    fn retained_nodes_are_scrubbed(nodes: &[Node]) -> Result<(), JsValue> {
        for node in nodes {
            if node.node_type() == Node::TEXT_NODE {
                check(
                    node.node_value().as_deref() == Some(""),
                    "retained raw Text still contains revoked presentation",
                )?;
            }
            if let Some(element) = node.dyn_ref::<Element>() {
                check(
                    !element.outer_html().contains("PRIVATE-JOIN-SENTINEL"),
                    "retained element contains private projection after disposal",
                )?;
            }
        }
        Ok(())
    }
    fn drop_check(document: &Document, slot: &Element) -> Result<(), JsValue> {
        let events = Rc::new(RefCell::new(IntentRecord::default()));
        let observed = Rc::clone(&events);
        let screen = PlayerJoinScreen::mount(
            document,
            slot,
            &view(
                true,
                JoinStamp {
                    generation: 40,
                    sequence: 1,
                },
                Mode::Invitation,
            ),
            PlayerJoinInput {
                identifier: "join-drop-check",
                invitation: "PRIVATE-INVITATION",
                player_name: "PRIVATE-NAME",
            },
            limits(),
            PlayerJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let action = required(screen.root(), ".join-fields button")?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("drop action unavailable"))?;
        let invitation = field(screen.root(), "invitation")?;
        let name = field(screen.root(), "name")?;
        let mut nodes = Vec::new();
        collect(screen.root().as_ref(), &mut nodes);
        drop(screen);
        action.click();
        check(
            events.borrow().count == 0,
            "retained action emitted after Drop",
        )?;
        check(
            invitation.value().is_empty() && name.value().is_empty(),
            "Drop retained private draft native values",
        )?;
        retained_nodes_are_scrubbed(&nodes)?;
        let observed = Rc::clone(&events);
        let display = DisplayJoinScreen::mount(
            document,
            slot,
            &view(
                false,
                JoinStamp {
                    generation: 40,
                    sequence: 1,
                },
                Mode::Host,
            ),
            DisplayJoinInput {
                identifier: "join-display-drop-check",
                invitation: "",
                player_name: "",
            },
            limits(),
            DisplayJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let host = required(display.root(), ".join-lobby button")?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("display Drop action unavailable"))?;
        let mut nodes = Vec::new();
        collect(display.root().as_ref(), &mut nodes);
        drop(display);
        host.click();
        check(
            events.borrow().count == 0,
            "retained display host action emitted after Drop",
        )?;
        retained_nodes_are_scrubbed(&nodes)
    }
    fn failed_replacement_check(document: &Document, slot: &Element) -> Result<(), JsValue> {
        let events = Rc::new(RefCell::new(IntentRecord::default()));
        let observed = Rc::clone(&events);
        let initial = JoinStamp {
            generation: 60,
            sequence: 1,
        };
        let mut player = PlayerJoinScreen::mount(
            document,
            slot,
            &view(true, initial, Mode::Invitation),
            PlayerJoinInput {
                identifier: "join-invalid-replacement",
                invitation: "PRIVATE-INVITATION",
                player_name: "PRIVATE-NAME",
            },
            limits(),
            PlayerJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let invitation = field(player.root(), "invitation")?;
        let action = required(player.root(), ".join-fields button")?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("invalid replacement action unavailable"))?;
        let mut nodes = Vec::new();
        collect(player.root().as_ref(), &mut nodes);
        let replacement_stamp = JoinStamp {
            generation: 61,
            sequence: 1,
        };
        let mut replacement = view(true, replacement_stamp, Mode::Invitation);
        replacement.panel_heading = "";
        check(
            player.replace_owner(&replacement).is_err(),
            "invalid new owner was accepted",
        )?;
        check(
            player
                .set_connection(PlayerJoinConnection::Connected)
                .is_err(),
            "reconnect revived rejected new ownership",
        )?;
        check(
            player
                .update(&view(true, initial, Mode::Invitation))
                .is_err(),
            "old scope revived revoked private mount",
        )?;
        check(
            invitation.value().is_empty(),
            "failed replacement retained private draft native value",
        )?;
        action.click();
        check(
            events.borrow().count == 0,
            "failed replacement retained actionable old callback",
        )?;
        retained_nodes_are_scrubbed(&nodes)?;
        let observed = Rc::clone(&events);
        let mut display = DisplayJoinScreen::mount(
            document,
            slot,
            &view(false, initial, Mode::Host),
            DisplayJoinInput {
                identifier: "join-display-invalid-replacement",
                invitation: "",
                player_name: "",
            },
            limits(),
            DisplayJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let action = required(display.root(), ".join-lobby button")?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("display invalid replacement action unavailable"))?;
        let mut nodes = Vec::new();
        collect(display.root().as_ref(), &mut nodes);
        let mut replacement = view(false, replacement_stamp, Mode::Host);
        replacement.panel_heading = "";
        check(
            display.replace_owner(&replacement).is_err(),
            "invalid display owner was accepted",
        )?;
        check(
            display
                .set_connection(DisplayJoinConnection::Connected)
                .is_err(),
            "reconnect revived rejected display owner",
        )?;
        action.click();
        check(
            events.borrow().count == 0,
            "failed display replacement retained host callback",
        )?;
        retained_nodes_are_scrubbed(&nodes)
    }

    fn exercise(fixture: &mut Fixture, document: &Document) -> Result<(), JsValue> {
        fixture
            .player
            .set_connection(PlayerJoinConnection::Connected)
            .map_err(error)?;
        fixture
            .display
            .set_connection(DisplayJoinConnection::Connected)
            .map_err(error)?;
        update(fixture, Mode::Invitation, Mode::Lobby)?;
        let player_root = fixture.player.root().clone();
        let display_root = fixture.display.root().clone();
        let input = field(&player_root, "name")?;
        let invitation = field(&player_root, "invitation")?;
        write_draft(&input, "PRIVATE-NAME · Mara")?;
        write_draft(&invitation, "PRIVATE-INVITATION · lantern")?;
        input.focus()?;
        input.set_selection_range(2, 6)?;
        let selection = (input.selection_start()?, input.selection_end()?);
        let retired_text = required(&player_root, "blockquote")?
            .first_child()
            .ok_or_else(|| JsValue::from_str("raw narrative Text unavailable"))?;
        for _ in 0..64 {
            update(fixture, Mode::Invitation, Mode::Lobby)?;
        }
        check(
            player_root.is_same_node(Some(fixture.player.root()))
                && display_root.is_same_node(Some(fixture.display.root())),
            "role root changed during ordinary updates",
        )?;
        let current_input = field(fixture.player.root(), "name")?;
        let current_input_node: &Node = current_input.as_ref();
        check(
            input.is_same_node(Some(current_input_node)),
            "input node changed during updates",
        )?;
        check(
            input.value() == "PRIVATE-NAME · Mara"
                && invitation.value() == "PRIVATE-INVITATION · lantern",
            "updates changed valid drafts",
        )?;
        check(
            document
                .active_element()
                .as_ref()
                .is_some_and(|node| input.is_same_node(Some(node))),
            "same-phase update lost focus",
        )?;
        check(
            selection == (input.selection_start()?, input.selection_end()?),
            "same-phase update changed selection",
        )?;
        check(
            retired_text.node_value().as_deref() == Some(""),
            "updated detached raw narrative Text was not scrubbed",
        )?;
        let stamp = fixture.player_stamp;
        let before = fixture.player_intents.borrow().count;
        click(fixture.player.root(), ".join-fields button")?;
        check(
            fixture.player_intents.borrow().last.as_ref()
                == Some(&JoinPhaseIntent::Join {
                    stamp,
                    invitation: "PRIVATE-INVITATION · lantern".into(),
                    player_name: "PRIVATE-NAME · Mara".into(),
                }),
            "join intent changed drafts or current stamp",
        )?;
        check(
            fixture.player_intents.borrow().count == before + 1,
            "same-phase updates accumulated listeners",
        )?;
        let public_before = fixture.display_intents.borrow().count;
        click(fixture.display.root(), ".join-fields button")?;
        click(fixture.display.root(), ".join-lobby button")?;
        check(
            fixture.display_intents.borrow().count == public_before,
            "display invented join or host permission",
        )?;
        privacy(fixture.display.root())?;
        update(fixture, Mode::Pending, Mode::Lobby)?;
        click(fixture.player.root(), ".join-fields button")?;
        check(
            fixture.player_intents.borrow().count == before + 1 && input.read_only(),
            "pending offer accepted input",
        )?;
        update(fixture, Mode::Rejected, Mode::Lobby)?;
        check(
            !input.read_only() && input.value() == "PRIVATE-NAME · Mara",
            "rejection lost editable draft",
        )?;
        input.focus()?;
        fixture
            .player
            .set_connection(PlayerJoinConnection::Offline)
            .map_err(error)?;
        fixture
            .display
            .set_connection(DisplayJoinConnection::Offline)
            .map_err(error)?;
        click(fixture.player.root(), ".join-fields button")?;
        check(
            fixture.player_intents.borrow().count == before + 1,
            "offline input escaped callback fence",
        )?;
        fixture
            .player
            .set_connection(PlayerJoinConnection::Reconnecting)
            .map_err(error)?;
        fixture
            .display
            .set_connection(DisplayJoinConnection::Reconnecting)
            .map_err(error)?;
        check(
            fixture
                .player
                .update(&view(true, stamp, Mode::Lobby))
                .map_err(error)?
                == JoinUpdate::StaleSequence,
            "stale reconnect view was accepted",
        )?;
        click(fixture.player.root(), ".join-fields button")?;
        check(
            fixture.player_intents.borrow().count == before + 1,
            "stale view resumed reconnect input",
        )?;
        fixture
            .player
            .set_connection(PlayerJoinConnection::Connected)
            .map_err(error)?;
        fixture
            .display
            .set_connection(DisplayJoinConnection::Connected)
            .map_err(error)?;
        check(
            input.value() == "PRIVATE-NAME · Mara"
                && document
                    .active_element()
                    .as_ref()
                    .is_some_and(|node| input.is_same_node(Some(node))),
            "reconnect lost valid draft or focus",
        )?;
        update(fixture, Mode::Lobby, Mode::Host)?;
        click(fixture.player.root(), ".join-lobby button")?;
        click(fixture.display.root(), ".join-lobby button")?;
        check(
            fixture.player_intents.borrow().last.as_ref()
                == Some(&JoinPhaseIntent::LobbyAction {
                    stamp: fixture.player_stamp,
                    offer_key: "synthetic-player-ready-current".into(),
                }),
            "player altered advertised lobby offer",
        )?;
        check(
            fixture.display_intents.borrow().last.as_ref()
                == Some(&JoinPhaseIntent::LobbyAction {
                    stamp: fixture.display_stamp,
                    offer_key: "synthetic-display-host-start-current".into(),
                }),
            "display altered advertised host offer",
        )?;
        let host_before = fixture.display_intents.borrow().count;
        let obsolete_host = required(fixture.display.root(), ".join-lobby button")?
            .dyn_into::<HtmlButtonElement>()
            .map_err(|_| JsValue::from_str("host action unavailable"))?;
        update(fixture, Mode::Lobby, Mode::Lobby)?;
        obsolete_host.click();
        check(
            fixture.display_intents.borrow().count == host_before,
            "removed host offer remained actionable",
        )?;
        let mut pending = view(true, fixture.player_stamp, Mode::Lobby);
        pending.stamp.sequence = pending.stamp.sequence.saturating_add(1);
        if let Some(offer) = &mut pending.lobby_offer {
            offer.pending = true;
        }
        check(
            fixture.player.update(&pending).map_err(error)? == JoinUpdate::Applied,
            "pending lobby view rejected",
        )?;
        fixture.player_stamp = pending.stamp;
        let pending_before = fixture.player_intents.borrow().count;
        click(fixture.player.root(), ".join-lobby button")?;
        check(
            fixture.player_intents.borrow().count == pending_before,
            "pending readiness intent escaped",
        )?;
        update(fixture, Mode::Lobby, Mode::Lobby)?;
        // Reproduce an actual keyed DOM failure rather than relying on native enums.
        required(fixture.player.root(), "[data-participant-key=mira]")?.remove();
        advance(&mut fixture.player_stamp);
        check(
            fixture
                .player
                .update(&view(true, fixture.player_stamp, Mode::Empty))
                .is_err(),
            "ordinary keyed DOM failure not triggered",
        )?;
        fixture
            .player
            .set_connection(PlayerJoinConnection::Connected)
            .map_err(error)?;
        click(fixture.player.root(), ".join-lobby button")?;
        check(
            fixture.player_intents.borrow().count == pending_before,
            "reconnect revived failed render offers",
        )?;
        let old = JoinStamp {
            generation: fixture.player_stamp.generation,
            sequence: fixture.player_stamp.sequence.saturating_sub(1),
        };
        check(
            fixture
                .player
                .update(&view(true, old, Mode::Lobby))
                .map_err(error)?
                == JoinUpdate::StaleSequence,
            "committed replay revived a failed attempt",
        )?;
        check(
            fixture
                .player
                .update(&view(true, fixture.player_stamp, Mode::Empty))
                .map_err(error)?
                == JoinUpdate::Applied,
            "exact failed stamp could not recover",
        )?;
        fixture.player_mode = Mode::Empty;
        click(fixture.player.root(), ".join-lobby button")?;
        check(
            fixture.player_intents.borrow().count == pending_before + 1,
            "recovered current offer unavailable",
        )?;
        let prior_player = fixture.player_stamp;
        let prior_display = fixture.display_stamp;
        fixture.player_stamp = JoinStamp {
            generation: prior_player.generation.saturating_add(1),
            sequence: 1,
        };
        fixture.display_stamp = JoinStamp {
            generation: prior_display.generation.saturating_add(1),
            sequence: 1,
        };
        let old_private_text = required(fixture.player.root(), "blockquote")?
            .first_child()
            .ok_or_else(|| JsValue::from_str("replacement narrative Text unavailable"))?;
        fixture
            .player
            .replace_owner(&view(true, fixture.player_stamp, Mode::Invitation))
            .map_err(error)?;
        fixture
            .display
            .replace_owner(&view(false, fixture.display_stamp, Mode::Lobby))
            .map_err(error)?;
        fixture.player_mode = Mode::Invitation;
        fixture.display_mode = Mode::Lobby;
        fixture.player_intents.borrow_mut().last = None;
        fixture.display_intents.borrow_mut().last = None;
        check(
            input.value().is_empty() && invitation.value().is_empty(),
            "ownership replacement retained obsolete private drafts",
        )?;
        check(
            old_private_text.node_value().as_deref() == Some(""),
            "replacement retained raw private Text",
        )?;
        check(
            fixture
                .player
                .update(&view(
                    true,
                    JoinStamp {
                        generation: prior_player.generation,
                        sequence: u64::MAX,
                    },
                    Mode::Lobby,
                ))
                .map_err(error)?
                == JoinUpdate::StaleGeneration,
            "obsolete binding generation returned",
        )?;
        privacy(fixture.display.root())?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("drop fixture body unavailable"))?;
        drop_check(document, &body)?;
        failed_replacement_check(document, &body)?;
        Ok(())
    }
    fn with_fixture(
        action: impl FnOnce(&mut Fixture) -> Result<(), JsValue>,
    ) -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owner = slot.borrow_mut();
            action(
                owner
                    .as_mut()
                    .ok_or_else(|| JsValue::from_str("join fixture disposed"))?,
            )
        })
    }
    fn status(document: &Document, result: Result<(), JsValue>, success: &str) {
        if let Some(node) = document.get_element_by_id("dual-join-status") {
            node.set_text_content(Some(if result.is_ok() {
                success
            } else {
                "FAIL · Mounted join fixture operation failed"
            }));
            if node
                .set_attribute("data-result", if result.is_ok() { "PASS" } else { "FAIL" })
                .is_err()
            {
                node.set_text_content(Some("FAIL · Join result attribute unavailable"));
            }
        }
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        check(
            !FIXTURE.with(|slot| slot.borrow().is_some()),
            "join fixture already mounted",
        )?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        document.set_title("DungeonFlux · Two-role join and lobby fixture");
        body.set_attribute("style", "margin:0;background:#070d14;color:#e9ddc8")?;
        let toolbar = document.create_element("nav")?;
        toolbar.set_attribute("aria-label", "Synthetic join fixture operations")?;
        toolbar.set_attribute(
            "style",
            "padding:16px;display:flex;flex-wrap:wrap;gap:8px;background:#111c2a",
        )?;
        let style = document.create_element("style")?;
        style.set_text_content(Some("#dual-join-toolbar button{min-height:44px;padding:10px 14px;border:1px solid #c3a26b;border-radius:4px;background:#182b3c;color:#f3d89f;font:13px system-ui;cursor:pointer}#dual-join-toolbar button:focus-visible{outline:2px solid #a3d9fb;outline-offset:3px}#dual-join-status{padding:16px;margin:0;font:14px/1.7 system-ui;overflow-wrap:anywhere}"));
        toolbar.set_id("dual-join-toolbar");
        toolbar.append_child(&style)?;
        let result = document.create_element("p")?;
        result.set_id("dual-join-status");
        result.set_attribute("role", "status")?;
        result.set_attribute("data-result", "ready")?;
        result.set_text_content(Some("Two separately constructed filtered projections · Synthetic input only · No server admission"));
        let player_slot = document.create_element("main")?;
        player_slot.set_id("join-player-slot");
        let display_slot = document.create_element("section")?;
        display_slot.set_id("join-display-slot");
        body.append_child(&toolbar)?;
        body.append_child(&result)?;
        body.append_child(&player_slot)?;
        body.append_child(&display_slot)?;
        let player_stamp = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        let display_stamp = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        let player_intents = Rc::new(RefCell::new(IntentRecord::default()));
        let display_intents = Rc::new(RefCell::new(IntentRecord::default()));
        let observed = Rc::clone(&player_intents);
        let player = PlayerJoinScreen::mount(
            &document,
            &player_slot,
            &view(true, player_stamp, Mode::Invitation),
            PlayerJoinInput {
                identifier: "join-player",
                invitation: "",
                player_name: "",
            },
            limits(),
            PlayerJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let observed = Rc::clone(&display_intents);
        let display = DisplayJoinScreen::mount(
            &document,
            &display_slot,
            &view(false, display_stamp, Mode::Lobby),
            DisplayJoinInput {
                identifier: "join-display",
                invitation: "",
                player_name: "",
            },
            limits(),
            DisplayJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                record.count = record.count.saturating_add(1);
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let mut fixture = Fixture {
            player,
            display,
            player_stamp,
            display_stamp,
            player_mode: Mode::Invitation,
            display_mode: Mode::Lobby,
            player_intents,
            display_intents,
            controls: Vec::new(),
        };
        for (operation, label) in [
            (0_u8, "Same-phase update"),
            (1, "Pending join"),
            (2, "Rejected join"),
            (3, "Lobby and host offers"),
            (4, "Public display without host offer"),
            (5, "Offline"),
            (6, "Reconnecting"),
            (7, "Reconnect"),
            (8, "Replace ownership and clear drafts"),
            (9, "Verify mounted lifecycle"),
            (10, "Fail optional art"),
            (11, "Restore optional art"),
            (12, "Revoke both clients"),
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
            let observed_document = document.clone();
            control.on_activate(move || {
                let outcome = match operation {
                    9 => with_fixture(|fixture| exercise(fixture, &observed_document)),
                    12 => dispose_fixture(),
                    _ => with_fixture(|fixture| {
                        match operation {
                            0 => { let modes = (fixture.player_mode, fixture.display_mode); update(fixture, modes.0, modes.1)?; }
                            1 => { let mode = fixture.display_mode; update(fixture, Mode::Pending, mode)?; }
                            2 => { let mode = fixture.display_mode; update(fixture, Mode::Rejected, mode)?; }
                            3 => update(fixture, Mode::Lobby, Mode::Host)?,
                            4 => { let mode = fixture.player_mode; update(fixture, mode, Mode::Lobby)?; }
                            5..=7 => {
                                fixture.player.set_connection(match operation { 5 => PlayerJoinConnection::Offline, 6 => PlayerJoinConnection::Reconnecting, _ => PlayerJoinConnection::Connected }).map_err(error)?;
                                fixture.display.set_connection(match operation { 5 => DisplayJoinConnection::Offline, 6 => DisplayJoinConnection::Reconnecting, _ => DisplayJoinConnection::Connected }).map_err(error)?;
                            }
                            8 => {
                                fixture.player_stamp = JoinStamp { generation: fixture.player_stamp.generation.saturating_add(1), sequence: 1 };
                                fixture.display_stamp = JoinStamp { generation: fixture.display_stamp.generation.saturating_add(1), sequence: 1 };
                                fixture.player.replace_owner(&view(true, fixture.player_stamp, Mode::Invitation)).map_err(error)?;
                                fixture.display.replace_owner(&view(false, fixture.display_stamp, Mode::Lobby)).map_err(error)?;
                                fixture.player_mode = Mode::Invitation; fixture.display_mode = Mode::Lobby;
                                fixture.player_intents.borrow_mut().last = None;
                                fixture.display_intents.borrow_mut().last = None;
                            }
                            10 | 11 => {
                                for screen in [fixture.player.root(), fixture.display.root()] {
                                    required(screen, ".scene-art")?.set_attribute("src", if operation == 10 { "assets/concept-art/fixture-missing-join-art.webp" } else { ConceptScene::Harbor.asset_path() })?;
                                    required(screen, ".narrator")?.set_attribute("src", if operation == 10 { "assets/concept-art/fixture-missing-join-avatar.webp" } else { "assets/concept-art/dm-avatar.webp" })?;
                                }
                            }
                            _ => return Err(JsValue::from_str("unknown join fixture operation")),
                        }
                        privacy(fixture.display.root())
                    }),
                };
                status(&observed_document, outcome, if operation == 9 { "PASS · 64 mounted updates; exact inputs; pending, rejected and transport fences; current offers; ordinary DOM failure recovery; ownership replacement; retained Text privacy; Drop disposal" } else { "PASS · Controlled join fixture operation completed; no game state was resolved" });
            }).map_err(error)?;
            toolbar.append_child(control.element())?;
            fixture.controls.push(control);
        }
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn exercise_join_mounts() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let outcome = with_fixture(|fixture| exercise(fixture, &document));
        status(
            &document,
            outcome.clone(),
            "PASS · Mounted join lifecycle checks completed",
        );
        outcome
    }
    #[wasm_bindgen]
    pub fn dispose_fixture() -> Result<(), JsValue> {
        let fixture = FIXTURE.with(|slot| slot.borrow_mut().take());
        if let Some(mut fixture) = fixture {
            let mut player_nodes = Vec::new();
            collect(fixture.player.root().as_ref(), &mut player_nodes);
            let mut display_nodes = Vec::new();
            collect(fixture.display.root().as_ref(), &mut display_nodes);
            let before = (
                fixture.player_intents.borrow().count,
                fixture.display_intents.borrow().count,
            );
            let player_action = required(fixture.player.root(), ".join-fields button")?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("retained player action unavailable"))?;
            let display_action = required(fixture.display.root(), ".join-lobby button")?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("retained display action unavailable"))?;
            let mut failure = None;
            for control in &fixture.controls {
                if let Err(value) = control.dispose() {
                    failure.get_or_insert(error(value));
                }
            }
            if let Err(value) = fixture.player.revoke() {
                failure.get_or_insert(error(value));
            }
            if let Err(value) = fixture.display.revoke() {
                failure.get_or_insert(error(value));
            }
            if let Err(value) = fixture.player.revoke() {
                failure.get_or_insert(error(value));
            }
            if let Err(value) = fixture.display.revoke() {
                failure.get_or_insert(error(value));
            }
            player_action.click();
            display_action.click();
            if let Err(value) = check(
                before
                    == (
                        fixture.player_intents.borrow().count,
                        fixture.display_intents.borrow().count,
                    ),
                "revoked retained action emitted",
            ) {
                failure.get_or_insert(value);
            }
            if let Err(value) = retained_nodes_are_scrubbed(&player_nodes) {
                failure.get_or_insert(value);
            }
            if let Err(value) = retained_nodes_are_scrubbed(&display_nodes) {
                failure.get_or_insert(value);
            }
            if let Some(failure) = failure {
                return Err(failure);
            }
        }
        Ok(())
    }
}
