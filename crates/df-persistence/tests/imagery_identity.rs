//! Real canonical account/member/session schemas, current credential checks,
//! restricted admission authority and deterministic two-connection races.
//! REQUIRE_SCRATCH_PG=1 makes an unavailable owned server fail the run.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use df_commerce::finance_ids::{
    AttemptId as FinanceAttemptId, CampaignKey, EffectId, OperationKey, PriceVersionId,
    ReservationId, SessionKey, TenantKey,
};
use df_commerce::snapshot::ReceiptRow;
use df_finance::{
    ImageAssetId, ImageJobId as FinanceJobId, ImageStageBinding, ImageStageId, OwnerScope,
    PublicationAttemptId,
};
use df_imagery::ImageOwner;
use df_persistence::imagery_identity::{
    self, BindOutcome, HostRunBinding, HostSeatBinding, HostSeatKey, IdentityError,
    LocalGuestBootstrap,
};
use df_persistence::{RunSessionAuthority, RunSessionError};
use df_types::{CampaignId, RunId};
use tokio_postgres::{Client, Config, NoTls};

const NOW_MS: i64 = 1_700_000_000_000;

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

const IDENTITY_MIGRATION: &str = include_str!("../migrations/0041_imagery_identity.sql");

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime")
}

async fn connect(config: &Config) -> Option<Client> {
    let (client, connection) = config.connect(NoTls).await.ok()?;
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    Some(client)
}

struct Scratch {
    admin: Config,
    database: String,
}

impl Scratch {
    async fn open() -> Option<Self> {
        Self::open_schema(true).await
    }

    async fn open_local() -> Option<Self> {
        Self::open_schema(false).await
    }

