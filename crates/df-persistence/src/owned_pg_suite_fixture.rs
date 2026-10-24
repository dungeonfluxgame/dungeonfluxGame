//! Ignored actual owned-PostgreSQL suite; ROOT must register and explicitly release invocation.
//! The ordinary native test run creates no runtime or connection for this suite.
use crate::checkpoint_codec::CodecLimits;
use crate::checkpoint_codec::decode_receipt;
use crate::decision_adapter::RecoverySource;
use crate::native_bridge::{NativeRepositoryOptions, NativeVerifierSource, PostgresRepository};
use crate::native_connection::{OwnedConnection, TransactionBounds, actor_block_on};
use crate::owned_pg_admin_fixture::{
    FixtureGrant, FixtureProof, install_fixture_current_epoch, seed_grant, seed_proof, seed_session,
};
use crate::owned_pg_admin_fixture::{physical_snapshot, validate_physical_commit};
use crate::owned_pg_commit_proxy_fixture::{
    CommitAckObservation, ProxyBounds, ProxyError, drop_commit_acknowledgement,
};
use crate::owned_pg_composition_fixture::{bind_registered_scope, compose_registered_repository};
use crate::owned_pg_fault_fixture::{InsertedFamily, install_fault, remove_fault};
use crate::owned_pg_membership_fixture::FixtureMembershipAuthority;
use crate::owned_pg_observations_fixture::{
    PhysicalObservation, observe_configured_read_refusal, observe_family_rollback,
    observe_historical_key_reload, observe_scope_refusal, observe_sequence_exhaustion,
    observe_success_and_retained,
};
use crate::owned_pg_scope_fixture::FixtureBoundInput;
use crate::owned_pg_scope_fixture::fixture_verifier;
use df_model::checkpoint::*;
use df_observe::OperationContext;
use df_session::submission::{
    CommitOutcome, OperationLookup, OperationScope, RepositoryError, SessionRepository,
};
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};
use tokio_postgres::{Config, NoTls, config::SslMode};

#[path = "owned_pg_recovery_lease_fixture.rs"]
mod recovery_lease;

// These private fixture results carry a still-owned driver on exhausted cleanup.
// Each close retains the original native join bound; there are at most three
// close attempts, never an unbounded retry. Successful cleanup cannot erase the
// first close error or qualify a failed shutdown as successful.
struct FixtureClosedRepository {
    repository: PostgresRepository<FixtureMembershipAuthority>,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
    final_cleanup: Option<Result<(), RepositoryError>>,
}
struct FixturePendingRepositoryCleanup {
    repository: PostgresRepository<FixtureMembershipAuthority>,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
    final_cleanup: Option<Result<(), RepositoryError>>,
}
fn fixture_driver_cleanup_pending(state: crate::native_connection::DiscardState) -> bool {
    matches!(
        state,
        crate::native_connection::DiscardState::NotStarted
            | crate::native_connection::DiscardState::JoinPending
    )
}
fn retain_fixture_repository_after_retry(
    mut repository: PostgresRepository<FixtureMembershipAuthority>,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
) -> Result<FixtureClosedRepository, Box<FixturePendingRepositoryCleanup>> {
    let final_cleanup = if fixture_driver_cleanup_pending(repository.fixture_discard_state()) {
        Some(repository.close())
    } else {
        None
    };
    if fixture_driver_cleanup_pending(repository.fixture_discard_state()) {
        return Err(Box::new(FixturePendingRepositoryCleanup {
            repository,
            first_close,
            retry_close,
            final_cleanup,
        }));
    }
    Ok(FixtureClosedRepository {
        repository,
        first_close,
        retry_close,
        final_cleanup,
    })
}
impl FixturePendingRepositoryCleanup {
    fn fail_fixture_while_retaining_owner(self: Box<Self>) -> ! {
        // This is terminal FAILURE of an isolated executable, never joined
        // shutdown evidence. The owner is live until process termination; ROOT's
        // runtime runner must report failure and reap only its owned process tree.
        // No implicit Drop of a pending driver, leaked owner or fabricated join.
        eprintln!(
            "registered fixture cleanup exhausted with repository owner retained: first={:?}, retry={:?}, final={:?}, pending={}; ROOT owned-runtime cleanup required; no joined shutdown claim",
            self.first_close,
            self.retry_close,
            self.final_cleanup,
            fixture_driver_cleanup_pending(self.repository.fixture_discard_state())
        );
        std::process::exit(1)
    }
}
struct FixtureClosedConnection {
    connection: OwnedConnection,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
    final_cleanup: Option<Result<(), RepositoryError>>,
}
struct FixturePendingConnectionCleanup {
    connection: OwnedConnection,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
    final_cleanup: Option<Result<(), RepositoryError>>,
}
fn retain_fixture_connection_after_retry(
    runtime: &tokio::runtime::Handle,
    mut connection: OwnedConnection,
    first_close: Result<(), RepositoryError>,
    retry_close: Option<Result<(), RepositoryError>>,
) -> Result<FixtureClosedConnection, Box<FixturePendingConnectionCleanup>> {
    let final_cleanup = if fixture_driver_cleanup_pending(connection.discard_state()) {
        Some(actor_block_on(runtime, connection.close()).and_then(|value| value))
    } else {
        None
    };
    if fixture_driver_cleanup_pending(connection.discard_state()) {
        return Err(Box::new(FixturePendingConnectionCleanup {
            connection,
            first_close,
            retry_close,
            final_cleanup,
        }));
    }
    Ok(FixtureClosedConnection {
        connection,
        first_close,
        retry_close,
        final_cleanup,
    })
}
impl FixturePendingConnectionCleanup {
    fn fail_fixture_while_retaining_owner(self: Box<Self>) -> ! {
        eprintln!(
            "registered fixture cleanup exhausted with connection owner retained: first={:?}, retry={:?}, final={:?}, pending={}; ROOT owned-runtime cleanup required; no joined shutdown claim",
            self.first_close,
            self.retry_close,
            self.final_cleanup,
            fixture_driver_cleanup_pending(self.connection.discard_state())
        );
        std::process::exit(1)
    }
}

// The blocking actor creates this future before entering its injected Handle.
// Construct the Tokio timer only when that Handle polls the future; the caller's
// already-captured absolute deadline and Elapsed error remain unchanged.
async fn within_deadline<T>(
    deadline: Instant,
    future: impl std::future::Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    timeout_at(deadline, future).await
}

#[test]
fn blocking_actor_constructs_fixture_deadline_before_entering_its_runtime() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let polled = std::cell::Cell::new(false);
    assert!(tokio::runtime::Handle::try_current().is_err());
    let deadline = Instant::now() + Duration::from_secs(1);
    let future = within_deadline(deadline, async {
        polled.set(true);
        7
    });
    assert!(!polled.get());
    assert!(tokio::runtime::Handle::try_current().is_err());
    assert_eq!(actor_block_on(runtime.handle(), future), Ok(Ok(7)));
    assert!(polled.get());
    let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    let pending = within_deadline(expired, std::future::pending::<()>());
    assert!(actor_block_on(runtime.handle(), pending).unwrap().is_err());
}

// Config::port appends: cloning a configured endpoint cannot replace its port.
// Rebuild the admitted loopback endpoint so each route has exactly one port.
fn fixture_lost_ack_configuration(template: &Config, port: u16) -> Result<Config, RepositoryError> {
    let [tokio_postgres::config::Host::Tcp(host)] = template.get_hosts() else {
        return Err(RepositoryError::Unavailable);
    };
    if host != "127.0.0.1"
        || !template.get_hostaddrs().is_empty()
        || template.get_ssl_mode() != SslMode::Disable
    {
        return Err(RepositoryError::Unavailable);
    }
    let database = template.get_dbname().ok_or(RepositoryError::Unavailable)?;
    let mut configuration = Config::new();
    configuration
        .host(host)
        .port(port)
        .dbname(database)
        .user("df_persistence_fixture_runtime_a")
        .ssl_mode(SslMode::Disable);
    Ok(configuration)
}

#[test]
fn lost_ack_configs_select_single_direct_and_proxy_ports_without_mutating_template() {
    let mut template = Config::new();
    template
        .host("127.0.0.1")
        .port(55449)
        .dbname("df_persistence_i01_configuration_test")
        .user("df_persistence_i01_configuration_admin")
        .ssl_mode(SslMode::Disable);
    let direct = fixture_lost_ack_configuration(&template, 55449).unwrap();
    let proxied = fixture_lost_ack_configuration(&template, 55450).unwrap();
    assert_eq!(direct.get_ports(), &[55449]);
    assert_eq!(proxied.get_ports(), &[55450]);
    for endpoint in [&direct, &proxied] {
        assert_eq!(endpoint.get_hosts(), template.get_hosts());
        assert_eq!(endpoint.get_dbname(), template.get_dbname());
        assert_eq!(
            endpoint.get_user(),
            Some("df_persistence_fixture_runtime_a")
        );
        assert_eq!(endpoint.get_ssl_mode(), SslMode::Disable);
        assert!(endpoint.get_hostaddrs().is_empty());
    }
    assert_eq!(template.get_ports(), &[55449]);
    assert_eq!(
        template.get_user(),
        Some("df_persistence_i01_configuration_admin")
    );
}

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn entity(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}

fn member(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
}

fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}

fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: revision(2, 8),
    }
}

fn content() -> ContentReference {
    ContentReference {
        package: label("fixture-package-1"),
        entry: label("fixture-entry-1"),
    }
}

fn rule() -> RuleReference {
    RuleReference {
        catalog: label("fixture-catalog-1"),
        source: label("fixture-source-1"),
        entry: label("fixture-entry-1"),
        clause: label("fixture-clause-1"),
    }
}

fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("fixture-rules-1"),
            catalog: label("fixture-catalog-1"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: label("fixture-sources-1"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: label("fixture-handler-1"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: label("fixture-content-1"),
            content_digest: ContentDigest([4; 32]),
            package: label("fixture-package-1"),
            package_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("fixture-source-1"),
            Some("fixture-native-1"),
            Some("fixture-wasm-1"),
            Some("fixture-config-1"),
            Some("fixture-content-1"),
        )
        .unwrap(),
    }
}

fn resource_constraints() -> Vec<ResourceConstraint> {
    vec![ResourceConstraint {
        owner: entity(4),
        resource: label("fixture-resource-1"),
        minimum: 0,
        maximum: 8,
        source: rule(),
    }]
}

fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}

fn state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 120,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member: member(3),
            character: Some(entity(4)),
        }],
        entities: vec![WorldEntity {
            id: entity(4),
            definition: content(),
            location: None,
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: label("fixture-entity-1"),
        }],
        characters: vec![CharacterState {
            entity: entity(4),
            build: content(),
            owner: member(3),
            choices: vec![],
        }],
        resources: vec![ResourceState {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            value: 4,
            minimum: 0,
            maximum: 8,
            source: rule(),
        }],
        inventory: vec![],
        facts: vec![],
        draws: vec![],
        decisions: vec![],
        pending: vec![],
        intents: vec![],
        timers: vec![],
        active_effects: vec![],
        knowledge: vec![],
        beliefs: vec![],
        memories: vec![],
        schedules: vec![],
        threats: vec![],
        relationships: vec![],
        conversations: vec![],
        obligations: vec![],
        narrative: NarrativeState {
            definition: content(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: content(),
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: continuity(),
    }
}

fn fact(value: u8, ordinal: u32) -> GameFact {
    GameFact {
        id: FactId::from_bytes(&[value; 16]).unwrap(),
        revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        ordinal,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: vec![entity(4)],
        },
    }
}

fn fixture_character_appearance() -> CharacterAppearance {
    CharacterAppearance {
        features: "Copper curls and a scar over the left eyebrow.".to_owned(),
        outfit: "A green coat with silver clasps.".to_owned(),
        outfit_revision: label("fixture-outfit-2"),
    }
}

fn continuity() -> ContinuityState {
    ContinuityState {
        creation: vec![],
        simulation: vec![],
        catch_up: None,
        environment: vec![],
        travel: vec![],
        witnesses: vec![],
        rumors: vec![],
        journal: vec![],
        summaries: vec![],
        retrieval: vec![],
        retrieved: vec![],
        consolidation: vec![],
        npcs: vec![],
        hooks: vec![],
        arcs: vec![],
        remote: None,
        presence: vec![],
        audio: None,
        private_offers: vec![],
        knowledge_cues: vec![],
        moments: vec![],
        demands: vec![],
        asset_jobs: vec![],
        asset_dependencies: vec![],
        canonical_packs: vec![CanonicalPack {
            revision: label("fixture-pack-1"),
            digest: ContentDigest([12; 32]),
            bible: VisualBible {
                revision: label("fixture-bible-1"),
                definition: content(),
                palette: vec![],
                style: "fixture style".to_owned(),
                references: vec![],
            },
            identities: vec![EntityIdentityRevision {
                entity: entity(4),
                revision: label("fixture-identity-1"),
                character_appearance: Some(fixture_character_appearance()),
                source_facts: vec![],
                appearances: vec![],
                voice: None,
                sound: vec![],
            }],
        }],
        shots: vec![],
        prefetch: None,
        scenes: vec![],
        item_origins: vec![],
        bookends: vec![],
        exports: vec![],
        critical_cues: vec![],
        content_candidates: vec![],
        content_admissions: vec![],
        recovery: RecoveryState {
            origin: None,
            retired_epochs: vec![],
            lost_ranges: vec![],
            suppression_generation: 0,
            redacted_records: vec![],
            unavailable_sources: vec![],
        },
    }
}

fn codec_limits() -> CodecLimits {
    CodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 8 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 256,
    }
}
fn admitted_case(session: u8) -> (Checkpoint, Checkpoint, GameInput) {
    admitted_case_at_revision(session, revision(2, 7))
}

fn admitted_case_at_revision(
    session: u8,
    current: SessionRevision,
) -> (Checkpoint, Checkpoint, GameInput) {
    admitted_case_for_operation(session, current, 6)
}

fn admitted_case_for_operation(
    session: u8,
    current: SessionRevision,
    operation_byte: u8,
) -> (Checkpoint, Checkpoint, GameInput) {
    let expected = Basis {
        session: SessionId::from_bytes(&[session; 16]).unwrap(),
        revision: current,
        ..basis()
    };
    let next = Basis {
        revision: expected.revision.next_sequence().unwrap(),
        ..expected
    };
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let baseline = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        expected,
        pins(),
        state(),
        inventory(),
        limits(),
    )
    .unwrap();
    let operation = OperationId::from_bytes(&[operation_byte; 16]).unwrap();
    let mut supplied = state();
    let mut fact = fact(7, 0);
    fact.operation = operation;
    fact.revision = next.revision;
    supplied.facts.push(fact.clone());
    let effect = DurableIntent {
        id: EffectId::from_bytes(&[session; 16]).unwrap(),
        basis: next,
        operation,
        slot: 0,
        kind: EffectKind::PublishPresentation,
        job: None,
        timer: None,
        generation: 2,
        status: DurableStatus::Pending,
        definition: content(),
    };
    supplied.intents.push(effect.clone());
    supplied.decisions.push(AcceptedDecision {
        operation,
        revision: next.revision,
        facts: vec![fact.id],
        draws: vec![],
        effects: vec![effect.id],
        source_policy: label("fixture-policy"),
        semantic_output: Some("registered fixture semantic decision".to_owned()),
    });
    let candidate = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next,
        pins(),
        supplied,
        inventory(),
        limits(),
    )
    .unwrap();
    let input = GameInput::Host(HostInput {
        basis: expected,
        operation,
        host: member(3),
        command: HostCommand::RequestCheckpoint,
    });
    (baseline, candidate, input)
}

