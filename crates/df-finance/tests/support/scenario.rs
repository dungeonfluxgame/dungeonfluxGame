//! The property scenario: seeded random operations with every invariant re-derived after each
//! step, then the whole attack catalog, optionally against a database with mutants applied half way
//! through (planning/finance-ledger-research.md 17, 21).

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the scenario"
)]

use df_commerce::finance_ids::{AttemptId, OperationKey, UsageEventId};
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{ReleaseReason, UsageSource};
use df_finance::admit::AdmissionOutcome;
use df_finance::service::FinanceError;

use super::catalog;
use super::invariants;
use super::mutants::{self, Mutant};
use super::world::{CURRENCY, Service, World, credit_command, payer, purchase};

/// xorshift64: reproducible, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn op_id(n: u16) -> OperationKey {
    OperationKey::from_array([
        0xc0,
        (n >> 8) as u8,
        n as u8,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
}

/// A refusal is a legitimate answer; anything the database or the wire says is a failure.
fn judge<T>(result: Result<T, FinanceError>, what: &str) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(
            FinanceError::Admission(_)
            | FinanceError::Funding(_)
            | FinanceError::Settle(_)
            | FinanceError::Lifecycle(_)
            | FinanceError::Fence(_)
            | FinanceError::Media(_),
        ) => Ok(None),
        Err(error) => Err(format!("operation failed: {what}: {error:?}")),
    }
}

struct Sim {
    rng: Rng,
    counter: u16,
    tag: u8,
    unsent: Vec<(AttemptId, u64, u64)>,
    sent: Vec<(AttemptId, u64, u64)>,
    /// Per operation: how many were applied and how many were refused.
    stats: std::collections::BTreeMap<&'static str, (u32, u32)>,
}

impl Sim {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed),
            counter: 0,
            tag: 0,
            unsent: Vec::new(),
            sent: Vec::new(),
            stats: std::collections::BTreeMap::new(),
        }
    }

    fn payer(&mut self) -> df_commerce::finance_ids::PayerId {
        payer(1 + self.rng.below(3) as u8)
    }

    fn note<T>(
        &mut self,
        what: &'static str,
        result: Result<T, FinanceError>,
    ) -> Result<Option<T>, String> {
        let judged = judge(result, what)?;
        let entry = self.stats.entry(what).or_default();
        if judged.is_some() {
            entry.0 += 1
        } else {
            entry.1 += 1
        }
        Ok(judged)
    }

    async fn step(&mut self, world: &World, service: &mut Service) -> Result<(), String> {
        self.counter += 1;
        let n = self.counter;
        let roll = self.rng.below(100);
        match roll {
            0..=9 => {
                let who = self.payer();
                let amount = 1_000_000 + self.rng.below(7_000_000);
                self.note(
                    "purchase",
                    service
                        .apply_purchase(purchase(who, &format!("pay_{n}"), amount))
                        .await,
                )?;
            }
            10..=14 => {
                let who = self.payer();
                self.note(
                    "trial",
                    service
                        .grant_trial(who, [n as u8 ^ 0x5a; 32], CURRENCY)
                        .await,
                )?;
            }
            15..=34 => {
                let who = self.payer();
                let amount = 100_000 + self.rng.below(3_000_000);
                self.note(
                    "allocate",
                    service
                        .allocate(who, world.session, op_id(n), amount, world.consent())
                        .await,
                )?;
            }
            35..=54 if self.tag < 100 => {
                self.tag += 1;
                let input = 10_000 + self.rng.below(2_000_000);
                let output = self.rng.below(500_000);
                let command = world.admit_command(self.tag, input, output);
                if let Some(AdmissionOutcome::Admitted { .. }) =
                    self.note("admit", service.admit(&command).await)?
                {
                    self.unsent.push((command.attempt, input, output));
                }
            }
            55..=64 if !self.unsent.is_empty() => {
                let index = self.rng.below(self.unsent.len() as u64) as usize;
                let (attempt, input, output) = self.unsent.swap_remove(index);
                self.note(
                    "prepare",
                    service
                        .prepare_dispatch(attempt, Deadlines::default())
                        .await,
                )?;
                self.note(
                    "send",
                    service
                        .mark_attempt_sent(attempt, [9; 16], Deadlines::default())
                        .await,
                )?;
                self.sent.push((attempt, input, output));
            }
            65..=79 if !self.sent.is_empty() => {
                let index = self.rng.below(self.sent.len() as u64) as usize;
                let (attempt, input, output) = self.sent.swap_remove(index);
                let units = vec![
                    ("text_input".to_owned(), input / 2),
                    ("text_output".to_owned(), output / 2),
                ];
                self.note(
                    "settle",
                    service.settle(attempt, units, UsageSource::Reported).await,
                )?;
            }
            80..=84 if !self.unsent.is_empty() => {
                let index = self.rng.below(self.unsent.len() as u64) as usize;
                let (attempt, _, _) = self.unsent.swap_remove(index);
                self.note(
                    "release",
                    service.release(attempt, ReleaseReason::NotDispatched).await,
                )?;
            }
            85..=99 => {
                let open = world
                    .admin
                    .query(
                        "SELECT usage_event_id, retail_micro_dollars - reversed_micro_dollars FROM finance.usage_event \
                          WHERE retail_micro_dollars > reversed_micro_dollars ORDER BY usage_event_id",
                        &[],
                    )
                    .await
                    .map_err(|error| format!("reading usage events: {error:?}"))?;
                if !open.is_empty() {
                    let row = &open[self.rng.below(open.len() as u64) as usize];
                    let usage = UsageEventId::from_slice(&row.get::<_, Vec<u8>>(0))
                        .ok_or("usage event id")?;
                    let remaining = row.get::<_, i64>(1) as u64;
                    let amount = (remaining / (1 + self.rng.below(3))).max(1);
                    self.note(
                        "credit",
                        service
                            .credit(credit_command(
                                usage,
                                op_id(n),
                                amount,
                                &format!("case-{n}"),
                            ))
                            .await,
                    )?;
                }
            }
            _ => {}
        }
        invariants::all(&world.admin)
            .await
            .map_err(|failure| format!("invariant: after step {n} ({roll}): {failure}"))
    }
}

/// The scenario against a database with `mutants` applied half way through the random operations.
pub async fn scenario(mutants: &[Mutant], seed: u64) -> Result<Vec<String>, String> {
    let Some(world) = World::open().await else {
        return Ok(vec!["SKIPPED".into()]);
    };
    let mut service = world.service().await;
    let mut sim = Sim::new(seed);
    // Seed money so most operations have something to do.
    for tag in 1..=3 {
        service
            .apply_purchase(purchase(payer(tag), &format!("seed_{tag}"), 8_000_000))
            .await
            .map_err(|error| format!("setup: {error:?}"))?;
        service
            .allocate(
                payer(tag),
                world.session,
                op_id(40_000 + u16::from(tag)),
                4_000_000,
                world.consent(),
            )
            .await
            .map_err(|error| format!("setup: {error:?}"))?;
    }
    let outcome = async {
        for _ in 0..70 {
            sim.step(&world, &mut service).await?;
        }
        for mutant in mutants {
            mutants::apply(&world.admin, *mutant).await;
        }
        for _ in 0..30 {
            sim.step(&world, &mut service).await?;
        }
        println!("operations (applied, refused): {:?}", sim.stats);
        let log = catalog::run_all(&world, &mut service).await?;
        invariants::all(&world.admin)
            .await
            .map_err(|failure| format!("invariant: after the attacks: {failure}"))?;
        Ok(log)
    }
    .await;
    drop(service);
    world.finish().await;
    outcome
}