    async fn open_schema(hosted: bool) -> Option<Self> {
        let port: u16 = std::env::var("DUNGEONFLUX_IDENTITY_PG_PORT")
            .unwrap_or("55471".to_owned())
            .parse()
            .expect("scratch port");
        assert!(!FORBIDDEN_PORTS.contains(&port));
        let mut admin = Config::new();
        admin
            .host(std::env::var("DUNGEONFLUX_IDENTITY_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port)
            .user(std::env::var("DUNGEONFLUX_IDENTITY_PG_USER").unwrap_or("postgres".into()))
            .dbname("postgres");
        if connect(&admin).await.is_none() {
            assert!(
                std::env::var_os("REQUIRE_SCRATCH_PG").is_none(),
                "required scratch PostgreSQL unavailable on 127.0.0.1:{port}"
            );
            return None;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        let database = format!("df_identity_{nanos}_{}_{serial}", std::process::id());
        connect(&admin)
            .await?
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        let scratch = Self { admin, database };
        scratch.migrate(hosted).await;
        Some(scratch)
    }

    async fn migrate(&self, hosted: bool) {
        let client = self.connect().await;
        client
            .batch_execute(include_str!("../migrations/0001_game_decisions.sql"))
            .await
            .unwrap();
        client.batch_execute("CREATE SCHEMA df_local_demo;
            CREATE TABLE df_local_demo.grants (credential bytea PRIMARY KEY CHECK(octet_length(credential)=32),
            tenant_id bytea NOT NULL, principal_id bytea NOT NULL, campaign_id bytea NOT NULL,
            effective_role bytea NOT NULL, access_revision bytea NOT NULL, session_id bytea NOT NULL,
            active boolean NOT NULL, expires_at timestamptz NOT NULL);
            CREATE TABLE df_local_demo.room_grants (operation_id bytea PRIMARY KEY CHECK(octet_length(operation_id)=16),
            canonical_fingerprint bytea NOT NULL CHECK(octet_length(canonical_fingerprint)=32),
            principal_id bytea NOT NULL UNIQUE CHECK(octet_length(principal_id)=16),
            credential bytea NOT NULL UNIQUE REFERENCES df_local_demo.grants);").await.unwrap();
        if !hosted {
            client
                .batch_execute(IDENTITY_MIGRATION)
                .await
                .expect("local identity migration");
            client
                .batch_execute(imagery_identity::IMAGERY_AUTHORITY_EVIDENCE_MIGRATION_SQL)
                .await
                .expect("local authority evidence migration");
            return;
        }
        client.batch_execute("CREATE SCHEMA accounts; SET search_path=accounts,public;
            DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_accounts') THEN CREATE ROLE df_accounts NOLOGIN; END IF;
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_imagery_admission') THEN CREATE ROLE df_imagery_admission NOLOGIN; END IF;
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='df_runtime_definer') THEN CREATE ROLE df_runtime_definer NOLOGIN; END IF;
            END $$;").await.unwrap();
        client
            .batch_execute(include_str!("../migrations/0005_accounts.sql"))
            .await
            .unwrap();
        client
            .batch_execute("SET search_path=public")
            .await
            .unwrap();
        client
            .batch_execute(include_str!("../migrations/0008_campaigns.sql"))
            .await
            .unwrap();
        client
            .batch_execute("CREATE SCHEMA df_runtime")
            .await
            .unwrap();
        let table_sessions = include_str!("../migrations/0016_table_sessions.sql");
        client
            .batch_execute(
                table_sessions
                    .split("CREATE INDEX table_sessions_campaign")
                    .next()
                    .unwrap(),
            )
            .await
            .unwrap();
        client.batch_execute("GRANT USAGE ON SCHEMA df_game,df_runtime TO df_runtime_definer;
            GRANT SELECT,UPDATE ON df_game.sessions,df_runtime.table_sessions TO df_runtime_definer;
            CREATE POLICY identity_definer ON df_game.sessions TO df_runtime_definer USING(true) WITH CHECK(true);").await.unwrap();
        client
            .batch_execute(IDENTITY_MIGRATION)
            .await
            .expect("apply identity migration");
        client
            .batch_execute(imagery_identity::IMAGERY_AUTHORITY_EVIDENCE_MIGRATION_SQL)
            .await
            .expect("hosted authority evidence migration");
    }

    fn config(&self) -> Config {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        config
    }

    async fn connect(&self) -> Client {
        connect(&self.config())
            .await
            .expect("scratch server answered at open")
    }

    async fn finish(self) {
        connect(&self.admin)
            .await
            .expect("scratch server answered at open")
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop test database");
    }
}

fn id(byte: u8) -> [u8; 16] {
    [byte; 16]
}

fn run_binding() -> HostRunBinding {
    HostRunBinding {
        tenant: id(0x51),
        campaign: id(0x52),
        hosted_session: id(0x53),
        gameplay_tenant: id(0x31),
        gameplay_session: id(0x54),
        run: id(0x55),
        fence: id(0x56),
    }
}

fn seat_binding(generation: u8) -> HostSeatBinding {
    HostSeatBinding {
        tenant: id(0x51),
        campaign: id(0x52),
        hosted_session: id(0x53),
        account: id(0x61),
        member: id(0x62),
        gameplay_member: id(0x63),
        run: id(0x55),
        gameplay_session: id(0x54),
        generation: id(generation),
        active: true,
    }
}

fn owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_bytes(&id(0x52)).expect("campaign"),
        run: RunId::from_bytes(&id(0x55)).expect("run"),
    }
}

fn stage_binding() -> ImageStageBinding {
    ImageStageBinding {
        owner_scope: OwnerScope::new(vec![0x02]).expect("scope"),
        job: FinanceJobId::new(id(0x71)).expect("job"),
        stage: ImageStageId::new(id(0x72)).expect("stage"),
        asset: ImageAssetId::new(id(0x73)).expect("asset"),
        publication_attempt: PublicationAttemptId::new(id(0x74)).expect("attempt"),
        tenant: TenantKey::from_array(id(0x51)),
        campaign: CampaignKey::from_array(id(0x52)),
        session: SessionKey::from_array(id(0x53)),
        operation: OperationKey::from_array(id(0x40)),
        attempt: FinanceAttemptId::from_array(id(0x41)),
        effect: EffectId::from_array(id(0x42)),
    }
}

fn receipt() -> ReceiptRow {
    ReceiptRow {
        fingerprint: [0x31u8; 32],
        reservation: ReservationId::from_array(id(0x43)),
        attempt: FinanceAttemptId::from_array(id(0x41)),
        retail_maximum_micro_dollars: 0,
        supplier_maximum_micro_dollars: 0,
        price_version: PriceVersionId::from_array(id(0x70)),
    }
}

fn seat_key() -> HostSeatKey {
    HostSeatKey {
        tenant: id(0x51),
        campaign: id(0x52),
        hosted_session: id(0x53),
        account: id(0x61),
        member: id(0x62),
    }
}
fn local_command(
    member: u8,
    credential: u8,
    account: u8,
    operation: u8,
    fingerprint: u8,
) -> LocalGuestBootstrap {
    LocalGuestBootstrap {
        member: id(member),
        session: id(0x54),
        credential: [credential; 32],
        candidate_account: id(account),
        operation: id(operation),
        fingerprint: [fingerprint; 32],
        generation: id(1),
    }
}
async fn seed(scratch: &Scratch) {
    let client = scratch.connect().await;
    let run = run_binding();
    let seat = seat_binding(1);
    client.execute("INSERT INTO accounts.account (account_id,email_normalized,email_display,status,
        credential_generation,created_at_ms) VALUES ($1::bytea,'seat@example.invalid','seat@example.invalid','active',1,0)",
        &[&seat.account.as_slice()]).await.unwrap();
    client.execute("INSERT INTO accounts.web_session (session_id,token_digest,account_id,credential_generation,
        created_at_ms,last_seen_at_ms,absolute_expires_at_ms,csrf_secret,device_label)
        VALUES ($1::bytea,$2::bytea,$3::bytea,1,(extract(epoch from clock_timestamp())*1000)::bigint,(extract(epoch from clock_timestamp())*1000)::bigint,4102444800000,$2::bytea,'phone')",
        &[&id(0x81).as_slice(), &[0x82u8; 32].as_slice(), &seat.account.as_slice()]).await.unwrap();
    client.execute("INSERT INTO campaigns.campaign (campaign_id,tenant_id,owner_account_id,name,seat_count,status,feed_epoch,created_at_ms)
        VALUES ($1::bytea,$2::bytea,$3::bytea,'Identity fixture',2,'active',$4::bytea,0)",
        &[&run.campaign.as_slice(), &run.tenant.as_slice(), &seat.account.as_slice(), &id(0x83).as_slice()]).await.unwrap();
    client.execute("INSERT INTO campaigns.campaign_member (member_id,campaign_id,account_id,role,holds_seat,status,joined_at_ms)
        VALUES ($1::bytea,$2::bytea,$3::bytea,'host',true,'active',0)",
        &[&seat.member.as_slice(), &seat.campaign.as_slice(), &seat.account.as_slice()]).await.unwrap();
    client.execute("INSERT INTO df_runtime.table_sessions VALUES ($1::bytea,$2::bytea,$3::bytea,'active',clock_timestamp(),NULL)",
        &[&run.tenant.as_slice(), &run.campaign.as_slice(), &run.hosted_session.as_slice()]).await.unwrap();
    client.execute("INSERT INTO df_game.sessions VALUES ($1::bytea,$2::bytea,$3::bytea,1,0,$4::bytea,clock_timestamp()+interval '1 hour')",
        &[&run.gameplay_tenant.as_slice(), &run.gameplay_session.as_slice(), &run.run.as_slice(), &run.fence.as_slice()]).await.unwrap();
    grant(&client, seat.gameplay_member, [0x91u8; 32]).await;
}

async fn grant(client: &Client, member: [u8; 16], credential: [u8; 32]) {
    client.execute("INSERT INTO df_local_demo.grants VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea,true,clock_timestamp()+interval '1 hour')",
        &[&credential.as_slice(), &id(0x31).as_slice(), &member.as_slice(), &id(0x32).as_slice(),
          &b"player".as_slice(), &id(1).as_slice(), &id(0x54).as_slice()]).await.unwrap();
}

async fn bound(scratch: &Scratch) {
    seed(scratch).await;
    let mut client = scratch.connect().await;
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        imagery_identity::bind_host_run(&tx, &run_binding(), NOW_MS).await,
        Ok(BindOutcome::Accepted)
    );
    assert_eq!(
        imagery_identity::bind_host_seat(&tx, &seat_binding(1), NOW_MS).await,
        Ok(BindOutcome::Accepted)
    );
    tx.commit().await.unwrap();
}

