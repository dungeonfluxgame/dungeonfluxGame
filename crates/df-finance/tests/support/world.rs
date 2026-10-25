//! A scratch database with a published price version and an open session, and helpers to build
//! services, commands and balance queries. Every service connects as `df_finance`, the role that
//! holds no write privilege, so every write in a test is a definer function.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the world"
)]

use std::sync::atomic::{AtomicU64, Ordering};

use df_commerce::admission::{Route, WorkClass};
use df_commerce::digest::business_key;
use df_commerce::finance_ids::{
    AllocationId, AttemptId, CampaignKey, EffectId, LotId, OperationKey, PayerId, PriceVersionId,
    SessionKey, TenantKey,
};
use df_commerce::funding_flows::{Consent, SessionPolicy};
use df_commerce::ledger::CurrencyTag;
use df_commerce::price_schedule::{PriceVersion, TariffLine};
use df_commerce::settlement::CreditReason;
use df_finance::admit::{AdmitCommand, priced_route};
use df_finance::fakes::{FakeFence, FakeMedia, FakeMembers, SeededIds, SystemClock};
use df_finance::held::CreditCommand;
use df_finance::ports::{FenceRequest, FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceService, PurchaseCommand, SessionOpening};
use tokio_postgres::Client;

use super::Scratch;

pub type Service = FinanceService<FakeFence, FakeMedia, FakeMembers>;

pub const CURRENCY: CurrencyTag = CurrencyTag::USD;

pub fn payer(tag: u8) -> PayerId {
    PayerId::from_array([0xa0 + tag; 16])
}

/// A confirmed payment from the fake processor.
pub fn purchase(
    payer: PayerId,
    payment_ref: &str,
    amount_micro_dollars: u64,
) -> PurchaseCommand<'_> {
    PurchaseCommand {
        payer,
        processor: "fake",
        payment_ref,
        currency: CURRENCY,
        amount_micro_dollars,
    }
}

/// A technical-failure credit by operator 1.
pub fn credit_command(
    usage_event: df_commerce::finance_ids::UsageEventId,
    operation: OperationKey,
    amount_micro_dollars: u64,
    case_ref: &str,
) -> CreditCommand {
    CreditCommand {
        usage_event,
        operation,
        amount_micro_dollars,
        reason: CreditReason::TechnicalFailure,
        actor_id: [1; 16],
        case_ref: case_ref.to_owned(),
        approver_id: None,
    }
}

pub fn operation(tag: u8) -> OperationKey {
    OperationKey::from_array([0x40 + tag; 16])
}

pub struct World {
    pub scratch: Scratch,
    pub admin: Client,
    pub version: PriceVersion,
    pub session: SessionKey,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub payers: Vec<PayerId>,
    next_seed: AtomicU64,
}

pub fn price_version() -> PriceVersion {
    let line = |category: &str, nano_usd: u64| TariffLine {
        provider: "openai".into(),
        model: "gpt-6-luna".into(),
        category: category.into(),
        context_tier_max_tokens: 0,
        nano_usd,
        per: 1_000_000,
    };
    PriceVersion {
        id: PriceVersionId::from_array([0x70; 16]),
        label: "pv-test1".into(),
        currency: CURRENCY,
        platform_rate_micro_dollars_per_hour: 2_000_000,
        markup_numerator: 3,
        markup_denominator: 2,
        // 0.10 and 0.50 micro-dollars per token, at the real rates of content/pricing.json.
        tariff: vec![
            line("text_input", 100_000_000),
            line("text_output", 500_000_000),
        ],
        proposed_pilot: true,
    }
}

pub fn route() -> Route {
    priced_route(
        "openai",
        "gpt-6-luna",
        "narration",
        &["text_input", "text_output"],
    )
}

