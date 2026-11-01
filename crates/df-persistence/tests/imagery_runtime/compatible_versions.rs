//! Genuine historical config snapshots publish real native assets before current-codec lookup.
use super::*;
use df_imagery::publication::{ImageResults, PublicationError, PublishedImage};
use df_persistence::imagery_publication::{
    PublicationAuthorityFuture, PublicationSource, PublicationSourceAuthority,
};
use tokio_postgres::Transaction;

fn legacy_config(snapshot: &VersionedSnapshot, revision: u8) -> VersionedSnapshot {
    assert_eq!(
        snapshot.revision(),
        12,
        "review suffix fixture on config codec changes"
    );
    assert!(matches!(revision, 10 | 11));
    let mut bytes = snapshot.bytes().to_vec();
    assert_eq!(
        bytes.last(),
        Some(&0),
        "fixture uses the absent v12 creator account"
    );
    bytes.truncate(bytes.len() - if revision == 10 { 14 } else { 1 });
    bytes[0] = revision;
    VersionedSnapshot::new(u32::from(revision), bytes, 1_048_576).unwrap()
}

#[derive(Clone)]
struct HistoricalJournal {
    inner: PostgresRuntimeJournal<TestIds>,
    config_revision: u8,
}
impl ImageJournal for HistoricalJournal {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        self.accept_scheduled(command, Urgency::Present).await
    }

    async fn accept_scheduled(
        &self,
        command: &AcceptCommand,
        urgency: Urgency,
    ) -> Result<AcceptOutcome, JournalError> {
        let mut historical = command.clone();
        historical.config = legacy_config(&command.config, self.config_revision);
        self.inner.accept_scheduled(&historical, urgency).await
    }

    async fn compatible_candidates(
        &self,
        owner: &ImageOwner,
        intent: &VersionedSnapshot,
        config: &VersionedSnapshot,
        after: Option<CompatibleCursor>,
    ) -> Result<Vec<OperationView>, JournalError> {
        self.inner
            .compatible_candidates(owner, intent, config, after)
            .await
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        self.inner.lookup_operation(owner, operation).await
    }

    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        self.inner.read_job(owner, job).await
    }

    async fn freeze_recipe(&self, command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        self.inner.freeze_recipe(command).await
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        self.inner.withdraw_interest(owner, interest, now_ms).await
    }
}

struct FixtureAuthority;
impl PublicationSourceAuthority for FixtureAuthority {
    fn lock_authority<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        current_owner: &'a ImageOwner,
        now_ms: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            if *current_owner == owner() && now_ms == NOW {
                Ok(())
            } else {
                Err(PublicationError::BindingConflict)
            }
        })
    }

    fn authorize_sources<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        job: &'a ImageJob,
        sources: &'a [PublicationSource],
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            if job.owner == owner() && sources.is_empty() {
                Ok(())
            } else {
                Err(PublicationError::BindingConflict)
            }
        })
    }
}

#[derive(Clone)]
struct AuthorizedResults(PostgresImagePublication);
impl ImageResults for AuthorizedResults {
    async fn read_result(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        self.0.read_result(owner, job).await
    }

    async fn read_reusable_result(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        now_ms: i64,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        self.0
            .read_reusable_result_with_authority(owner, job, now_ms, &FixtureAuthority)
            .await
    }
}

async fn row_counts(client: &Client) -> Vec<i64> {
    let row = client
        .query_one(
            "SELECT (SELECT count(*) FROM imagery.image_job), \
         (SELECT count(*) FROM imagery.image_operation), \
         (SELECT count(*) FROM imagery.image_interest), \
         (SELECT count(*) FROM imagery.image_execution_stage), \
         (SELECT count(*) FROM imagery.image_dispatch_attempt), \
         (SELECT count(*) FROM media.asset), \
         (SELECT count(*) FROM media.quota_reservation), \
         (SELECT count(*) FROM finance.provider_attempt)",
            &[],
        )
        .await
        .unwrap();
    (0..row.len()).map(|column| row.get(column)).collect()
}