#[test]
fn canonical_principal_rejects_revocation_expiry_and_stale_credentials() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut client = scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91u8; 32]
            )
            .await,
            Ok(true)
        );
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x92u8; 32]
            )
            .await,
            Err(IdentityError::Refused)
        );
        tx.commit().await.unwrap();
        client
            .execute(
                "UPDATE campaigns.campaign_member SET status='removed',removed_by_role='host'",
                &[],
            )
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91u8; 32]
            )
            .await,
            Err(IdentityError::Refused)
        );
        tx.rollback().await.unwrap();
        client
            .execute(
                "UPDATE campaigns.campaign_member SET status='active',removed_by_role=NULL",
                &[],
            )
            .await
            .unwrap();
        client
            .execute(
                "UPDATE df_game.sessions SET lease_until=clock_timestamp()-interval '1 second'",
                &[],
            )
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::PgRunSessionAuthority
                .verify_canonical_run(&tx, &owner(), &stage_binding(), &receipt())
                .await,
            Err(RunSessionError::Refused)
        );
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn restart_installs_a_new_fence_monotonically_and_preserves_the_account() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut client = scratch.connect().await;
        client
            .execute(
                "UPDATE df_game.sessions SET owner_fence=$1::bytea",
                &[&id(0x57).as_slice()],
            )
            .await
            .unwrap();
        let mut recovered = run_binding();
        recovered.fence = id(0x57);
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::rebind_host_run(&tx, &recovered, &run_binding().fence).await,
            Ok(BindOutcome::Accepted)
        );
        tx.commit().await.unwrap();
        drop(client);
        let mut client = scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::rebind_host_run(&tx, &recovered, &run_binding().fence).await,
            Ok(BindOutcome::Existing)
        );
        assert_eq!(
            imagery_identity::read_host_seat_by_member(
                &tx,
                &id(0x51),
                &id(0x52),
                &id(0x53),
                &id(0x62)
            )
            .await
            .unwrap()
            .unwrap()
            .account,
            id(0x61)
        );
        assert_eq!(
            tx.query_one(
                "SELECT owner_epoch FROM imagery.image_host_run_binding",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
            2
        );
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn rotation_and_revoke_replay_without_reviving_a_committed_revoke() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut client = scratch.connect().await;
        for (old, new) in [(1, 2), (1, 2), (2, 3), (1, 2)] {
            let tx = client.transaction().await.unwrap();
            assert_eq!(
                imagery_identity::rotate_host_seat(&tx, &seat_key(), &id(old), &id(new), NOW_MS)
                    .await,
                Ok(())
            );
            tx.commit().await.unwrap();
        }
        for _ in 0..2 {
            let tx = client.transaction().await.unwrap();
            assert_eq!(
                imagery_identity::revoke_host_seat(&tx, &seat_key(), NOW_MS).await,
                Ok(())
            );
            tx.commit().await.unwrap();
        }
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::rotate_host_seat(&tx, &seat_key(), &id(3), &id(4), NOW_MS).await,
            Err(IdentityError::Conflict)
        );
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn local_accounts_are_per_authenticated_member_and_survive_credential_rotation() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        seed(&scratch).await;
        let mut client = scratch.connect().await;
        grant(&client, id(0x64), [0x92u8; 32]).await;
        let tx = client.transaction().await.unwrap();
        let first = imagery_identity::bootstrap_local_guest(
            &tx,
            &local_command(0x63, 0x91, 0xa1, 0xb1, 1),
            NOW_MS,
        )
        .await
        .unwrap()
        .0;
        let second = imagery_identity::bootstrap_local_guest(
            &tx,
            &local_command(0x64, 0x92, 0xa2, 0xb2, 2),
            NOW_MS,
        )
        .await
        .unwrap()
        .0;
        assert_ne!(first.account, second.account);
        tx.commit().await.unwrap();
        client
            .execute(
                "UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",
                &[&[0x91u8; 32].as_slice()],
            )
            .await
            .unwrap();
        grant(&client, id(0x63), [0x93u8; 32]).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_local_credential(&tx, &id(0x63), &id(0x54), &[0x91u8; 32])
                .await,
            Err(IdentityError::Refused)
        );
        assert_eq!(
            imagery_identity::bootstrap_local_guest(
                &tx,
                &local_command(0x63, 0x93, 0xaf, 0xb1, 1),
                NOW_MS
            )
            .await
            .unwrap()
            .0
            .account,
            first.account
        );
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn admission_role_checks_the_real_run_but_cannot_mutate_identity() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut client = scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_admission")
            .await
            .unwrap();
        assert_eq!(
            imagery_identity::PgRunSessionAuthority
                .verify_canonical_run(&tx, &owner(), &stage_binding(), &receipt())
                .await,
            Ok(())
        );
        assert_eq!(
            tx.execute(
                "UPDATE imagery.image_host_seat_binding SET active=false",
                &[]
            )
            .await
            .unwrap_err()
            .as_db_error()
            .unwrap()
            .code()
            .code(),
            "42501"
        );
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn two_connection_bind_race_replays_without_an_aborted_transaction() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        seed(&scratch).await;
        let mut first = scratch.connect().await;
        let mut second = scratch.connect().await;
        let one = first.transaction().await.unwrap();
        let two = second.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::bind_host_run(&one, &run_binding(), NOW_MS).await,
            Ok(BindOutcome::Accepted)
        );
        let competing =
            async { imagery_identity::bind_host_run(&two, &run_binding(), NOW_MS).await };
        let commit = async {
            one.commit().await.unwrap();
        };
        let (result, ()) = tokio::join!(competing, commit);
        assert_eq!(result, Ok(BindOutcome::Existing));
        assert_eq!(
            two.query_one("SELECT 1", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            1
        );
        two.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn canonical_revoke_and_takeover_cannot_commit_inside_an_authorized_transaction() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut authorized = scratch.connect().await;
        let mut contender = scratch.connect().await;
        let tx = authorized.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91; 32]
            )
            .await,
            Ok(true)
        );
        for table in [
            "accounts.account",
            "accounts.web_session",
            "campaigns.campaign_member",
            "df_game.sessions",
            "df_local_demo.grants",
            "imagery.image_host_seat_binding",
        ] {
            let other = contender.transaction().await.unwrap();
            let error = other
                .query(&format!("SELECT 1 FROM {table} FOR UPDATE NOWAIT"), &[])
                .await
                .unwrap_err();
            assert_eq!(
                error.as_db_error().unwrap().code().code(),
                "55P03",
                "{table} must stay locked"
            );
            other.rollback().await.unwrap();
        }
        tx.commit().await.unwrap();
        contender
            .execute(
                "UPDATE df_game.sessions SET owner_fence=$1::bytea",
                &[&id(0x59).as_slice()],
            )
            .await
            .unwrap();
        let tx = authorized.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::PgRunSessionAuthority
                .verify_canonical_run(&tx, &owner(), &stage_binding(), &receipt())
                .await,
            Err(RunSessionError::Refused)
        );
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn canonical_account_session_and_role_are_current_authority() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        bound(&scratch).await;
        let mut client = scratch.connect().await;
        client
            .execute("UPDATE campaigns.campaign_member SET role='player'", &[])
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91; 32]
            )
            .await,
            Ok(false)
        );
        tx.commit().await.unwrap();
        client
            .execute(
                "UPDATE accounts.web_session SET revoked_at_ms=1,revoke_reason='logout'",
                &[],
            )
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91; 32]
            )
            .await,
            Err(IdentityError::Refused)
        );
        tx.rollback().await.unwrap();
        client
            .execute(
                "UPDATE accounts.web_session SET revoked_at_ms=NULL,revoke_reason=NULL",
                &[],
            )
            .await
            .unwrap();
        client
            .execute("UPDATE accounts.account SET credential_generation=2", &[])
            .await
            .unwrap();
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            imagery_identity::verify_host_principal(
                &tx,
                &run_binding(),
                &seat_binding(1),
                &id(0x81),
                &[0x91; 32]
            )
            .await,
            Err(IdentityError::Refused)
        );
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn two_connection_local_bootstrap_reconciles_the_winner_and_changed_intent() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        seed(&scratch).await;
        let mut first = scratch.connect().await;
        let mut second = scratch.connect().await;
        let one = first.transaction().await.unwrap();
        let two = second.transaction().await.unwrap();
        let accepted = imagery_identity::bootstrap_local_guest(
            &one,
            &local_command(0x63, 0x91, 0xa1, 0xb1, 1),
            NOW_MS,
        )
        .await
        .unwrap()
        .0;
        let competing = async {
            imagery_identity::bootstrap_local_guest(
                &two,
                &local_command(0x63, 0x91, 0xaf, 0xb1, 1),
                NOW_MS,
            )
            .await
        };
        let commit = async {
            one.commit().await.unwrap();
        };
        let (winner, ()) = tokio::join!(competing, commit);
        assert_eq!(winner.unwrap(), (accepted, BindOutcome::Existing));
        assert_eq!(
            imagery_identity::bootstrap_local_guest(
                &two,
                &local_command(0x63, 0x91, 0xaf, 0xb1, 2),
                NOW_MS
            )
            .await,
            Err(IdentityError::Conflict)
        );
        assert_eq!(
            two.query_one("SELECT 1", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            1
        );
        two.commit().await.unwrap();
        scratch.finish().await;
    });
}