impl World {
    /// A migrated database with a published version and an open session, or `None` (SKIPPED).
    pub async fn open() -> Option<Self> {
        let scratch = Scratch::open().await?;
        let admin = scratch.connect().await;
        let mut world = Self {
            scratch,
            admin,
            version: price_version(),
            session: SessionKey::from_array([0x51; 16]),
            tenant: TenantKey::from_array([0x52; 16]),
            campaign: CampaignKey::from_array([0x53; 16]),
            payers: (1..=6).map(payer).collect(),
            next_seed: AtomicU64::new(1000),
        };
        let mut service = world.service().await;
        service
            .publish_price_version(&world.version)
            .await
            .expect("publish the price version");
        service
            .open_session(SessionOpening {
                session: world.session,
                tenant: world.tenant,
                campaign: world.campaign,
                price_version: world.version.id,
                currency: CURRENCY,
                policy: SessionPolicy {
                    hourly_ceiling_micro_dollars: 20_000_000,
                    session_ceiling_micro_dollars: None,
                    per_request_threshold_micro_dollars: 5_000_000,
                },
            })
            .await
            .expect("open the session");
        drop(service);
        world.payers = (1..=6).map(payer).collect();
        Some(world)
    }

    /// A service on a new connection, acting as `df_finance`.
    pub async fn service(&self) -> Service {
        self.service_with(FenceVerdict::Current, MediaVerdict::Admitted)
            .await
    }

    pub async fn service_with(&self, fence: FenceVerdict, media: MediaVerdict) -> Service {
        self.build(fence, media, self.payers.clone(), FinanceConfig::default())
            .await
    }

    /// A service with its own tunables, for the tests of maintenance and deadlines.
    pub async fn service_with_config(&self, config: FinanceConfig) -> Service {
        self.build(
            FenceVerdict::Current,
            MediaVerdict::Admitted,
            self.payers.clone(),
            config,
        )
        .await
    }

    /// Opens another session on the same price version, as a table with its own pot.
    pub async fn open_session(
        &self,
        service: &mut Service,
        session: SessionKey,
        policy: SessionPolicy,
    ) {
        service
            .open_session(SessionOpening {
                session,
                tenant: self.tenant,
                campaign: self.campaign,
                price_version: self.version.id,
                currency: CURRENCY,
                policy,
            })
            .await
            .expect("open the session");
    }

    pub async fn service_with_members(&self, members: Vec<PayerId>) -> Service {
        self.build(
            FenceVerdict::Current,
            MediaVerdict::Admitted,
            members,
            FinanceConfig::default(),
        )
        .await
    }

    /// Puts a state the round-1 flows cannot reach into the database (a withdrawn allocation, a
    /// frozen lot): written by the superuser with the triggers off.
    pub async fn seed(&self, sql: &str) {
        self.admin
            .batch_execute(&format!("SET session_replication_role = replica; {sql}; SET session_replication_role = origin"))
            .await
            .unwrap_or_else(|error| panic!("seed: {error:?}: {sql}"));
    }

    async fn build(
        &self,
        fence: FenceVerdict,
        media: MediaVerdict,
        members: Vec<PayerId>,
        config: FinanceConfig,
    ) -> Service {
        let client = self.scratch.connect().await;
        client
            .batch_execute("SET ROLE df_finance")
            .await
            .expect("set role");
        // A login as df_finance carries these as role defaults; SET ROLE from the harness does not.
        client
            .batch_execute("SET lock_timeout = '2s'; SET statement_timeout = '30s'")
            .await
            .expect("timeouts");
        let seed = self.next_seed.fetch_add(1, Ordering::SeqCst);
        FinanceService::new(
            client,
            FakeFence::new(fence),
            FakeMedia::new(media),
            FakeMembers::new(members),
            Box::new(SystemClock),
            Box::new(SeededIds::new(seed)),
            config,
        )
    }

    pub fn fence(&self) -> FenceRequest {
        FenceRequest {
            tenant: self.tenant,
            campaign: self.campaign,
            recovery_epoch: 1,
            claim_seq: 1,
            placement_id: [0x54; 16],
        }
    }

