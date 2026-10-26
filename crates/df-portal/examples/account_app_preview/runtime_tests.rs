//! Separate preview executable: its audit sink cannot replace the library tests' global sink.
use df_campaign::session_link::{HttpRuntime, runtime_protocol::LaunchSecret};
use df_portal::app_api::{
    fakes::FakeCampaigns,
    ports::{Actor, CampaignDirectory},
};
use std::{
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

static AUDIT: StdMutex<Vec<String>> = StdMutex::new(Vec::new());
fn capture(_: &'static str, line: String) {
    AUDIT.lock().unwrap().push(line);
}

#[tokio::test]
async fn unavailable_runtime_keeps_links_absent_and_records_the_session_reason() {
    df_observe::install_audit_sink(capture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let campaigns = FakeCampaigns::new("http://127.0.0.1:8099", Arc::new(df_customer::SystemClock));
    campaigns.use_runtime(
        HttpRuntime::new(
            address,
            LaunchSecret::new(&"r".repeat(32)).unwrap(),
            Duration::from_secs(1),
        )
        .unwrap(),
    );
    let who = Actor {
        account: df_types::AccountId::from_bytes(&[2; 16]).unwrap(),
        display_name: "Host".into(),
        is_verified: true,
    };
    let campaign = campaigns.seed_campaign(&who, "Unavailable");
    let session = campaigns.seed_lobby(campaign).unwrap();
    assert!(
        campaigns
            .start_session(&who, session)
            .await
            .unwrap()
            .has_started
    );
    assert!(campaigns.game_links(&who, session).await.unwrap().is_none());
    let session = df_campaign::secrets::hex(session.as_bytes());
    assert!(
        AUDIT
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(&format!(
                "SESSION preview_placement session={session} reason=unavailable"
            )))
    );
}