fn local_run_grant() -> imagery_identity::LocalRunGrant {
    imagery_identity::LocalRunGrant {
        session: run_binding().gameplay_session,
        run: run_binding().run,
        fence: run_binding().fence,
        bootstrap_credential: [0x94; 32],
    }
}

async fn seed_local_run(scratch: &Scratch) {
    let client = scratch.connect().await;
    let run = run_binding();
    client.execute("INSERT INTO df_game.sessions VALUES ($1::bytea,$2::bytea,$3::bytea,1,0,$4::bytea,clock_timestamp()+interval '1 hour')",
        &[&run.gameplay_tenant.as_slice(),&run.gameplay_session.as_slice(),&run.run.as_slice(),&run.fence.as_slice()]).await.unwrap();
    grant(
        &client,
        df_persistence::local_demo_scope::PLAYER,
        [0x94; 32],
    )
    .await;
    grant(&client, id(0x63), [0x91; 32]).await;
}

#[test]
fn local_run_uses_only_existing_local_authority_without_hosted_or_account_rows() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open_local().await else {return;};
        seed_local_run(&scratch).await;
        let mut client=scratch.connect().await;
        let tx=client.transaction().await.unwrap();
        let authority=imagery_identity::resolve_local_run(&tx,&local_run_grant()).await.unwrap();
        assert_eq!(authority.tenant,df_persistence::local_demo_scope::TENANT);
        assert_eq!(authority.campaign,df_persistence::local_demo_scope::CAMPAIGN);
        assert_eq!(authority.run,run_binding().run);
        assert_eq!(imagery_identity::require_hosted_identity_schema(&tx).await,Err(IdentityError::Refused));
        let absent=tx.query_one("SELECT to_regclass('accounts.account') IS NULL AND to_regclass('campaigns.campaign') IS NULL AND to_regclass('df_runtime.table_sessions') IS NULL",&[]).await.unwrap();
        assert!(absent.get::<_,bool>(0));
        assert_eq!(tx.query_one("SELECT COUNT(*) FROM imagery.image_host_run_binding",&[]).await.unwrap().get::<_,i64>(0),0);
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn local_run_refuses_regular_guests_and_changed_run_session_fence_or_bootstrap_role() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open_local().await else {return;};
        seed_local_run(&scratch).await;
        let mut client=scratch.connect().await;
        for field in 0..4 {
            let mut proof=local_run_grant();
            match field {0=>proof.bootstrap_credential=[0x91;32],1=>proof.run=id(0x88),
                2=>proof.session=id(0x89),_=>proof.fence=id(0x90)}
            let tx=client.transaction().await.unwrap();
            assert_eq!(imagery_identity::resolve_local_run(&tx,&proof).await,Err(IdentityError::Refused));
            tx.rollback().await.unwrap();
        }
        client.execute("UPDATE df_local_demo.grants SET effective_role=$1::bytea WHERE principal_id=$2::bytea",
            &[&b"display".as_slice(),&df_persistence::local_demo_scope::PLAYER.as_slice()]).await.unwrap();
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::resolve_local_run(&tx,&local_run_grant()).await,Err(IdentityError::Refused));
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn local_run_refuses_expired_owner_and_revoked_bootstrap_and_rebinds_only_current_owner() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open_local().await else {return;};
        seed_local_run(&scratch).await;
        let mut client=scratch.connect().await;
        client.execute("UPDATE df_game.sessions SET lease_until=clock_timestamp()-interval '1 second'",&[]).await.unwrap();
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::resolve_local_run(&tx,&local_run_grant()).await,Err(IdentityError::Refused));
        tx.rollback().await.unwrap();
        client.execute("UPDATE df_game.sessions SET lease_until=clock_timestamp()+interval '1 hour',owner_fence=$1::bytea",&[&id(0xa1).as_slice()]).await.unwrap();
        let mut current=local_run_grant();current.fence=id(0xa1);
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::resolve_local_run(&tx,&local_run_grant()).await,Err(IdentityError::Refused));
        assert!(imagery_identity::resolve_local_run(&tx,&current).await.is_ok());
        tx.commit().await.unwrap();
        client.execute("UPDATE df_local_demo.grants SET active=false WHERE credential=$1::bytea",&[&[0x94u8;32].as_slice()]).await.unwrap();
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::resolve_local_run(&tx,&current).await,Err(IdentityError::Refused));
        tx.rollback().await.unwrap();
        grant(&client,df_persistence::local_demo_scope::PLAYER,[0x95;32]).await;
        let mut rotated=current.clone();rotated.bootstrap_credential=[0x95;32];
        let tx=client.transaction().await.unwrap();
        assert!(imagery_identity::resolve_local_run(&tx,&rotated).await.is_ok());
        tx.commit().await.unwrap();
        client.execute("UPDATE df_local_demo.grants SET expires_at=clock_timestamp()-interval '1 second' WHERE credential=$1::bytea",&[&[0x95u8;32].as_slice()]).await.unwrap();
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::resolve_local_run(&tx,&rotated).await,Err(IdentityError::Refused));
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn local_run_holds_owner_and_bootstrap_rows_until_operation_commit() {
    runtime().block_on(async {
        let Some(scratch) = Scratch::open_local().await else {
            return;
        };
        seed_local_run(&scratch).await;
        let mut authorized = scratch.connect().await;
        let mut contender = scratch.connect().await;
        let tx = authorized.transaction().await.unwrap();
        assert!(
            imagery_identity::resolve_local_run(&tx, &local_run_grant())
                .await
                .is_ok()
        );
        for table in ["df_game.sessions", "df_local_demo.grants"] {
            let other = contender.transaction().await.unwrap();
            let error = other
                .query(&format!("SELECT 1 FROM {table} FOR UPDATE NOWAIT"), &[])
                .await
                .unwrap_err();
            assert_eq!(error.as_db_error().unwrap().code().code(), "55P03");
            other.rollback().await.unwrap();
        }
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}

#[test]
fn hosted_mode_requires_installed_identity_definer_and_admission_privileges() {
    runtime().block_on(async {
        let Some(scratch)=Scratch::open().await else {return;};
        let mut client=scratch.connect().await;
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::require_hosted_identity_schema(&tx).await,Ok(()));
        tx.commit().await.unwrap();
        client.batch_execute("REVOKE EXECUTE ON FUNCTION imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea) FROM df_imagery_admission").await.unwrap();
        let tx=client.transaction().await.unwrap();
        assert_eq!(imagery_identity::require_hosted_identity_schema(&tx).await,Err(IdentityError::Refused));
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}

#[path = "imagery_identity/authority_evidence.rs"]
mod authority_evidence;
