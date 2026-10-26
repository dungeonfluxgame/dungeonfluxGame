use super::*;
use crate::{account_closure::TimeoutPolicy, postgres_tls::PostgresTransport};
use df_customer::{
    AccountStore, Reauthentication,
    contract::{Harness, START_MS},
    integration::{DAY_MS, IssueToken, Peppers, PersonalToken, TokenRefusal},
};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls};

struct Connection {
    client: Client,
    driver: JoinHandle<()>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
async fn connect(config: &Config) -> Connection {
    let (client, connection) =
        tokio::time::timeout(std::time::Duration::from_secs(5), config.connect(NoTls))
            .await
            .expect("scratch connect deadline")
            .expect("APIBUILD2 scratch PostgreSQL");
    Connection {
        client,
        driver: tokio::spawn(async move {
            connection.await.expect("owned scratch connection");
        }),
    }
}
struct Fixture {
    admin: Connection,
    data: Connection,
    database: String,
}
impl Fixture {
    async fn open() -> (Self, PostgresAccountStore) {
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(55911)
            .user("postgres")
            .dbname("postgres");
        let admin = connect(&config).await;
        let database = format!(
            "apibuild2_pat_{}",
            df_customer::SecretToken::generate()
                .unwrap()
                .text()
                .get(..16)
                .unwrap()
        );
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        config.dbname(&database);
        let store = PostgresAccountStore::open(
            config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap();
        let data = connect(&config).await;
        (
            Self {
                admin,
                data,
                database,
            },
            store,
        )
    }
    async fn finish(self) {
        let Self {
            admin,
            data,
            database,
        } = self;
        drop(data);
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {
                let count:i64=admin.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname=$1",&[&database]).await.unwrap().get(0);
                if count==0 {break;}tokio::task::yield_now().await;
            }
        }).await.expect("owned scratch connection closure");
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database}"))
            .await
            .unwrap();
    }
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}
fn peppers() -> Peppers {
    Peppers::new(7, [73; 32], None).unwrap()
}
async fn issue(
    h: &Harness<PostgresAccountStore>,
    caller: &df_customer::Authenticated,
) -> df_customer::integration::IssuedToken {
    match h
        .service
        .issue_personal_token(
            caller,
            Reauthentication::RecentSignIn,
            &peppers(),
            "My trusted script",
            vec![Scope::Account],
            7,
        )
        .await
        .unwrap()
    {
        IssueToken::Issued(token) => token,
        IssueToken::Refused(reason) => panic!("unexpected refusal {reason:?}"),
    }
}

#[test]
fn pat_secret_is_shown_once_and_only_keyed_digest_and_metadata_are_stored() {
    run(async {
        let (f, store) = Fixture::open().await;
        let h = Harness::start(store.clone()).await;
        let (caller, _) = h.verified_account("pat@example.test").await;
        let issued = issue(&h, &caller).await;
        let secret = issued.secret.reveal();
        assert!(
            h.service
                .integration_principal(&secret, &peppers())
                .await
                .unwrap()
                .is_some()
        );
        let listed = h.service.personal_tokens(&caller).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].last_used_at_ms.is_some());
        let row = f
            .data
            .client
            .query_one(
                "SELECT secret_digest,pepper_id,to_jsonb(p)::text FROM accounts.personal_token p",
                &[],
            )
            .await
            .unwrap();
        let digest: Vec<u8> = row.get(0);
        assert_eq!(digest.len(), 32);
        assert_eq!(row.get::<_, i64>(1), 7);
        let json: String = row.get(2);
        assert!(!json.contains(&secret));
        assert!(!json.contains("secret\":"));
        let parsed = PersonalToken::parse(&secret).unwrap();
        assert_eq!(digest, peppers().digest(&parsed));
        drop(h);
        drop(store);
        f.finish().await;
    });
}

