use df_protocol::common as rpc;
use prost::Message;

#[test]
fn reclaim_contract_round_trips_read_offer_and_typed_refusal_without_changing_join() {
    let read = rpc::ReclaimRoomRequest {
        room_code: "ROOM".to_owned(),
        operation_id: None,
        seat_token: Vec::new(),
    };
    assert_eq!(
        rpc::ReclaimRoomRequest::decode(read.encode_to_vec().as_slice()).unwrap(),
        read
    );
    let offer = rpc::ReclaimRoomResponse {
        outcome: None,
        away_heroes: vec![rpc::AwayHero {
            name: "Brakka".to_owned(),
            seat_token: vec![2; 32],
        }],
    };
    assert_eq!(
        rpc::ReclaimRoomResponse::decode(offer.encode_to_vec().as_slice()).unwrap(),
        offer
    );
    for reason in [
        rpc::SeatReclaimRefusal::SeatNotAway,
        rpc::SeatReclaimRefusal::SeatTokenUnknown,
        rpc::SeatReclaimRefusal::SeatRoomCodeWrong,
        rpc::SeatReclaimRefusal::SeatOperationConflict,
    ] {
        let refused = rpc::ReclaimRoomResponse {
            outcome: Some(rpc::reclaim_room_response::Outcome::Refused(reason as i32)),
            away_heroes: Vec::new(),
        };
        assert_eq!(
            rpc::ReclaimRoomResponse::decode(refused.encode_to_vec().as_slice()).unwrap(),
            refused
        );
    }
    let old_join = rpc::JoinRoomRequest {
        room_code: "ROOM".to_owned(),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![3; 16]),
        }),
        join_secret: vec![4; 32],
    };
    assert_eq!(
        rpc::JoinRoomRequest::decode(old_join.encode_to_vec().as_slice()).unwrap(),
        old_join
    );
}
