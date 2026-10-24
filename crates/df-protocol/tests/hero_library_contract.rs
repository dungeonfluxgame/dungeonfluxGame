use df_protocol::common as rpc;
use prost::Message;

#[test]
fn library_identity_round_trips_without_campaign_mechanics() {
    let identity = rpc::LibraryHeroIdentity {
        name: "Aster".into(),
        design: vec![rpc::JourneyChoice {
            group_id: "species".into(),
            option_id: "elf".into(),
        }],
        identity: Some(rpc::HeroIdentity {
            appearance: Some(rpc::HeroAppearance {
                public_description: "  \u{96ea}\r\nlook \u{1f409}  ".into(),
                ..Default::default()
            }),
            backstory: "Public story".into(),
            ..Default::default()
        }),
        portrait_scenery: "A glade\nwith mist".into(),
        private_backstory: "Owner only".into(),
        sealed_backstory: "Sealed".into(),
    };
    let response = rpc::LoadHeroResponse {
        hero: Some(identity.clone()),
        summary: Some(rpc::LibraryHeroSummary {
            hero_id: 12,
            revision: 4,
            ..Default::default()
        }),
    };
    let decoded = rpc::LoadHeroResponse::decode(response.encode_to_vec().as_slice()).unwrap();
    assert_eq!(decoded.hero, Some(identity));
    assert_eq!(decoded.summary.unwrap().revision, 4);
}
#[test]
fn seat_save_accepts_loaded_reference_and_skips_forged_identity() {
    let request = rpc::SaveMySeatHeroRequest {
        key: vec![3; 32],
        scope: Some(rpc::WatchViewRequest::default()),
        loaded_hero: Some(rpc::LibraryHeroReference {
            hero_id: 12,
            revision: 4,
        }),
    };
    let mut bytes = request.encode_to_vec();
    bytes.extend([0x7a, 4, b'F', b'a', b'k', b'e']);
    assert_eq!(
        rpc::SaveMySeatHeroRequest::decode(bytes.as_slice()).unwrap(),
        request
    );
    assert_eq!(
        rpc::ListMyHeroesRequest {
            key: vec![0xab; 32],
            after_hero_id: 0,
        }
        .encode_to_vec(),
        [vec![0x0a, 32], vec![0xab; 32]].concat()
    );
}