#[test]
fn pat_create_limit_is_ten_live_tokens_and_revoke_is_owner_scoped() {
    run(async {
        let (f, store) = Fixture::open().await;
        let h = Harness::start(store.clone()).await;
        let (caller, _) = h.verified_account("ten@example.test").await;
        let (other, _) = h.verified_account("other@example.test").await;
        let mut tokens = Vec::new();
        for _ in 0..10 {
            tokens.push(issue(&h, &caller).await);
        }
        assert!(matches!(
            h.service
                .issue_personal_token(
                    &caller,
                    Reauthentication::RecentSignIn,
                    &peppers(),
                    "eleventh",
                    vec![Scope::Account],
                    7
                )
                .await
                .unwrap(),
            IssueToken::Refused(TokenRefusal::LimitReached)
        ));
        assert_eq!(
            h.service
                .revoke_personal_token(
                    &other,
                    Reauthentication::RecentSignIn,
                    tokens[0].metadata.id
                )
                .await
                .unwrap(),
            Ok(false)
        );
        assert_eq!(
            h.service
                .revoke_personal_token(
                    &caller,
                    Reauthentication::RecentSignIn,
                    tokens[0].metadata.id
                )
                .await
                .unwrap(),
            Ok(true)
        );
        assert!(
            h.service
                .integration_principal(&tokens[0].secret.reveal(), &peppers())
                .await
                .unwrap()
                .is_none()
        );
        let replacement = issue(&h, &caller).await;
        assert!(
            h.service
                .integration_principal(&replacement.secret.reveal(), &peppers())
                .await
                .unwrap()
                .is_some()
        );
        drop(h);
        drop(store);
        f.finish().await;
    });
}

#[test]
fn pat_expiry_is_exact_and_closure_irreversibly_revokes_before_reopen() {
    run(async {
        let (f, store) = Fixture::open().await;
        let h = Harness::start(store.clone()).await;
        let (caller, _) = h.verified_account("expiry@example.test").await;
        let issued = issue(&h, &caller).await;
        let secret = issued.secret.reveal();
        h.clock.advance_ms(7 * DAY_MS - 1);
        assert!(
            h.service
                .integration_principal(&secret, &peppers())
                .await
                .unwrap()
                .is_some()
        );
        h.clock.advance_ms(1);
        assert!(
            h.service
                .integration_principal(&secret, &peppers())
                .await
                .unwrap()
                .is_none()
        );
        // Store-level closure is the account transaction boundary, independent of HTTP proofs.
        let second = StoredToken {
            metadata: TokenMetadata {
                id: TokenId([99; 16]),
                account: caller.account.id,
                label: "closure".into(),
                scopes: vec![Scope::Account],
                created_at_ms: START_MS + 7 * DAY_MS,
                expires_at_ms: START_MS + 14 * DAY_MS,
                revoked_at_ms: None,
                last_used_at_ms: None,
            },
            pepper_id: 7,
            digest: [32; 32],
        };
        assert_eq!(
            store.create_personal_token(second).await.unwrap(),
            CreateTokenOutcome::Created
        );
        assert!(matches!(store.close_account(caller.account.id,START_MS+7*DAY_MS,START_MS+8*DAY_MS).await.unwrap(),df_customer::CloseOutcome::Closed { .. }));
        let revoked = store
            .personal_token(TokenId([99; 16]))
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.metadata.revoked_at_ms.is_some());
        assert!(store.reopen_account(caller.account.id,START_MS+7*DAY_MS).await.unwrap());
        assert!(
            store
                .personal_token(TokenId([99; 16]))
                .await
                .unwrap()
                .unwrap()
                .metadata
                .revoked_at_ms
                .is_some()
        );
        drop(h);
        drop(store);
        f.finish().await;
    });
}

