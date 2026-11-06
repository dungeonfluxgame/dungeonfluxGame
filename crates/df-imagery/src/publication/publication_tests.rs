use super::*;

#[test]
fn frozen_output_round_trips_distinct_variants_and_full_plan() {
    let master = ImageIntent::new("image/png", 2, 3, ImageAlpha::Transparent).unwrap();
    let display = ImageIntent::new("image/webp", 1, 2, ImageAlpha::Opaque).unwrap();
    let frozen = FrozenOutput::new(
        master,
        Some(display),
        b"inputs-config-model-profile-transform-revisions".to_vec(),
    )
    .unwrap();
    let snapshot = frozen.snapshot().unwrap();
    assert_eq!(FrozenOutput::decode(&snapshot).unwrap(), frozen);
    let changed = FrozenOutput::new(master, Some(display), b"changed execution plan".to_vec())
        .unwrap()
        .snapshot()
        .unwrap();
    assert_ne!(recipe_digest(&snapshot), recipe_digest(&changed));
}

#[test]
fn frozen_output_rejects_unknown_versions_truncation_and_trailing_bytes() {
    let master = ImageIntent::new("image/png", 2, 3, ImageAlpha::Opaque).unwrap();
    let snapshot = FrozenOutput::new(master, None, b"execution plan".to_vec())
        .unwrap()
        .snapshot()
        .unwrap();
    let unknown =
        VersionedSnapshot::new(2, snapshot.bytes().to_vec(), MAXIMUM_RECIPE_BYTES).unwrap();
    assert_eq!(
        FrozenOutput::decode(&unknown),
        Err(PublicationError::InvalidRecipe)
    );
    for length in 1..snapshot.bytes().len() {
        let truncated =
            VersionedSnapshot::new(1, snapshot.bytes()[..length].to_vec(), MAXIMUM_RECIPE_BYTES)
                .unwrap();
        assert!(FrozenOutput::decode(&truncated).is_err());
    }
    let mut bytes = snapshot.bytes().to_vec();
    bytes.push(0);
    let trailing = VersionedSnapshot::new(1, bytes, MAXIMUM_RECIPE_BYTES).unwrap();
    assert_eq!(
        FrozenOutput::decode(&trailing),
        Err(PublicationError::InvalidRecipe)
    );
}
