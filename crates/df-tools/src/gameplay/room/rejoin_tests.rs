//! Native execution of the browser's storage contract with fresh tab stores.
#[path = "../../gameplay_browser/player_storage.rs"]
mod player_storage;

use super::*;
use df_protocol::common::room_service_server::RoomService;
use std::sync::Arc;

struct AwayOwner;
impl Reducer<actor::Call> for AwayOwner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::AwaySeats { reply, .. } => {
                let _ = reply.send(Ok(vec![AwayHero {
                    member: [1; 16],
                    name: "Brannoc".to_owned(),
                    token: [2; 32],
                }]));
            }
            _ => panic!("unexpected actor call"),
        }
    }
}

#[tokio::test]
async fn reclaim_rpc_exposes_tv_codes_only_to_the_authenticated_table_display() {
    let (_, updates) = tokio::sync::watch::channel(journey::initial().unwrap());
    let (actor, inbox) = df_session::inbox::bounded_inbox();
    let thread = std::thread::spawn(move || inbox.run(&mut AwayOwner));
    let service = Service {
        look_ahead: Default::default(),
        actor: actor.clone(),
        display_credential: [11; 32],
        updates,
        streams: Arc::new(tokio::sync::Semaphore::new(1)),
        voice: super::super::voice::Voice::disabled(
            super::super::startup_configuration::VoiceOff::Qualification,
        ),
        npc_voice: Arc::new(super::super::npc_reply::NpcVoice::silent()),
        table_log: super::super::table_log::TableLog::new(),
        social: super::super::social::SocialLedger::none(),
        living: super::super::living_scenes::LivingScenes::disabled(),
        layout: super::super::dev_control::layout::LayoutState::new(),
        portraits: super::super::creator_portraits::Portraits::disabled(),
    };
    for binding in [None, Some([10; 32]), Some([12; 32]), Some([11; 32])] {
        let mut request = Request::new(rpc::ReclaimRoomRequest {
            room_code: journey::room_code(),
            ..Default::default()
        });
        if let Some(binding) = binding {
            request.metadata_mut().insert(
                "x-df-local-binding",
                journey::hex(&binding).parse().unwrap(),
            );
        }
        let reply = service.reclaim(request).await.unwrap().into_inner();
        assert!(reply.outcome.is_none());
        assert_eq!(reply.away_heroes[0].name, "Brannoc");
        assert!(
            !reply
                .encode_to_vec()
                .windows(32)
                .any(|bytes| bytes == [2; 32])
        );
        if binding == Some([11; 32]) {
            assert_eq!(reply.away_heroes[0].seat_token.len(), 32);
            assert!(reply.away_heroes[0].seat_token.starts_with(b"DFREJOIN"));
        } else {
            assert_eq!(reply.away_heroes[0].seat_token, [1; 16]);
        }
    }
    let mut malformed = Request::new(rpc::ReclaimRoomRequest::default());
    malformed
        .metadata_mut()
        .insert("x-df-local-binding", "wrong".parse().unwrap());
    assert_eq!(
        service.reclaim(malformed).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    actor.stop().unwrap();
    thread.join().unwrap().unwrap();
}
