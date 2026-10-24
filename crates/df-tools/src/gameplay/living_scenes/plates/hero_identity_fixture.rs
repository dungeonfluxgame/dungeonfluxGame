//! Accepted checkpoint fixtures shared by the canonical and plate regressions.
use crate::gameplay::{hero_identity, hero_library, model};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};

pub(in crate::gameplay::living_scenes) const LOOKS: [&str; 2] = [
    "A tall copper-scaled dragonborn with swept silver horns and a violet wool coat.\nA turquoise stripe crosses the shoulders.",
    "An elder tiefling with curled ivory horns, emerald skin and a short white mohawk. A yellow cloak has a round black clasp.",
];

pub(in crate::gameplay::living_scenes) fn portrait_bytes() -> Vec<u8> {
    let image = image::DynamicImage::new_rgb8(32, 48);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

pub(in crate::gameplay::living_scenes) fn portrait_asset() -> AssetReference {
    let bytes = portrait_bytes();
    AssetReference {
        key: model::label(&"4".repeat(64)).unwrap(),
        digest: ContentDigest(Sha256::digest(&bytes).into()),
        byte_length: bytes.len() as u64,
        kind: AssetKind::Image,
    }
}

pub(in crate::gameplay::living_scenes) fn checkpoint(custom: bool) -> Checkpoint {
    let base = model::initial().unwrap();
    let mut state = base.state().clone();
    let world = state.entities[0].clone();
    let character = state.characters[0].clone();
    state.members.clear();
    state.entities.clear();
    state.characters.clear();
    for index in 0..2 {
        let entity = EntityId::from_bytes(&[0x70 + index as u8; 16]).unwrap();
        let member = df_types::MemberId::from_bytes(&[0x60 + index as u8; 16]).unwrap();
        state.members.push(MembershipLink {
            member,
            character: Some(entity),
        });
        let mut world = world.clone();
        world.id = entity;
        state.entities.push(world.clone());
        let dossier = hero_identity::accepted_visual_dossier("portrait-hero-1").unwrap();
        let mut raw = dossier.identity().unwrap().identity().clone();
        raw.backstory = "SEALED_STORY_NEVER_IMAGE".to_owned();
        raw.origin = "PRIVATE_ORIGIN_NEVER_IMAGE".to_owned();
        if custom {
            raw.appearance.public_description = LOOKS[index].to_owned();
            raw.appearance.build = "wiry".to_owned();
        }
        let identity = raw.validate().unwrap();
        let species = if custom {
            ["dragonborn", "tiefling"][index]
        } else {
            "dwarf"
        };
        let name = ["Brakka", "Sigrun"][index];
        let mut choices = vec![
            ("name".to_owned(), name.to_owned()),
            ("species".to_owned(), species.to_owned()),
            ("class".to_owned(), "fighter-1".to_owned()),
            ("background".to_owned(), "soldier".to_owned()),
            ("portrait".to_owned(), "portrait-hero-1".to_owned()),
            (
                "identity-appearance:accepted-profile".to_owned(),
                "portrait-hero-1".to_owned(),
            ),
        ];
        choices.extend(
            identity
                .canonical_appearance_choices()
                .into_iter()
                .map(|(field, value)| (format!("identity-appearance:{field}"), value.to_owned())),
        );
        choices.extend(identity.canonical_public_choices());
        let selection = rpc::CharacterSelection {
            name: name.to_owned(),
            identity: Some(identity.to_rpc()),
            portrait_scenery: "SCENERY_NEVER_CANONICAL".to_owned(),
            ..Default::default()
        };
        if custom {
            choices.extend(
                hero_library::identity_choices(Some(&selection))
                    .unwrap()
                    .into_iter()
                    .map(|(key, value)| (key.as_str().to_owned(), value.as_str().to_owned())),
            );
        }
        let mut pack = identity.canonical_visual_pack(entity, species).unwrap();
        if custom && index == 0 {
            let asset = portrait_asset();
            choices.extend([
                (
                    "identity-portrait:key".to_owned(),
                    asset.key.as_str().to_owned(),
                ),
                (
                    "identity-portrait:digest".to_owned(),
                    crate::gameplay::journey::hex(&asset.digest.0),
                ),
                (
                    "identity-portrait:bytes".to_owned(),
                    asset.byte_length.to_string(),
                ),
                ("identity-portrait:revision".to_owned(), "1".to_owned()),
            ]);
            pack.identities[0].appearances.push(asset);
        }
        state.continuity.canonical_packs.push(pack);
        let mut character = character.clone();
        character.entity = entity;
        character.owner = member;
        character.choices = choices
            .into_iter()
            .map(|(offer, selected)| AcceptedChoice {
                participant: member,
                offer: model::label(&offer).unwrap(),
                selected: model::label(&selected).unwrap(),
                source: model::rule().unwrap(),
            })
            .collect();
        state.characters.push(character);
        let item_id = EntityId::from_bytes(&[0x78 + index as u8; 16]).unwrap();
        let item = model::content(["greatsword", "longbow"][index]).unwrap();
        let mut item_world = world.clone();
        item_world.id = item_id;
        item_world.definition = item.clone();
        item_world.location = Some(entity);
        state.entities.push(item_world);
        state.inventory.push(InventoryItem {
            item: item_id,
            owner: entity,
            quantity: 1,
            source: model::rule().unwrap(),
            origin: item,
            attunement_owner: None,
        });
    }
    recheckpoint(base.basis(), state)
}

pub(in crate::gameplay::living_scenes) fn recheckpoint(
    basis: Basis,
    state: GameState,
) -> Checkpoint {
    let assets = state
        .continuity
        .canonical_packs
        .iter()
        .flat_map(|pack| &pack.identities)
        .flat_map(|identity| identity.appearances.clone())
        .collect::<Vec<_>>();
    let resources = crate::gameplay::journey::resources_for(&state).unwrap();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        model::pins().unwrap(),
        state,
        ReferenceInventory {
            rules: &[
                model::rule().unwrap(),
                crate::gameplay::journey::rule().unwrap(),
            ],
            content: &model::contents().unwrap(),
            resources: &resources,
            assets: &assets,
        },
        model::limits(),
    )
    .unwrap()
}
