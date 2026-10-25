//! PostgreSQL save-store tests against the scratch server (skipped without
//! `DUNGEONFLUX_RUNTIME_PG_PORT`, like the runtime suites).

use df_types::{CampaignId, OperationId, TenantId};

use crate::save_store::{
    BranchId, CheckpointEnvelope, CreateSavePoint, PostgresSaveStore, SaveLink, SaveLinkKind,
    SavePointId,
};

fn id<T>(byte: u8, make: impl Fn(&[u8]) -> T) -> T {
    make(&[byte; 16])
}

fn request(seed: u8) -> CreateSavePoint {
    CreateSavePoint {
        save_point_id: SavePointId::from_bytes(&[seed; 16]).expect("id"),
        tenant_id: id(0xa1, |b| TenantId::from_bytes(b).expect("tenant")),
        campaign_id: id(0xc1, |b| CampaignId::from_bytes(b).expect("campaign")),
        branch_id: BranchId::from_bytes(&[1; 16]).expect("branch"),
        parent_save_point_id: None,
        checkpoint: CheckpointEnvelope {
            bytes: vec![seed; 32],
            codec_version: 3,
            schema_version: 1,
            content_version: 1,
        },
        links: vec![
            SaveLink {
                kind: SaveLinkKind::FinanceAdmission,
                linked_id: [0xf1; 16],
            },
            SaveLink {
                kind: SaveLinkKind::PaidGeneration,
                linked_id: [0x61; 16],
            },
        ],
        operation_id: OperationId::from_bytes(&[seed; 16]).expect("op"),
        fingerprint: [seed; 32],
        result_bytes: b"original-result".to_vec(),
    }
}

#[test]
fn postgres_save_point_commit_before_reply_and_branch_keep_links_forward_only() {
    let Some(port) = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    else {
        eprintln!("SKIPPED: DUNGEONFLUX_RUNTIME_PG_PORT not set");
        return;
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async move {
            let database = format!(
                "o1_save_test_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            );
            let mut admin_config = tokio_postgres::Config::new();
            admin_config
                .host("127.0.0.1")
                .port(port)
                .user("postgres")
                .dbname("postgres");
            let (admin, admin_connection) = admin_config
                .connect(tokio_postgres::NoTls)
                .await
                .expect("admin");
            tokio::spawn(async move {
                let _ = admin_connection.await;
            });
            admin
                .batch_execute(&format!("CREATE DATABASE {database}"))
                .await
                .expect("create db");
            let mut config = tokio_postgres::Config::new();
            config
                .host("127.0.0.1")
                .port(port)
                .user("postgres")
                .dbname(&database);
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .expect("connect");
            tokio::spawn(async move {
                let _ = connection.await;
            });
            let mut store = PostgresSaveStore::open(client).await.expect("open");

            let first = request(7);
            let committed = store.create_save_point(&first).await.expect("commit");
            assert!(committed.committed_now);
            // Injected failure after commit, before reply: the caller lost `committed`.
            // The retry returns the stored result and writes no second save point.
            let retried = store.create_save_point(&first).await.expect("retry");
            assert!(!retried.committed_now);
            assert_eq!(retried.save_point_id, committed.save_point_id);
            assert_eq!(retried.result_bytes, b"original-result".to_vec());
            let mut conflict = first.clone();
            conflict.fingerprint = [9; 32];
            assert_eq!(
                store.create_save_point(&conflict).await,
                Err(crate::save_store::SaveError::OperationConflict)
            );

            // Branch/restore from the save: a new head, the same forward-only links.
            let mut branch = request(8);
            branch.save_point_id = SavePointId::from_bytes(&[8; 16]).expect("id");
            branch.branch_id = BranchId::from_bytes(&[2; 16]).expect("branch");
            branch.operation_id = OperationId::from_bytes(&[8; 16]).expect("op");
            let branched = store
                .branch_from(first.save_point_id, &branch)
                .await
                .expect("branch");
            assert!(branched.committed_now);
            assert_eq!(
                store
                    .link_count(branched.save_point_id)
                    .await
                    .expect("links"),
                2
            );
            assert_eq!(
                store.link_count(first.save_point_id).await.expect("links"),
                2
            );

            let fence = store
                .claim_resume_fence(first.tenant_id, first.campaign_id, [4; 16])
                .await
                .expect("fence");
            let fence2 = store
                .claim_resume_fence(first.tenant_id, first.campaign_id, [4; 16])
                .await
                .expect("fence 2");
            assert!(fence2.claim_seq > fence.claim_seq);

            admin
                .batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"))
                .await
                .expect("drop db");
        });
}
