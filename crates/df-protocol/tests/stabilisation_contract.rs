use df_protocol::common as rpc;
use prost::Message;

#[test]
fn stabilisation_action_values_and_outcome_payload_round_trip() {
    // The wire actions represent SRD 5.2.1 p18 stabilization and p97 Healer's Kit use.
    assert_eq!(rpc::GameplayActionKind::StabiliseWithHealersKit as i32, 22);
    assert_eq!(rpc::GameplayActionKind::StabiliseMedicine as i32, 23);
    assert_eq!(rpc::OutcomeKind::Stabilisation as i32, 6);

    let outcomes = [
        rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Stabilisation as i32,
            actor_name: "Brakka".to_owned(),
            target_name: "Sigrun".to_owned(),
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::HealersKit as i32,
                die: 0,
                wisdom_modifier: 0,
                dc: 0,
                succeeded: true,
                uses_left: 9,
            }),
            ..Default::default()
        },
        rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Stabilisation as i32,
            actor_name: "Brakka".to_owned(),
            target_name: "Sigrun".to_owned(),
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::Medicine as i32,
                die: 7,
                wisdom_modifier: 1,
                dc: 10,
                succeeded: false,
                uses_left: 0,
            }),
            ..Default::default()
        },
    ];

    for outcome in outcomes {
        let encoded = outcome.encode_to_vec();
        let decoded = rpc::CombatOutcome::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded, outcome);
    }
}
