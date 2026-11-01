use super::*;
use crate::gameplay::startup_configuration::ContentRating;
use df_imagery::{ArtworkSubject, ImageOwner};

fn thing() -> SceneThing {
    let model: crate::gameplay::adventure::scene_model::SceneModel =
        serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/merchant-shop/scene-model.json"
        )))
        .unwrap();
    let mut thing = model.things[0].clone();
    thing.publicly_visible = true;
    thing.provenance.source_id = "authored:public-object".into();
    thing.closer_look = "PRIVATE UNRELEASED MARK".into();
    thing
}
fn owner(run: u8) -> ImageOwner {
    ImageOwner::Run {
        campaign: df_types::CampaignId::from_bytes(&[1; 16]).unwrap(),
        run: df_types::RunId::from_bytes(&[run; 16]).unwrap(),
    }
}
fn request(thing: &SceneThing, detail: Option<&str>) -> ImageRequest {
    item_image_request(ContentRating::Restrained, thing, detail).unwrap()
}

#[test]
fn public_item_preserves_full_prompt_and_uses_stable_source_revision_with_private_run_selection() {
    let thing = thing();
    let original = request(&thing, None);
    let full_prompt = original.prompt.as_str().to_owned();
    let revision = crate::gameplay::imagery::central::public_artwork_revision(&original);
    let first = item_artwork_request(
        original.clone(),
        &thing,
        None,
        "instance-a:revision-7",
        Some(&owner(2)),
    )
    .unwrap();
    let second = item_artwork_request(
        original,
        &thing,
        None,
        "instance-b:revision-7",
        Some(&owner(3)),
    )
    .unwrap();
    assert!(first.prompt.as_str().starts_with(&full_prompt));
    assert!(
        first
            .prompt
            .as_str()
            .contains("Object source identity: authored:public-object.")
    );
    assert!(!first.prompt.as_str().contains(&thing.closer_look));
    assert!(!first.prompt.as_str().contains("instance-a"));
    assert_eq!(first.prompt.as_str(), second.prompt.as_str());
    let a = first.prompt.artwork().unwrap();
    let b = second.prompt.artwork().unwrap();
    assert!(
        matches!(&a.spec.subject, ArtworkSubject::Item(key) if key.as_str() == thing.provenance.source_id)
    );
    assert_eq!(a.spec.public_revision, revision);
    assert_eq!(a.spec, b.spec);
    assert_ne!(a.selection, b.selection);
    assert_eq!(
        a.selection.run,
        df_types::RunId::from_bytes(&[2; 16]).unwrap()
    );
    let replay = item_artwork_request(
        request(&thing, None),
        &thing,
        None,
        "instance-a:revision-7",
        Some(&owner(2)),
    )
    .unwrap();
    assert_eq!(replay.prompt.artwork(), first.prompt.artwork());
}

#[test]
fn private_observed_and_draft_item_requests_keep_the_original_non_artifact_contract() {
    let mut thing = thing();
    let draft = ImageOwner::Draft {
        account: df_types::AccountId::from_bytes(&[4; 16]).unwrap(),
    };
    for (public, detail, actual_owner) in [
        (false, None, owner(2)),
        (true, Some("listener-safe private trace"), owner(2)),
        (true, None, draft),
    ] {
        thing.publicly_visible = public;
        let original = request(&thing, detail);
        let before = original.prompt.as_str().to_owned();
        let actual = item_artwork_request(
            original,
            &thing,
            detail,
            "actual-job:revision",
            Some(&actual_owner),
        )
        .unwrap();
        assert!(actual.prompt.artwork().is_none());
        assert_eq!(actual.prompt.as_str(), before);
    }
}

#[test]
fn changed_public_design_changes_revision_and_empty_or_oversized_provenance_uses_content_key() {
    let mut thing = thing();
    let first = item_artwork_request(
        request(&thing, None),
        &thing,
        None,
        "job:1",
        Some(&owner(2)),
    )
    .unwrap();
    thing.detail.push_str(" Its rim now has a visible chip.");
    let changed = item_artwork_request(
        request(&thing, None),
        &thing,
        None,
        "job:2",
        Some(&owner(2)),
    )
    .unwrap();
    assert_ne!(
        first.prompt.artwork().unwrap().spec.public_revision,
        changed.prompt.artwork().unwrap().spec.public_revision
    );
    for source in [
        String::new(),
        "x".repeat(df_imagery::MAXIMUM_ARTWORK_SUBJECT_BYTES + 1),
    ] {
        thing.provenance.source_id = source;
        let original = request(&thing, None);
        let expected = crate::gameplay::journey::hex(
            &crate::gameplay::imagery::central::public_artwork_revision(&original),
        );
        let actual =
            item_artwork_request(original, &thing, None, "actual-job:2", Some(&owner(2))).unwrap();
        assert_eq!(
            actual.prompt.artwork().unwrap().spec.subject.key().as_str(),
            expected
        );
    }
}
