//! The actual async fight submitter prepares a destination and publishes only committed spills.
use super::*;
use crate::gameplay::{dialogue::SceneFightOrigin, fight_foresight};

struct ArenaJev(usize);

impl fight_foresight::Jev for ArenaJev {
    fn forecast<'a>(
        &'a self,
        state: &'a Value,
        _choices: &'a [String],
    ) -> fight_foresight::ChoiceFuture<'a> {
        assert_eq!(state["fight_starting"], true);
        Box::pin(async { Ok(self.0) })
    }
}

struct InstalledArena;

impl Drop for InstalledArena {
    fn drop(&mut self) {
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = None);
    }
}

struct FightOwner {
    checkpoint: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    table: Arc<TableLog>,
    replayed: bool,
    refused: bool,
}

impl Reducer<actor::Call> for FightOwner {
    fn reduce(&mut self, _sequence: AdmissionSequence, input: actor::Call) {
        match input {
            actor::Call::SpeechMember { reply, .. } => {
                let _ = reply.send(Ok(native::first()));
            }
            actor::Call::FightAftermath {
                expected_basis,
                preparation: Some(preparation),
                reply,
                ..
            } => {
                assert_eq!(expected_basis, self.checkpoint.basis());
                let saved = journey::living_scene::prepared_fight(
                    self.checkpoint.state(),
                    &preparation.fight_id,
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    serde_json::to_value(saved).unwrap(),
                    serde_json::to_value(preparation).unwrap(),
                );
                let _ = reply.send(Ok(self.checkpoint.clone()));
            }
            actor::Call::Submit { request, reply, .. } => {
                let operation = OperationId::from_bytes(
                    request
                        .operation_id
                        .as_ref()
                        .unwrap()
                        .value
                        .as_ref()
                        .unwrap(),
                )
                .unwrap();
                if self.refused {
                    let _ = reply.send(Ok(wire::committed(wire::rejected(
                        self.checkpoint.basis(),
                        operation,
                        rpc::RejectionCode::InvalidSelection,
                    ))));
                    return;
                }
                let input =
                    wire::journey_input(&request, native::first(), &self.checkpoint).unwrap();
                let mut faces = [20, 19, 1, 1, 1, 1].into_iter();
                let committed =
                    journey::stage_with_supplier(&self.checkpoint, &input, &mut |sides| {
                        Ok(if sides == 20 {
                            faces.next().unwrap()
                        } else {
                            sides.min(3)
                        })
                    })
                    .unwrap();
                let accepted =
                    journey::accepted(committed.state().decisions.last().unwrap()).unwrap();
                let receipt = wire::receipt(
                    committed.basis(),
                    operation,
                    rpc::decision_receipt::Outcome::Accepted(Box::new(accepted)),
                    self.replayed,
                );
                self.checkpoint = committed.clone();
                self.updates.send_replace(committed);
                // The owner's combat publication happens before the submitter receives its receipt.
                self.table.push(
                    "Jessa",
                    rpc::TableRole::Npc,
                    0,
                    rpc::TableKind::Action,
                    "moves 1 square (5 ft)",
                );
                let _ = reply.send(Ok(wire::committed(receipt)));
            }
            _ => panic!("the arena adapter must never submit a standalone place move"),
        }
    }
}

fn fight_table(
    current: Checkpoint,
    loaded: pack::LoadedPack,
    voice: Arc<NpcVoice>,
    director: Arc<fight_foresight::FightForesight>,
    replayed: bool,
    refused: bool,
) -> Table {
    let (updates_in, updates) = watch::channel(current.clone());
    let (sender, inbox): (_, ActorLoop<actor::Call>) = bounded_inbox();
    let table = TableLog::new();
    let service = Service {
        portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
        display_credential: [0x82; 32],
        layout: crate::gameplay::dev_control::layout::LayoutState::new(),
        actor: sender.clone(),
        updates: updates.clone(),
        streams: Arc::new(Semaphore::new(2)),
        voice: Voice::disabled(VoiceOff::Qualification),
        npc_voice: voice,
        table_log: table.clone(),
        social: SocialLedger::none(),
        living: LivingScenes::disabled(),
    };
    let actor_table = table.clone();
    let owner = std::thread::spawn(move || {
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = Some(director));
        pack::with_test_pack(loaded, || {
            let mut owner = FightOwner {
                checkpoint: current,
                updates: updates_in,
                table: actor_table,
                replayed,
                refused,
            };
            let _ = inbox.run(&mut owner);
        });
    });
    Table {
        service,
        table,
        updates,
        owner: Some(owner),
        sender,
    }
}

