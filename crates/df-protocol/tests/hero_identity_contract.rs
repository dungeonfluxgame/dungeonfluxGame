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

#[test]
fn creator_catalog_contract_round_trips_full_design_and_preserves_old_fields() {
    use df_protocol::common::{HeroPortraitRequest, SupportedCharacterDesign};
    let legacy = HeroPortraitRequest::decode(&[66, 5, b'd', b'w', b'a', b'r', b'f'][..]).unwrap();
    assert_eq!(legacy.species, "dwarf");
    assert!(legacy.character_class.is_empty());
    assert!(legacy.background.is_empty());
    let request = HeroPortraitRequest {
        species: "aasimar".to_owned(),
        character_class: "wizard-1".to_owned(),
        background: "sage".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        HeroPortraitRequest::decode(request.encode_to_vec().as_slice()).unwrap(),
        request
    );
    let offer = CharacterCreationOffer {
        supported_designs: vec![SupportedCharacterDesign {
            species_id: "dwarf".to_owned(),
            class_id: "fighter-1".to_owned(),
            background_id: "soldier".to_owned(),
        }],
        ..Default::default()
    };
    assert_eq!(
        CharacterCreationOffer::decode(offer.encode_to_vec().as_slice()).unwrap(),
        offer
    );
}

#[test]
fn portrait_scenery_round_trips_separately_from_origin_and_old_clients_default_empty() {
    use df_protocol::common::{HeroPortraitRequest, JourneyChoice, JourneyOption};
    let scenery = "Ruins  \r\nwaterfall";
    let character = CharacterSelection {
        portrait_scenery: scenery.to_owned(),
        choices: vec![JourneyChoice {
            group_id: "background".to_owned(),
            option_id: "sage".to_owned(),
        }],
        ..Default::default()
    };
    assert_eq!(
        CharacterSelection::decode(character.encode_to_vec().as_slice()).unwrap(),
        character
    );
    assert_eq!(
        CharacterSelection::decode(&[][..])
            .unwrap()
            .portrait_scenery,
        ""
    );
    let request = HeroPortraitRequest {
        scenery: scenery.to_owned(),
        ..Default::default()
    };
    assert_eq!(
        HeroPortraitRequest::decode(request.encode_to_vec().as_slice()).unwrap(),
        request
    );
    let offer = CharacterCreationOffer {
        portrait_scenery_options: vec![JourneyOption {
            option_id: "forest".to_owned(),
            description: scenery.to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(
        CharacterCreationOffer::decode(offer.encode_to_vec().as_slice()).unwrap(),
        offer
    );
}
