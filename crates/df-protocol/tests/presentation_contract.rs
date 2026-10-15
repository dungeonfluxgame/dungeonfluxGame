use df_protocol::common::PreviewHeroIdentityRequest;
use prost::Message;

#[test]
fn hero_preview_preserves_the_joined_device_binding_at_field_seven() {
    // Tag 7 carries ClientBindingId { value: [1] }; old readers discarded it.
    let bytes = [58, 3, 10, 1, 1];
    let preview = PreviewHeroIdentityRequest::decode(bytes.as_slice()).unwrap();
    assert_eq!(preview.encode_to_vec(), bytes);
}

#[test]
fn timing_upload_round_trips_authenticated_scope_and_replay_receipt() {
    use df_protocol::common::{
        ClientBindingId, PresentationTelemetryRequest, PresentationTelemetryResponse,
        WatchViewRequest,
    };
    let request = PresentationTelemetryRequest {
        scope: Some(WatchViewRequest {
            client_binding_id: Some(ClientBindingId {
                value: Some(vec![1; 16]),
            }),
            ..Default::default()
        }),
        producer_id: vec![2; 16],
        batch_sequence: 1,
        otlp_logs: vec![10, 0],
        dropped_records: 3,
    };
    assert_eq!(
        PresentationTelemetryRequest::decode(request.encode_to_vec().as_slice()).unwrap(),
        request
    );
    let response = PresentationTelemetryResponse {
        batch_sequence: 1,
        accepted_records: 2,
        replayed: true,
    };
    assert_eq!(
        PresentationTelemetryResponse::decode(response.encode_to_vec().as_slice()).unwrap(),
        response
    );
}
