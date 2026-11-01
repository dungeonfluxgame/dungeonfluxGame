//! Real disposable PostgreSQL proof of indexed and legacy-compatible candidate pagination.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use df_assets::{Audience, IdSource};
use df_imagery::*;
use df_imagery::{CompatibleCursor, MAXIMUM_COMPATIBLE_CANDIDATES};
use df_persistence::PostgresImageJournal;
use df_persistence::postgres_tls::PostgresTransport;
use df_types::{AccountId, OperationId};
use tokio_postgres::{Client, Config, NoTls};

const NOW_MS: i64 = 1_700_000_000_000;

struct Connection {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Deref for Connection {
    type Target = Client;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}
impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn connect(config: &Config) -> Connection {
    let (client, connection) = config.connect(NoTls).await.expect("owned PG55471");
    let task = tokio::spawn(async move {
        // A dropped database ends its own connection with an administrator error; only the test sees it.
        if let Err(error) = connection.await {
            eprintln!("scratch PostgreSQL connection closed: {error}");
        }
    });
    Connection { client, task }
}

struct Scratch {
    admin: Config,
    config: Config,
}
impl Scratch {
    async fn open() -> Self {
        assert_eq!(
            std::env::var("DUNGEONFLUX_REQUIRE_SCRATCH_PG").as_deref(),
            Ok("1"),
            "this test requires explicit owned scratch PostgreSQL opt-in"
        );
        let mut admin = Config::new();
        admin
            .host("127.0.0.1")
            .port(55471)
            .user("postgres")
            .dbname("postgres");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!("df_compatible_{}_{}", std::process::id(), nanos);
        connect(&admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let mut config = admin.clone();
        config.dbname(&database);
        Self { admin, config }
    }

    async fn finish(self) {
        let database = self.config.get_dbname().unwrap();
        connect(&self.admin)
            .await
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .unwrap();
    }
}

#[derive(Clone)]
struct Ids(Arc<AtomicU64>);
impl IdSource for Ids {
    fn next_id_bytes(&self) -> [u8; 16] {
        let number = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut bytes = [0; 16];
        bytes[8..].copy_from_slice(&number.to_be_bytes());
        bytes
    }
}

struct Clock;
impl ServiceClock for Clock {
    fn now_ms(&self) -> i64 {
        NOW_MS
    }
}
struct Authority(ImageOwner);
impl ImageAuthorizer for Authority {
    type Principal = ();

    fn authorize_request(&self, _: &(), request: &ImageRequest) -> Result<(), AuthDenied> {
        (request.identity.owner == self.0)
            .then_some(())
            .ok_or(AuthDenied)
    }

    fn authorize_owner(
        &self,
        _: &(),
        owner: &ImageOwner,
        _: ServiceAction,
    ) -> Result<(), AuthDenied> {
        (*owner == self.0).then_some(()).ok_or(AuthDenied)
    }

    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        (view.job.owner == self.0 && *stored.owner == self.0)
            .then_some(())
            .ok_or(AuthDenied)
    }
}

fn owner(byte: u8) -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&[byte; 16]).unwrap(),
    }
}

fn request(owner: ImageOwner, number: u64) -> ImageRequest {
    let mut bytes = [0; 16];
    bytes[8..].copy_from_slice(&number.to_be_bytes());
    let output = RasterOutputSpec::new(
        Dimensions::new(1, 1).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner,
            operation: OperationId::from_bytes(&bytes).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a quiet inn", 1).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: number,
        output: OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), output),
    }
}

#[tokio::test]
async fn real_journal_pages_legacy_history_without_cross_owner_leaks_or_interest_revival() {
    let scratch = Scratch::open().await;
    let journal = PostgresImageJournal::open(
        scratch.config.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        2,
    )
    .await
    .unwrap();
    let ids = Ids(Arc::new(AtomicU64::new(0)));
    let service = ImageService::new(
        journal.clone(),
        ImageConfig::defaults().unwrap(),
        Clock,
        ids.clone(),
        Authority(owner(1)),
    )
    .unwrap();
    let mut oldest = None;
    for number in 1..=65 {
        let mut request = request(owner(1), number);
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        let RequestOutcome::Recorded(view) = service.request(&(), &request).await.unwrap() else {
            panic!("distinct source revisions must create distinct demand");
        };
        if number == 1 {
            oldest = Some(view);
        }
    }
    let oldest = oldest.unwrap();
    let foreign_service = ImageService::new(
        journal.clone(),
        ImageConfig::defaults().unwrap(),
        Clock,
        ids.clone(),
        Authority(owner(2)),
    )
    .unwrap();
    let mut foreign = request(owner(2), 1);
    foreign.identity.fingerprint = foreign_service.canonical_fingerprint(&foreign).unwrap();
    foreign_service.request(&(), &foreign).await.unwrap();

    let raw = connect(&scratch.config).await;
    let indexed: i64 = raw
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_job WHERE compatible_key IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(indexed, 66);
    let current_versions: i64 = raw
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_job WHERE compatible_key_version=$1",
            &[&i16::try_from(df_imagery::COMPATIBLE_KEY_VERSION).unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(current_versions, 66);
    raw.execute(
        "UPDATE imagery.image_job SET compatible_key=decode(repeat('99',32),'hex'), compatible_key_version=1",
        &[],
    )
    .await
    .unwrap();
    raw.execute(
        "UPDATE imagery.image_job SET compatible_key_version=NULL WHERE job_id=(SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 ORDER BY job_id DESC LIMIT 1)",
        &[&df_imagery::owner_scope_bytes(&owner(1))],
    )
    .await
    .unwrap();
    raw.execute(
        "UPDATE imagery.image_job SET compatible_key=NULL,compatible_key_version=NULL WHERE job_id=$1",
        &[&oldest.job.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    journal
        .withdraw_interest(&owner(1), oldest.interest, NOW_MS + 1)
        .await
        .unwrap();
    let retained = journal
        .lookup_operation(&owner(1), oldest.handle.operation)
        .await
        .unwrap()
        .unwrap();
    let before_ids = ids.0.load(Ordering::SeqCst);
    let first = journal
        .compatible_candidates(&owner(1), &retained.job.intent, &retained.job.config, None)
        .await
        .unwrap();
    assert_eq!(first.len(), MAXIMUM_COMPATIBLE_CANDIDATES);
    assert!(first.iter().all(|view| view.job.owner == owner(1)));
    let last = first.last().unwrap();
    let second = journal
        .compatible_candidates(
            &owner(1),
            &retained.job.intent,
            &retained.job.config,
            Some(CompatibleCursor {
                created_at_ms: last.job.created_at_ms,
                job: last.job.job,
            }),
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].receipt.operation, oldest.handle.operation);
    assert_eq!(second[0].job.state, JobState::Retired);
    assert_eq!(second[0].interest.state, InterestState::Withdrawn);
    assert_eq!(second[0], retained);
    assert_eq!(ids.0.load(Ordering::SeqCst), before_ids);
    let rows: i64 = raw
        .query_one("SELECT COUNT(*) FROM imagery.image_operation", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 66);
    drop(raw);
    drop(service);
    drop(foreign_service);
    drop(journal);
    scratch.finish().await;
}