    /// An admission of `input` and `output` tokens (a maximum), with ids from `tag`.
    pub fn admit_command(&self, tag: u8, input_tokens: u64, output_tokens: u64) -> AdmitCommand {
        AdmitCommand {
            operation: operation(tag),
            attempt: AttemptId::from_array([0x60 + tag; 16]),
            effect: EffectId::from_array([0x80 + tag; 16]),
            fence: self.fence(),
            session: self.session,
            route: route(),
            work_class: WorkClass::Essential,
            maximum_units: vec![
                ("text_input".into(), input_tokens),
                ("text_output".into(), output_tokens),
            ],
            estimate_units: vec![
                ("text_input".into(), input_tokens / 2),
                ("text_output".into(), output_tokens / 2),
            ],
            quote: None,
            request_digest: business_key("test_request", &[&[tag]]),
            media: None,
        }
    }

    pub fn consent(&self) -> Consent {
        Consent {
            approved_hourly_ceiling_micro_dollars: 20_000_000,
            approved_session_ceiling_micro_dollars: None,
            approval_threshold_micro_dollars: 500_000,
            terms_version: 1,
        }
    }

    /// The natural balance of the key with these dimensions, summed (a missing key is 0).
    pub async fn natural(
        &self,
        code: i32,
        payer: Option<PayerId>,
        lot: Option<LotId>,
        allocation: Option<AllocationId>,
    ) -> i64 {
        let code = i16::try_from(code).expect("code");
        let payer = payer.map(|id| id.as_bytes().to_vec());
        let lot = lot.map(|id| id.as_bytes().to_vec());
        let allocation = allocation.map(|id| id.as_bytes().to_vec());
        self.admin
            .query_one(
                "SELECT coalesce(sum(b.natural_micro_dollars), 0)::bigint FROM finance.account_balance b \
                 JOIN finance.account_key k ON k.key_id = b.key_id \
                 WHERE k.code = $1 AND ($2::bytea IS NULL OR k.payer_account_id = $2) \
                   AND ($3::bytea IS NULL OR k.lot_id = $3) AND ($4::bytea IS NULL OR k.allocation_id = $4)",
                &[&code, &payer, &lot, &allocation],
            )
            .await
            .expect("balance")
            .get(0)
    }

    /// The signed sum of every line on accounts of `code` (for the accounts that keep no balance row).
    pub async fn line_sum(&self, code: i32) -> i64 {
        let code = i16::try_from(code).expect("code");
        self.admin
            .query_one(
                "SELECT coalesce(sum(l.amount_micro_dollars), 0)::bigint FROM finance.journal_line l \
                 JOIN finance.account_key k ON k.key_id = l.key_id WHERE k.code = $1",
                &[&code],
            )
            .await
            .expect("line sum")
            .get(0)
    }

    pub async fn count(&self, table: &str) -> i64 {
        self.admin
            .query_one(&format!("SELECT count(*) FROM finance.{table}"), &[])
            .await
            .expect("count")
            .get(0)
    }

    /// The lot ids of a payer, oldest first.
    pub async fn lots_of(&self, payer: PayerId) -> Vec<LotId> {
        self.admin
            .query(
                "SELECT lot_id FROM finance.funding_lot WHERE payer_account_id = $1 ORDER BY created_at_ms, lot_id",
                &[&payer.as_bytes().to_vec()],
            )
            .await
            .expect("lots")
            .iter()
            .map(|row| LotId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("lot id"))
            .collect()
    }

    /// The payer's allocation in the world's session.
    pub async fn allocation_of(&self, payer: PayerId) -> Option<AllocationId> {
        self.admin
            .query_opt(
                "SELECT allocation_id FROM finance.allocation WHERE session_id = $1 AND payer_account_id = $2",
                &[&self.session.as_bytes().to_vec(), &payer.as_bytes().to_vec()],
            )
            .await
            .expect("allocation")
            .map(|row| AllocationId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("allocation id"))
    }

    pub async fn finish(self) {
        drop(self.admin);
        self.scratch.finish().await;
    }
}
