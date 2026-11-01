use super::*;

fn call(kind: PaidStageKind) -> StageCall {
    StageCall {
        job: JobId([1; 16]),
        stage: StageId([2; 16]),
        dispatch: [3; 16],
        parent_dispatch: None,
        kind,
        input_revision: 1,
        input_digest: [4; 32],
        config_revision: 2,
        config_digest: [5; 32],
        planned_publication: (!kind.is_standalone()).then_some(StagePublication {
            asset: AssetId([6; 16]),
            publication: PublicationId([7; 16]),
        }),
        payload_digest: [8; 32],
    }
}
#[test]
fn image_stage_wire_and_digest_preserve_existing_immutable_quote_identity() {
    let call = call(PaidStageKind::Generation);
    let value = serde_json::to_value(&call).unwrap();
    assert_eq!(value["asset"], serde_json::json!(vec![6_u8; 16]));
    assert_eq!(value["publication"], serde_json::json!(vec![7_u8; 16]));
    assert!(value.get("planned_publication").is_none());
    assert_eq!(serde_json::from_value::<StageCall>(value).unwrap(), call);
    let expected = business_key(
        "df-commerce/draft-stage-call-v1",
        &[
            &[1; 16],
            &[2; 16],
            &[3; 16],
            &[],
            b"generation",
            &1_u32.to_be_bytes(),
            &[4; 32],
            &2_u64.to_be_bytes(),
            &[5; 32],
            &[6; 16],
            &[7; 16],
            &[8; 32],
        ],
    );
    assert_eq!(call.digest(), expected);
}
#[test]
fn standalone_wire_has_truthful_null_pair_and_refuses_any_partial_or_media_identity() {
    let call = call(PaidStageKind::HeadLocations);
    let value = serde_json::to_value(&call).unwrap();
    assert!(value["asset"].is_null());
    assert!(value["publication"].is_null());
    assert_eq!(
        serde_json::from_value::<StageCall>(value.clone()).unwrap(),
        call
    );
    let mut partial = value.clone();
    partial["asset"] = serde_json::json!(vec![6_u8; 16]);
    assert!(serde_json::from_value::<StageCall>(partial).is_err());
    let mut fabricated = value.clone();
    fabricated["asset"] = serde_json::json!(vec![6_u8; 16]);
    fabricated["publication"] = serde_json::json!(vec![7_u8; 16]);
    assert!(serde_json::from_value::<StageCall>(fabricated).is_err());
    let mut image = value;
    image["kind"] = serde_json::json!("generation");
    assert!(serde_json::from_value::<StageCall>(image).is_err());
}
#[test]
fn all_standalone_kinds_are_closed_no_media_contracts() {
    for kind in [
        PaidStageKind::PhotoInspection,
        PaidStageKind::HeadLocations,
        PaidStageKind::Segmentation,
        PaidStageKind::StagingIdentity,
        PaidStageKind::StagingFacing,
    ] {
        assert!(kind.is_standalone());
        let original = call(kind);
        let recovered: StageCall =
            serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
        assert_eq!(original, recovered);
        assert!(recovered.planned_publication.is_none());
    }
    assert!(!PaidStageKind::Classification.is_standalone());
}