#[test]
fn owned_pg_suite_candidates_are_real_complete_canonical_checkpoints() {
    let (baseline, candidate, input) = admitted_case(30);
    assert_eq!(
        candidate.basis().revision,
        baseline.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(candidate.state().facts.len(), 1);
    assert_eq!(candidate.state().intents.len(), 1);
    assert_eq!(candidate.state().decisions.len(), 1);
    assert_eq!(
        candidate.state().continuity.canonical_packs[0].identities[0].character_appearance,
        Some(fixture_character_appearance())
    );
    assert!(input.retained_bytes().is_some());
}

#[test]
#[ignore = "ROOT registered isolated PG/schema/roles/fault-function and explicit fixture release required"]
fn actual_registered_pg_all_four_families_retained_and_each_insert_fault_rollback() {
    let stage = std::cell::Cell::new("configuration and runtime");
    if let Err(error) = run_registered_suite(&stage) {
        panic!(
            "actual registered PostgreSQL fixture failed at {}: {error:?}",
            stage.get()
        );
    }
}
fn run_registered_suite(stage: &std::cell::Cell<&'static str>) -> Result<(), RepositoryError> {
    // These values are fixture identifiers only, never hosted credentials or production config.
    // ROOT's explicit invocation binds the registered loopback port and database name.
    let port: u16 = std::env::var("DUNGEONFLUX_I01_OWNED_PG_PORT")
        .map_err(|_| RepositoryError::Unavailable)?
        .parse()
        .map_err(|_| RepositoryError::Unavailable)?;
    let proxy_port: u16 = std::env::var("DUNGEONFLUX_I01_OWNED_PROXY_PORT")
        .map_err(|_| RepositoryError::Unavailable)?
        .parse()
        .map_err(|_| RepositoryError::Unavailable)?;
    let database = std::env::var("DUNGEONFLUX_I01_OWNED_PG_DATABASE")
        .map_err(|_| RepositoryError::Unavailable)?;
    let admin_role = std::env::var("DUNGEONFLUX_I01_OWNED_PG_ADMIN_ROLE")
        .map_err(|_| RepositoryError::Unavailable)?;
    if port == 0
        || port == 55439
        || proxy_port == 0
        || proxy_port == 55439
        || proxy_port == port
        || !database.starts_with("df_persistence_i01_")
        || admin_role.is_empty()
        || admin_role.len() > 63
    {
        return Err(RepositoryError::Unavailable);
    }
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|_| RepositoryError::Unavailable)?;
    let handle = runtime.handle().clone();
    let bounds = TransactionBounds {
        transaction: Duration::from_secs(5),
        rollback: Duration::from_secs(2),
        driver_join: Duration::from_secs(2),
    };
    let mut configuration = Config::new();
    configuration
        .host("127.0.0.1")
        .port(port)
        .dbname(&database)
        .user(&admin_role)
        .ssl_mode(SslMode::Disable);
    let (admin_client, admin_driver) = actor_block_on(
        &handle,
        within_deadline(
            Instant::now() + bounds.transaction,
            configuration.connect(NoTls),
        ),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let mut owned_admin =
        OwnedConnection::from_connected(&handle, admin_client, admin_driver, bounds)?;
    let mut admin = owned_admin.take_client()?;
    let tenant = [80; 16];
    let principal = [81; 16];
    let campaign = [82; 16];
    let fence = [83; 16];
    let make_grant = || FixtureGrant {
        service_role: "df_persistence_fixture_runtime_a",
        tenant: &tenant,
        principal: &principal,
        campaign: &campaign,
        role: b"gm",
        access_revision: b"fixture-access-1",
        lifetime_seconds: 120,
    };
    actor_block_on(
        &handle,
        within_deadline(Instant::now() + bounds.transaction, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_grant(&transaction, &make_grant()).await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    for (index, (family, epoch, sequence, exhausted)) in [
        (None, 2, 7, false),
        (Some(InsertedFamily::Checkpoint), 2, 7, false),
        (Some(InsertedFamily::Fact), 2, 7, false),
        (Some(InsertedFamily::Operation), 2, 7, false),
        (Some(InsertedFamily::Intent), 2, 7, false),
        (Some(InsertedFamily::FinalRevisionCas), 2, 7, false),
        (None, u64::MAX, (i64::MAX as u64) + 1, false),
        (None, u64::MAX, u64::MAX - 1, false),
        (None, u64::MAX, u64::MAX - 1, true),
    ]
    .into_iter()
    .enumerate()
    {
        let (mut baseline, candidate, mut input) = admitted_case_at_revision(
            30 + u8::try_from(index).map_err(|_| RepositoryError::Capacity)?,
            revision(epoch, sequence),
        );
        let rules = vec![rule()];
        let entries = vec![content()];
        let resources = resource_constraints();
        let inventory = || ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        };
        let fingerprint = [u8::try_from(index + 1).map_err(|_| RepositoryError::Capacity)?; 32];
        let operation = [if exhausted { 9 } else { 6 }; 16];
        if exhausted {
            baseline = Checkpoint::new(
                CHECKPOINT_SCHEMA,
                candidate.basis(),
                pins(),
                state(),
                inventory(),
                limits(),
            )
            .map_err(|_| RepositoryError::InvalidCandidate)?;
            input = GameInput::Host(HostInput {
                basis: baseline.basis(),
                operation: OperationId::from_bytes(&operation)
                    .map_err(|_| RepositoryError::InvalidCandidate)?,
                host: member(3),
                command: HostCommand::RequestCheckpoint,
            });
        }
        let proof = actor_block_on(
            &handle,
            within_deadline(Instant::now() + bounds.transaction, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                seed_session(
                    &transaction,
                    &tenant,
                    &fence,
                    &baseline,
                    inventory(),
                    (limits(), codec_limits()),
                    120,
                )
                .await?;
                let proof = seed_proof(
                    &transaction,
                    &FixtureProof {
                        grant: make_grant(),
                        basis: baseline.basis(),
                        operation: &operation,
                        namespace: b"fixture/host/v1",
                        canonical_fingerprint: &fingerprint,
                        fence: &fence,
                        mode: 3,
                        lookup_only: false,
                    },
                )
                .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok::<tokio_postgres::Row, RepositoryError>(proof)
            }),
        )?
        .map_err(|_| RepositoryError::Unavailable)??;
        let context = OperationContext {
            trace_parent: String::new(),
            build: "persistence-owned-pg-fixture".to_owned(),
        };
        configuration.user("df_persistence_fixture_runtime_a");
        let (client, driver) = actor_block_on(
            &handle,
            within_deadline(
                Instant::now() + bounds.transaction,
                configuration.connect(NoTls),
            ),
        )?
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
        let connection = OwnedConnection::from_connected(&handle, client, driver, bounds)?;
        let mut authority = FixtureMembershipAuthority {
            runtime: handle.clone(),
            connection,
            query_bound: bounds.transaction,
            context: context.clone(),
        };
        let now: i64 = actor_block_on(
            &handle,
            within_deadline(
                Instant::now() + bounds.transaction,
                admin.query_one(
                    "SELECT floor(extract(epoch FROM clock_timestamp()))::bigint",
                    &[],
                ),
            ),
        )?
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?
        .try_get(0)
        .map_err(|_| RepositoryError::Unavailable)?;
        let (scope, verifier) = bind_registered_scope(
            &mut authority,
            &proof,
            FixtureBoundInput {
                input,
                pins: pins(),
            },
            u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
        )?;
        let recovery = RecoverySource {
            rules: rules.clone(),
            content: entries.clone(),
            resource_resolver: None,
            asset_resolver: None,
            resources: resources.clone(),
            assets: vec![],
            limits: limits(),
        };
        let mut repository = compose_registered_repository(
            authority,
            codec_limits(),
            16384,
            verifier,
            recovery,
            bounds,
        )?;
        let physical_deadline = Instant::now() + Duration::from_secs(30);
        stage.set(match family {
            None if exhausted => {
                "full u64 maximum sequence refusal and unchanged physical baseline"
            }
            None if epoch == u64::MAX => {
                "full u64 high revision commit, receipt and complete reload"
            }
            None => "success and retained receipt observation",
            Some(InsertedFamily::Checkpoint) => {
                "checkpoint fault rollback and exact retry observation"
            }
            Some(InsertedFamily::Fact) => "fact fault rollback and exact retry observation",
            Some(InsertedFamily::Operation) => {
                "operation fault rollback and exact retry observation"
            }
            Some(InsertedFamily::Intent) => "intent fault rollback and exact retry observation",
            Some(InsertedFamily::FinalRevisionCas) => {
                "final revision CAS refusal after four writes, rollback and exact retry"
            }
            Some(
                InsertedFamily::LeaseBeforeCas
                | InsertedFamily::FactOrdinal
                | InsertedFamily::IntentSlot
                | InsertedFamily::EffectIdentity
                | InsertedFamily::CreatedJob
                | InsertedFamily::CreatedTimer,
            ) => "additional registered identity or lease boundary fixture",
        });
        let observed = {
            let physical = PhysicalObservation {
                runtime: &handle,
                administrator: &admin,
                inventory: inventory(),
                model_limits: limits(),
                codec_limits: codec_limits(),
                maximum_receipt_bytes: 16384,
                deadline: physical_deadline,
            };
            match family {
                None if exhausted => observe_sequence_exhaustion(
                    &mut repository,
                    &scope,
                    &baseline,
                    &candidate,
                    &context,
                    &physical,
                )
                .map(|_| None),
                None => observe_success_and_retained(
                    &mut repository,
                    &scope,
                    &candidate,
                    baseline.basis(),
                    &context,
                    &physical,
                )
                .and_then(|receipt| {
                    stage
                        .set("actual persisted current codec metadata and legacy/mismatch refusal");
                    observe_persisted_codec_metadata(
                        &physical,
                        &mut repository,
                        &scope,
                        &candidate,
                        &context,
                    )?;
                    Ok(Some(receipt))
                }),
                Some(family) => observe_family_rollback(
                    &mut repository,
                    &scope,
                    &candidate,
                    baseline.basis(),
                    family,
                    &context,
                    &physical,
                )
                .map(|_| None),
            }
        };
        if observed.is_ok() {
            stage.set("registered repository joined close");
        }
        let closed = repository.close();
        let retained = observed?;
        closed?;
        if index == 0 {
            // Exercise the actual ordinary-library constructor: absence of issuer/verifier
            // must refuse even this genuinely DB-produced capability before BEGIN.
            configuration.user("df_persistence_fixture_runtime_a");
            let (client, driver) = actor_block_on(
                &handle,
                within_deadline(
                    Instant::now() + bounds.transaction,
                    configuration.connect(NoTls),
                ),
            )?
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)?;
            let initialized = actor_block_on(
                &handle,
                PostgresRepository::<FixtureMembershipAuthority>::from_connected_no_tls(
                    handle.clone(),
                    client,
                    driver,
                    NativeRepositoryOptions {
                        transaction_bounds: bounds,
                        codec_limits: codec_limits(),
                        maximum_receipt_bytes: 16384,
                        verifier: None,
                        recovery: Some(RecoverySource {
                            rules: rules.clone(),
                            content: entries.clone(),
                            resource_resolver: None,
                            asset_resolver: None,
                            resources: resources.clone(),
                            assets: vec![],
                            limits: limits(),
                        }),
                    },
                    &context,
                ),
            )?;
            let mut unconfigured = match initialized {
                Ok(repository) => repository,
                Err(mut failure) => {
                    let error = failure.error();
                    actor_block_on(&handle, failure.close())??;
                    return Err(error);
                }
            };
            let refused = observe_scope_refusal(
                &mut unconfigured,
                &scope,
                &candidate,
                baseline.basis(),
                &context,
            );
            let closed = unconfigured.close();
            refused?;
            closed?;
            // A different authenticated service role cannot reuse A's private proof or capability.
            configuration.user("df_persistence_fixture_runtime_b");
            let (client, driver) = actor_block_on(
                &handle,
                within_deadline(
                    Instant::now() + bounds.transaction,
                    configuration.connect(NoTls),
                ),
            )?
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)?;
            let mut connection = OwnedConnection::from_connected(&handle, client, driver, bounds)?;
            let client = connection.take_client()?;
            let verifier = actor_block_on(&handle, async {
                let deadline = Instant::now() + bounds.transaction;
                // Unsigned GUCs cannot create the private current transaction binding.
                timeout_at(deadline,client.query_one("SELECT set_config('df.tenant_id',$1,false), set_config('df.session_id',$2,false)",
                    &[&"forged-tenant",&"forged-session"]))
                    .await.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
                let count: i64 = timeout_at(
                    deadline,
                    client.query_one("SELECT count(*) FROM df_game.checkpoints", &[]),
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?
                .map_err(|_| RepositoryError::Unavailable)?
                .try_get(0)
                .map_err(|_| RepositoryError::Unavailable)?;
                if count != 0 {
                    return Err(RepositoryError::Unauthorized);
                }
                fixture_verifier(&client, deadline).await
            })??;
            connection.return_client(client)?;
            let peer_authority = FixtureMembershipAuthority {
                runtime: handle.clone(),
                connection,
                query_bound: bounds.transaction,
                context: context.clone(),
            };
            let recovery = RecoverySource {
                rules: rules.clone(),
                content: entries.clone(),
                resource_resolver: None,
                asset_resolver: None,
                resources: resources.clone(),
                assets: vec![],
                limits: limits(),
            };
            let mut peer = compose_registered_repository(
                peer_authority,
                codec_limits(),
                16384,
                verifier,
                recovery,
                bounds,
            )?;
            let refused =
                observe_scope_refusal(&mut peer, &scope, &candidate, baseline.basis(), &context);
            let closed = peer.close();
            refused?;
            closed?;
            // Adapter configuration, never the scope, sets the SQL and decoder ceilings.
            configuration.user("df_persistence_fixture_runtime_a");
            for document_too_small in [false, true] {
                let mut authority =
                    connect_fixture_authority(&handle, &configuration, bounds, &context)?;
                let (_, verifier) = bind_registered_scope(
                    &mut authority,
                    &proof,
                    FixtureBoundInput {
                        input: scope.bound_input.clone(),
                        pins: pins(),
                    },
                    u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
                )?;
                let mut configured = codec_limits();
                if document_too_small {
                    configured.maximum_document_bytes = 1;
                }
                let recovery = RecoverySource {
                    rules: rules.clone(),
                    content: entries.clone(),
                    resource_resolver: None,
                    asset_resolver: None,
                    resources: resources.clone(),
                    assets: vec![],
                    limits: limits(),
                };
                let mut limited = compose_registered_repository(
                    authority,
                    configured,
                    if document_too_small { 16384 } else { 1 },
                    verifier,
                    recovery,
                    bounds,
                )?;
                let observed = observe_configured_read_refusal(
                    &mut limited,
                    &scope,
                    &context,
                    document_too_small,
                );
                let closed = limited.close();
                observed?;
                closed?;
            }
            let retained = retained.ok_or(RepositoryError::InvalidReceipt)?;
            let current = fixture_later_epoch(candidate.basis())?;
            let fresh_fence = [84; 16];
            let refreshed = actor_block_on(
                &handle,
                within_deadline(Instant::now() + bounds.transaction, async {
                    let transaction = admin
                        .transaction()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    install_fixture_current_epoch(
                        &transaction,
                        &tenant,
                        candidate.basis(),
                        &fresh_fence,
                        &current,
                        inventory(),
                        (limits(), codec_limits()),
                    )
                    .await?;
                    // Refresh current owner proof while retaining the original operation-key epoch.
                    let proof = seed_proof(
                        &transaction,
                        &FixtureProof {
                            grant: make_grant(),
                            basis: baseline.basis(),
                            operation: &operation,
                            namespace: b"fixture/host/v1",
                            canonical_fingerprint: &fingerprint,
                            fence: &fresh_fence,
                            mode: 3,
                            lookup_only: true,
                        },
                    )
                    .await?;
                    transaction
                        .commit()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    Ok::<tokio_postgres::Row, RepositoryError>(proof)
                }),
            )?
            .map_err(|_| RepositoryError::Unavailable)??;
            let mut authority =
                connect_fixture_authority(&handle, &configuration, bounds, &context)?;
            let (historical, verifier) = bind_registered_scope(
                &mut authority,
                &refreshed,
                FixtureBoundInput {
                    input: scope.bound_input.clone(),
                    pins: pins(),
                },
                u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
            )?;
            let recovery = RecoverySource {
                rules: rules.clone(),
                content: entries.clone(),
                resource_resolver: None,
                asset_resolver: None,
                resources: resources.clone(),
                assets: vec![],
                limits: limits(),
            };
            let mut recovered = compose_registered_repository(
                authority,
                codec_limits(),
                16384,
                verifier,
                recovery,
                bounds,
            )?;
            let physical = PhysicalObservation {
                runtime: &handle,
                administrator: &admin,
                inventory: inventory(),
                model_limits: limits(),
                codec_limits: codec_limits(),
                maximum_receipt_bytes: 16384,
                deadline: physical_deadline,
            };
            let observed = observe_historical_key_reload(
                &mut recovered,
                &historical,
                &retained,
                (&candidate, baseline.basis()),
                &current,
                &context,
                &physical,
            );
            let closed = recovered.close();
            observed?;
            closed?;
            // Without a retained record the same historical epoch cannot authorize a new write.
            let next_operation = [99; 16];
            let unmatched = actor_block_on(
                &handle,
                within_deadline(Instant::now() + bounds.transaction, async {
                    let transaction = admin
                        .transaction()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    let proof = seed_proof(
                        &transaction,
                        &FixtureProof {
                            grant: make_grant(),
                            basis: baseline.basis(),
                            operation: &next_operation,
                            namespace: b"fixture/host/v1",
                            canonical_fingerprint: &[99; 32],
                            fence: &fresh_fence,
                            mode: 3,
                            lookup_only: false,
                        },
                    )
                    .await?;
                    transaction
                        .commit()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    Ok::<tokio_postgres::Row, RepositoryError>(proof)
                }),
            )?
            .map_err(|_| RepositoryError::Unavailable)??;
            let unmatched_input = GameInput::Host(HostInput {
                basis: baseline.basis(),
                operation: OperationId::from_bytes(&next_operation)
                    .map_err(|_| RepositoryError::InputBinding)?,
                host: member(3),
                command: HostCommand::RequestCheckpoint,
            });
            let mut authority =
                connect_fixture_authority(&handle, &configuration, bounds, &context)?;
            let (unmatched_scope, verifier) = bind_registered_scope(
                &mut authority,
                &unmatched,
                FixtureBoundInput {
                    input: unmatched_input,
                    pins: pins(),
                },
                u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
            )?;
            let recovery = RecoverySource {
                rules: rules.clone(),
                content: entries.clone(),
                resource_resolver: None,
                asset_resolver: None,
                resources: resources.clone(),
                assets: vec![],
                limits: limits(),
            };
            let mut writer = compose_registered_repository(
                authority,
                codec_limits(),
                16384,
                verifier,
                recovery,
                bounds,
            )?;
            let outcome =
                writer.commit_decision(&unmatched_scope, &candidate, current.basis(), &context);
            let closed = writer.close();
            closed?;
            if !matches!(outcome, Err(RepositoryError::StaleEpoch)) {
                return Err(RepositoryError::InvalidReceipt);
            }
        }
    }
    let lost_ack = observe_registered_lost_commit_ack(
        &handle,
        &mut admin,
        &configuration,
        port,
        proxy_port,
        bounds,
        stage,
    );
    let next_cases = lost_ack.and_then(|_| {
        observe_registered_race_and_owner(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_uniqueness_and_recovery_seed(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_retained_refusals(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_current_grant_refusals(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        recovery_lease::observe_registered_recovery_lease(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_actor_postgres_lifetime(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_forced_driver_join(
            &handle,
            &mut admin,
            &configuration,
            port,
            bounds,
            stage,
        )?;
        observe_registered_actor_lost_commit_ack(
            &handle,
            &mut admin,
            &configuration,
            port,
            proxy_port,
            bounds,
            stage,
        )
    });
    if next_cases.is_ok() {
        stage.set("administrator joined close after race and owner refusals");
    }
    owned_admin.return_client(admin)?;
    let closed = actor_block_on(&handle, owned_admin.close())?;
    next_cases?;
    closed?;
    Ok(())
}

fn connect_fixture_authority(
    handle: &tokio::runtime::Handle,
    configuration: &Config,
    bounds: TransactionBounds,
    context: &OperationContext,
) -> Result<FixtureMembershipAuthority, RepositoryError> {
    let (client, driver) = actor_block_on(
        handle,
        within_deadline(
            Instant::now() + bounds.transaction,
            configuration.connect(NoTls),
        ),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let connection = OwnedConnection::from_connected(handle, client, driver, bounds)?;
    Ok(FixtureMembershipAuthority {
        runtime: handle.clone(),
        connection,
        query_bound: bounds.transaction,
        context: context.clone(),
    })
}
fn fixture_later_epoch(previous: Basis) -> Result<Checkpoint, RepositoryError> {
    let epoch = previous
        .revision
        .epoch()
        .get()
        .checked_add(1)
        .ok_or(RepositoryError::SequenceExhausted)?;
    let current = Basis {
        revision: SessionRevision::new(
            RecoveryEpoch::new(epoch).map_err(|_| RepositoryError::InvalidCandidate)?,
            0,
        ),
        ..previous
    };
    let mut supplied = state();
    supplied
        .continuity
        .recovery
        .retired_epochs
        .push(previous.revision.epoch());
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current,
        pins(),
        supplied,
        ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        },
        limits(),
    )
    .map_err(|_| RepositoryError::InvalidCandidate)
}
#[test]
fn historical_read_fixture_checkpoint_is_complete_admitted_current_epoch() {
    let (_, candidate, _) = admitted_case(30);
    let current = fixture_later_epoch(candidate.basis()).unwrap();
    assert!(current.basis().revision > candidate.basis().revision);
    assert_eq!(
        current.state().continuity.recovery.retired_epochs,
        vec![candidate.basis().revision.epoch()]
    );
}

/// The fixture owner retains its proxy task until a bounded join completes. Drop
/// only aborts emergency work; it never supplies a successful cleanup observation.
struct OwnedAckProxy {
    task: Option<JoinHandle<Result<CommitAckObservation, ProxyError>>>,
}
impl OwnedAckProxy {
    async fn observe(&mut self, deadline: Instant) -> Result<CommitAckObservation, ProxyError> {
        let task = self.task.as_mut().ok_or(ProxyError::Protocol)?;
        match timeout_at(deadline, task).await {
            Ok(Ok(result)) => {
                self.task.take();
                result
            }
            Ok(Err(_)) => {
                self.task.take();
                Err(ProxyError::Io)
            }
            Err(_) => Err(ProxyError::Deadline),
        }
    }
    async fn close(&mut self, join_bound: Duration) -> Result<(), ProxyError> {
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        task.abort();
        let deadline = Instant::now()
            .checked_add(join_bound)
            .ok_or(ProxyError::Capacity)?;
        match timeout_at(deadline, task).await {
            Ok(Ok(_)) => {
                self.task.take();
                Ok(())
            }
            Ok(Err(error)) if error.is_cancelled() => {
                self.task.take();
                Ok(())
            }
            Ok(Err(_)) => {
                self.task.take();
                Err(ProxyError::Io)
            }
            // Keep the aborted handle owned for the explicit bounded retry.
            Err(_) => Err(ProxyError::Deadline),
        }
    }
}
impl Drop for OwnedAckProxy {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
fn proxy_failure(error: ProxyError) -> RepositoryError {
    eprintln!(
        "owned PostgreSQL acknowledgement proxy failure: {error:?}; no commit proof accepted"
    );
    match error {
        ProxyError::Capacity => RepositoryError::Capacity,
        ProxyError::Protocol => RepositoryError::InvalidReceipt,
        ProxyError::Io | ProxyError::Deadline | ProxyError::ClosedBeforeCommit => {
            RepositoryError::Unavailable
        }
    }
}

/// ROOT must register both loopback ports/listener, database, roles and tasks before
/// invoking the ignored suite. This case uses a real current DB membership and
/// private scope proof; the proxy controls transport only and supplies no authority.
fn observe_registered_lost_commit_ack(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    postgres_port: u16,
    proxy_port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    stage.set("lost acknowledgement canonical session and proof seed");
    let (baseline, candidate, input) = admitted_case(40);
    let tenant = [80; 16];
    let principal = [81; 16];
    let campaign = [82; 16];
    let fence = [83; 16];
    let operation = [6; 16];
    let fingerprint = [42; 32];
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let recovery = || RecoverySource {
        rules: rules.clone(),
        content: entries.clone(),
        resource_resolver: None,
        asset_resolver: None,
        resources: resources.clone(),
        assets: vec![],
        limits: limits(),
    };
    let case_deadline = Instant::now() + Duration::from_secs(30);
    let proof = actor_block_on(
        handle,
        within_deadline(case_deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: FixtureGrant {
                        service_role: "df_persistence_fixture_runtime_a",
                        tenant: &tenant,
                        principal: &principal,
                        campaign: &campaign,
                        role: b"gm",
                        access_revision: b"fixture-access-1",
                        lifetime_seconds: 120,
                    },
                    basis: baseline.basis(),
                    operation: &operation,
                    namespace: b"fixture/ack/v1",
                    canonical_fingerprint: &fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<tokio_postgres::Row, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    stage.set("lost acknowledgement current database time");
    let now: i64 = actor_block_on(
        handle,
        within_deadline(
            case_deadline,
            admin.query_one(
                "SELECT floor(extract(epoch FROM clock_timestamp()))::bigint",
                &[],
            ),
        ),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?
    .try_get(0)
    .map_err(|_| RepositoryError::Unavailable)?;
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-pg-lost-ack".to_owned(),
    };
    let direct = fixture_lost_ack_configuration(configuration, postgres_port)?;
    stage.set("lost acknowledgement direct authority connection");
    let mut authority = connect_fixture_authority(handle, &direct, bounds, &context)?;
    stage.set("lost acknowledgement current membership and scope binding");
    let produced = bind_registered_scope(
        &mut authority,
        &proof,
        FixtureBoundInput {
            input: input.clone(),
            pins: pins(),
        },
        u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
    );
    if produced.is_ok() {
        stage.set("lost acknowledgement direct authority joined close");
    }
    let authority_closed = actor_block_on(handle, authority.connection.close())?;
    let (scope, _direct_verifier) = produced?;
    authority_closed?;
    stage.set("lost acknowledgement owned proxy listener bind");
    let listener = actor_block_on(
        handle,
        within_deadline(case_deadline, TcpListener::bind(("127.0.0.1", proxy_port))),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let mut proxy = OwnedAckProxy {
        task: Some(handle.spawn(async move {
            let (frontend, peer) = timeout_at(case_deadline, listener.accept())
                .await
                .map_err(|_| ProxyError::Deadline)?
                .map_err(|_| ProxyError::Io)?;
            if !peer.ip().is_loopback() {
                return Err(ProxyError::Protocol);
            }
            // Exactly one accepted adapter connection: release the registered listener.
            drop(listener);
            let postgres = timeout_at(
                case_deadline,
                TcpStream::connect(("127.0.0.1", postgres_port)),
            )
            .await
            .map_err(|_| ProxyError::Deadline)?
            .map_err(|_| ProxyError::Io)?;
            drop_commit_acknowledgement(
                frontend,
                postgres,
                ProxyBounds {
                    maximum_frame_bytes: 4 * 1024 * 1024,
                    maximum_suppressed_response_bytes: 4096,
                    deadline: case_deadline,
                },
            )
            .await
        })),
    };
    let proxied = fixture_lost_ack_configuration(configuration, proxy_port)?;
    let attempt = (|| -> Result<(), RepositoryError> {
        stage.set("lost acknowledgement proxied client connection");
        let (client, driver) = actor_block_on(
            handle,
            within_deadline(case_deadline, proxied.connect(NoTls)),
        )?
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
        stage.set("lost acknowledgement native initializer on proxied connection");
        let initialized = actor_block_on(
            handle,
            PostgresRepository::<FixtureMembershipAuthority>::from_connected_no_tls(
                handle.clone(),
                client,
                driver,
                NativeRepositoryOptions {
                    transaction_bounds: bounds,
                    codec_limits: codec_limits(),
                    maximum_receipt_bytes: 16384,
                    verifier: Some(NativeVerifierSource {
                        query: "SELECT * FROM df_fixture_authority.bind_scope($1::bytea,$2::bytea)",
                        maximum_binding_bytes: 32,
                        maximum_authority_value_bytes: 128,
                        maximum_namespace_bytes: 128,
                    }),
                    recovery: Some(recovery()),
                },
                &context,
            ),
        )?;
        let mut repository = match initialized {
            Ok(repository) => repository,
            Err(mut failure) => {
                let error = failure.error();
                let closed = actor_block_on(handle, failure.close())?;
                if failure.cleanup_pending() {
                    eprintln!("lost-ack setup driver join remains pending; case refuses proof");
                }
                closed?;
                return Err(error);
            }
        };
        stage.set("lost acknowledgement decision commit and joined driver close");
        let outcome = repository.commit_decision(&scope, &candidate, baseline.basis(), &context);
        let closed = repository.close();
        if closed.is_err() {
            eprintln!(
                "lost-ack native driver close failed or join remains pending; no proof accepted"
            );
        }
        closed?;
        if !matches!(outcome, Ok(CommitOutcome::Indeterminate)) {
            return Err(RepositoryError::InvalidReceipt);
        }
        Ok(())
    })();
    let observed = if attempt.is_ok() {
        stage.set("lost acknowledgement actual COMMIT and idle proxy observation");
        Some(actor_block_on(handle, proxy.observe(case_deadline))?)
    } else {
        None
    };
    // Keep the first failure's static context across the original bounded cleanup.
    let observed_stage = stage.get();
    let prior_failed = attempt.is_err() || observed.as_ref().is_some_and(|result| result.is_err());
    if !prior_failed {
        stage.set("lost acknowledgement owned proxy joined close");
    }
    let mut proxy_closed = actor_block_on(handle, proxy.close(bounds.driver_join))?;
    if proxy_closed.is_err() {
        proxy_closed = actor_block_on(handle, proxy.close(bounds.driver_join))?;
    }
    if proxy.task.is_some() {
        eprintln!(
            "lost-ack proxy aborted task still owned after two bounded joins; cleanup pending, no proof accepted"
        );
    }
    if prior_failed {
        stage.set(observed_stage);
    }
    proxy_closed.map_err(proxy_failure)?;
    attempt?;
    let observation = observed
        .ok_or(RepositoryError::Unavailable)?
        .map_err(proxy_failure)?;
    if !observation.frontend_commit_forwarded
        || !observation.postgres_commit_complete_observed
        || !observation.postgres_ready_idle_observed
        || observation.suppressed_response_bytes == 0
        || observation.suppressed_response_bytes > 4096
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    // The adapter never saw an acknowledgement; independent real rows and complete
    // decoded storage, rather than the proxy's COMMIT text, establish durability.
    stage.set("lost acknowledgement independent four-family snapshot and decode");
    let before = actor_block_on(
        handle,
        physical_snapshot(
            admin,
            &tenant,
            candidate.basis(),
            codec_limits().maximum_document_bytes,
            case_deadline,
        ),
    )??;
    validate_physical_commit(&before, &candidate, inventory(), limits(), codec_limits())?;
    let stored = decode_receipt(
        before
            .retained_receipt
            .as_ref()
            .ok_or(RepositoryError::InvalidReceipt)?,
        baseline.basis().session,
        scope.operation(),
        16384,
        codec_limits(),
    )
    .map_err(|_| RepositoryError::InvalidReceipt)?;
    stage.set("lost acknowledgement fresh current membership and full scope equality");
    let mut authority = connect_fixture_authority(handle, &direct, bounds, &context)?;
    let (fresh_scope, verifier) = bind_registered_scope(
        &mut authority,
        &proof,
        FixtureBoundInput {
            input,
            pins: pins(),
        },
        u64::try_from(now).map_err(|_| RepositoryError::Unavailable)?,
    )?;
    if scope.capture_uncertainty_key(16384)? != fresh_scope.capture_uncertainty_key(16384)? {
        let closed = actor_block_on(handle, authority.connection.close())?;
        closed?;
        return Err(RepositoryError::InputBinding);
    }
    let mut recovered = compose_registered_repository(
        authority,
        codec_limits(),
        16384,
        verifier,
        recovery(),
        bounds,
    )?;
    let reconciled = (|| -> Result<(), RepositoryError> {
        stage.set("lost acknowledgement retained receipt lookup");
        match recovered.lookup_operation(&fresh_scope, &context)? {
            OperationLookup::Committed(receipt) if receipt == stored => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        // A real AFTER INSERT fault would expose any duplicate execution of the
        // checkpoint family. Always remove it before classifying the retry result.
        actor_block_on(
            handle,
            install_fault(admin, InsertedFamily::Checkpoint, case_deadline),
        )??;
        stage.set("lost acknowledgement exact retry under duplicate-insert fault");
        let retry = recovered.commit_decision(&fresh_scope, &candidate, baseline.basis(), &context);
        let removed = actor_block_on(
            handle,
            remove_fault(admin, InsertedFamily::Checkpoint, case_deadline),
        )?;
        removed?;
        match retry? {
            CommitOutcome::PreviouslyCommitted(receipt) if receipt == stored => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        stage.set("lost acknowledgement current complete checkpoint reload");
        if recovered.load_current(&fresh_scope, &context)? != candidate {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    })();
    if reconciled.is_ok() {
        stage.set("lost acknowledgement recovered repository joined close");
    }
    let recovered_closed = recovered.close();
    reconciled?;
    recovered_closed?;
    stage.set("lost acknowledgement unchanged final four-family snapshot");
    let after = actor_block_on(
        handle,
        physical_snapshot(
            admin,
            &tenant,
            candidate.basis(),
            codec_limits().maximum_document_bytes,
            case_deadline,
        ),
    )??;
    if after.family_counts != before.family_counts
        || after.envelope != before.envelope
        || after.retained_receipt != before.retained_receipt
        || after.session_sequence != before.session_sequence
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    validate_physical_commit(&after, &candidate, inventory(), limits(), codec_limits())
}

/// Produce each scope through real df-auth membership and its exact owned connection.
/// No public trait assertion, unsigned GUC, or caller-provided authority is accepted.
fn registered_case_repository(
    handle: &tokio::runtime::Handle,
    endpoint: (&Config, u16),
    proof: &tokio_postgres::Row,
    input: GameInput,
    now: u64,
    bounds: TransactionBounds,
    context: &OperationContext,
) -> Result<
    (
        PostgresRepository<FixtureMembershipAuthority>,
        crate::native_scope::NativeScope<FixtureMembershipAuthority>,
        i32,
    ),
    RepositoryError,
> {
    let (configuration, port) = endpoint;
    let direct = fixture_lost_ack_configuration(configuration, port)?;
    let mut authority = connect_fixture_authority(handle, &direct, bounds, context)?;
    let produced = bind_registered_scope(
        &mut authority,
        proof,
        FixtureBoundInput {
            input,
            pins: pins(),
        },
        now,
    );
    let (scope, verifier) = match produced {
        Ok(value) => value,
        Err(error) => {
            actor_block_on(handle, authority.connection.close())??;
            return Err(error);
        }
    };
    let client = authority.connection.take_client()?;
    let pid = actor_block_on(
        handle,
        within_deadline(
            Instant::now() + bounds.transaction,
            client.query_one("SELECT pg_backend_pid()", &[]),
        ),
    )
    .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
    .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
    .and_then(|row| {
        row.try_get::<_, i32>(0)
            .map_err(|_| RepositoryError::Unavailable)
    });
    authority.connection.return_client(client)?;
    let pid = match pid {
        Ok(value) if value > 0 => value,
        _ => {
            actor_block_on(handle, authority.connection.close())??;
            return Err(RepositoryError::Unavailable);
        }
    };
    let recovery = RecoverySource {
        rules: vec![rule()],
        content: vec![content()],
        resource_resolver: None,
        asset_resolver: None,
        resources: resource_constraints(),
        assets: vec![],
        limits: limits(),
    };
    let repository = compose_registered_repository(
        authority,
        codec_limits(),
        16384,
        verifier,
        recovery,
        bounds,
    )?;
    Ok((repository, scope, pid))
}

fn fixture_database_now(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    deadline: Instant,
) -> Result<u64, RepositoryError> {
    let row = actor_block_on(
        handle,
        within_deadline(
            deadline,
            admin.query_one(
                "SELECT floor(extract(epoch FROM clock_timestamp()))::bigint",
                &[],
            ),
        ),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let now: i64 = row.try_get(0).map_err(|_| RepositoryError::Unavailable)?;
    u64::try_from(now).map_err(|_| RepositoryError::Unavailable)
}

fn observe_registered_race_and_owner(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [81; 16];
    let second_principal = [84; 16];
    let campaign = [82; 16];
    let fence = [83; 16];
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let grant = || FixtureGrant {
        service_role: "df_persistence_fixture_runtime_a",
        tenant: &tenant,
        principal: &principal,
        campaign: &campaign,
        role: b"gm",
        access_revision: b"fixture-access-1",
        lifetime_seconds: 120,
    };
    let second_grant = || FixtureGrant {
        service_role: "df_persistence_fixture_runtime_a",
        tenant: &tenant,
        principal: &second_principal,
        campaign: &campaign,
        role: b"gm",
        access_revision: b"fixture-access-1",
        lifetime_seconds: 120,
    };
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-pg-race-owner".to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let (baseline, candidate_a, input_a) = admitted_case_for_operation(41, revision(2, 7), 6);
    let (_, candidate_b, input_b) = admitted_case_for_operation(41, revision(2, 7), 8);
    stage.set("independent connection race canonical session and exact scope seeds");
    let (proof_a, proof_b) = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            // bind_scope retains FOR UPDATE on the current grant. Use two actual,
            // separately checked principals/grants so authority serialization cannot
            // impersonate two competing waits on the common session row.
            seed_grant(&transaction, &second_grant()).await?;
            let proof_a = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &[6; 16],
                    namespace: b"fixture/race/v1",
                    canonical_fingerprint: &[46; 32],
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            let proof_b = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: second_grant(),
                    basis: baseline.basis(),
                    operation: &[8; 16],
                    namespace: b"fixture/race/v1",
                    canonical_fingerprint: &[48; 32],
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>((proof_a, proof_b))
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let now = fixture_database_now(handle, admin, deadline)?;
    let (mut repository_a, scope_a, pid_a) = registered_case_repository(
        handle,
        (configuration, port),
        &proof_a,
        input_a.clone(),
        now,
        bounds,
        &context,
    )?;
    let second = registered_case_repository(
        handle,
        (configuration, port),
        &proof_b,
        input_b.clone(),
        now,
        bounds,
        &context,
    );
    let (mut repository_b, scope_b, pid_b) = match second {
        Ok(value) => value,
        Err(error) => {
            repository_a.close()?;
            return Err(error);
        }
    };
    if pid_a == pid_b {
        repository_a.close()?;
        repository_b.close()?;
        return Err(RepositoryError::InvalidReceipt);
    }
    // Hold the actual common row until PostgreSQL reports BOTH independent backends
    // waiting for it. The barrier starts both joined blocking actor calls together.
    // Release this lock even if the wait observation fails, then join both actors.
    stage.set("independent connection race both actual backends waiting on common row");
    let expected = baseline.basis();
    let (result_a, result_b, observed) = {
        let holding_result = actor_block_on(
            handle,
            within_deadline(deadline, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                transaction
                    .query_one(
                        "SELECT in_epoch_sequence::text FROM df_game.sessions
            WHERE tenant_id=$1::bytea AND session_id=$2::bytea FOR UPDATE",
                        &[
                            &tenant.as_slice(),
                            &baseline.basis().session.as_bytes().as_slice(),
                        ],
                    )
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                let row = transaction
                    .query_one("SELECT pg_backend_pid()", &[])
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                let holding_pid: i32 = row.try_get(0).map_err(|_| RepositoryError::Unavailable)?;
                Ok::<_, RepositoryError>((transaction, holding_pid))
            }),
        )
        .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
        .and_then(|value| value);
        let (holding, holding_pid) = match holding_result {
            Ok(value) => value,
            Err(error) => {
                let a_closed = repository_a.close();
                let b_closed = repository_b.close();
                a_closed?;
                b_closed?;
                return Err(error);
            }
        };
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|threads| {
            let start = &barrier;
            let a_context = &context;
            let a_candidate = &candidate_a;
            let a_scope = &scope_a;
            let a = threads.spawn(move || {
                start.wait();
                let committed =
                    repository_a.commit_decision(a_scope, a_candidate, expected, a_context);
                let closed = repository_a.close();
                (committed, closed)
            });
            let start = &barrier;
            let b_context = &context;
            let b_candidate = &candidate_b;
            let b_scope = &scope_b;
            let b = threads.spawn(move || {
                start.wait();
                let committed =
                    repository_b.commit_decision(b_scope, b_candidate, expected, b_context);
                let closed = repository_b.close();
                (committed, closed)
            });
            barrier.wait();
            let observed = actor_block_on(handle, async {
                let bound = Instant::now() + Duration::from_secs(3);
                let waiting = within_deadline(bound, async {
                    loop {
                        // PostgreSQL18 caches activity status in this holding transaction.
                        // Refresh it before every bounded sample, including query/wait state.
                        holding
                            .query_one("SELECT pg_stat_clear_snapshot()", &[])
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        // PostgreSQL may queue the second row waiter behind the first
                        // tuple locker. Follow the actual blocker chain, not a claim that
                        // both immediate blockers must be the administrator's xid lock.
                        let row = holding
                            .query_one(
                                "SELECT count(*)::bigint FROM pg_stat_activity a
                        WHERE a.pid IN ($1::integer,$2::integer) AND a.wait_event_type='Lock'
                            AND a.query LIKE '%FROM df_game.sessions%'
                            AND EXISTS (WITH RECURSIVE blockers(pid) AS (
                                SELECT unnest(pg_blocking_pids(a.pid))
                                UNION SELECT unnest(pg_blocking_pids(blockers.pid)) FROM blockers)
                                SELECT 1 FROM blockers WHERE pid=$3::integer)",
                                &[&pid_a, &pid_b, &holding_pid],
                            )
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        let count: i64 =
                            row.try_get(0).map_err(|_| RepositoryError::Unavailable)?;
                        if count == 2 {
                            return Ok::<_, RepositoryError>(());
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .map_err(|_| RepositoryError::Unavailable)
                .and_then(|value| value);
                let released = within_deadline(deadline, holding.commit())
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?
                    .map_err(|_| RepositoryError::Unavailable);
                released?;
                waiting
            });
            // Always join both owned actors before propagating either result.
            let result_a = a.join();
            let result_b = b.join();
            (result_a, result_b, observed)
        })
    };
    let (a, a_closed) = result_a.map_err(|_| RepositoryError::Unavailable)?;
    let (b, b_closed) = result_b.map_err(|_| RepositoryError::Unavailable)?;
    a_closed?;
    b_closed?;
    observed??;
    stage.set("independent connection race exact one winner and independent four family receipt");
    let (winner, winner_receipt, winner_input, winner_proof, loser_input, loser_proof) =
        match (a, b) {
            (Ok(CommitOutcome::Confirmed(receipt)), Err(RepositoryError::RevisionConflict)) => {
                (&candidate_a, receipt, input_a, &proof_a, input_b, &proof_b)
            }
            (Err(RepositoryError::RevisionConflict), Ok(CommitOutcome::Confirmed(receipt))) => {
                (&candidate_b, receipt, input_b, &proof_b, input_a, &proof_a)
            }
            _ => return Err(RepositoryError::InvalidReceipt),
        };
    let snapshot = actor_block_on(
        handle,
        physical_snapshot(
            admin,
            &tenant,
            winner.basis(),
            codec_limits().maximum_document_bytes,
            deadline,
        ),
    )??;
    validate_physical_commit(&snapshot, winner, inventory(), limits(), codec_limits())?;
    let bytes = snapshot
        .retained_receipt
        .as_ref()
        .ok_or(RepositoryError::InvalidReceipt)?;
    if decode_receipt(
        bytes,
        winner.basis().session,
        winner_receipt.decision().operation,
        16384,
        codec_limits(),
    )
    .map_err(|_| RepositoryError::InvalidReceipt)?
        != winner_receipt
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    let now = fixture_database_now(handle, admin, deadline)?;
    for (proof, input, is_winner) in [
        (winner_proof, winner_input, true),
        (loser_proof, loser_input, false),
    ] {
        let (mut repository, scope, _) = registered_case_repository(
            handle,
            (configuration, port),
            proof,
            input,
            now,
            bounds,
            &context,
        )?;
        let checked = (|| {
            match (is_winner, repository.lookup_operation(&scope, &context)?) {
                (true, OperationLookup::Committed(receipt)) if receipt == winner_receipt => {}
                (false, OperationLookup::NotRecorded) => {}
                _ => return Err(RepositoryError::InvalidReceipt),
            }
            if repository.load_current(&scope, &context)? != *winner {
                return Err(RepositoryError::InvalidCandidate);
            }
            let outcome = repository.commit_decision(&scope, winner, expected, &context);
            if is_winner {
                if !matches!(outcome, Ok(CommitOutcome::PreviouslyCommitted(receipt)) if receipt == winner_receipt)
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
            } else if !matches!(outcome, Err(RepositoryError::RevisionConflict)) {
                return Err(RepositoryError::InvalidReceipt);
            }
            Ok(())
        })();
        let closed = repository.close();
        checked?;
        closed?;
    }
    println!(
        "registered physical case: two independent connections, two observed lock waiters, one confirmed and one revision conflict"
    );
    observe_registered_owner_refusals(handle, admin, configuration, port, bounds, stage)
}

fn observe_registered_owner_refusals(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [81; 16];
    let campaign = [82; 16];
    let fence = [83; 16];
    let (baseline, candidate, input) = admitted_case(42);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    stage.set("owner refusal canonical session and real proof seed");
    let proof = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: FixtureGrant {
                        service_role: "df_persistence_fixture_runtime_a",
                        tenant: &tenant,
                        principal: &principal,
                        campaign: &campaign,
                        role: b"gm",
                        access_revision: b"fixture-access-1",
                        lifetime_seconds: 120,
                    },
                    basis: baseline.basis(),
                    operation: &[6; 16],
                    namespace: b"fixture/owner/v1",
                    canonical_fingerprint: &[49; 32],
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let now = fixture_database_now(handle, admin, deadline)?;
    let (mut repository, scope, _) = registered_case_repository(
        handle,
        (configuration, port),
        &proof,
        input,
        now,
        bounds,
        &OperationContext {
            trace_parent: String::new(),
            build: "persistence-owned-pg-owner".to_owned(),
        },
    )?;
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-pg-owner".to_owned(),
    };
    let physical = PhysicalObservation {
        runtime: handle,
        administrator: admin,
        inventory: inventory(),
        model_limits: limits(),
        codec_limits: codec_limits(),
        maximum_receipt_bytes: 16384,
        deadline,
    };
    let outcome = crate::owned_pg_observations_fixture::observe_current_owner_refusals(
        &mut repository,
        &scope,
        &baseline,
        &candidate,
        &context,
        &physical,
    );
    let closed = repository.close();
    outcome?;
    closed?;
    println!(
        "registered physical case: wrong run, stale epoch, stale sequence, stale fence, exact database expiry, rollback and same connection reuse"
    );
    Ok(())
}

fn fixture_candidate_with_created_identity(candidate: &Checkpoint, timer: bool) -> Checkpoint {
    let mut supplied = candidate.state().clone();
    let session_id = candidate.basis().session;
    let identity = session_id.as_bytes();
    let created = &mut supplied.intents[0];
    if timer {
        let id = TimerId::from_bytes(identity).unwrap();
        created.kind = EffectKind::ArmTimer;
        created.timer = Some(id);
        supplied.timers.push(OwnedTimer {
            id,
            basis: candidate.basis(),
            generation: 2,
            due: LogicalTime {
                ticks: 150,
                ticks_per_second: 10,
            },
            source: rule(),
            status: DurableStatus::Pending,
        });
    } else {
        created.kind = EffectKind::RunAi;
        created.job = Some(JobId::from_bytes(identity).unwrap());
    }
    // Cancellation references the real newly created identity but owns another
    // effect and slot; it must not collide with the creation-only partial index.
    let mut cancellation = supplied.intents[0].clone();
    cancellation.id = EffectId::from_bytes(&[identity[0] + 100; 16]).unwrap();
    cancellation.slot = 1;
    cancellation.kind = if timer {
        EffectKind::CancelTimer
    } else {
        EffectKind::CancelJob
    };
    supplied.decisions[0].effects.push(cancellation.id);
    supplied.intents.push(cancellation);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        candidate.basis(),
        candidate.pins().clone(),
        supplied,
        ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        },
        limits(),
    )
    .unwrap()
}

/// The fixture supplies an already observed replay draw. It does not request or
/// execute random generation, a mechanics reducer, or a paid provider.
fn fixture_recovery_candidate() -> (Checkpoint, Checkpoint, GameInput) {
    let (baseline, candidate, input) = admitted_case(49);
    let mut supplied = candidate.state().clone();
    let operation = supplied.decisions[0].operation;
    supplied.draws.push(ActualDraw {
        operation,
        ordinal: 0,
        resolution: ResolutionId::from_bytes(&[61; 16]).unwrap(),
        window: WindowId::from_bytes(&[62; 16]).unwrap(),
        sides: 20,
        value: 18,
        source: rule(),
    });
    let mut observed = fact(63, 1);
    observed.revision = candidate.basis().revision;
    observed.value = FactValue::DrawAccepted {
        operation,
        ordinal: 0,
    };
    supplied.decisions[0].facts.push(observed.id);
    supplied.decisions[0].draws.push(0);
    supplied.facts.push(observed);

    // The accepted fixture ContentEvent is the source for these structural
    // references. The hook's positive consent generation is supplied by this
    // test fixture, not generated by a gameplay or consent action.
    let source = supplied.facts[0].id;
    let hook = RecordId::from_bytes(&[64; 16]).unwrap();
    supplied.narrative.completed_beats.push(content());
    supplied.narrative.open_threads.push(content());
    supplied.narrative.accepted_facts.push(source);
    supplied.continuity.hooks.push(CharacterHook {
        id: hook,
        member: member(3),
        definition: content(),
        source_facts: vec![source],
        consent_generation: 1,
        audience: AudienceScope::Members(vec![member(3)]),
    });
    supplied.continuity.arcs.push(StoryArc {
        id: RecordId::from_bytes(&[65; 16]).unwrap(),
        definition: content(),
        phase: content(),
        source_facts: vec![source],
        active_hooks: vec![hook],
    });
    supplied.threats.push(ThreatClock {
        id: RecordId::from_bytes(&[66; 16]).unwrap(),
        definition: content(),
        progress: 0,
        capacity: 3,
    });
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let candidate = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        candidate.basis(),
        candidate.pins().clone(),
        supplied,
        ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    (baseline, candidate, input)
}

fn assert_fixture_structural_recovery(restored: &Checkpoint) -> Result<(), RepositoryError> {
    let state = restored.state();
    let source = state
        .facts
        .first()
        .ok_or(RepositoryError::InvalidCandidate)?;
    if source.ordinal != 0
        || !matches!(&source.value, FactValue::ContentEvent { definition, subjects }
            if *definition == content() && subjects.as_slice() == [entity(4)])
        || state.narrative.completed_beats != [content()]
        || state.narrative.open_threads != [content()]
        || state.narrative.accepted_facts != [source.id]
        || !state.decisions.iter().any(|decision| {
            decision.source_policy == label("fixture-policy")
                && decision.operation == source.operation
                && decision.revision == source.revision
                && decision.facts.get(source.ordinal as usize) == Some(&source.id)
        })
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let [hook] = state.continuity.hooks.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let [arc] = state.continuity.arcs.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let [clock] = state.threats.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if hook.id != RecordId::from_bytes(&[64; 16]).unwrap()
        || hook.member != member(3)
        || hook.definition != content()
        || hook.source_facts != [source.id]
        || hook.consent_generation != 1
        || hook.audience != AudienceScope::Members(vec![member(3)])
        || arc.id != RecordId::from_bytes(&[65; 16]).unwrap()
        || arc.definition != content()
        || arc.phase != content()
        || arc.source_facts != [source.id]
        || arc.active_hooks != [hook.id]
        || clock.id != RecordId::from_bytes(&[66; 16]).unwrap()
        || clock.definition != content()
        || clock.progress != 0
        || clock.capacity != 3
        || restored.pins() != &pins()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

#[test]
fn physical_fixture_header_expectations_match_current_encoded_records() {
    let (_, candidate, _) = fixture_recovery_candidate();
    let receipt = df_session::submission::DecisionReceipt::new(
        candidate.basis(),
        candidate.state().decisions[0].clone(),
        16384,
    )
    .unwrap();
    let records = [
        (
            *b"DFCP",
            crate::checkpoint_codec::encode_checkpoint(&candidate, codec_limits()),
        ),
        (
            *b"DFFA",
            crate::checkpoint_codec::encode_fact(&candidate.state().facts[0], codec_limits()),
        ),
        (
            *b"DFRC",
            crate::checkpoint_codec::encode_receipt(&receipt, codec_limits()),
        ),
        (
            *b"DFIT",
            crate::checkpoint_codec::encode_intent(&candidate.state().intents[0], codec_limits()),
        ),
    ];
    for (magic, encoded) in records {
        let encoded = encoded.unwrap();
        let expected = crate::owned_pg_admin_fixture::fixture_storage_header(magic).unwrap();
        assert_eq!(encoded.get(..expected.len()), Some(expected.as_slice()));
        crate::checkpoint_codec::validate_storage_format(
            crate::checkpoint_codec::STORAGE_CODEC_VERSION,
            &magic,
            &encoded,
        )
        .unwrap();
    }
}

#[test]
fn fixture_structural_history_candidate_is_source_admitted_and_codec_retained() {
    let (baseline, candidate, input) = fixture_recovery_candidate();
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    assert_eq!(
        candidate.basis().revision,
        baseline.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(
        candidate.basis().session,
        SessionId::from_bytes(&[49; 16]).unwrap()
    );
    candidate
        .validate_admitted(candidate.basis(), &pins(), inventory(), limits())
        .unwrap();
    assert_fixture_structural_recovery(&candidate).unwrap();
    assert_fixture_codec_retains_complete(&candidate);
    assert_eq!(candidate.state().draws[0].value, 18);
    assert_eq!(candidate.state().decisions[0].draws, vec![0]);
    assert_eq!(candidate.state().facts.len(), 2);
    assert_eq!(candidate.state().intents.len(), 1);
    let GameInput::Host(host) = input else {
        panic!("registered closed host input required");
    };
    assert_eq!(host.basis, baseline.basis());
    assert_eq!(host.command, HostCommand::RequestCheckpoint);

    let mut foreign_pins = candidate.pins().clone();
    foreign_pins.content.package_digest = ContentDigest([99; 32]);
    assert_eq!(
        candidate.validate_admitted(candidate.basis(), &foreign_pins, inventory(), limits()),
        Err(CheckpointError::ContentMismatch)
    );
    assert_eq!(
        candidate.validate_admitted(
            candidate.basis(),
            candidate.pins(),
            ReferenceInventory {
                content: &[],
                ..inventory()
            },
            limits(),
        ),
        Err(CheckpointError::InvalidReference)
    );
}

#[test]
fn additional_owned_candidates_preserve_closed_input_draw_and_cancellation_identities() {
    for timer in [false, true] {
        let (_, candidate, input) = admitted_case(if timer { 47 } else { 46 });
        let with_identity = fixture_candidate_with_created_identity(&candidate, timer);
        let GameInput::Host(host) = input else {
            panic!("registered closed host input required");
        };
        assert_eq!(
            host.basis.revision.next_sequence().unwrap(),
            with_identity.basis().revision
        );
        assert_eq!(with_identity.state().intents.len(), 2);
        assert_eq!(
            with_identity.state().intents[0].job,
            with_identity.state().intents[1].job
        );
        assert_eq!(
            with_identity.state().intents[0].timer,
            with_identity.state().intents[1].timer
        );
        assert_ne!(
            with_identity.state().intents[0].id,
            with_identity.state().intents[1].id
        );
        assert_fixture_codec_retains_complete(&with_identity);
    }
    let (_, candidate, _) = fixture_recovery_candidate();
    assert_eq!(candidate.state().draws[0].value, 18);
    assert_eq!(candidate.state().decisions[0].draws, vec![0]);
    assert_eq!(candidate.state().facts.len(), 2);
    assert_fixture_codec_retains_complete(&candidate);
}

fn assert_fixture_codec_retains_complete(candidate: &Checkpoint) {
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let encoded = crate::checkpoint_codec::encode_checkpoint(candidate, codec_limits()).unwrap();
    let recovered = crate::checkpoint_codec::decode_checkpoint(
        &encoded,
        candidate.basis(),
        candidate.pins(),
        ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        },
        limits(),
        codec_limits(),
    )
    .unwrap();
    assert_eq!(recovered, *candidate);
}

fn observe_registered_uniqueness_and_recovery_seed(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [81; 16];
    let campaign = [82; 16];
    let fence = [83; 16];
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-pg-unique-recovery".to_owned(),
    };
    for (session_byte, family) in [
        (43, Some(InsertedFamily::FactOrdinal)),
        (44, Some(InsertedFamily::IntentSlot)),
        (45, Some(InsertedFamily::EffectIdentity)),
        (46, Some(InsertedFamily::CreatedJob)),
        (47, Some(InsertedFamily::CreatedTimer)),
        (48, Some(InsertedFamily::LeaseBeforeCas)),
        (49, None),
    ] {
        let (baseline, mut candidate, input) = if session_byte == 49 {
            fixture_recovery_candidate()
        } else {
            admitted_case(session_byte)
        };
        if session_byte == 46 || session_byte == 47 {
            candidate = fixture_candidate_with_created_identity(&candidate, session_byte == 47);
        }
        stage.set(match family {
            Some(InsertedFamily::FactOrdinal) => {
                "actual fact ordinal constraint rollback and exact retry"
            }
            Some(InsertedFamily::IntentSlot) => {
                "actual intent slot constraint rollback and exact retry"
            }
            Some(InsertedFamily::EffectIdentity) => {
                "actual global effect identity constraint rollback and exact retry"
            }
            Some(InsertedFamily::CreatedJob) => {
                "actual created job partial unique index and valid cancellation reference"
            }
            Some(InsertedFamily::CreatedTimer) => {
                "actual created timer partial unique index and valid cancellation reference"
            }
            Some(InsertedFamily::LeaseBeforeCas) => {
                "lease expires at four family write barrier before strict clock CAS"
            }
            _ => "complete recovery checkpoint with observed draw and retained intent seed",
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let proof = actor_block_on(
            handle,
            within_deadline(deadline, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                seed_session(
                    &transaction,
                    &tenant,
                    &fence,
                    &baseline,
                    inventory(),
                    (limits(), codec_limits()),
                    120,
                )
                .await?;
                let proof = seed_proof(
                    &transaction,
                    &FixtureProof {
                        grant: FixtureGrant {
                            service_role: "df_persistence_fixture_runtime_a",
                            tenant: &tenant,
                            principal: &principal,
                            campaign: &campaign,
                            role: b"gm",
                            access_revision: b"fixture-access-1",
                            lifetime_seconds: 120,
                        },
                        basis: baseline.basis(),
                        operation: &[6; 16],
                        namespace: b"fixture/identity/v1",
                        canonical_fingerprint: &[session_byte; 32],
                        fence: &fence,
                        mode: 3,
                        lookup_only: false,
                    },
                )
                .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok::<_, RepositoryError>(proof)
            }),
        )?
        .map_err(|_| RepositoryError::Unavailable)??;
        let now = fixture_database_now(handle, admin, deadline)?;
        let (mut repository, scope, _) = registered_case_repository(
            handle,
            (configuration, port),
            &proof,
            input,
            now,
            bounds,
            &context,
        )?;
        let physical = PhysicalObservation {
            runtime: handle,
            administrator: admin,
            inventory: inventory(),
            model_limits: limits(),
            codec_limits: codec_limits(),
            maximum_receipt_bytes: 16384,
            deadline,
        };
        let tested = match family {
            Some(family) => observe_family_rollback(
                &mut repository,
                &scope,
                &candidate,
                baseline.basis(),
                family,
                &context,
                &physical,
            ),
            None => observe_success_and_retained(
                &mut repository,
                &scope,
                &candidate,
                baseline.basis(),
                &context,
                &physical,
            )
            .map(|_| ()),
        };
        let closed = repository.close();
        tested?;
        closed?;
        if session_byte == 46 || session_byte == 47 {
            let row = actor_block_on(handle, within_deadline(deadline, admin.query_one(
                "SELECT count(*)::bigint, count(*) FILTER (WHERE intent_kind IN (1,2,3,4))::bigint
                    FROM df_game.intents WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
                &[&tenant.as_slice(), &candidate.basis().session.as_bytes().as_slice()])))?
                .map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?;
            if row
                .try_get::<_, i64>(0)
                .map_err(|_| RepositoryError::Unavailable)?
                != 2
                || row
                    .try_get::<_, i64>(1)
                    .map_err(|_| RepositoryError::Unavailable)?
                    != 1
            {
                return Err(RepositoryError::InvalidReceipt);
            }
        }
        println!(
            "registered physical additional identity/lease/recovery case: {}",
            session_byte
        );
    }
    Ok(())
}

/// Invoked separately only after ROOT's runner has actually stopped and restarted
/// its exact owned postmaster over the same retained data directory. No seeding,
/// reducer execution or decision write occurs in this readback process.
#[test]
#[ignore = "requires ROOT registered owned postmaster restart and exact executable release"]
fn actual_registered_pg_restart_retains_complete_state_draws_intents_and_receipt() {
    let stage = std::cell::Cell::new("restart readback configuration");
    if let Err(error) = run_registered_restart_readback(&stage) {
        panic!("registered restart stage '{}': {:?}", stage.get(), error);
    }
}

fn run_registered_restart_readback(
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let port: u16 = std::env::var("DUNGEONFLUX_I01_OWNED_PG_PORT")
        .map_err(|_| RepositoryError::Unavailable)?
        .parse()
        .map_err(|_| RepositoryError::Unavailable)?;
    let database = std::env::var("DUNGEONFLUX_I01_OWNED_PG_DATABASE")
        .map_err(|_| RepositoryError::Unavailable)?;
    let admin_role = std::env::var("DUNGEONFLUX_I01_OWNED_PG_ADMIN_ROLE")
        .map_err(|_| RepositoryError::Unavailable)?;
    if port == 0
        || port == 55439
        || !database.starts_with("df_persistence_i01_")
        || admin_role.is_empty()
        || admin_role.len() > 63
    {
        return Err(RepositoryError::Unavailable);
    }
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|_| RepositoryError::Unavailable)?;
    let handle = runtime.handle().clone();
    let bounds = TransactionBounds {
        transaction: Duration::from_secs(5),
        rollback: Duration::from_secs(2),
        driver_join: Duration::from_secs(2),
    };
    let mut configuration = Config::new();
    configuration
        .host("127.0.0.1")
        .port(port)
        .dbname(&database)
        .user(&admin_role)
        .ssl_mode(SslMode::Disable);
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-pg-restart-readback".to_owned(),
    };
    let mut authority = connect_fixture_authority(&handle, &configuration, bounds, &context)?;
    let admin = authority.connection.take_client()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let observed = (|| {
        stage.set("restart retained actual scope proof and current membership");
        let (baseline, candidate, input) = fixture_recovery_candidate();
        let proof = actor_block_on(&handle, within_deadline(deadline, admin.query_one(
            "SELECT binding, tenant_id, principal_id, campaign_id, effective_role, access_revision,
                session_id, operation_id, command_namespace, recovery_epoch::text, fingerprint_version,
                canonical_fingerprint, owner_fence, execution_mode, lookup_only
                FROM df_fixture_authority.scope_proofs WHERE tenant_id=$1::bytea AND session_id=$2::bytea
                AND operation_id=$3::bytea AND command_namespace=$4::bytea",
            &[&[80_u8;16].as_slice(), &candidate.basis().session.as_bytes().as_slice(), &[6_u8;16].as_slice(),
                &b"fixture/identity/v1".as_slice()])))?
            .map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?;
        let now = fixture_database_now(&handle, &admin, deadline)?;
        let (mut repository, scope, _) = registered_case_repository(
            &handle,
            (&configuration, port),
            &proof,
            input,
            now,
            bounds,
            &context,
        )?;
        let checked = (|| {
            stage.set("restart exact retained receipt and full canonical observed draw state");
            let receipt = match repository.lookup_operation(&scope, &context)? {
                OperationLookup::Committed(receipt) => receipt,
                _ => return Err(RepositoryError::InvalidReceipt),
            };
            let restored = repository.load_current(&scope, &context)?;
            if receipt.basis() != candidate.basis()
                || receipt.decision() != &candidate.state().decisions[0]
                || restored != candidate
                || restored
                    .state()
                    .continuity
                    .canonical_packs
                    .first()
                    .and_then(|pack| pack.identities.first())
                    .and_then(|identity| identity.character_appearance.as_ref())
                    != Some(&fixture_character_appearance())
            {
                return Err(RepositoryError::InvalidReceipt);
            }
            assert_fixture_structural_recovery(&restored)?;
            let snapshot = actor_block_on(
                &handle,
                physical_snapshot(
                    &admin,
                    &[80; 16],
                    candidate.basis(),
                    codec_limits().maximum_document_bytes,
                    deadline,
                ),
            )??;
            let rules = vec![rule()];
            let entries = vec![content()];
            let resources = resource_constraints();
            validate_physical_commit(
                &snapshot,
                &candidate,
                ReferenceInventory {
                    rules: &rules,
                    content: &entries,
                    resources: &resources,
                    assets: &[],
                },
                limits(),
                codec_limits(),
            )?;
            let document = snapshot
                .envelope
                .as_ref()
                .ok_or(RepositoryError::InvalidCandidate)?;
            if *document
                != crate::checkpoint_codec::encode_checkpoint(&candidate, codec_limits())
                    .map_err(|_| RepositoryError::InvalidCandidate)?
                || baseline.basis().revision.sequence() != 7
            {
                return Err(RepositoryError::InvalidCandidate);
            }
            println!(
                "registered physical restart: exact complete envelope, retained receipt, observed draw 18/20, durable intent and exact character features/outfit/outfit revision recovered without decision execution"
            );
            Ok(())
        })();
        let closed = repository.close();
        checked?;
        closed?;
        Ok(())
    })();
    authority.connection.return_client(admin)?;
    let closed = actor_block_on(&handle, authority.connection.close())?;
    observed?;
    closed?;
    Ok(())
}

// A different admitted native build and run are current. The baseline remains a
// complete canonical checkpoint, but has no accepted decision and cannot be a
// new candidate for either the committed revision or this current basis/pin set.
fn retained_fixture_changed_run_and_pins(previous: Basis) -> Result<Checkpoint, RepositoryError> {
    let later = fixture_later_epoch(previous)?;
    let current_basis = Basis {
        run: RunId::from_bytes(&[90; 16]).map_err(|_| RepositoryError::InvalidCandidate)?,
        ..later.basis()
    };
    let mut current_pins = pins();
    current_pins.build = BuildIdentity::new(
        Some("fixture-source-2"),
        Some("fixture-native-2"),
        Some("fixture-wasm-2"),
        Some("fixture-config-2"),
        Some("fixture-content-1"),
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current_basis,
        current_pins,
        later.state().clone(),
        ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        },
        limits(),
    )
    .map_err(|_| RepositoryError::InvalidCandidate)
}

#[test]
fn retained_fixture_obsolete_candidate_is_canonical_but_invalid_for_changed_current_run_and_pins() {
    let (baseline, committed, _) = admitted_case(50);
    let current = retained_fixture_changed_run_and_pins(committed.basis()).unwrap();
    assert_ne!(baseline.basis().run, current.basis().run);
    assert_ne!(baseline.pins(), current.pins());
    assert!(baseline.state().decisions.is_empty());
    assert!(
        baseline
            .validate_resume(current.basis(), current.pins())
            .is_err()
    );
    let encoded = crate::checkpoint_codec::encode_checkpoint(&current, codec_limits()).unwrap();
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    assert_eq!(
        crate::checkpoint_codec::decode_checkpoint(
            &encoded,
            current.basis(),
            current.pins(),
            ReferenceInventory {
                rules: &rules,
                content: &entries,
                resources: &resources,
                assets: &[]
            },
            limits(),
            codec_limits()
        )
        .unwrap(),
        current
    );
}

// Full independent stored documents, sorted by their exact representation, detect
// any replay or projection mutation. This fixed fixture session belongs only to
// the registered administrator; no request may choose the tenant/session.
fn retained_fixture_all_family_bytes(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    tenant: &[u8; 16],
    session: SessionId,
    deadline: Instant,
) -> Result<Vec<String>, RepositoryError> {
    let rows = actor_block_on(
        handle,
        within_deadline(
            deadline,
            admin.query(
                "SELECT document FROM (
            SELECT jsonb_build_array('checkpoints',to_jsonb(c))::text AS document
                FROM df_game.checkpoints c WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            UNION ALL SELECT jsonb_build_array('facts',to_jsonb(f))::text
                FROM df_game.facts f WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            UNION ALL SELECT jsonb_build_array('operations',to_jsonb(o))::text
                FROM df_game.operations o WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            UNION ALL SELECT jsonb_build_array('intents',to_jsonb(i))::text
                FROM df_game.intents i WHERE tenant_id=$1::bytea AND session_id=$2::bytea
        ) retained ORDER BY document COLLATE \"C\"",
                &[&tenant.as_slice(), &session.as_bytes().as_slice()],
            ),
        ),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    rows.iter()
        .map(|row| row.try_get(0).map_err(|_| RepositoryError::Unavailable))
        .collect()
}

fn observe_registered_retained_refusals(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [87; 16];
    let campaign = [82; 16];
    let fence = [88; 16];
    let operation = [6; 16];
    let fingerprint = [56; 32];
    let namespace = b"fixture/retained/v1";
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-retained-refusals".to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let (baseline, candidate, input) = admitted_case(50);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let grant = || FixtureGrant {
        service_role: "df_persistence_fixture_runtime_a",
        tenant: &tenant,
        principal: &principal,
        campaign: &campaign,
        role: b"gm",
        access_revision: b"fixture-access-1",
        lifetime_seconds: 120,
    };
    stage.set("retained case canonical baseline and current verified grant");
    let proof = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            seed_grant(&transaction, &grant()).await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &operation,
                    namespace,
                    canonical_fingerprint: &fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let now = fixture_database_now(handle, admin, deadline)?;
    let (mut original, scope, _) = registered_case_repository(
        handle,
        (configuration, port),
        &proof,
        input.clone(),
        now,
        bounds,
        &context,
    )?;
    let physical = PhysicalObservation {
        runtime: handle,
        administrator: admin,
        inventory: inventory(),
        model_limits: limits(),
        codec_limits: codec_limits(),
        maximum_receipt_bytes: 16384,
        deadline,
    };
    let committed = observe_success_and_retained(
        &mut original,
        &scope,
        &candidate,
        baseline.basis(),
        &context,
        &physical,
    );
    let closed = original.close();
    let retained = committed?;
    closed?;
    let old_snapshot = actor_block_on(
        handle,
        physical_snapshot(
            admin,
            &tenant,
            candidate.basis(),
            codec_limits().maximum_document_bytes,
            deadline,
        ),
    )??;
    let original_receipt_bytes = old_snapshot
        .retained_receipt
        .ok_or(RepositoryError::InvalidReceipt)?;
    let current = retained_fixture_changed_run_and_pins(candidate.basis())?;
    stage.set("install canonical changed current run and build pins while retaining old receipt");
    actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            // Registered fixture restore, not a production restore API. Preserve all
            // immutable old four-family rows and insert the fully admitted new bytes.
            let bytes = crate::checkpoint_codec::encode_checkpoint(&current, codec_limits())
                .map_err(|_| RepositoryError::InvalidCandidate)?;
            let decoded = crate::checkpoint_codec::decode_checkpoint(
                &bytes,
                current.basis(),
                current.pins(),
                inventory(),
                limits(),
                codec_limits(),
            )
            .map_err(|_| RepositoryError::InvalidCandidate)?;
            if decoded != current {
                return Err(RepositoryError::InvalidCandidate);
            }
            let (epoch, sequence) =
                crate::revision_codec::encode_revision(current.basis().revision);
            let changed = transaction
                .execute(
                    crate::sql::INSERT_CHECKPOINT,
                    &[
                        &tenant.as_slice(),
                        &current.basis().session.as_bytes().as_slice(),
                        &epoch,
                        &sequence,
                        &current.basis().run.as_bytes().as_slice(),
                        &i32::from(current.schema()),
                        &crate::checkpoint_codec::STORAGE_CODEC_VERSION,
                        &bytes,
                    ],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            if changed != 1 {
                return Err(RepositoryError::Unavailable);
            }
            let changed = transaction
                .execute(
                    "UPDATE df_game.sessions SET run_id=$3::bytea,
            recovery_epoch=$4::text::numeric,in_epoch_sequence=$5::text::numeric
            WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
                    &[
                        &tenant.as_slice(),
                        &current.basis().session.as_bytes().as_slice(),
                        &current.basis().run.as_bytes().as_slice(),
                        &epoch,
                        &sequence,
                    ],
                )
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            if changed != 1 {
                return Err(RepositoryError::Unavailable);
            }
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let before = retained_fixture_all_family_bytes(
        handle,
        admin,
        &tenant,
        baseline.basis().session,
        deadline,
    )?;
    let direct = fixture_lost_ack_configuration(configuration, port)?;
    let make_repository = |proof: &tokio_postgres::Row, bound_input: GameInput| {
        let mut authority = connect_fixture_authority(handle, &direct, bounds, &context)?;
        let produced = bind_registered_scope(
            &mut authority,
            proof,
            FixtureBoundInput {
                input: bound_input,
                pins: current.pins().clone(),
            },
            now,
        );
        let (scope, verifier) = match produced {
            Ok(value) => value,
            Err(error) => {
                actor_block_on(handle, authority.connection.close())??;
                return Err(error);
            }
        };
        let repository = compose_registered_repository(
            authority,
            codec_limits(),
            16384,
            verifier,
            RecoverySource {
                rules: rules.clone(),
                content: entries.clone(),
                resource_resolver: None,
                asset_resolver: None,
                resources: resources.clone(),
                assets: vec![],
                limits: limits(),
            },
            bounds,
        )?;
        Ok::<_, RepositoryError>((repository, scope))
    };
    let (mut recovered, historical) = make_repository(&proof, input.clone())?;
    actor_block_on(
        handle,
        install_fault(admin, InsertedFamily::Checkpoint, deadline),
    )??;
    let observed = (|| {
        stage.set("retained duplicate wins over changed current run pins and canonical missing-decision candidate");
        if baseline.basis().run == current.basis().run
            || baseline.pins() == current.pins()
            || !baseline.state().decisions.is_empty()
            || baseline
                .validate_resume(current.basis(), current.pins())
                .is_ok()
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        for _ in 0..2 {
            match recovered.lookup_operation(&historical, &context)? {
                OperationLookup::Committed(receipt) if receipt == retained => {}
                _ => return Err(RepositoryError::InvalidReceipt),
            }
            match recovered.commit_decision(&historical, &baseline, current.basis(), &context)? {
                CommitOutcome::PreviouslyCommitted(receipt) if receipt == retained => {}
                _ => return Err(RepositoryError::InvalidReceipt),
            }
            if recovered.load_current(&historical, &context)? != current {
                return Err(RepositoryError::InvalidCandidate);
            }
        }
        if retained_fixture_all_family_bytes(
            handle,
            admin,
            &tenant,
            baseline.basis().session,
            deadline,
        )? != before
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        stage.set("actual separately verified exact-key fingerprint mismatch refuses without candidate validation or write");
        let mismatch = actor_block_on(
            handle,
            within_deadline(deadline, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                let proof = seed_proof(
                    &transaction,
                    &FixtureProof {
                        grant: grant(),
                        basis: baseline.basis(),
                        operation: &operation,
                        namespace,
                        canonical_fingerprint: &[57; 32],
                        fence: &fence,
                        mode: 3,
                        lookup_only: false,
                    },
                )
                .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok::<_, RepositoryError>(proof)
            }),
        )?
        .map_err(|_| RepositoryError::Unavailable)??;
        let (mut conflicting, conflict_scope) = make_repository(&mismatch, input.clone())?;
        let conflict_observed = (|| {
            if !matches!(
                conflicting.lookup_operation(&conflict_scope, &context)?,
                OperationLookup::Conflict
            ) || !matches!(
                conflicting.commit_decision(&conflict_scope, &baseline, current.basis(), &context),
                Err(RepositoryError::OperationConflict)
            ) {
                return Err(RepositoryError::InvalidReceipt);
            }
            if conflicting.load_current(&conflict_scope, &context)? != current {
                return Err(RepositoryError::InvalidCandidate);
            }
            Ok(())
        })();
        let closed = conflicting.close();
        conflict_observed?;
        closed?;
        stage
            .set("actual retained receipt tombstone remains indeterminate and refuses reexecution");
        let session = baseline.basis().session;
        let key_epoch = baseline.basis().revision.epoch().get().to_string();
        let key_parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
            &tenant.as_slice(),
            &session.as_bytes().as_slice(),
            &principal.as_slice(),
            &namespace.as_slice(),
            &key_epoch,
            &operation.as_slice(),
        ];
        let changed=actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_game.operations SET receipt=NULL WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND principal_id=$3::bytea AND command_namespace=$4::bytea AND recovery_epoch=$5::text::numeric AND operation_id=$6::bytea",key_parameters)))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if changed != 1 {
            return Err(RepositoryError::Unavailable);
        }
        let tombstone_observed = (|| {
            if !matches!(
                recovered.lookup_operation(&historical, &context)?,
                OperationLookup::ExpiredOrIndeterminate
            ) || !matches!(
                recovered.commit_decision(&historical, &baseline, current.basis(), &context),
                Err(RepositoryError::RetiredNamespace)
            ) {
                return Err(RepositoryError::InvalidReceipt);
            }
            let snapshot = actor_block_on(
                handle,
                physical_snapshot(
                    admin,
                    &tenant,
                    candidate.basis(),
                    codec_limits().maximum_document_bytes,
                    deadline,
                ),
            )??;
            if snapshot.family_counts != [1, 1, 1, 1]
                || snapshot.retained_receipt.is_some()
                || snapshot.envelope != old_snapshot.envelope
            {
                return Err(RepositoryError::InvalidReceipt);
            }
            if recovered.load_current(&historical, &context)? != current {
                return Err(RepositoryError::InvalidCandidate);
            }
            Ok(())
        })();
        let restore_parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
            &tenant.as_slice(),
            &session.as_bytes().as_slice(),
            &principal.as_slice(),
            &namespace.as_slice(),
            &key_epoch,
            &operation.as_slice(),
            &original_receipt_bytes,
        ];
        let restored=actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_game.operations SET receipt=$7::bytea WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND principal_id=$3::bytea AND command_namespace=$4::bytea AND recovery_epoch=$5::text::numeric AND operation_id=$6::bytea",restore_parameters)))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if restored != 1 {
            return Err(RepositoryError::Unavailable);
        }
        tombstone_observed?;
        if retained_fixture_all_family_bytes(handle, admin, &tenant, session, deadline)? != before {
            return Err(RepositoryError::InvalidReceipt);
        }
        stage.set(
            "retired namespace keeps retained result but never reports absent keys safe for rerun",
        );
        let inserted=actor_block_on(handle,within_deadline(deadline,admin.execute("INSERT INTO df_game.retired_namespaces (tenant_id,session_id,principal_id,command_namespace,recovery_epoch) VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::text::numeric)",&key_parameters[..5])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if inserted != 1 {
            return Err(RepositoryError::Unavailable);
        }
        let retirement_observed = (|| {
            match recovered.lookup_operation(&historical, &context)? {
                OperationLookup::Committed(receipt) if receipt == retained => {}
                _ => return Err(RepositoryError::InvalidReceipt),
            }
            match recovered.commit_decision(&historical, &baseline, current.basis(), &context)? {
                CommitOutcome::PreviouslyCommitted(receipt) if receipt == retained => {}
                _ => return Err(RepositoryError::InvalidReceipt),
            }
            let missing_operation = [8; 16];
            let other_namespace = b"fixture/retained/unretired/v1";
            for (case_namespace, retired) in [
                (namespace.as_slice(), true),
                (other_namespace.as_slice(), false),
            ] {
                let missing_proof = actor_block_on(
                    handle,
                    within_deadline(deadline, async {
                        let transaction = admin
                            .transaction()
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        let proof = seed_proof(
                            &transaction,
                            &FixtureProof {
                                grant: grant(),
                                basis: baseline.basis(),
                                operation: &missing_operation,
                                namespace: case_namespace,
                                canonical_fingerprint: &[58; 32],
                                fence: &fence,
                                mode: 3,
                                lookup_only: false,
                            },
                        )
                        .await?;
                        transaction
                            .commit()
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        Ok::<_, RepositoryError>(proof)
                    }),
                )?
                .map_err(|_| RepositoryError::Unavailable)??;
                let missing_input = GameInput::Host(HostInput {
                    basis: baseline.basis(),
                    operation: OperationId::from_bytes(&missing_operation)
                        .map_err(|_| RepositoryError::InputBinding)?,
                    host: member(3),
                    command: HostCommand::RequestCheckpoint,
                });
                let (mut missing, missing_scope) = make_repository(&missing_proof, missing_input)?;
                let missing_observed = (|| {
                    let lookup = missing.lookup_operation(&missing_scope, &context)?;
                    let commit = missing.commit_decision(
                        &missing_scope,
                        &baseline,
                        current.basis(),
                        &context,
                    );
                    if retired {
                        if !matches!(lookup, OperationLookup::ExpiredOrIndeterminate)
                            || !matches!(commit, Err(RepositoryError::RetiredNamespace))
                        {
                            return Err(RepositoryError::InvalidReceipt);
                        }
                    } else if !matches!(lookup, OperationLookup::NotRecorded)
                        || !matches!(commit, Err(RepositoryError::StaleEpoch))
                    {
                        return Err(RepositoryError::InvalidReceipt);
                    }
                    if missing.load_current(&missing_scope, &context)? != current {
                        return Err(RepositoryError::InvalidCandidate);
                    }
                    Ok(())
                })();
                let closed = missing.close();
                missing_observed?;
                closed?;
            }
            Ok(())
        })();
        let removed=actor_block_on(handle,within_deadline(deadline,admin.execute("DELETE FROM df_game.retired_namespaces WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND principal_id=$3::bytea AND command_namespace=$4::bytea AND recovery_epoch=$5::text::numeric",&key_parameters[..5])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if removed != 1 {
            return Err(RepositoryError::Unavailable);
        }
        retirement_observed?;
        if retained_fixture_all_family_bytes(handle, admin, &tenant, session, deadline)? != before {
            return Err(RepositoryError::InvalidReceipt);
        }
        match recovered.lookup_operation(&historical, &context)? {
            OperationLookup::Committed(receipt) if receipt == retained => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        match recovered.commit_decision(&historical, &baseline, current.basis(), &context)? {
            CommitOutcome::PreviouslyCommitted(receipt) if receipt == retained => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        if recovered.load_current(&historical, &context)? != current {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    })();
    let removed = actor_block_on(
        handle,
        remove_fault(admin, InsertedFamily::Checkpoint, deadline),
    )?;
    let closed = recovered.close();
    observed?;
    removed?;
    closed?;
    println!(
        "registered physical retained cases: changed current run/build pins, invalid-for-current canonical missing-decision candidate, fingerprint conflict, tombstone, retired namespace and independent full-family nonreexecution bytes"
    );
    Ok(())
}

// Owned fixture-only negative authority cases. Every proof and grant is produced
// by the registered administrator. Mutation of private scope data here is an
// adversarial test, never a public constructor or a continuing authority issuer.
fn grant_fixture_expect_unauthorized(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    candidate: &Checkpoint,
    expected: Basis,
    context: &OperationContext,
) -> Result<(), RepositoryError> {
    let lookup = repository.lookup_operation(scope, context);
    let commit = repository.commit_decision(scope, candidate, expected, context);
    let load = repository.load_current(scope, context);
    if !matches!(lookup, Err(RepositoryError::Unauthorized))
        || !matches!(commit, Err(RepositoryError::Unauthorized))
        || !matches!(load, Err(RepositoryError::Unauthorized))
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

fn observe_registered_current_grant_refusals(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [91; 16];
    let campaign = [82; 16];
    let fence = [92; 16];
    let operation = [6; 16];
    let fingerprint = [62; 32];
    let namespace = b"fixture/current-grant/v1";
    let deadline = Instant::now() + Duration::from_secs(30);
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-current-grant-refusals".to_owned(),
    };
    let (baseline, candidate, input) = admitted_case(52);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let grant = || FixtureGrant {
        service_role: "df_persistence_fixture_runtime_a",
        tenant: &tenant,
        principal: &principal,
        campaign: &campaign,
        role: b"gm",
        access_revision: b"fixture-access-1",
        lifetime_seconds: 120,
    };
    stage.set("current grant refusal baseline and actual registered proof");
    let proof = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            seed_grant(&transaction, &grant()).await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &operation,
                    namespace,
                    canonical_fingerprint: &fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let now = fixture_database_now(handle, admin, deadline)?;
    let (mut repository, mut scope, _) = registered_case_repository(
        handle,
        (configuration, port),
        &proof,
        input.clone(),
        now,
        bounds,
        &context,
    )?;
    let observed = (|| {
        let original = actor_block_on(handle, within_deadline(deadline, admin.query_one(
            "SELECT g.expires_at::text,p.expires_at::text FROM df_fixture_authority.grants g JOIN df_fixture_authority.scope_proofs p USING(service_role,tenant_id,principal_id,campaign_id) WHERE p.binding=$1::bytea",
            &[&scope.binding.as_slice()])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        let grant_expiry: String = original
            .try_get(0)
            .map_err(|_| RepositoryError::Unavailable)?;
        let proof_expiry: String = original
            .try_get(1)
            .map_err(|_| RepositoryError::Unavailable)?;
        let before = retained_fixture_all_family_bytes(
            handle,
            admin,
            &tenant,
            baseline.basis().session,
            deadline,
        )?;
        for (case, sql) in [
            (
                "actual current grant revoked",
                "UPDATE df_fixture_authority.grants SET active=false WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",
            ),
            (
                "actual current grant role changed",
                "UPDATE df_fixture_authority.grants SET effective_role=decode('706c61796572','hex') WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",
            ),
            (
                "actual current grant access revision changed",
                "UPDATE df_fixture_authority.grants SET access_revision=decode('666978747572652d6163636573732d32','hex') WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",
            ),
            (
                "actual current grant expiry boundary",
                "UPDATE df_fixture_authority.grants SET expires_at=clock_timestamp() WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",
            ),
        ] {
            stage.set(case);
            let parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
                &"df_persistence_fixture_runtime_a",
                &tenant.as_slice(),
                &principal.as_slice(),
                &campaign.as_slice(),
            ];
            let changed = actor_block_on(
                handle,
                within_deadline(deadline, admin.execute(sql, parameters)),
            )?
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)?;
            if changed != 1 {
                return Err(RepositoryError::Unavailable);
            }
            let refused = grant_fixture_expect_unauthorized(
                &mut repository,
                &scope,
                &candidate,
                baseline.basis(),
                &context,
            );
            // Restore the exact saved grant expiry even if a refusal assertion fails.
            let restored = actor_block_on(handle, within_deadline(deadline, admin.execute(
                "UPDATE df_fixture_authority.grants SET active=true,effective_role=$5::bytea,access_revision=$6::bytea,expires_at=$7::text::timestamptz WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",
                &[&"df_persistence_fixture_runtime_a", &tenant.as_slice(), &principal.as_slice(), &campaign.as_slice(), &b"gm".as_slice(), &b"fixture-access-1".as_slice(), &grant_expiry])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
            refused?;
            if restored != 1
                || !matches!(
                    repository.lookup_operation(&scope, &context)?,
                    OperationLookup::NotRecorded
                )
                || repository.load_current(&scope, &context)? != baseline
                || retained_fixture_all_family_bytes(
                    handle,
                    admin,
                    &tenant,
                    baseline.basis().session,
                    deadline,
                )? != before
            {
                return Err(RepositoryError::InvalidReceipt);
            }
        }
        stage.set("actual opaque proof expiry boundary");
        let changed = actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_fixture_authority.scope_proofs SET expires_at=clock_timestamp() WHERE binding=$1::bytea",&[&scope.binding.as_slice()])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if changed != 1 {
            return Err(RepositoryError::Unavailable);
        }
        let refused = grant_fixture_expect_unauthorized(
            &mut repository,
            &scope,
            &candidate,
            baseline.basis(),
            &context,
        );
        let restored = actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_fixture_authority.scope_proofs SET expires_at=$2::text::timestamptz WHERE binding=$1::bytea",&[&scope.binding.as_slice(),&proof_expiry])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        refused?;
        if restored != 1 || repository.load_current(&scope, &context)? != baseline {
            return Err(RepositoryError::InvalidCandidate);
        }
        stage.set(
            "private forged tenant and principal cannot replace checked membership capability",
        );
        let original_tenant = std::mem::replace(&mut scope.tenant, [93; 16]);
        let refused = grant_fixture_expect_unauthorized(
            &mut repository,
            &scope,
            &candidate,
            baseline.basis(),
            &context,
        );
        scope.tenant = original_tenant;
        refused?;
        let original_principal = std::mem::replace(&mut scope.principal, [94; 16]);
        let refused = grant_fixture_expect_unauthorized(
            &mut repository,
            &scope,
            &candidate,
            baseline.basis(),
            &context,
        );
        scope.principal = original_principal;
        refused?;
        // Real foreign-scope proofs are seeded with matching current grants, then
        // substituted for the original opaque token. All other trusted fields stay
        // unchanged. This tests verifier equality, not just absent-row refusal.
        for (case, service_role, other_tenant, other_principal, other_campaign) in [
            (
                "actual proof from other tenant",
                "df_persistence_fixture_runtime_a",
                [93; 16],
                principal,
                campaign,
            ),
            (
                "actual proof from other principal",
                "df_persistence_fixture_runtime_a",
                tenant,
                [94; 16],
                campaign,
            ),
            (
                "actual proof from other campaign",
                "df_persistence_fixture_runtime_a",
                tenant,
                principal,
                [95; 16],
            ),
            (
                "actual proof from other authenticated service role",
                "df_persistence_fixture_runtime_b",
                tenant,
                principal,
                campaign,
            ),
        ] {
            stage.set(case);
            let other_proof = actor_block_on(
                handle,
                within_deadline(deadline, async {
                    let transaction = admin
                        .transaction()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    let alternative = || FixtureGrant {
                        service_role,
                        tenant: &other_tenant,
                        principal: &other_principal,
                        campaign: &other_campaign,
                        role: b"gm",
                        access_revision: b"fixture-access-1",
                        lifetime_seconds: 120,
                    };
                    seed_grant(&transaction, &alternative()).await?;
                    let proof = seed_proof(
                        &transaction,
                        &FixtureProof {
                            grant: alternative(),
                            basis: baseline.basis(),
                            operation: &operation,
                            namespace,
                            canonical_fingerprint: &fingerprint,
                            fence: &fence,
                            mode: 3,
                            lookup_only: false,
                        },
                    )
                    .await?;
                    transaction
                        .commit()
                        .await
                        .map_err(|_| RepositoryError::Unavailable)?;
                    Ok::<_, RepositoryError>(proof)
                }),
            )?
            .map_err(|_| RepositoryError::Unavailable)??;
            let other_binding: Vec<u8> = other_proof
                .try_get("binding")
                .map_err(|_| RepositoryError::Unauthorized)?;
            if other_binding.len() != 32 {
                return Err(RepositoryError::Unauthorized);
            }
            let original_binding = std::mem::replace(&mut scope.binding, other_binding);
            let refused = grant_fixture_expect_unauthorized(
                &mut repository,
                &scope,
                &candidate,
                baseline.basis(),
                &context,
            );
            scope.binding = original_binding;
            refused?;
            if !matches!(
                repository.lookup_operation(&scope, &context)?,
                OperationLookup::NotRecorded
            ) || repository.load_current(&scope, &context)? != baseline
                || retained_fixture_all_family_bytes(
                    handle,
                    admin,
                    &tenant,
                    baseline.basis().session,
                    deadline,
                )? != before
            {
                return Err(RepositoryError::InvalidReceipt);
            }
        }
        stage
            .set("same real connection confirms one decision after all current authority refusals");
        let physical = PhysicalObservation {
            runtime: handle,
            administrator: admin,
            inventory: inventory(),
            model_limits: limits(),
            codec_limits: codec_limits(),
            maximum_receipt_bytes: 16384,
            deadline,
        };
        let receipt = observe_success_and_retained(
            &mut repository,
            &scope,
            &candidate,
            baseline.basis(),
            &context,
            &physical,
        )?;
        let committed = retained_fixture_all_family_bytes(
            handle,
            admin,
            &tenant,
            baseline.basis().session,
            deadline,
        )?;
        stage.set("revoked current grant refuses even a historically retained exact-key receipt");
        let changed = actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_fixture_authority.grants SET active=false WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",&[&"df_persistence_fixture_runtime_a",&tenant.as_slice(),&principal.as_slice(),&campaign.as_slice()])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if changed != 1 {
            return Err(RepositoryError::Unavailable);
        }
        let refused = grant_fixture_expect_unauthorized(
            &mut repository,
            &scope,
            &baseline,
            candidate.basis(),
            &context,
        );
        let restored = actor_block_on(handle,within_deadline(deadline,admin.execute("UPDATE df_fixture_authority.grants SET active=true WHERE service_role=$1::text::name AND tenant_id=$2::bytea AND principal_id=$3::bytea AND campaign_id=$4::bytea",&[&"df_persistence_fixture_runtime_a",&tenant.as_slice(),&principal.as_slice(),&campaign.as_slice()])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        refused?;
        if restored != 1 {
            return Err(RepositoryError::Unavailable);
        }
        match repository.lookup_operation(&scope, &context)? {
            OperationLookup::Committed(value) if value == receipt => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        match repository.commit_decision(&scope, &baseline, candidate.basis(), &context)? {
            CommitOutcome::PreviouslyCommitted(value) if value == receipt => {}
            _ => return Err(RepositoryError::InvalidReceipt),
        }
        if repository.load_current(&scope, &context)? != candidate
            || retained_fixture_all_family_bytes(
                handle,
                admin,
                &tenant,
                baseline.basis().session,
                deadline,
            )? != committed
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        Ok(())
    })();
    let closed = repository.close();
    observed?;
    closed?;
    println!(
        "registered physical current authority cases: revoked/changed role/access revision/expiry, expired proof, forged tenant/principal, actual cross-tenant/principal/campaign/service-role proofs, retained result authorization and same-connection recovery; causal job issuer remains unqualified"
    );
    Ok(())
}

// These controlled canonical ports prove native lifetime and commit ordering only.
// The fixture does not claim production game mechanics or dispatch paid providers.
struct LifetimeFixtureEngine {
    baseline: Checkpoint,
    candidate: Checkpoint,
    input: GameInput,
    observed: std::sync::Arc<std::sync::Mutex<[u64; 3]>>,
}
impl
    df_session::submission::SessionEngine<
        crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    > for LifetimeFixtureEngine
{
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        scope.validate_input(input)?;
        if *current != self.baseline || *input != self.input {
            return Err(RepositoryError::InvalidCandidate);
        }
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        observed[0] = observed[0]
            .checked_add(1)
            .ok_or(RepositoryError::Capacity)?;
        Ok(self.candidate.clone())
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        if *checkpoint == self.baseline || *checkpoint == self.candidate {
            Ok(())
        } else {
            Err(RepositoryError::InvalidCandidate)
        }
    }
}
struct LifetimeFixturePublication<'a> {
    runtime: &'a tokio::runtime::Handle,
    administrator: &'a tokio_postgres::Client,
    candidate: Checkpoint,
    deadline: Instant,
    observed: std::sync::Arc<std::sync::Mutex<[u64; 3]>>,
}
impl LifetimeFixturePublication<'_> {
    fn independently_visible(
        &self,
        scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    ) -> Result<(), RepositoryError> {
        let current=actor_block_on(self.runtime,within_deadline(self.deadline,self.administrator.query_one(
            "SELECT EXISTS(SELECT 1 FROM df_fixture_authority.scope_proofs p JOIN df_fixture_authority.grants g USING(service_role,tenant_id,principal_id,campaign_id) JOIN df_game.sessions s ON s.tenant_id=p.tenant_id AND s.session_id=p.session_id WHERE p.binding=$1::bytea AND p.tenant_id=$2::bytea AND p.principal_id=$3::bytea AND p.session_id=$4::bytea AND p.service_role='df_persistence_fixture_runtime_a' AND g.active AND g.expires_at>clock_timestamp() AND p.expires_at>clock_timestamp() AND g.effective_role=p.effective_role AND g.access_revision=p.access_revision AND s.owner_fence=p.owner_fence AND s.lease_until>clock_timestamp() AND s.run_id=$5::bytea AND s.recovery_epoch=$6::text::numeric AND s.in_epoch_sequence=$7::text::numeric)",
            &[&scope.binding.as_slice(),&scope.tenant.as_slice(),&scope.principal.as_slice(),&scope.session.as_bytes().as_slice(),&self.candidate.basis().run.as_bytes().as_slice(),&self.candidate.basis().revision.epoch().get().to_string(),&self.candidate.basis().revision.sequence().to_string()])))?.map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if !current
            .try_get::<_, bool>(0)
            .map_err(|_| RepositoryError::Unavailable)?
        {
            return Err(RepositoryError::Unauthorized);
        }
        let snapshot = actor_block_on(
            self.runtime,
            physical_snapshot(
                self.administrator,
                &scope.tenant,
                self.candidate.basis(),
                codec_limits().maximum_document_bytes,
                self.deadline,
            ),
        )??;
        let rules = vec![rule()];
        let entries = vec![content()];
        let resources = resource_constraints();
        validate_physical_commit(
            &snapshot,
            &self.candidate,
            ReferenceInventory {
                rules: &rules,
                content: &entries,
                resources: &resources,
                assets: &[],
            },
            limits(),
            codec_limits(),
        )?;
        let document = snapshot
            .retained_receipt
            .as_ref()
            .ok_or(RepositoryError::InvalidReceipt)?;
        let receipt = decode_receipt(
            document,
            scope.session,
            scope.operation,
            16384,
            codec_limits(),
        )
        .map_err(|_| RepositoryError::InvalidReceipt)?;
        if receipt.basis() != self.candidate.basis()
            || !self
                .candidate
                .state()
                .decisions
                .iter()
                .any(|decision| decision == receipt.decision())
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        Ok(())
    }
}
impl
    df_session::submission::PublicationOwner<
        crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    > for LifetimeFixturePublication<'_>
{
    fn publish_committed(
        &mut self,
        scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
        checkpoint: &Checkpoint,
    ) -> Result<(), df_session::submission::DeliveryError> {
        {
            let mut observed = self
                .observed
                .lock()
                .map_err(|_| df_session::submission::DeliveryError::Unavailable)?;
            observed[1] = observed[1]
                .checked_add(1)
                .ok_or(df_session::submission::DeliveryError::Unavailable)?;
        }
        if *checkpoint != self.candidate {
            return Err(df_session::submission::DeliveryError::Unavailable);
        }
        self.independently_visible(scope)
            .map_err(|_| df_session::submission::DeliveryError::Unavailable)
    }
    fn wake_committed_intents(
        &mut self,
        scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    ) -> Result<(), df_session::submission::DeliveryError> {
        {
            let mut observed = self
                .observed
                .lock()
                .map_err(|_| df_session::submission::DeliveryError::Unavailable)?;
            observed[2] = observed[2]
                .checked_add(1)
                .ok_or(df_session::submission::DeliveryError::Unavailable)?;
        }
        self.independently_visible(scope)
            .map_err(|_| df_session::submission::DeliveryError::Unavailable)
    }
}

fn observe_registered_actor_postgres_lifetime(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    for (session_byte, dropped_receiver, fail_after_insert, block_before_commit) in [
        (53, false, false, false),
        (54, true, false, false),
        (55, false, true, false),
        (58, false, false, true),
    ] {
        stage.set(match (dropped_receiver,fail_after_insert) {(true,_)=>"actual accepted receiver dropped then inbox stop/drain into_repository close/join",(_,true)=>"actual actor PostgreSQL insertion rollback never confirms or publishes",_=>"actual actor confirmed receipt with independently visible durable publication before joined close"});
        let tenant = [80; 16];
        let principal = [session_byte + 43; 16];
        let campaign = [82; 16];
        let fence = [session_byte + 53; 16];
        let operation = [6; 16];
        let fingerprint = [session_byte + 11; 32];
        let namespace = b"fixture/actor-lifetime/v1";
        let deadline = Instant::now() + Duration::from_secs(30);
        let context = OperationContext {
            trace_parent: String::new(),
            build: "persistence-actor-lifetime-only".to_owned(),
        };
        let (baseline, candidate, input) = admitted_case(session_byte);
        let rules = vec![rule()];
        let entries = vec![content()];
        let resources = resource_constraints();
        let inventory = || ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        };
        let proof = actor_block_on(
            handle,
            within_deadline(deadline, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                let grant = || FixtureGrant {
                    service_role: "df_persistence_fixture_runtime_a",
                    tenant: &tenant,
                    principal: &principal,
                    campaign: &campaign,
                    role: b"gm",
                    access_revision: b"fixture-access-1",
                    lifetime_seconds: 120,
                };
                seed_session(
                    &transaction,
                    &tenant,
                    &fence,
                    &baseline,
                    inventory(),
                    (limits(), codec_limits()),
                    120,
                )
                .await?;
                seed_grant(&transaction, &grant()).await?;
                let proof = seed_proof(
                    &transaction,
                    &FixtureProof {
                        grant: grant(),
                        basis: baseline.basis(),
                        operation: &operation,
                        namespace,
                        canonical_fingerprint: &fingerprint,
                        fence: &fence,
                        mode: 3,
                        lookup_only: false,
                    },
                )
                .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok::<_, RepositoryError>(proof)
            }),
        )?
        .map_err(|_| RepositoryError::Unavailable)??;
        let now = fixture_database_now(handle, admin, deadline)?;
        let (repository, scope, backend_pid) = registered_case_repository(
            handle,
            (configuration, port),
            &proof,
            input.clone(),
            now,
            bounds,
            &context,
        )?;
        let before = retained_fixture_all_family_bytes(
            handle,
            admin,
            &tenant,
            baseline.basis().session,
            deadline,
        )?;
        let (sender, actor_loop) = df_session::inbox::bounded_inbox::<
            df_session::submission::OwnedInput<
                crate::native_scope::NativeScope<FixtureMembershipAuthority>,
            >,
        >();
        let (item, receipt) =
            df_session::submission::OwnedInput::new(context, scope, input.clone());
        let admitted = sender
            .try_submit(item)
            .map_err(|_| RepositoryError::Capacity)?;
        if admitted.0 != 1
            || !matches!(
                receipt.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            )
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let receipt = if dropped_receiver {
            drop(receipt);
            None
        } else {
            Some(receipt)
        };
        sender.stop().map_err(|_| RepositoryError::Unavailable)?;
        if sender
            .usage()
            .map_err(|_| RepositoryError::Unavailable)?
            .accepting
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let observed = std::sync::Arc::new(std::sync::Mutex::new([0_u64; 3]));
        if fail_after_insert {
            actor_block_on(handle, install_fault(admin, InsertedFamily::Fact, deadline))??;
        }
        // The registered administrator owns this controlled table lock. The
        // writer can insert its checkpoint, facts and operation, but must wait
        // before its intent INSERT and COMMIT. Independent reads use the same
        // administrator backend, which cannot see the writer's uncommitted rows.
        let holding_pid = if block_before_commit {
            stage.set("actual actor waits before COMMIT at intent INSERT; no receipt/publication/wake or independently visible family writes");
            Some(
                actor_block_on(
                    handle,
                    within_deadline(deadline, async {
                        let row = admin
                            .query_one("SELECT pg_backend_pid()", &[])
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        let pid = row
                            .try_get::<_, i32>(0)
                            .map_err(|_| RepositoryError::Unavailable)?;
                        admin
                            .batch_execute("BEGIN; LOCK TABLE df_game.intents IN SHARE MODE")
                            .await
                            .map_err(|_| RepositoryError::Unavailable)?;
                        Ok::<_, RepositoryError>(pid)
                    }),
                )
                .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
                .and_then(|value| value)?,
            )
        } else {
            None
        };
        let mut blocked_observation = Ok(());
        let mut blocker_release = Ok(());
        let actor_result = std::thread::scope(|threads| {
            let publication = LifetimeFixturePublication {
                runtime: handle,
                administrator: &*admin,
                candidate: candidate.clone(),
                deadline,
                observed: observed.clone(),
            };
            let engine = LifetimeFixtureEngine {
                baseline: baseline.clone(),
                candidate: candidate.clone(),
                input,
                observed: observed.clone(),
            };
            let initial = baseline.clone();
            let actor = threads.spawn(move || {
                // This dedicated joined blocking actor uses the existing runtime.
                let actor_id = std::thread::current().id();
                if tokio::runtime::Handle::try_current().is_ok() {
                    return Err(RepositoryError::Unavailable);
                }
                let mut owner = df_session::submission::DurableOwner::new(
                    repository,
                    engine,
                    publication,
                    initial,
                    16384,
                )?;
                let drained = actor_loop
                    .run(&mut owner)
                    .map_err(|_| RepositoryError::Unavailable);
                let checkpoint = owner.checkpoint().clone();
                let mut returned = owner.into_repository();
                let close = returned.close();
                // Retain the original failure and its owner. Retry on this same
                // actor while the injected runtime is alive, before parent join.
                // A successful retry proves cleanup only, never a successful first close.
                let retry_close = if close.is_err() {
                    Some(returned.close())
                } else {
                    None
                };
                // Exhausted retries return the still-owned repository to the
                // fixture owner; this actor never drops it on a retry error.
                let cleanup = retain_fixture_repository_after_retry(returned, close, retry_close);
                let close_id = std::thread::current().id();
                Ok::<_, RepositoryError>((drained, checkpoint, cleanup, actor_id, close_id))
            });
            if let Some(holding_pid) = holding_pid {
                // Observe an actual PostgreSQL lock wait, not a timing guess.
                // Keep observation failures until AFTER releasing the lock and
                // joining this owned actor, so no assertion strands accepted work.
                blocked_observation = (|| {
                    actor_block_on(handle, within_deadline(Instant::now() + Duration::from_secs(3), async {
                        loop {
                            admin.query_one("SELECT pg_stat_clear_snapshot()", &[]).await
                                .map_err(|_| RepositoryError::Unavailable)?;
                            let row = admin.query_one(
                                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND datname=current_database() AND wait_event_type='Lock' AND query LIKE '%INSERT INTO df_game.intents%' AND $2::integer=ANY(pg_blocking_pids(pid)))",
                                &[&backend_pid, &holding_pid]).await.map_err(|_| RepositoryError::Unavailable)?;
                            if row.try_get::<_,bool>(0).map_err(|_| RepositoryError::Unavailable)? { break; }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Ok::<_,RepositoryError>(())
                    })).and_then(|value| value.map_err(|_| RepositoryError::Unavailable)).and_then(|value| value)?;
                    if actor.is_finished()
                        || *observed.lock().map_err(|_| RepositoryError::Unavailable)? != [1, 0, 0]
                        || !matches!(
                            receipt
                                .as_ref()
                                .ok_or(RepositoryError::InvalidReceipt)?
                                .try_recv(),
                            Err(std::sync::mpsc::TryRecvError::Empty)
                        )
                        || retained_fixture_all_family_bytes(
                            handle,
                            admin,
                            &tenant,
                            baseline.basis().session,
                            deadline,
                        )? != before
                    {
                        return Err(RepositoryError::InvalidReceipt);
                    }
                    Ok(())
                })();
                // COMMIT only releases the administrator's fixture lock. This
                // transaction has made no game writes; the actual actor still
                // owns and commits its independent four-family decision.
                blocker_release = actor_block_on(
                    handle,
                    within_deadline(deadline, admin.batch_execute("COMMIT")),
                )
                .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
                .and_then(|value| value.map_err(|_| RepositoryError::Unavailable));
                if blocker_release.is_err() {
                    // Retain the original failed release while attempting one
                    // bounded rollback; never report cleanup as successful release.
                    let rollback = actor_block_on(
                        handle,
                        within_deadline(deadline, admin.batch_execute("ROLLBACK")),
                    )
                    .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
                    .and_then(|value| value.map_err(|_| RepositoryError::Unavailable));
                    if rollback.is_err() {
                        blocker_release = Err(RepositoryError::UnresolvedCommit);
                    }
                }
            }
            actor.join().map_err(|_| RepositoryError::Unavailable)?
        });
        let (drained, checkpoint, cleanup, actor_id, close_id) = actor_result?;
        let settled = match cleanup {
            Ok(settled) => settled,
            Err(pending) => pending.fail_fixture_while_retaining_owner(),
        };
        // Match a retained pending owner before any fallible administrative work.
        let removed = if fail_after_insert {
            actor_block_on(handle, remove_fault(admin, InsertedFamily::Fact, deadline))?
        } else {
            Ok(())
        };
        // Only a physically joined driver may be dropped here. Preserve the
        // original error even if a second/final attempt completed cleanup.
        drop(settled.repository);
        settled.first_close?;
        if let Some(retry) = settled.retry_close {
            retry?;
        }
        if let Some(final_cleanup) = settled.final_cleanup {
            final_cleanup?;
        }
        removed?;
        blocked_observation?;
        blocker_release?;
        if actor_id == std::thread::current().id() || actor_id != close_id {
            return Err(RepositoryError::Unavailable);
        }
        let drained = drained?;
        if drained.reduced_inputs != 1 || drained.last_sequence != Some(admitted) {
            return Err(RepositoryError::InvalidReceipt);
        }
        let counters = *observed.lock().map_err(|_| RepositoryError::Unavailable)?;
        if fail_after_insert {
            if counters != [1, 0, 0]
                || checkpoint != baseline
                || retained_fixture_all_family_bytes(
                    handle,
                    admin,
                    &tenant,
                    baseline.basis().session,
                    deadline,
                )? != before
            {
                return Err(RepositoryError::InvalidReceipt);
            }
            let outcome = receipt
                .ok_or(RepositoryError::InvalidReceipt)?
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| RepositoryError::InvalidReceipt)?;
            if !matches!(
                outcome,
                df_session::submission::SubmissionOutcome::Refused(RepositoryError::Unavailable)
            ) {
                return Err(RepositoryError::InvalidReceipt);
            }
        } else {
            if counters != [1, 1, 1] || checkpoint != candidate {
                return Err(RepositoryError::InvalidReceipt);
            }
            let snapshot = actor_block_on(
                handle,
                physical_snapshot(
                    admin,
                    &tenant,
                    candidate.basis(),
                    codec_limits().maximum_document_bytes,
                    deadline,
                ),
            )??;
            validate_physical_commit(&snapshot, &candidate, inventory(), limits(), codec_limits())?;
            let document = snapshot
                .retained_receipt
                .as_ref()
                .ok_or(RepositoryError::InvalidReceipt)?;
            let stored = decode_receipt(
                document,
                candidate.basis().session,
                OperationId::from_bytes(&operation).map_err(|_| RepositoryError::InputBinding)?,
                16384,
                codec_limits(),
            )
            .map_err(|_| RepositoryError::InvalidReceipt)?;
            if let Some(receipt) = receipt {
                let outcome = receipt
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| RepositoryError::InvalidReceipt)?;
                if !matches!(outcome,df_session::submission::SubmissionOutcome::Confirmed(ref value) if *value==stored)
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
            }
        }
        // Driver close/join alone is not a server-process observation. The
        // independent administrator observes that exact owned backend disappear.
        actor_block_on(handle,within_deadline(deadline,async {
            loop {
                let row=admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND datname=current_database())",&[&backend_pid]).await.map_err(|_|RepositoryError::Unavailable)?;
                if !row.try_get::<_,bool>(0).map_err(|_|RepositoryError::Unavailable)? {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,RepositoryError>(())
        }))?.map_err(|_|RepositoryError::Unavailable)??;
    }
    println!(
        "registered physical blocked actor preCOMMIT: actual intent INSERT waits on registered administrator table lock; independent complete four-family bytes unchanged; one engine decision, no receipt/publication/wake while blocked; lock released before owned actor joined; then exact confirmed decision and joined native close before parent join; controlled mechanics only"
    );
    println!(
        "registered physical actual Session12 actor lifecycle: mailbox admission never confirmed; normal receipt and dropped receiver preserve one durable decision; real insertion refusal has no publication; inbox stop/drain -> same actor into_repository -> explicit joined native close -> parent join -> exact backend absence; controlled mechanics/publication only, Unknown and forced close timeout remain unqualified"
    );
    Ok(())
}

fn observe_persisted_codec_metadata(
    physical: &PhysicalObservation<'_>,
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    candidate: &Checkpoint,
    context: &OperationContext,
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = candidate.basis().session;
    let (epoch, sequence) = crate::revision_codec::encode_revision(candidate.basis().revision);
    let current = crate::checkpoint_codec::STORAGE_CODEC_VERSION;
    let mismatched = current.checked_add(1).ok_or(RepositoryError::Capacity)?;
    for rejected in [1_i32, mismatched] {
        // Only this fresh registered fixture row is deliberately corrupted;
        // restore the metadata before proving reuse of the actual adapter.
        let changed = actor_block_on(
            physical.runtime,
            within_deadline(
                physical.deadline,
                physical.administrator.execute(
                    "UPDATE df_game.checkpoints SET codec_version=$5::integer WHERE tenant_id=$1::bytea
                     AND session_id=$2::bytea AND recovery_epoch=$3::text::numeric
                     AND in_epoch_sequence=$4::text::numeric AND codec_version=$6::integer",
                    &[
                        &tenant,
                        &session.as_bytes().as_slice(),
                        &epoch,
                        &sequence,
                        &rejected,
                        &current,
                    ],
                ),
            ),
        )?
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
        if changed != 1 {
            return Err(RepositoryError::InvalidCandidate);
        }
        let refused = matches!(
            repository.load_current(scope, context),
            Err(RepositoryError::InvalidCandidate)
        );
        let restored = actor_block_on(
            physical.runtime,
            within_deadline(
                physical.deadline,
                physical.administrator.execute(
                    "UPDATE df_game.checkpoints SET codec_version=$6::integer WHERE tenant_id=$1::bytea
                     AND session_id=$2::bytea AND recovery_epoch=$3::text::numeric
                     AND in_epoch_sequence=$4::text::numeric AND codec_version=$5::integer",
                    &[
                        &tenant,
                        &session.as_bytes().as_slice(),
                        &epoch,
                        &sequence,
                        &rejected,
                        &current,
                    ],
                ),
            ),
        )?
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
        if restored != 1 || !refused || repository.load_current(scope, context)? != *candidate {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    let snapshot = actor_block_on(
        physical.runtime,
        physical_snapshot(
            physical.administrator,
            &scope.tenant,
            candidate.basis(),
            physical.codec_limits.maximum_document_bytes,
            physical.deadline,
        ),
    )??;
    validate_physical_commit(
        &snapshot,
        candidate,
        ReferenceInventory {
            rules: physical.inventory.rules,
            content: physical.inventory.content,
            resources: physical.inventory.resources,
            assets: physical.inventory.assets,
        },
        physical.model_limits,
        physical.codec_limits,
    )?;
    println!(
        "registered physical codec{} storage boundary: all four format metadata values match encoded records; legacy1 and mismatched{} checkpoint metadata refuse through actual load_current; exact full candidate reload after each restoration on same adapter; canonical schema{} retained",
        current, mismatched, CHECKPOINT_SCHEMA
    );
    Ok(())
}

fn observe_administrator_identity_before_close(
    pid: i32,
    backend_start: &str,
) -> Result<(), RepositoryError> {
    // The administrator handshake runs over a Unix-domain socket, which
    // stable std only provides on Unix; without it the identity is unavailable.
    #[cfg(not(unix))]
    {
        let _ = (pid, backend_start);
        return Err(RepositoryError::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        if pid <= 0
            || pid == 80873
            || backend_start.is_empty()
            || backend_start.len() > 96
            || backend_start.contains(['\n', '\r'])
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let path = std::env::var("DUNGEONFLUX_I01_ADMIN_IDENTITY_SOCKET")
            .map_err(|_| RepositoryError::Unavailable)?;
        if path.is_empty() || path.len() > 100 {
            return Err(RepositoryError::Unavailable);
        }
        let mut stream = UnixStream::connect(path).map_err(|_| RepositoryError::Unavailable)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let report = format!("READY {pid} {backend_start}\n");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| RepositoryError::Unavailable)?;
        stream
            .write_all(report.as_bytes())
            .map_err(|_| RepositoryError::Unavailable)?;
        let expected = format!("OBSERVED {pid} {backend_start}\n");
        let mut acknowledgement = vec![0_u8; expected.len()];
        let mut received = 0;
        while received < acknowledgement.len() {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or(RepositoryError::Unavailable)?;
            stream
                .set_read_timeout(Some(remaining))
                .map_err(|_| RepositoryError::Unavailable)?;
            let count = stream
                .read(&mut acknowledgement[received..])
                .map_err(|_| RepositoryError::Unavailable)?;
            if count == 0 {
                return Err(RepositoryError::Unavailable);
            }
            received += count;
        }
        if acknowledgement != expected.as_bytes() {
            return Err(RepositoryError::InvalidReceipt);
        }
        Ok(())
    }
}

// ROOT starts a separate fresh owned cluster with exactly one durability
// setting disabled. This suite never changes a server setting or reloads it.
// Administrator seed rows are fixture setup, not a durable decision claim.
#[test]
#[ignore = "requires separately registered owned unsafe PostgreSQL startup profile"]
fn actual_registered_pg_unsafe_durability_refuses_before_all_family_writes() {
    let stage = std::cell::Cell::new("unsafe owned startup profile identity");
    if let Err(error) = run_registered_unsafe_durability(&stage) {
        panic!(
            "actual registered unsafe PostgreSQL refusal failed at {}: {error:?}",
            stage.get()
        );
    }
}

fn run_registered_unsafe_durability(
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let disabled = std::env::var("DUNGEONFLUX_I01_OWNED_UNSAFE_SETTING")
        .map_err(|_| RepositoryError::Unavailable)?;
    if !matches!(disabled.as_str(), "fsync" | "full_page_writes") {
        return Err(RepositoryError::Unavailable);
    }
    let port: u16 = std::env::var("DUNGEONFLUX_I01_OWNED_PG_PORT")
        .map_err(|_| RepositoryError::Unavailable)?
        .parse()
        .map_err(|_| RepositoryError::Unavailable)?;
    let database = std::env::var("DUNGEONFLUX_I01_OWNED_PG_DATABASE")
        .map_err(|_| RepositoryError::Unavailable)?;
    let admin_role = std::env::var("DUNGEONFLUX_I01_OWNED_PG_ADMIN_ROLE")
        .map_err(|_| RepositoryError::Unavailable)?;
    if port == 0
        || port == 55439
        || !database.starts_with("df_persistence_i01_")
        || admin_role.is_empty()
        || admin_role.len() > 63
    {
        return Err(RepositoryError::Unavailable);
    }
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|_| RepositoryError::Unavailable)?;
    let handle = runtime.handle().clone();
    let bounds = TransactionBounds {
        transaction: Duration::from_secs(5),
        rollback: Duration::from_secs(2),
        driver_join: Duration::from_secs(2),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut configuration = Config::new();
    configuration
        .host("127.0.0.1")
        .port(port)
        .dbname(&database)
        .user(&admin_role)
        .ssl_mode(SslMode::Disable);
    let (client, connection) = actor_block_on(
        &handle,
        within_deadline(deadline, configuration.connect(NoTls)),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let mut administrator = OwnedConnection::from_connected(&handle, client, connection, bounds)?;
    let mut admin = administrator.take_client()?;
    let mut administrator_backend = None;
    let checked = (|| {
        let identity = actor_block_on(&handle, within_deadline(deadline, admin.query_one(
            "SELECT current_database(),current_user,inet_server_port(),current_setting('fsync'),current_setting('full_page_writes'),current_setting('synchronous_commit'),current_setting('ssl'),current_setting('listen_addresses'),pg_backend_pid(),(SELECT backend_start::text FROM pg_stat_activity WHERE pid=pg_backend_pid())", &[])))?
            .map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        let administrator_pid = identity
            .try_get::<_, i32>(8)
            .map_err(|_| RepositoryError::Unavailable)?;
        let administrator_start = identity
            .try_get::<_, String>(9)
            .map_err(|_| RepositoryError::Unavailable)?;
        if administrator_pid <= 0 || administrator_pid == 80873 || administrator_start.is_empty() {
            return Err(RepositoryError::InvalidReceipt);
        }
        administrator_backend = Some((administrator_pid, administrator_start));
        let expected_fsync = if disabled == "fsync" { "off" } else { "on" };
        let expected_full_pages = if disabled == "full_page_writes" {
            "off"
        } else {
            "on"
        };
        if identity
            .try_get::<_, String>(0)
            .map_err(|_| RepositoryError::Unavailable)?
            != database
            || identity
                .try_get::<_, String>(1)
                .map_err(|_| RepositoryError::Unavailable)?
                != admin_role
            || identity
                .try_get::<_, i32>(2)
                .map_err(|_| RepositoryError::Unavailable)?
                != i32::from(port)
            || identity
                .try_get::<_, String>(3)
                .map_err(|_| RepositoryError::Unavailable)?
                != expected_fsync
            || identity
                .try_get::<_, String>(4)
                .map_err(|_| RepositoryError::Unavailable)?
                != expected_full_pages
            || identity
                .try_get::<_, String>(5)
                .map_err(|_| RepositoryError::Unavailable)?
                != "on"
            || identity
                .try_get::<_, String>(6)
                .map_err(|_| RepositoryError::Unavailable)?
                != "off"
            || identity
                .try_get::<_, String>(7)
                .map_err(|_| RepositoryError::Unavailable)?
                != "127.0.0.1"
        {
            return Err(RepositoryError::Unavailable);
        }
        let settings = actor_block_on(&handle, within_deadline(deadline, admin.query(
            "SELECT name,context,source,pending_restart FROM pg_settings WHERE name IN ('fsync','full_page_writes') ORDER BY name", &[])))?
            .map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
        if settings.len() != 2 {
            return Err(RepositoryError::Unavailable);
        }
        for (row, expected_name) in settings.iter().zip(["fsync", "full_page_writes"]) {
            if row
                .try_get::<_, String>(0)
                .map_err(|_| RepositoryError::Unavailable)?
                != expected_name
                || row
                    .try_get::<_, String>(1)
                    .map_err(|_| RepositoryError::Unavailable)?
                    != "sighup"
                || row
                    .try_get::<_, String>(2)
                    .map_err(|_| RepositoryError::Unavailable)?
                    != "command line"
                || row
                    .try_get::<_, bool>(3)
                    .map_err(|_| RepositoryError::Unavailable)?
            {
                return Err(RepositoryError::Unavailable);
            }
        }
        stage.set("unsafe profile real canonical baseline and current registered authority seed");
        let tenant = [80; 16];
        let principal = [103; 16];
        let campaign = [82; 16];
        let fence = [104; 16];
        let operation = [6; 16];
        let fingerprint = [75; 32];
        let namespace = b"fixture/unsafe-durability/v1";
        let (baseline, candidate, input) = admitted_case(56);
        let rules = vec![rule()];
        let entries = vec![content()];
        let resources = resource_constraints();
        let inventory = || ReferenceInventory {
            rules: &rules,
            content: &entries,
            resources: &resources,
            assets: &[],
        };
        let grant = || FixtureGrant {
            service_role: "df_persistence_fixture_runtime_a",
            tenant: &tenant,
            principal: &principal,
            campaign: &campaign,
            role: b"gm",
            access_revision: b"fixture-access-1",
            lifetime_seconds: 120,
        };
        let proof = actor_block_on(
            &handle,
            within_deadline(deadline, async {
                let transaction = admin
                    .transaction()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                seed_session(
                    &transaction,
                    &tenant,
                    &fence,
                    &baseline,
                    inventory(),
                    (limits(), codec_limits()),
                    120,
                )
                .await?;
                seed_grant(&transaction, &grant()).await?;
                let proof = seed_proof(
                    &transaction,
                    &FixtureProof {
                        grant: grant(),
                        basis: baseline.basis(),
                        operation: &operation,
                        namespace,
                        canonical_fingerprint: &fingerprint,
                        fence: &fence,
                        mode: 3,
                        lookup_only: false,
                    },
                )
                .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| RepositoryError::Unavailable)?;
                Ok::<_, RepositoryError>(proof)
            }),
        )?
        .map_err(|_| RepositoryError::Unavailable)??;
        let context = OperationContext {
            trace_parent: String::new(),
            build: "persistence-unsafe-config-refusal-only".to_owned(),
        };
        let now = fixture_database_now(&handle, &admin, deadline)?;
        let (mut repository, scope, backend_pid) = registered_case_repository(
            &handle,
            (&configuration, port),
            &proof,
            input,
            now,
            bounds,
            &context,
        )?;
        let observed = (|| {
            let before = retained_fixture_all_family_bytes(
                &handle,
                &admin,
                &tenant,
                baseline.basis().session,
                deadline,
            )?;
            for _ in 0..2 {
                stage.set("actual unsafe profile lookup refuses with confirmed rollback and unchanged rows");
                if !matches!(
                    repository.lookup_operation(&scope, &context),
                    Err(RepositoryError::Unavailable)
                ) {
                    return Err(RepositoryError::InvalidReceipt);
                }
                stage
                    .set("actual unsafe profile commit refuses before any durable family mutation");
                if !matches!(
                    repository.commit_decision(&scope, &candidate, baseline.basis(), &context),
                    Err(RepositoryError::Unavailable)
                ) {
                    return Err(RepositoryError::InvalidReceipt);
                }
                stage
                    .set("actual unsafe profile load refuses and same connection remains reusable");
                if !matches!(
                    repository.load_current(&scope, &context),
                    Err(RepositoryError::Unavailable)
                ) {
                    return Err(RepositoryError::InvalidReceipt);
                }
                if retained_fixture_all_family_bytes(
                    &handle,
                    &admin,
                    &tenant,
                    baseline.basis().session,
                    deadline,
                )? != before
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let idle=actor_block_on(&handle,within_deadline(deadline,admin.query_one(
                    "SELECT state='idle' AND xact_start IS NULL FROM pg_stat_activity WHERE pid=$1::integer AND datname=current_database() AND usename='df_persistence_fixture_runtime_a'",&[&backend_pid])))?
                    .map_err(|_|RepositoryError::Unavailable)?.map_err(|_|RepositoryError::Unavailable)?;
                if !idle
                    .try_get::<_, bool>(0)
                    .map_err(|_| RepositoryError::Unavailable)?
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
            }
            Ok(())
        })();
        // Explicit close remains observed even when an assertion fails.
        let closed = repository.close();
        let retry = if closed.is_err() {
            Some(repository.close())
        } else {
            None
        };
        let settled = match retain_fixture_repository_after_retry(repository, closed, retry) {
            Ok(settled) => settled,
            Err(pending) => pending.fail_fixture_while_retaining_owner(),
        };
        drop(settled.repository);
        settled.first_close?;
        if let Some(retry) = settled.retry_close {
            retry?;
        }
        if let Some(final_cleanup) = settled.final_cleanup {
            final_cleanup?;
        }
        observed?;
        actor_block_on(&handle,within_deadline(deadline,async {
            loop {
                let row=admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND datname=current_database())",&[&backend_pid])
                    .await.map_err(|_|RepositoryError::Unavailable)?;
                if !row.try_get::<_,bool>(0).map_err(|_|RepositoryError::Unavailable)? {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,RepositoryError>(())
        }))?.map_err(|_|RepositoryError::Unavailable)??;
        println!(
            "registered physical unsafe durability refusal: {disabled}=off, other durability settings on; real current scope; lookup/commit/load repeated on same connection; unchanged full four-family bytes; confirmed rollback; explicit joined close and exact backend absence; fixture setup makes no unsafe durability claim"
        );
        Ok(())
    })();
    administrator.return_client(admin)?;
    stage.set(
        "unsafe final administrator exact owned OS identity acknowledged before graceful close",
    );
    let identity_observed = administrator_backend
        .as_ref()
        .ok_or(RepositoryError::InvalidReceipt)
        .and_then(|(pid, start)| observe_administrator_identity_before_close(*pid, start));
    stage.set("unsafe final administrator normal Terminate/actual Connection bounded join; backend disappearance still requires ROOT observation");
    let closed =
        actor_block_on(&handle, administrator.fixture_close_gracefully()).and_then(|value| value);
    let retry = if closed.is_err() {
        Some(actor_block_on(&handle, administrator.close()).and_then(|value| value))
    } else {
        None
    };
    let settled = match retain_fixture_connection_after_retry(&handle, administrator, closed, retry)
    {
        Ok(settled) => settled,
        Err(pending) => pending.fail_fixture_while_retaining_owner(),
    };
    drop(settled.connection);
    settled.first_close?;
    if let Some(retry) = settled.retry_close {
        retry?;
    }
    if let Some(final_cleanup) = settled.final_cleanup {
        final_cleanup?;
    }
    identity_observed?;
    checked?;
    let (administrator_pid, administrator_start) =
        administrator_backend.ok_or(RepositoryError::InvalidReceipt)?;
    println!(
        "registered physical unsafe administrator shutdown: pid={administrator_pid}, backend_start={administrator_start}; owned actual Connection gracefully joined; ROOT must observe backend disappearance before fast stop"
    );
    Ok(())
}

// Scheduling is controlled around a REAL connected driver future, never a
// replacement JoinHandle or sleeping surrogate. Two owned runtime workers allow
// one actual driver poll to be held while the other drives the unchanged timer.
fn observe_registered_forced_driver_join(
    administrator_runtime: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    use crate::native_connection::{DiscardState, FixturePollBarrierControl};
    stage.set(
        "forced actual driver JoinPending registered canonical scope and owned two-worker runtime",
    );
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| RepositoryError::Unavailable)?;
    let handle = runtime.handle().clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    let tenant = [80; 16];
    let principal = [105; 16];
    let campaign = [82; 16];
    let fence = [106; 16];
    let operation = [6; 16];
    let fingerprint = [76; 32];
    let namespace = b"fixture/forced-driver-join/v1";
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-real-driver-join-lifetime-only".to_owned(),
    };
    let (baseline, candidate, input) = admitted_case(57);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    let proof = actor_block_on(
        administrator_runtime,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            let grant = || FixtureGrant {
                service_role: "df_persistence_fixture_runtime_a",
                tenant: &tenant,
                principal: &principal,
                campaign: &campaign,
                role: b"gm",
                access_revision: b"fixture-access-1",
                lifetime_seconds: 120,
            };
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            seed_grant(&transaction, &grant()).await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &operation,
                    namespace,
                    canonical_fingerprint: &fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let control = FixturePollBarrierControl::new(Duration::from_secs(12))?;
    let barrier = control.barrier.clone();
    let direct = fixture_lost_ack_configuration(configuration, port)?;
    let (client, connection) =
        actor_block_on(&handle, within_deadline(deadline, direct.connect(NoTls)))?
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)?;
    let owned = OwnedConnection::from_connected_with_poll_barrier(
        &handle,
        client,
        connection,
        bounds,
        barrier.clone(),
    )?;
    let mut authority = FixtureMembershipAuthority {
        runtime: handle.clone(),
        connection: owned,
        query_bound: bounds.transaction,
        context: context.clone(),
    };
    let now = fixture_database_now(administrator_runtime, admin, deadline)?;
    // Both scopes are independently issued by the existing actual membership
    // producer on this exact connection, with the same complete retained key.
    let (scope, verifier) = bind_registered_scope(
        &mut authority,
        &proof,
        FixtureBoundInput {
            input: input.clone(),
            pins: pins(),
        },
        now,
    )?;
    let (reuse_scope, _) = bind_registered_scope(
        &mut authority,
        &proof,
        FixtureBoundInput {
            input: input.clone(),
            pins: pins(),
        },
        now,
    )?;
    let client = authority.connection.take_client()?;
    let pid_row = actor_block_on(
        &handle,
        within_deadline(deadline, client.query_one("SELECT pg_backend_pid()", &[])),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let backend_pid: i32 = pid_row
        .try_get(0)
        .map_err(|_| RepositoryError::Unavailable)?;
    authority.connection.return_client(client)?;
    if backend_pid <= 0 {
        return Err(RepositoryError::Unavailable);
    }
    let recovery = RecoverySource {
        rules: vec![rule()],
        content: vec![content()],
        resource_resolver: None,
        asset_resolver: None,
        resources: resource_constraints(),
        assets: vec![],
        limits: limits(),
    };
    let mut repository = compose_registered_repository(
        authority,
        codec_limits(),
        16384,
        verifier,
        recovery,
        bounds,
    )?;
    if !matches!(
        repository.lookup_operation(&reuse_scope, &context)?,
        OperationLookup::NotRecorded
    ) || repository.load_current(&reuse_scope, &context)? != baseline
    {
        let closed = repository.close();
        closed?;
        return Err(RepositoryError::InvalidReceipt);
    }
    let (sender, actor_loop) = df_session::inbox::bounded_inbox::<
        df_session::submission::OwnedInput<
            crate::native_scope::NativeScope<FixtureMembershipAuthority>,
        >,
    >();
    let (item, receipt) =
        df_session::submission::OwnedInput::new(context.clone(), scope, input.clone());
    let admitted = sender
        .try_submit(item)
        .map_err(|_| RepositoryError::Capacity)?;
    if admitted.0 != 1
        || !matches!(
            receipt.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        )
    {
        let closed = repository.close();
        closed?;
        return Err(RepositoryError::InvalidReceipt);
    }
    sender.stop().map_err(|_| RepositoryError::Unavailable)?;
    let counters = std::sync::Arc::new(std::sync::Mutex::new([0_u64; 3]));
    let (pending_tx, pending_rx) = std::sync::mpsc::sync_channel(1);
    let (retry_tx, retry_rx) = std::sync::mpsc::channel();
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    stage.set("actual driver poll held; first bounded close timeout communicated while repository remains on same actor");
    let observed = std::thread::scope(|threads| {
        let publication = LifetimeFixturePublication {
            runtime: administrator_runtime,
            administrator: &*admin,
            candidate: candidate.clone(),
            deadline,
            observed: counters.clone(),
        };
        let engine = LifetimeFixtureEngine {
            baseline: baseline.clone(),
            candidate: candidate.clone(),
            input,
            observed: counters.clone(),
        };
        let initial = baseline.clone();
        let retry_candidate = candidate.clone();
        let expected = baseline.basis();
        let actor_barrier = barrier.clone();
        let actor_finished = finished.clone();
        let entered = control.entered;
        let actor = threads.spawn(move || {
            let actor_id = std::thread::current().id();
            let mut owner = df_session::submission::DurableOwner::new(
                repository,
                engine,
                publication,
                initial,
                16384,
            )?;
            let drained = actor_loop
                .run(&mut owner)
                .map_err(|_| RepositoryError::Unavailable);
            let checkpoint = owner.checkpoint().clone();
            let mut returned = owner.into_repository();
            let armed = actor_barrier.arm();
            let driver_id = if armed.is_ok() {
                entered.recv_timeout(Duration::from_secs(1)).ok()
            } else {
                None
            };
            // Never begin the forced close until the actual poll is observed held.
            let first = if driver_id.is_some() && actor_barrier.waiting() {
                returned.close()
            } else {
                Err(RepositoryError::Unavailable)
            };
            let pending = returned.fixture_discard_state();
            let reuse_refused = matches!(
                returned.lookup_operation(&reuse_scope, &context),
                Err(RepositoryError::Unavailable)
            ) && matches!(
                returned.commit_decision(&reuse_scope, &retry_candidate, expected, &context),
                Err(RepositoryError::Unavailable)
            ) && matches!(
                returned.load_current(&reuse_scope, &context),
                Err(RepositoryError::Unavailable)
            );
            // Keep the same actual worker held through a SECOND bounded close.
            // The actor owns the repository across both failed joins.
            let second = returned.close();
            let second_pending = returned.fixture_discard_state();
            let report_sent = pending_tx
                .send((
                    first,
                    pending,
                    actor_id,
                    driver_id,
                    reuse_refused,
                    second,
                    second_pending,
                ))
                .is_ok();
            // The repository and its aborted actual driver remain owned HERE;
            // the parent observes firstErr/state before it releases that driver.
            let retry_released = retry_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            let retry = returned.close();
            // Keep a second failure explicit and retain its owner through one
            // final bounded actor-side cleanup; never turn retryErr into PASS.
            let cleanup = if retry.is_err() {
                Some(returned.close())
            } else {
                None
            };
            let final_state = returned.fixture_discard_state();
            let close_id = std::thread::current().id();
            actor_finished.store(true, std::sync::atomic::Ordering::Release);
            Ok::<_, RepositoryError>((
                drained,
                checkpoint,
                first,
                pending,
                second,
                second_pending,
                retry,
                cleanup,
                final_state,
                returned,
                actor_id,
                close_id,
                driver_id,
                reuse_refused,
                report_sent,
                retry_released,
            ))
        });
        let pending = pending_rx.recv_timeout(Duration::from_secs(5));
        let actor_was_waiting =
            !actor.is_finished() && !finished.load(std::sync::atomic::Ordering::Acquire);
        let driver_was_waiting = barrier.waiting();
        // Every parent observation path releases the driver and actor controls
        // BEFORE joining. Timeout/disconnect is retained as failure, not success.
        let driver_released = control.release.send(()).is_ok();
        let retry_released = retry_tx.send(()).is_ok();
        let joined = actor.join();
        (
            pending,
            actor_was_waiting,
            driver_was_waiting,
            driver_released,
            retry_released,
            joined,
        )
    });
    let (pending, actor_was_waiting, driver_was_waiting, driver_released, retry_released, joined) =
        observed;
    let result = joined.map_err(|_| RepositoryError::Unavailable)?;
    let (
        drained,
        checkpoint,
        first,
        pending_state,
        second,
        second_pending,
        retry,
        cleanup,
        final_state,
        mut returned,
        actor_id,
        close_id,
        driver_id,
        reuse_refused,
        report_sent,
        actor_retry_released,
    ) = result?;
    // If this controlled retry unexpectedly failed, retain the returned owner
    // through an explicit bounded emergency cleanup. This path is NOT a joined
    // actor-shutdown proof and the original retry failure still fails the suite.
    let emergency = if matches!(final_state, DiscardState::JoinPending) {
        Some(returned.close())
    } else {
        None
    };
    let checks = (|| {
        let report = pending.map_err(|_| RepositoryError::Unavailable)?;
        if first != Err(RepositoryError::Unavailable)
            || report.0 != first
            || !matches!(pending_state, DiscardState::JoinPending)
            || !matches!(report.1, DiscardState::JoinPending)
            || report.2 != actor_id
            || report.3 != driver_id
            || !report.4
            || second != Err(RepositoryError::Unavailable)
            || report.5 != second
            || !matches!(second_pending, DiscardState::JoinPending)
            || !matches!(report.6, DiscardState::JoinPending)
            || !reuse_refused
            || !actor_was_waiting
            || !driver_was_waiting
            || !driver_released
            || !retry_released
            || !actor_retry_released
            || !report_sent
            || barrier.failed()
            || actor_id == std::thread::current().id()
            || actor_id != close_id
            || driver_id.is_none_or(|id| id == actor_id || id == std::thread::current().id())
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        retry?;
        if cleanup.is_some()
            || emergency.is_some()
            || !matches!(
                final_state,
                DiscardState::Joined
                    | DiscardState::JoinedWithDriverError
                    | DiscardState::JoinedCancelled
            )
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let drained = drained?;
        if drained.reduced_inputs != 1
            || drained.last_sequence != Some(admitted)
            || checkpoint != candidate
            || *counters.lock().map_err(|_| RepositoryError::Unavailable)? != [1, 1, 1]
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let outcome = receipt
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        if !matches!(outcome,df_session::submission::SubmissionOutcome::Confirmed(ref value)
            if value.basis()==candidate.basis() && value.decision()==&candidate.state().decisions[0])
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let snapshot = actor_block_on(
            administrator_runtime,
            physical_snapshot(
                admin,
                &tenant,
                candidate.basis(),
                codec_limits().maximum_document_bytes,
                deadline,
            ),
        )??;
        validate_physical_commit(&snapshot, &candidate, inventory(), limits(), codec_limits())?;
        actor_block_on(administrator_runtime,within_deadline(deadline,async {
            loop {
                let row=admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND datname=current_database())",&[&backend_pid])
                    .await.map_err(|_|RepositoryError::Unavailable)?;
                if !row.try_get::<_,bool>(0).map_err(|_|RepositoryError::Unavailable)? {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,RepositoryError>(())
        }))?.map_err(|_|RepositoryError::Unavailable)??;
        Ok(())
    })();
    // No error propagation may implicitly drop an unjoined returned owner.
    // Existing emergency cleanup remains failure evidence, never normal order.
    let settled = match retain_fixture_repository_after_retry(returned, retry, emergency) {
        Ok(settled) => settled,
        Err(pending) => pending.fail_fixture_while_retaining_owner(),
    };
    drop(settled.repository);
    checks?;
    settled.first_close?;
    if let Some(retry) = settled.retry_close {
        retry?;
    }
    if let Some(final_cleanup) = settled.final_cleanup {
        final_cleanup?;
    }
    if let Some(cleanup) = cleanup {
        cleanup?;
    }
    println!(
        "registered physical actual driver JoinPending: actual connected Connection poll held on real worker; other worker drives unchanged bounded close timer; first and second bounded Unavailable/JoinPending communicated with actor still owning repository; all3 poisoned operations refused; parent releases real poll only after second failure; same actor third close joins before parent join/runtime teardown; confirmed receipt/full four-family decision remains unchanged; exact backend absent; blocked preCOMMIT observed in physical11; actor Unknown remains unqualified"
    );
    Ok(())
}

// Both canonical host inputs are accepted by this controlled engine. A second
// reduction would therefore be visible; an engine-specific refusal cannot stand
// in for the repository/actor uncertainty fence. No real rules mechanics are claimed.
struct ActorUnknownFixtureEngine {
    baseline: Checkpoint,
    candidates: [(Checkpoint, GameInput); 2],
    observed: std::sync::Arc<std::sync::Mutex<[u64; 3]>>,
}
impl
    df_session::submission::SessionEngine<
        crate::native_scope::NativeScope<FixtureMembershipAuthority>,
    > for ActorUnknownFixtureEngine
{
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &crate::native_scope::NativeScope<FixtureMembershipAuthority>,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        scope.validate_input(input)?;
        if *current != self.baseline {
            return Err(RepositoryError::InvalidCandidate);
        }
        let candidate = self
            .candidates
            .iter()
            .find(|(_, admitted)| admitted == input)
            .map(|(candidate, _)| candidate)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        observed[0] = observed[0]
            .checked_add(1)
            .ok_or(RepositoryError::Capacity)?;
        Ok(candidate.clone())
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        if *checkpoint == self.baseline
            || self
                .candidates
                .iter()
                .any(|(candidate, _)| candidate == checkpoint)
        {
            Ok(())
        } else {
            Err(RepositoryError::InvalidCandidate)
        }
    }
}

/// One controlled SessionEngine reduction commits through the existing registered
/// proxy. Its lost acknowledgement leaves this SAME actor and repository unresolved;
/// a separate observer proves the commit without reconnecting or replacing that owner.
fn observe_registered_actor_lost_commit_ack(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    postgres_port: u16,
    proxy_port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [80; 16];
    let principal = [102; 16];
    let campaign = [82; 16];
    let fence = [112; 16];
    let operation = [6; 16];
    let later_operation = [7; 16];
    let fingerprint = [70; 32];
    let later_fingerprint = [71; 32];
    let namespace = b"fixture/actor-unknown/v1";
    let deadline = Instant::now() + Duration::from_secs(30);
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-actor-unknown-controlled-ports".to_owned(),
    };
    let (baseline, candidate, input) = admitted_case(59);
    let (_, later_candidate, later_input) =
        admitted_case_for_operation(59, baseline.basis().revision, 7);
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &entries,
        resources: &resources,
        assets: &[],
    };
    stage.set("actor Unknown canonical session and two current registered operation proofs");
    let (proof, later_proof) = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                inventory(),
                (limits(), codec_limits()),
                120,
            )
            .await?;
            let grant = || FixtureGrant {
                service_role: "df_persistence_fixture_runtime_a",
                tenant: &tenant,
                principal: &principal,
                campaign: &campaign,
                role: b"gm",
                access_revision: b"fixture-access-1",
                lifetime_seconds: 120,
            };
            seed_grant(&transaction, &grant()).await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &operation,
                    namespace,
                    canonical_fingerprint: &fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            let later_proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &later_operation,
                    namespace,
                    canonical_fingerprint: &later_fingerprint,
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>((proof, later_proof))
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;
    let now = fixture_database_now(handle, admin, deadline)?;
    let listener = actor_block_on(
        handle,
        within_deadline(deadline, TcpListener::bind(("127.0.0.1", proxy_port))),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    let mut proxy = OwnedAckProxy {
        task: Some(handle.spawn(async move {
            let (frontend, peer) = timeout_at(deadline, listener.accept())
                .await
                .map_err(|_| ProxyError::Deadline)?
                .map_err(|_| ProxyError::Io)?;
            if !peer.ip().is_loopback() {
                return Err(ProxyError::Protocol);
            }
            drop(listener);
            let postgres = timeout_at(deadline, TcpStream::connect(("127.0.0.1", postgres_port)))
                .await
                .map_err(|_| ProxyError::Deadline)?
                .map_err(|_| ProxyError::Io)?;
            drop_commit_acknowledgement(
                frontend,
                postgres,
                ProxyBounds {
                    maximum_frame_bytes: 4 * 1024 * 1024,
                    maximum_suppressed_response_bytes: 4096,
                    deadline,
                },
            )
            .await
        })),
    };
    let observed_case = (|| -> Result<(), RepositoryError> {
        stage.set("actor Unknown actual proxied authority and three independently produced scopes");
        let proxied = fixture_lost_ack_configuration(configuration, proxy_port)?;
        crate::owned_pg_composition_fixture::validate_fixture_composition(
            codec_limits(),
            16384,
            bounds,
        )?;
        let mut authority = connect_fixture_authority(handle, &proxied, bounds, &context)?;
        let produced = (|| {
            let (scope, verifier) = bind_registered_scope(
                &mut authority,
                &proof,
                FixtureBoundInput {
                    input: input.clone(),
                    pins: pins(),
                },
                now,
            )?;
            let (retry_scope, _) = bind_registered_scope(
                &mut authority,
                &proof,
                FixtureBoundInput {
                    input: input.clone(),
                    pins: pins(),
                },
                now,
            )?;
            let (later_scope, _) = bind_registered_scope(
                &mut authority,
                &later_proof,
                FixtureBoundInput {
                    input: later_input.clone(),
                    pins: pins(),
                },
                now,
            )?;
            if scope.capture_uncertainty_key(16384)?
                != retry_scope.capture_uncertainty_key(16384)?
                || scope.capture_uncertainty_key(16384)?
                    == later_scope.capture_uncertainty_key(16384)?
            {
                return Err(RepositoryError::InputBinding);
            }
            let client = authority.connection.take_client()?;
            let identity = actor_block_on(handle, within_deadline(deadline, client.query_one(
                "SELECT pg_backend_pid(),backend_start::text FROM pg_stat_activity WHERE pid=pg_backend_pid()",
                &[]))).and_then(|v| v.map_err(|_| RepositoryError::Unavailable))
                .and_then(|v| v.map_err(|_| RepositoryError::Unavailable));
            authority.connection.return_client(client)?;
            let row = identity?;
            let pid = row
                .try_get::<_, i32>(0)
                .map_err(|_| RepositoryError::Unavailable)?;
            let start = row
                .try_get::<_, String>(1)
                .map_err(|_| RepositoryError::Unavailable)?;
            if pid <= 1 || pid == 80873 || start.is_empty() {
                return Err(RepositoryError::Unavailable);
            }
            // The live SQL connection pins this PID while its OS parent/start are observed.
            let os_identity = std::process::Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "pid=,ppid=,lstart="])
                .output()
                .map_err(|_| RepositoryError::Unavailable)?;
            let os_identity = if os_identity.status.success() && os_identity.stdout.len() <= 2048 {
                String::from_utf8(os_identity.stdout).map_err(|_| RepositoryError::Unavailable)?
            } else {
                return Err(RepositoryError::Unavailable);
            };
            let fields: Vec<&str> = os_identity.split_whitespace().collect();
            if fields.len() != 7
                || fields[0].parse::<i32>().ok() != Some(pid)
                || fields[1].parse::<i32>().is_err()
            {
                return Err(RepositoryError::Unavailable);
            }
            Ok::<_, RepositoryError>((
                scope,
                retry_scope,
                later_scope,
                verifier,
                pid,
                start,
                os_identity,
            ))
        })();
        let (scope, retry_scope, later_scope, verifier, backend_pid, backend_start, os_identity) =
            match produced {
                Ok(value) => value,
                Err(error) => {
                    let first =
                        actor_block_on(handle, authority.connection.close()).and_then(|v| v);
                    let retry = if first.is_err() {
                        Some(actor_block_on(handle, authority.connection.close()).and_then(|v| v))
                    } else {
                        None
                    };
                    let settled = match retain_fixture_connection_after_retry(
                        handle,
                        authority.into_owned_connection(),
                        first,
                        retry,
                    ) {
                        Ok(value) => value,
                        Err(pending) => pending.fail_fixture_while_retaining_owner(),
                    };
                    settled.first_close?;
                    if let Some(value) = settled.retry_close {
                        value?;
                    }
                    if let Some(value) = settled.final_cleanup {
                        value?;
                    }
                    return Err(error);
                }
            };
        let mut repository = compose_registered_repository(
            authority,
            codec_limits(),
            16384,
            verifier,
            RecoverySource {
                rules: rules.clone(),
                content: entries.clone(),
                resource_resolver: None,
                asset_resolver: None,
                resources: resources.clone(),
                assets: vec![],
                limits: limits(),
            },
            bounds,
        )?;
        let (sender, actor_loop) = df_session::inbox::bounded_inbox::<
            df_session::submission::OwnedInput<
                crate::native_scope::NativeScope<FixtureMembershipAuthority>,
            >,
        >();
        let (item, receipt) =
            df_session::submission::OwnedInput::new(context.clone(), scope, input.clone());
        let first_admission = match sender.try_submit(item) {
            Ok(value) => value,
            Err(_) => {
                let stopped = sender.stop().map_err(|_| RepositoryError::Unavailable);
                let first = repository.close();
                let retry = if first.is_err() {
                    Some(repository.close())
                } else {
                    None
                };
                let settled = match retain_fixture_repository_after_retry(repository, first, retry)
                {
                    Ok(value) => value,
                    Err(pending) => pending.fail_fixture_while_retaining_owner(),
                };
                settled.first_close?;
                if let Some(value) = settled.retry_close {
                    value?;
                }
                if let Some(value) = settled.final_cleanup {
                    value?;
                }
                stopped?;
                return Err(RepositoryError::Capacity);
            }
        };
        let admission_has_no_response = matches!(
            receipt.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        let observed = std::sync::Arc::new(std::sync::Mutex::new([0_u64; 3]));
        stage.set(
            "actor Unknown actual admitted engine decision commits with suppressed acknowledgement",
        );
        let read_admin = &*admin;
        let (parent_observation, stopped, joined) = std::thread::scope(|threads| {
            let engine = ActorUnknownFixtureEngine {
                baseline: baseline.clone(),
                candidates: [
                    (candidate.clone(), input.clone()),
                    (later_candidate.clone(), later_input.clone()),
                ],
                observed: observed.clone(),
            };
            let publication = LifetimeFixturePublication {
                runtime: handle,
                administrator: read_admin,
                candidate: candidate.clone(),
                deadline,
                observed: observed.clone(),
            };
            let initial = baseline.clone();
            let actor = threads.spawn(move || {
                let actor_id = std::thread::current().id();
                if tokio::runtime::Handle::try_current().is_ok() {
                    return Err(RepositoryError::Unavailable);
                }
                let mut owner = df_session::submission::DurableOwner::new(
                    repository,
                    engine,
                    publication,
                    initial,
                    16384,
                )?;
                let drained = actor_loop
                    .run(&mut owner)
                    .map_err(|_| RepositoryError::Unavailable);
                let checkpoint = owner.checkpoint().clone();
                let mut returned = owner.into_repository();
                let first_close = returned.close();
                let retry_close = if first_close.is_err() {
                    Some(returned.close())
                } else {
                    None
                };
                let cleanup =
                    retain_fixture_repository_after_retry(returned, first_close, retry_close);
                Ok::<_, RepositoryError>((
                    drained,
                    checkpoint,
                    cleanup,
                    actor_id,
                    std::thread::current().id(),
                ))
            });
            // No early propagation here: stop the inbox and explicitly join this actor
            // even if a response, proxy observation or independent read fails.
            let parent_observation = (|| {
                if first_admission.0 != 1 || !admission_has_no_response {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let wait = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or(RepositoryError::Unavailable)?;
                let first = receipt
                    .recv_timeout(wait)
                    .map_err(|_| RepositoryError::InvalidReceipt)?;
                if first != df_session::submission::SubmissionOutcome::LookupRequired
                    || *observed.lock().map_err(|_| RepositoryError::Unavailable)? != [1, 0, 0]
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
                stage.set("actor Unknown actual PostgreSQL COMMIT C and idle Z acknowledgement suppressed");
                let ack =
                    actor_block_on(handle, proxy.observe(deadline))?.map_err(proxy_failure)?;
                if !ack.frontend_commit_forwarded
                    || !ack.postgres_commit_complete_observed
                    || !ack.postgres_ready_idle_observed
                    || ack.suppressed_response_bytes == 0
                    || ack.suppressed_response_bytes > 4096
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let snapshot = actor_block_on(
                    handle,
                    physical_snapshot(
                        read_admin,
                        &tenant,
                        candidate.basis(),
                        codec_limits().maximum_document_bytes,
                        deadline,
                    ),
                )??;
                validate_physical_commit(
                    &snapshot,
                    &candidate,
                    inventory(),
                    limits(),
                    codec_limits(),
                )?;
                // The committed revision has one checkpoint; the complete session
                // additionally retains its independently seeded baseline checkpoint.
                if snapshot.family_counts != [1, 1, 1, 1] {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let totals = actor_block_on(handle, within_deadline(deadline, read_admin.query_one(
                    "SELECT
                        (SELECT count(*) FROM df_game.checkpoints WHERE tenant_id=$1::bytea AND session_id=$2::bytea),
                        (SELECT count(*) FROM df_game.facts WHERE tenant_id=$1::bytea AND session_id=$2::bytea),
                        (SELECT count(*) FROM df_game.operations WHERE tenant_id=$1::bytea AND session_id=$2::bytea),
                        (SELECT count(*) FROM df_game.intents WHERE tenant_id=$1::bytea AND session_id=$2::bytea)",
                    &[&tenant.as_slice(), &baseline.basis().session.as_bytes().as_slice()])))?
                    .map_err(|_| RepositoryError::Unavailable)?
                    .map_err(|_| RepositoryError::Unavailable)?;
                let mut family_totals = [0_i64; 4];
                for (column, count) in family_totals.iter_mut().enumerate() {
                    *count = totals
                        .try_get(column)
                        .map_err(|_| RepositoryError::Unavailable)?;
                }
                if family_totals != [2, 1, 1, 1] {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let stored = decode_receipt(
                    snapshot
                        .retained_receipt
                        .as_ref()
                        .ok_or(RepositoryError::InvalidReceipt)?,
                    baseline.basis().session,
                    OperationId::from_bytes(&operation)
                        .map_err(|_| RepositoryError::InputBinding)?,
                    16384,
                    codec_limits(),
                )
                .map_err(|_| RepositoryError::InvalidReceipt)?;
                if stored.basis() != candidate.basis()
                    || stored.decision() != &candidate.state().decisions[0]
                {
                    return Err(RepositoryError::InvalidReceipt);
                }
                let committed = retained_fixture_all_family_bytes(
                    handle,
                    read_admin,
                    &tenant,
                    baseline.basis().session,
                    deadline,
                )?;
                stage.set("actor Unknown same input and different valid input cannot bypass poisoned repository");
                let (same_item, same_receipt) = df_session::submission::OwnedInput::new(
                    context.clone(),
                    retry_scope,
                    input.clone(),
                );
                let second = sender
                    .try_submit(same_item)
                    .map_err(|_| RepositoryError::Capacity)?;
                let (different_item, different_receipt) = df_session::submission::OwnedInput::new(
                    context.clone(),
                    later_scope,
                    later_input,
                );
                let third = sender
                    .try_submit(different_item)
                    .map_err(|_| RepositoryError::Capacity)?;
                if second.0 != 2 || third.0 != 3 {
                    return Err(RepositoryError::InvalidReceipt);
                }
                for (receipt, expected) in [
                    (
                        same_receipt,
                        df_session::submission::SubmissionOutcome::Refused(
                            RepositoryError::Unavailable,
                        ),
                    ),
                    (
                        different_receipt,
                        df_session::submission::SubmissionOutcome::Refused(
                            RepositoryError::Unavailable,
                        ),
                    ),
                ] {
                    let wait = deadline
                        .checked_duration_since(Instant::now())
                        .ok_or(RepositoryError::Unavailable)?;
                    if receipt
                        .recv_timeout(wait)
                        .map_err(|_| RepositoryError::InvalidReceipt)?
                        != expected
                        || *observed.lock().map_err(|_| RepositoryError::Unavailable)? != [1, 0, 0]
                        || retained_fixture_all_family_bytes(
                            handle,
                            read_admin,
                            &tenant,
                            baseline.basis().session,
                            deadline,
                        )? != committed
                    {
                        return Err(RepositoryError::InvalidReceipt);
                    }
                }
                Ok::<_, RepositoryError>((committed, third))
            })();
            let stopped = sender.stop().map_err(|_| RepositoryError::Unavailable);
            // A failed proxy observation still owns the task; close it before joining
            // the actor so its sockets cannot strand the accepted bounded transaction.
            let proxy_close = actor_block_on(handle, proxy.close(bounds.driver_join))
                .and_then(|v| v.map_err(proxy_failure));
            let proxy_retry = if proxy_close.is_err() {
                Some(
                    actor_block_on(handle, proxy.close(bounds.driver_join))
                        .and_then(|v| v.map_err(proxy_failure)),
                )
            } else {
                None
            };
            if proxy.task.is_some() {
                eprintln!(
                    "actor Unknown proxy cleanup exhausted with owner retained; ROOT process cleanup required; no joined proof"
                );
                std::process::exit(1);
            }
            let parent_observation = parent_observation.and_then(|v| {
                proxy_close?;
                if let Some(retry) = proxy_retry {
                    retry?;
                }
                Ok(v)
            });
            let joined = actor
                .join()
                .map_err(|_| RepositoryError::Unavailable)
                .and_then(|v| v);
            (parent_observation, stopped, joined)
        });
        let (drained, checkpoint, cleanup, actor_id, close_id) = joined?;
        let settled = match cleanup {
            Ok(value) => value,
            Err(pending) => pending.fail_fixture_while_retaining_owner(),
        };
        settled.first_close?;
        if let Some(value) = settled.retry_close {
            value?;
        }
        if let Some(value) = settled.final_cleanup {
            value?;
        }
        stopped?;
        let (committed, last_admission) = parent_observation?;
        if actor_id == std::thread::current().id()
            || actor_id != close_id
            || checkpoint != baseline
            || *observed.lock().map_err(|_| RepositoryError::Unavailable)? != [1, 0, 0]
            || sender
                .usage()
                .map_err(|_| RepositoryError::Unavailable)?
                .accepting
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        let drained = drained?;
        if drained.reduced_inputs != 3
            || drained.last_sequence != Some(last_admission)
            || retained_fixture_all_family_bytes(
                handle,
                admin,
                &tenant,
                baseline.basis().session,
                deadline,
            )? != committed
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        stage.set("actor Unknown same actor native driver joined before parent join and exact backend absent");
        actor_block_on(handle, within_deadline(deadline, async {
            loop {
                admin.query_one("SELECT pg_stat_clear_snapshot()", &[]).await.map_err(|_| RepositoryError::Unavailable)?;
                let row = admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND backend_start::text=$2::text AND datname=current_database())",
                    &[&backend_pid,&backend_start]).await.map_err(|_| RepositoryError::Unavailable)?;
                if !row.try_get::<_,bool>(0).map_err(|_| RepositoryError::Unavailable)? { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,RepositoryError>(())
        }))?.map_err(|_| RepositoryError::Unavailable)??;
        let after_os = std::process::Command::new("/bin/ps")
            .args(["-p", &backend_pid.to_string(), "-o", "pid=,ppid=,lstart="])
            .output()
            .map_err(|_| RepositoryError::Unavailable)?;
        if after_os.status.success() {
            let signature =
                std::str::from_utf8(&after_os.stdout).map_err(|_| RepositoryError::Unavailable)?;
            let fields: Vec<&str> = signature.split_whitespace().collect();
            if signature == os_identity
                || fields.len() != 7
                || fields[0].parse::<i32>().ok() != Some(backend_pid)
                || fields[1].parse::<i32>().is_err()
                || !after_os.stderr.is_empty()
            {
                return Err(RepositoryError::Unavailable);
            }
        } else if after_os.status.code() != Some(1)
            || !after_os.stdout.is_empty()
            || !after_os.stderr.is_empty()
        {
            // Exact ps PID absence is exit1+empty. Other failures prove nothing.
            return Err(RepositoryError::Unavailable);
        }
        println!(
            "registered physical actual actor lost COMMIT acknowledgement: session59; one engine decision; LookupRequired then same/different Refused(Unavailable) on poisoned repository; no publication/wake; four-family counts2/1/1/1 and complete codec{}/schema{} appearance equal; inbox stop/drain -> same actor native joined close -> parent join; backend_pid={backend_pid} backend_start={backend_start} OSidentity={}; exact actor recovery remains unqualified",
            crate::checkpoint_codec::STORAGE_CODEC_VERSION,
            CHECKPOINT_SCHEMA,
            os_identity.trim()
        );
        Ok(())
    })();
    let closed = actor_block_on(handle, proxy.close(bounds.driver_join))
        .and_then(|v| v.map_err(proxy_failure));
    let retry = if closed.is_err() {
        Some(
            actor_block_on(handle, proxy.close(bounds.driver_join))
                .and_then(|v| v.map_err(proxy_failure)),
        )
    } else {
        None
    };
    if proxy.task.is_some() {
        eprintln!(
            "actor Unknown setup proxy cleanup exhausted with owner retained; ROOT cleanup required; no joined proof"
        );
        std::process::exit(1);
    }
    observed_case?;
    closed?;
    if let Some(value) = retry {
        value?;
    }
    Ok(())
}