#[test]
fn pat_creation_and_revocation_require_a_five_minute_proof_and_rotation_rekeys_on_use() {
    run(async {
        let (f, store) = Fixture::open().await;
        let h = Harness::start(store.clone()).await;
        let (caller, _) = h.verified_account("proof@example.test").await;
        let issued = issue(&h, &caller).await;
        let rotated = Peppers::new(8, [81; 32], Some((7, [73; 32], START_MS + DAY_MS))).unwrap();
        assert!(
            h.service
                .integration_principal(&issued.secret.reveal(), &rotated)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store
                .personal_token(issued.metadata.id)
                .await
                .unwrap()
                .unwrap()
                .pepper_id,
            8
        );
        h.clock.advance_ms(300_001);
        assert!(matches!(
            h.service
                .issue_personal_token(
                    &caller,
                    Reauthentication::RecentSignIn,
                    &peppers(),
                    "late",
                    vec![Scope::Account],
                    7
                )
                .await
                .unwrap(),
            IssueToken::Refused(TokenRefusal::ReauthenticationRequired)
        ));
        assert_eq!(
            h.service
                .revoke_personal_token(&caller, Reauthentication::RecentSignIn, issued.metadata.id)
                .await
                .unwrap(),
            Err(TokenRefusal::ReauthenticationRequired)
        );
        drop(h);
        drop(store);
        f.finish().await;
    });
}

#[test]
fn pat_concurrent_creation_serializes_the_last_available_slot(){run(async{
    let(f,store)=Fixture::open().await;let h=Harness::start(store.clone()).await;let(caller,_)=h.verified_account("race@example.test").await;
    for _ in 0..9{issue(&h,&caller).await;}
    let peppers=peppers();
    let(a,b)=tokio::join!(h.service.issue_personal_token(&caller,Reauthentication::RecentSignIn,&peppers,"race one",vec![Scope::Account],7),h.service.issue_personal_token(&caller,Reauthentication::RecentSignIn,&peppers,"race two",vec![Scope::Account],7));
    let outcomes=[a.unwrap(),b.unwrap()];assert_eq!(outcomes.iter().filter(|o|matches!(o,IssueToken::Issued(_))).count(),1);assert_eq!(outcomes.iter().filter(|o|matches!(o,IssueToken::Refused(TokenRefusal::LimitReached))).count(),1);
    assert_eq!(h.service.personal_tokens(&caller).await.unwrap().len(),10);
    drop(h);drop(store);f.finish().await;
});}

#[test]
fn pat_revocation_rolls_back_with_an_uncommitted_account_closure(){run(async{
    let(mut f,store)=Fixture::open().await;let h=Harness::start(store.clone()).await;let(caller,_)=h.verified_account("rollback@example.test").await;let issued=issue(&h,&caller).await;
    let transaction=f.data.client.transaction().await.unwrap();transaction.execute("UPDATE accounts.account SET status='closing',delete_after_ms=$2 WHERE account_id=$1",&[&caller.account.id.as_bytes().as_slice(),&i64::try_from(START_MS+DAY_MS).unwrap()]).await.unwrap();
    assert!(transaction.query_one("SELECT revoked_at_ms IS NOT NULL FROM accounts.personal_token WHERE token_id=$1",&[&issued.metadata.id.0.as_slice()]).await.unwrap().get::<_,bool>(0));transaction.rollback().await.unwrap();
    assert!(h.service.integration_principal(&issued.secret.reveal(),&peppers()).await.unwrap().is_some());drop(h);drop(store);f.finish().await;
});}

#[test]
fn pat_concurrent_rotation_accepts_a_token_already_rekeyed_by_another_request(){run(async{
    let(f,store)=Fixture::open().await;let h=Harness::start(store.clone()).await;let(caller,_)=h.verified_account("rotation-race@example.test").await;let issued=issue(&h,&caller).await;
    let original=store.personal_token(issued.metadata.id).await.unwrap().unwrap();let active=Peppers::new(8,[81;32],Some((7,[73;32],START_MS+DAY_MS))).unwrap();let digest=active.digest(&issued.secret);
    assert!(store.used_personal_token(issued.metadata.id,original.digest,8,digest,START_MS).await.unwrap());
    assert!(store.used_personal_token(issued.metadata.id,original.digest,8,digest,START_MS).await.unwrap());
    drop(h);drop(store);f.finish().await;
});}
