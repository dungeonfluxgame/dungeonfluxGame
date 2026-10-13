use super::{adventure, beat_script_runtime, beat_script_tests, journey, model, pack, wire};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::MemberId;

#[test]
fn ended_players_have_one_restart_offer_and_only_one_durable_restart_decision() {
    let tables = adventure::Tables::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables"),
    )
    .unwrap();
    let mut generated = adventure::generate(
        &tables,
        42,
        adventure::GenerateOptions::default(),
        &adventure::ForcedEntries::from([("chapter.type".to_owned(), "mystery".to_owned())]),
    )
    .unwrap();
    generated.tuning.tactical = None;
    let documents = generated.documents(true).unwrap();
    let (loaded, _) = adventure::record::reload(&documents).unwrap();
    pack::with_test_pack(loaded, || {
        let first = MemberId::from_bytes(&[0x61; 16]).unwrap();
        let second = MemberId::from_bytes(&[0x62; 16]).unwrap();
        let mut current = super::beat_script_variety_tests::prepared();
        for (operation, id) in [
            (10, "begin-story"),
            (11, "look-crates"),
            (12, "look-desk"),
            (13, "leave-investigation"),
            (14, "choose-harbor-scene"),
            (15, "resolve-errand"),
            (16, "close-epilogue"),
        ] {
            current = super::beat_script_variety_tests::act(&current, first, operation, id);
        }
        assert!(beat_script_runtime::ended(current.state()).unwrap());
        let before = current.clone();
        for member in [first, second] {
            let offers = beat_script_runtime::view_offers(&current, member).unwrap();
            assert_eq!(offers.len(), 1);
            assert_eq!(offers[0].0, rpc::GameplayActionKind::StartNewAdventure);
            assert_eq!(offers[0].1, "Start a new adventure");
        }
        let input = beat_script_tests::input(&current, first, 60, "start-new-adventure", vec![]);
        current = journey::stage_with_supplier(&current, &input, &mut |_| {
            panic!("restart draws no dice")
        })
        .unwrap();
        current = super::beat_script_variety_tests::persisted(&current);
        assert!(journey::new_adventure_requested(&current));
        assert_eq!(current.state().characters, before.state().characters);
        assert_eq!(
            current.state().decisions.len(),
            before.state().decisions.len() + 1
        );
        assert!(
            beat_script_runtime::view_offers(&current, second)
                .unwrap()
                .is_empty()
        );
        let second_input =
            beat_script_tests::input(&current, second, 61, "start-new-adventure", vec![]);
        assert_eq!(
            journey::stage_with_supplier(&current, &second_input, &mut |_| panic!(
                "refused restart draws no dice"
            )),
            Err(RepositoryError::InvalidCandidate)
        );
        assert!(
            wire::journey_view(
                &current,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                first
            )
            .is_ok()
        );
        assert_eq!(
            journey::session_machine::SessionState::of(&current).unwrap(),
            journey::session_machine::SessionState::Preparing
        );
        assert!(
            journey::stage_with_supplier(
                &before,
                &beat_script_tests::input(
                    &before,
                    model::member().unwrap(),
                    62,
                    "start-new-adventure",
                    vec![]
                ),
                &mut |_| panic!("unauthorized restart draws no dice")
            )
            .is_err()
        );
    });
}
