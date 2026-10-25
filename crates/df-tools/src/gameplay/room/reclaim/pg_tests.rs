use super::*;

/// Run only on a coordinator-created empty loopback database. This fixture neither starts
/// PostgreSQL nor drops any schema; a reused database is refused by the seed migration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicitly registered empty coordinator-owned PostgreSQL database"]
async fn actual_pg_reclaim_revokes_grant_replays_and_survives_issuer_restart() {
    use df_persistence::{NativeCodecLimits, local_demo_scope as local};
    use tokio_postgres::{Config, NoTls, config::Host};
    let configuration: Config = std::env::var("DF_SEAT_RECLAIM_TEST_DATABASE")
        .expect("coordinator supplies its empty loopback database")
        .parse()
        .unwrap();
    assert_eq!(configuration.get_hosts().len(), 1);
    assert!(matches!(&configuration.get_hosts()[0], Host::Tcp(host) if host == "127.0.0.1"));
    assert!(
        configuration
            .get_dbname()
            .unwrap()
            .starts_with("df_gameplay_demo_")
    );
    let (checkpoint, fixture) = fixture();
    let member = fixture.member;
    let session = checkpoint.basis().session;
    let basis = checkpoint.basis();
    let (mut client, connection) = configuration.connect(NoTls).await.unwrap();
    let driver = tokio::spawn(connection);
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1_048_576,
        maximum_allocated_bytes: 4_194_304,
        maximum_collection_items: 4096,
        maximum_text_bytes: 65536,
    };
    local::initialize(&mut client, &checkpoint, [7; 16], [8; 32], [9; 32], codec)
        .await
        .unwrap();
    // Seed the already committed Join grant exactly as complete_room_join stores it. The
    // canonical checkpoint already contains that Join's unchanged member/hero allocation.
    client.execute("INSERT INTO df_local_demo.grants SELECT $1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea,true,lease_until FROM df_game.sessions WHERE tenant_id=$2::bytea AND session_id=$7::bytea",
        &[&[1_u8; 32].as_slice(), &local::TENANT.as_slice(), &member.as_slice(), &local::CAMPAIGN.as_slice(), &b"player".as_slice(), &b"local-demo-access-1".as_slice(), &session.as_bytes().as_slice()],
    ).await.unwrap();
    client.execute("INSERT INTO df_local_demo.room_grants VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea)",
        &[&[7_u8; 16].as_slice(), &[7_u8; 32].as_slice(), &member.as_slice(), &[1_u8; 32].as_slice()],
    ).await.unwrap();
    let runtime = tokio::runtime::Handle::current();
    let mut issuer = LocalDemoScopeIssuer::new(runtime.clone(), client, session, [7; 16]).unwrap();
    let checkpoint_for_actor = checkpoint.clone();
    let mut issuer = tokio::task::spawn_blocking(move || {
        issuer.register_room_seat(member, [2; 32], 1000).unwrap();
        assert!(issuer.authenticate(&[1; 32]).is_ok());
        assert!(issuer.away_room_seats(30_999).unwrap().is_empty());
        assert!(issuer.away_room_seats(31_000).unwrap()[0].newly_away);
        assert!(!issuer.away_room_seats(31_000).unwrap()[0].newly_away);
        assert!(matches!(
            admit_reclaim(
                &mut issuer,
                request(8),
                true,
                &checkpoint_for_actor,
                30_999,
                [3; 32],
                [4; 32]
            )
            .unwrap(),
            Err(ReclaimRefusal::Seat(SeatReclaimRefusal::NotAway))
        ));
        let joined = admit_reclaim(
            &mut issuer,
            request(9),
            true,
            &checkpoint_for_actor,
            31_000,
            [3; 32],
            [4; 32],
        )
        .unwrap()
        .unwrap();
        assert_eq!(joined.client_binding_id.unwrap().value.unwrap(), member);
        assert_eq!(
            issuer.authenticate(&[1; 32]).unwrap_err(),
            RepositoryError::Unauthorized
        );
        assert!(issuer.authenticate(&[3; 32]).is_ok());
        assert!(matches!(
            admit_reclaim(
                &mut issuer,
                request(10),
                true,
                &checkpoint_for_actor,
                31_000,
                [5; 32],
                [6; 32]
            )
            .unwrap(),
            Err(ReclaimRefusal::Seat(SeatReclaimRefusal::UnknownToken))
        ));
        let replay = admit_reclaim(
            &mut issuer,
            request(9),
            true,
            &checkpoint_for_actor,
            31_001,
            [5; 32],
            [6; 32],
        )
        .unwrap()
        .unwrap();
        assert_eq!(replay.local_binding, journey::hex(&[3; 32]));
        assert!(replay.receipt.unwrap().replayed);
        let mut active = request(11);
        active.token = [4; 32];
        assert!(matches!(
            admit_reclaim(
                &mut issuer,
                active,
                true,
                &checkpoint_for_actor,
                31_001,
                [5; 32],
                [6; 32]
            )
            .unwrap(),
            Err(ReclaimRefusal::Seat(SeatReclaimRefusal::NotAway))
        ));
        issuer.release_owner().unwrap();
        issuer
    })
    .await
    .unwrap();
    issuer.close_owned().await.unwrap();
    assert!(driver.await.unwrap().is_ok());
    drop(issuer);
    let (mut client, connection) = configuration.connect(NoTls).await.unwrap();
    let driver = tokio::spawn(connection);
    let saved = client.query_one("SELECT run_id,recovery_epoch::text,in_epoch_sequence::text FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea", &[&local::TENANT.as_slice(), &session.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(saved.get::<_, Vec<u8>>("run_id"), basis.run.as_bytes());
    assert_eq!(
        saved.get::<_, String>("in_epoch_sequence"),
        basis.revision.sequence().to_string()
    );
    // The session lease and every grant lapse while the server is down; restore must reissue them.
    client
        .execute(
            "UPDATE df_local_demo.grants SET expires_at=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&local::TENANT.as_slice(), &session.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let counts_before = grant_seat_and_join_counts(&client, session.as_bytes().as_slice()).await;
    let startup = local::admit_startup(
        &mut client,
        &checkpoint,
        local::DemoStartupIdentity {
            fence: [10; 16],
            player_credential: [11; 32],
            display_credential: [12; 32],
        },
        codec,
        &model::recovery().unwrap(),
        |current| model::checkpoint(current.basis(), current.state().clone()).map(|_| ()),
    )
    .await
    .unwrap();
    assert!(startup.restored);
    assert_eq!(startup.checkpoint, checkpoint);
    assert_eq!(
        grant_seat_and_join_counts(&client, session.as_bytes().as_slice()).await,
        counts_before
    );
    let active = client
        .query_one(
            "SELECT count(*) FILTER (WHERE active), count(*) FILTER (WHERE active AND expires_at > clock_timestamp()) FROM df_local_demo.grants WHERE session_id=$1::bytea",
            &[&session.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(active.get::<_, i64>(0), 3);
    assert_eq!(active.get::<_, i64>(1), 3);
    let reclaimed = client
        .query_one(
            "SELECT active FROM df_local_demo.grants WHERE credential=$1::bytea",
            &[&[1_u8; 32].as_slice()],
        )
        .await
        .unwrap();
    assert!(!reclaimed.get::<_, bool>(0));
    let checkpoint = startup.checkpoint;
    let mut restored = LocalDemoScopeIssuer::new(runtime, client, session, [10; 16]).unwrap();
    let mut restored = tokio::task::spawn_blocking(move || {
        assert_eq!(
            restored.authenticate(&[1; 32]).unwrap_err(),
            RepositoryError::Unauthorized
        );
        assert!(restored.authenticate(&[3; 32]).is_ok());
        let replay = admit_reclaim(
            &mut restored,
            request(9),
            true,
            &checkpoint,
            31_002,
            [5; 32],
            [6; 32],
        )
        .unwrap()
        .unwrap();
        assert_eq!(replay.local_binding, journey::hex(&[3; 32]));
        assert!(replay.receipt.unwrap().replayed);
        restored
    })
    .await
    .unwrap();
    restored.close_owned().await.unwrap();
    assert!(driver.await.unwrap().is_ok());
}

/// Grant, seat and join-grant row counts for one session; a restore must leave them unchanged.
async fn grant_seat_and_join_counts(
    client: &tokio_postgres::Client,
    session: &[u8],
) -> (i64, i64, i64) {
    let row = client
        .query_one(
            "SELECT (SELECT count(*) FROM df_local_demo.grants WHERE session_id=$1::bytea), (SELECT count(*) FROM df_local_demo.room_seats WHERE session_id=$1::bytea), (SELECT count(*) FROM df_local_demo.room_grants)",
            &[&session],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2))
}

/// The coordinator-created loopback database named by `DF_SEAT_RECLAIM_TEST_DATABASE`. The ignored
/// test below needs its own empty database, because the seed migration refuses a reused one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicitly registered empty coordinator-owned PostgreSQL database"]
async fn actual_pg_restore_refuses_a_revoked_player_grant_after_lapse() {
    use df_persistence::{NativeCodecLimits, local_demo_scope as local};
    use tokio_postgres::{Config, NoTls, config::Host};
    let configuration: Config = std::env::var("DF_SEAT_RECLAIM_TEST_DATABASE")
        .expect("coordinator supplies its empty loopback database")
        .parse()
        .unwrap();
    assert_eq!(configuration.get_hosts().len(), 1);
    assert!(matches!(&configuration.get_hosts()[0], Host::Tcp(host) if host == "127.0.0.1"));
    assert!(
        configuration
            .get_dbname()
            .unwrap()
            .starts_with("df_gameplay_demo_")
    );
    let (checkpoint, _fixture) = fixture();
    let session = checkpoint.basis().session;
    let (mut client, connection) = configuration.connect(NoTls).await.unwrap();
    let driver = tokio::spawn(connection);
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1_048_576,
        maximum_allocated_bytes: 4_194_304,
        maximum_collection_items: 4096,
        maximum_text_bytes: 65536,
    };
    local::initialize(&mut client, &checkpoint, [7; 16], [8; 32], [9; 32], codec)
        .await
        .unwrap();
    client
        .execute(
            "UPDATE df_game.sessions SET lease_until=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&local::TENANT.as_slice(), &session.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    client
        .execute(
            "UPDATE df_local_demo.grants SET expires_at=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&local::TENANT.as_slice(), &session.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    client
        .execute(
            "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
            &[&[8_u8; 32].as_slice()],
        )
        .await
        .unwrap();
    let Err(refusal) = local::admit_startup(
        &mut client,
        &checkpoint,
        local::DemoStartupIdentity {
            fence: [10; 16],
            player_credential: [11; 32],
            display_credential: [12; 32],
        },
        codec,
        &model::recovery().unwrap(),
        |current| model::checkpoint(current.basis(), current.state().clone()).map(|_| ()),
    )
    .await
    else {
        panic!("a revoked player grant must refuse the restore");
    };
    assert_eq!(refusal.stage, "retained_access_unavailable");
    assert_eq!(refusal.class, RepositoryError::Unauthorized);
    let fence = client
        .query_one(
            "SELECT owner_fence FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&local::TENANT.as_slice(), &session.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(fence.get::<_, Vec<u8>>(0), [7_u8; 16].to_vec());
    drop(client);
    assert!(driver.await.unwrap().is_ok());
}