fn check_publication(choice: usize, replayed: bool, refused: bool, creation_fails: bool) {
    with_shop(|opening, loaded| {
        // Pin the prerequisite before the baseline used for the single assault decision.
        let scene = runtime::scene(opening.state()).unwrap();
        let possessions = scene
            .cast
            .iter()
            .filter_map(|person| person.stat_block.as_ref().map(|profile| (person, profile)))
            .map(|(person, profile)| {
                (
                    person.id.clone(),
                    crate::gameplay::social::fight_witness::stat_block_possessions(profile)
                        .unwrap(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let preparation = serde_json::from_value(json!({
            "fight_id": journey::hex(&[0x77; 16]),
            "place": scene.setting.as_ref().unwrap().place_id,
            "scene": scene.key,
            "possessions": possessions,
        }))
        .unwrap();
        let opening =
            journey::living_scene::commit_fight_preparation(&opening, operation(0x76), preparation)
                .unwrap();
        let director = Arc::new(fight_foresight::FightForesight::new(Box::new(ArenaJev(
            choice,
        ))));
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = Some(director.clone()));
        let _installed = InstalledArena;
        let exits = FakeEndpoint::start(vec![]);
        let story_replies = if creation_fails {
            vec![
                Reply::Status(500, json!({"error": "unavailable"})),
                Reply::Status(500, json!({"error": "unavailable"})),
            ]
        } else {
            vec![Reply::Output(answer(
                &place_slot(),
                &people_slot(),
                &things_slot(),
            ))]
        };
        let storyteller = FakeEndpoint::start(story_replies);
        let cache = cache_directory(&format!(
            "arena-opening-{choice}-{replayed}-{refused}-{creation_fails}"
        ));
        let table = fight_table(
            opening.clone(),
            loaded,
            voice_of(
                &exits,
                teller(storyteller.address, 0, Duration::from_secs(5), &cache),
            ),
            director,
            replayed,
            refused,
        );
        let request = line(&table, 0x77, "I attack Corin.");
        let result = run(table.service.start_scene_fight(
            [1; 32],
            SceneFightOrigin::of(&request),
            "shop-adventurer",
            &request.text,
        ));
        assert_eq!(result.is_err(), refused);
        let rows = table.table.lines();
        let spill = rows
            .iter()
            .filter(|row| row.text == "The fight spills out into Market Street.")
            .collect::<Vec<_>>();
        let moved = choice == 2 && !creation_fails && !refused;
        assert_eq!(spill.len(), usize::from(moved && !replayed));
        assert_eq!(
            place::entries(table.current().state()).unwrap().len(),
            usize::from(moved)
        );
        if moved && !replayed {
            assert_eq!(spill[0].speaker, "Narrator");
            assert_eq!(spill[0].role, rpc::TableRole::Narrator as i32);
            assert_eq!(spill[0].kind, rpc::TableKind::Action as i32);
            let combat = rows.iter().find(|row| row.speaker == "Jessa").unwrap();
            assert!(spill[0].sequence < combat.sequence);
            assert!(result.unwrap().narration.contains(&spill[0].text));
        }
        if refused || replayed {
            assert!(rows.iter().all(|row| row.speaker != "Narrator"));
        }
        if !refused {
            assert_eq!(
                table.current().state().decisions.len(),
                opening.state().decisions.len() + 1
            );
        }
    });
}

#[test]
fn start_scene_fight_publishes_exactly_one_spill_row_after_a_moved_fight_and_none_after_a_flat_one()
{
    check_publication(2, false, false, false);
    check_publication(3, false, false, false);
}

#[test]
fn start_scene_fight_drops_reserved_narration_after_refusal_or_replay() {
    check_publication(2, false, true, false);
    check_publication(2, true, false, false);
}

#[test]
fn start_scene_fight_submits_a_flat_fight_when_destination_creation_fails() {
    check_publication(2, false, false, true);
}