#[tokio::test]
async fn real_v10_and_v11_ready_reuse_v12_after_restart_with_no_new_work_or_writes() {
    for revision in [10, 11] {
        let scratch = Scratch::open().await;
        let config = scratch.config.clone();
        let transport = PostgresTransport::PlaintextLoopbackOnly;
        let client = connect(&config).await;
        client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
        let ids = TestIds(Arc::new(AtomicU64::new(100)));
        let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 2)
            .await
            .unwrap();
        let capacity = CapacityScope::new("runtime/compatible-versions".to_owned()).unwrap();
        execution
            .set_capacity_bounds(
                capacity.as_str(),
                &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
                NOW,
            )
            .await
            .unwrap();
        execution
            .set_execution_gate(capacity.as_str(), 1, true, NOW)
            .await
            .unwrap();
        let journal = PostgresRuntimeJournal::open_existing(
            config.clone(),
            transport.clone(),
            2,
            Arc::new(ids.clone()),
            capacity.clone(),
            1,
            true,
        )
        .unwrap();
        let historical = HistoricalJournal {
            inner: journal.clone(),
            config_revision: revision,
        };
        let publication =
            PostgresImagePublication::open_existing(config.clone(), transport.clone(), 2)
                .await
                .unwrap();
        let mut produced_config = ImageConfig::defaults().unwrap();
        produced_config.mode = GlobalMode::LocalOnly;
        let service = ImageService::new(
            historical.clone(),
            produced_config.clone(),
            TestClock,
            ids.clone(),
            TestAuthority(owner()),
        )
        .unwrap();
        let mut demand = request();
        demand.identity.fingerprint = service.canonical_fingerprint(&demand).unwrap();
        let RequestOutcome::Recorded(accepted) = service.request(&(), &demand).await.unwrap()
        else {
            panic!("historical demand must be accepted once");
        };
        let stored = historical
            .read_job(&owner(), accepted.job)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.config.revision(), u32::from(revision));

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/runtime-test")
            .join(config.get_dbname().unwrap());
        let objects = Arc::new(NativeFileStore::new(&root, 16_777_216).unwrap());
        let catalog = Arc::new(
            PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
                .await
                .unwrap(),
        );
        let build_publisher = || {
            NativeImagePublisher::new(
                RuntimePublicationSettings {
                    catalog: CatalogConfig::default(),
                    layout: KeyLayout::new(ObjectEnvironment::new("runtime-test").unwrap()),
                    decode_concurrency: 1,
                },
                RuntimePublicationPorts {
                    catalog: catalog.clone(),
                    publication: publication.clone(),
                    objects: objects.clone(),
                    ids: Arc::new(ids.clone()),
                    clock: Arc::new(TestClock),
                },
            )
            .unwrap()
            .with_source_authority(Arc::new(FixtureAuthority))
        };
        let runtime = NativeImageRuntime {
            journal: historical,
            execution,
            sources: NoSources,
            publisher: build_publisher(),
            admission: build_publisher(),
            clock: TestClock,
            ids: ids.clone(),
            credentials: NoCredentials,
            costs: NoCost,
            cost_policy: NoCostPolicy,
            usage: PostgresImageUsage::open_existing(config.clone(), transport.clone(), 2).unwrap(),
            telemetry: TestTelemetry,
            settings: RuntimeSettings {
                config: produced_config.clone(),
                active_config_revision: 1,
                capacity: capacity.clone(),
                poll_interval: Duration::from_millis(10),
            },
        };
        let actual = runtime.run_next().await.unwrap().unwrap();
        assert_eq!(
            catalog.asset(actual.asset).await.unwrap().unwrap().state,
            df_assets::AssetState::Ready
        );
        let old_key_version: i16 = if revision == 10 {
            1
        } else {
            i16::try_from(COMPATIBLE_KEY_VERSION).unwrap()
        };
        let old_key = [0x77_u8; 32];
        client.execute(
            "UPDATE imagery.image_job SET compatible_key=$2,compatible_key_version=$3 WHERE job_id=$1",
            &[&accepted.job.as_bytes().as_slice(), &old_key.as_slice(), &old_key_version],
        ).await.unwrap();
        drop(service);
        let before = row_counts(&client).await;
        let before_view = journal
            .lookup_operation(&owner(), accepted.handle.operation)
            .await
            .unwrap();
        let before_asset = catalog.asset(actual.asset).await.unwrap();
        let ids_before = ids.0.load(Ordering::SeqCst);
        let restarted_journal = PostgresRuntimeJournal::open_existing(
            config.clone(),
            transport.clone(),
            2,
            Arc::new(ids.clone()),
            capacity,
            1,
            true,
        )
        .unwrap();
        let mut current = produced_config;
        current.mode = GlobalMode::CacheOnly;
        current.quota.enabled = false;
        current.quota.cooldown_ms += 1;
        current.quota.maximum_accounts += 1;
        current.backends[0].max_concurrent_jobs += 1;
        let restarted = ImageService::new(
            restarted_journal,
            current,
            TestClock,
            ids.clone(),
            TestAuthority(owner()),
        )
        .unwrap()
        .with_results(AuthorizedResults(publication.clone()))
        .with_observations(
            PostgresImageObservations::open_existing(config.clone(), transport, 2).unwrap(),
        );
        for (index, execution) in [
            ExecutionMode::Live,
            ExecutionMode::PreparedOnly,
            ExecutionMode::Replay,
        ]
        .into_iter()
        .enumerate()
        {
            let mut lookup = demand.clone();
            lookup.identity.operation =
                OperationId::from_bytes(&[40 + u8::try_from(index).unwrap(); 16]).unwrap();
            lookup.execution = execution;
            lookup.identity.fingerprint = restarted.canonical_fingerprint(&lookup).unwrap();
            let RequestOutcome::Reused(reused) = restarted.request(&(), &lookup).await.unwrap()
            else {
                panic!("historical Ready must not become a miss or new demand");
            };
            assert_eq!(reused.result, Some(actual.clone()));
            assert_eq!(reused.handle, accepted.handle);
        }
        assert_eq!(ids.0.load(Ordering::SeqCst), ids_before);
        assert_eq!(row_counts(&client).await, before);
        assert_eq!(
            journal
                .lookup_operation(&owner(), accepted.handle.operation)
                .await
                .unwrap(),
            before_view
        );
        assert_eq!(catalog.asset(actual.asset).await.unwrap(), before_asset);
        let hint = client.query_one(
            "SELECT compatible_key,compatible_key_version FROM imagery.image_job WHERE job_id=$1",
            &[&accepted.job.as_bytes().as_slice()],
        ).await.unwrap();
        assert_eq!(hint.get::<_, Vec<u8>>(0), old_key);
        assert_eq!(hint.get::<_, Option<i16>>(1), Some(old_key_version));
        assert!(runtime.run_next().await.unwrap().is_none());
        assert_eq!(row_counts(&client).await, before);
        drop(restarted);
        drop(runtime);
        drop(journal);
        drop(client);
        scratch.finish().await;
    }
}
