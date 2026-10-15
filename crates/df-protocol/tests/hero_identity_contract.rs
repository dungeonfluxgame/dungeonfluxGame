use df_protocol::common::{
    CharacterCreationOffer, CharacterSelection, HeroAppearance, HeroIdentity, JourneyGroup,
};
use prost::Message;

#[test]
fn character_selection_keeps_absent_identity_distinct_from_an_empty_choice() {
    let absent = CharacterSelection::default();
    let chosen = CharacterSelection {
        identity: Some(HeroIdentity {
            appearance: Some(HeroAppearance::default()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let absent_bytes = absent.encode_to_vec();
    let chosen_bytes = chosen.encode_to_vec();

    assert_ne!(absent_bytes, chosen_bytes);
    assert_eq!(
        CharacterSelection::decode(absent_bytes.as_slice())
            .unwrap()
            .identity,
        None
    );
    assert_eq!(
        CharacterSelection::decode(chosen_bytes.as_slice())
            .unwrap()
            .identity,
        chosen.identity
    );
}

#[test]
fn creation_offer_round_trips_identity_groups_and_server_defaults() {
    let offer = CharacterCreationOffer {
        identity_groups: vec![JourneyGroup {
            group_id: "origin".to_owned(),
            ..Default::default()
        }],
        default_identity: Some(HeroIdentity {
            origin: "From the coast".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let decoded = CharacterCreationOffer::decode(offer.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded.identity_groups[0].group_id, "origin");
    assert_eq!(decoded.default_identity, offer.default_identity);
}
