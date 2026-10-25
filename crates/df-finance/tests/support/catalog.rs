//! The attack catalog: events no honest planner would send, each aimed at one guard of the boundary
//! and each expected to be refused with a named SQLSTATE and message. The property tests run the
//! whole catalog against the real migrations, and against a database with one guard removed, where
//! the attack that guard stops must get through (or be stopped by something else, which the pinned
//! message reveals).

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the catalog"
)]

use std::collections::BTreeMap;

use df_commerce::admission::{AdmissionDecision, AdmissionRequest, decide_admission};
use df_commerce::finance_ids::{
    AllocationId, AttemptId, EntryId, EventId, LotId, OperationKey, PayerId, ReservationId,
    UsageEventId,
};
use df_commerce::funding_flows::{AllocateRequest, plan_allocation};
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{
    CreditReason, CreditRequest, SettleRequest, UsageSource, plan_credit, plan_settlement,
};
use df_finance::wire::{
    event_json, parse_admission_snapshot, parse_allocate_snapshot, parse_credit_snapshot,
    parse_held_snapshot,
};
use serde_json::{Value, json};

use super::attack::{
    fee_event, forged_trial, now_ms, run_event, run_event_with_settings, service_client,
};
use super::sqlstate;
use super::world::{CURRENCY, Service, World, payer, purchase};

/// A refusal the catalog expects: the SQLSTATEs it may carry and a fragment of the message.
#[derive(Clone, Copy)]
pub struct Expect {
    pub codes: &'static [&'static str],
    pub message: &'static str,
}

/// `Ok` is the log line of a refusal that matched; `Err` says the attack got through, or was
/// refused for another reason than the one pinned.
pub type Refusal = Result<String, String>;

pub fn refused(
    name: &str,
    outcome: Result<Value, tokio_postgres::Error>,
    expect: Expect,
) -> Refusal {
    let Err(error) = outcome else {
        return Err(format!("attack accepted: {name}"));
    };
    let code = sqlstate(&error).unwrap_or_else(|| format!("{error:?}"));
    let message = error
        .as_db_error()
        .map_or(String::new(), |db| db.message().to_owned());
    if expect.codes.contains(&code.as_str()) && message.contains(expect.message) {
        let shown: String = message.chars().take(100).collect();
        Ok(format!("{name}: refused with {code}: {shown}"))
    } else {
        Err(format!(
            "attack refused for the wrong reason: {name}: {code} {message}"
        ))
    }
}

fn plain(name: &str, outcome: Result<(), tokio_postgres::Error>) -> Refusal {
    match outcome {
        Ok(()) => Err(format!("attack accepted: {name}")),
        Err(error) => Ok(format!(
            "{name}: refused with {}",
            sqlstate(&error).unwrap_or_default()
        )),
    }
}

fn op_key(salt: u8) -> OperationKey {
    OperationKey::from_array([0xd0, salt, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn event_ids(salt: u8) -> (EventId, EntryId, UsageEventId) {
    (
        EventId::from_array([salt; 16]),
        EntryId::from_array([salt.wrapping_add(1); 16]),
        UsageEventId::from_array([salt.wrapping_add(2); 16]),
    )
}

fn line_index(event: &Value, code: i64) -> usize {
    event["entries"][0]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .position(|line| line["code"] == code)
        .unwrap_or_else(|| panic!("no line on account {code}"))
}

fn shift(event: &mut Value, code: i64, by: i64) {
    let index = line_index(event, code);
    let line = &mut event["entries"][0]["lines"][index];
    line["amount"] = json!(line["amount"].as_i64().expect("amount") + by);
}

fn failure(message: &str, error: impl std::fmt::Debug) -> String {
    format!("{message}: {error:?}")
}

/// An admitted, prepared and sent attempt of `input` and `output` tokens.
pub async fn sent_attempt(
    world: &World,
    service: &mut Service,
    tag: u8,
    input: u64,
    output: u64,
) -> Result<AttemptId, String> {
    let command = world.admit_command(tag, input, output);
    service
        .admit(&command)
        .await
        .map_err(|e| failure("admit", e))?;
    service
        .prepare_dispatch(command.attempt, Deadlines::default())
        .await
        .map_err(|e| failure("prepare", e))?;
    service
        .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
        .await
        .map_err(|e| failure("send", e))?;
    Ok(command.attempt)
}

/// Everything the equation attacks need, made once.
pub struct Fixture {
    pub attempts: BTreeMap<u8, AttemptId>,
    pub settled_usage: UsageEventId,
    pub foreign: (AllocationId, LotId),
    pub payer_of: BTreeMap<AllocationId, PayerId>,
}

/// Funds payers 3 and 4, admits and sends one attempt per equation attack, settles one for the
/// credit attack, then funds payer 5, whose keys exist but belong to none of those reservations.
pub async fn fixture(world: &World, service: &mut Service) -> Result<Fixture, String> {
    for (tag, reference) in [(3_u8, "fx_pay_3"), (4, "fx_pay_4")] {
        service
            .apply_purchase(purchase(payer(tag), reference, 20_000_000))
            .await
            .map_err(|e| failure("setup purchase", e))?;
        service
            .allocate(
                payer(tag),
                world.session,
                op_key(tag),
                8_000_000,
                world.consent(),
            )
            .await
            .map_err(|e| failure("setup allocate", e))?;
    }
    let mut attempts = BTreeMap::new();
    for number in 1..=8_u8 {
        let attempt = sent_attempt(world, service, 100 + number, 1_000_000, 1_000_000).await?;
        attempts.insert(number, attempt);
    }
    let settled = attempts[&5];
    service
        .settle(
            settled,
            vec![("text_input".into(), 100_000), ("text_output".into(), 0)],
            UsageSource::Reported,
        )
        .await
        .map_err(|e| failure("setup settle", e))?;
    let settled_usage = usage_event_of(world, settled).await;
    service
        .apply_purchase(purchase(payer(5), "fx_pay_5", 20_000_000))
        .await
        .map_err(|e| failure("setup purchase", e))?;
    service
        .allocate(
            payer(5),
            world.session,
            op_key(5),
            3_000_000,
            world.consent(),
        )
        .await
        .map_err(|e| failure("setup allocate", e))?;
    let allocation = world
        .allocation_of(payer(5))
        .await
        .ok_or("no allocation for payer 5")?;
    let lot = world.lots_of(payer(5)).await[0];
    let payer_of = world
        .admin
        .query(
            "SELECT allocation_id, payer_account_id FROM finance.allocation",
            &[],
        )
        .await
        .map_err(|e| failure("allocations", e))?
        .iter()
        .filter_map(|row| {
            Some((
                AllocationId::from_slice(&row.get::<_, Vec<u8>>(0))?,
                PayerId::from_slice(&row.get::<_, Vec<u8>>(1))?,
            ))
        })
        .collect();
    Ok(Fixture {
        attempts,
        settled_usage,
        foreign: (allocation, lot),
        payer_of,
    })
}

async fn usage_event_of(world: &World, attempt: AttemptId) -> UsageEventId {
    let row = world
        .admin
        .query_one(
            "SELECT usage_event_id FROM finance.usage_event WHERE attempt_id = $1",
            &[&attempt.as_bytes().to_vec()],
        )
        .await
        .expect("usage event");
    UsageEventId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("id")
}

/// An honest settlement of `attempt`, altered by `mutate`, committed by hand.
pub async fn forged_settlement(
    world: &World,
    attempt: AttemptId,
    salt: u8,
    mutate: impl FnOnce(&mut Value),
) -> Result<Value, tokio_postgres::Error> {
    let (event_id, entry_id, usage_event) = event_ids(salt);
    let request = SettleRequest {
        event_id,
        entry_id,
        usage_event,
        actual_units: vec![("text_input".into(), 100_000), ("text_output".into(), 0)],
        usage_source: UsageSource::Reported,
        now_ms: now_ms(),
    };
    run_event(
        world,
        Some(("settlement", json!({"attempt": attempt.hex()}))),
        |snapshot| {
            let held = parse_held_snapshot(snapshot)
                .expect("snapshot")
                .expect("a held attempt");
            let plan = plan_settlement(&held, &request).expect("an honest settlement plan");
            let mut event = event_json(&plan);
            mutate(&mut event);
            event
        },
    )
    .await
}

/// E1: the supplier cost the entry books is 1,000 more than the usage event says.
pub async fn e1(world: &World, fixture: &Fixture) -> Refusal {
    refused(
        "a settlement that mis-states the supplier cost",
        forged_settlement(world, fixture.attempts[&1], 0x10, |event| {
            shift(event, 5100, 1000);
            shift(event, 2300, -1000);
        })
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E1 failed",
        },
    )
}

/// E2: the reserved debit is 1,000 more than the shares charged and released.
pub async fn e2(world: &World, fixture: &Fixture) -> Refusal {
    refused(
        "a settlement whose reserved debit differs from its shares",
        forged_settlement(world, fixture.attempts[&2], 0x20, |event| {
            shift(event, 2130, 1000);
            shift(event, 2120, -1000);
        })
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E2 failed",
        },
    )
}

/// E3: the surplus returns to the allocation key of a payer that is not in the reservation.
pub async fn e3(world: &World, fixture: &Fixture) -> Refusal {
    let (allocation, lot) = fixture.foreign;
    refused(
        "a settlement that returns to a key outside the reservation",
        forged_settlement(world, fixture.attempts[&3], 0x30, |event| {
            let index = line_index(event, 2120);
            let line = &mut event["entries"][0]["lines"][index];
            line["allocation"] = json!(allocation.hex());
            line["lot"] = json!(lot.hex());
        })
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E3 failed",
        },
    )
}

/// E4: the surplus returns to the payer's available key instead of the allocation it came from.
pub async fn e4(world: &World, fixture: &Fixture) -> Refusal {
    let payers = fixture.payer_of.clone();
    refused(
        "a settlement that returns to the wrong key",
        forged_settlement(world, fixture.attempts[&4], 0x40, |event| {
            let index = line_index(event, 2120);
            let line = &mut event["entries"][0]["lines"][index];
            let allocation =
                AllocationId::from_hex(line["allocation"].as_str().expect("allocation"))
                    .expect("allocation id");
            line["code"] = json!(2110);
            line["payer"] = json!(payers[&allocation].hex());
            line["allocation"] = Value::Null;
        })
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E4 failed",
        },
    )
}

/// E5: a credit whose debit side is 1,000 more than the usage event's reversed change.
pub async fn e5(world: &World, fixture: &Fixture) -> Refusal {
    let (event_id, entry_id, _) = event_ids(0x50);
    let request = CreditRequest {
        event_id,
        entry_id,
        operation: op_key(0x50),
        amount_micro_dollars: 1_000,
        reason: CreditReason::TechnicalFailure,
        actor_id: [1; 16],
        case_ref: "fx-case".into(),
        approver_id: None,
        now_ms: now_ms(),
    };
    refused(
        "a credit that mis-states the reversal",
        run_event(
            world,
            Some((
                "credit",
                json!({"usage_event": fixture.settled_usage.hex()}),
            )),
            |snapshot| {
                let snapshot = parse_credit_snapshot(snapshot).expect("snapshot");
                let plan = plan_credit(&snapshot, &request).expect("an honest credit plan");
                let mut event = event_json(&plan);
                shift(&mut event, 4110, 1000);
                shift(&mut event, 2110, -1000);
                event
            },
        )
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E5 failed",
        },
    )
}

/// E6: a reservation entry that holds 1,000 more than the reservation's retail maximum.
pub async fn e6(world: &World, _fixture: &Fixture) -> Refusal {
    let command = world.admit_command(112, 100_000, 50_000);
    let (event_id, entry_id, _) = event_ids(0x60);
    let request_for = |snapshot: &df_commerce::admission::AdmissionSnapshot| AdmissionRequest {
        event_id,
        entry_id,
        reservation: ReservationId::from_array([0x63; 16]),
        attempt: command.attempt,
        effect: command.effect,
        operation: command.operation,
        tenant: command.fence.tenant,
        campaign: command.fence.campaign,
        session: command.session,
        route: command.route.clone(),
        work_class: command.work_class,
        maximum_units: command.maximum_units.clone(),
        estimate_units: command.estimate_units.clone(),
        quote: None,
        request_digest: command.request_digest,
        authorized_payers: snapshot
            .allocations
            .iter()
            .map(|(allocation, _)| allocation.payer)
            .collect(),
        now_ms: now_ms(),
        reserved_expiry_ms: 300_000,
    };
    refused(
        "a reservation that holds more than its maximum",
        run_event(
            world,
            Some((
                "admission",
                json!({
                    "session": world.session.hex(),
                    "principal": command.fence.campaign.hex(),
                    "operation": command.operation.hex(),
                    "now_ms": now_ms(),
                }),
            )),
            |snapshot| {
                let snapshot = parse_admission_snapshot(snapshot).expect("snapshot");
                let AdmissionDecision::Admitted(admitted) =
                    decide_admission(&snapshot, &request_for(&snapshot)).expect("admission")
                else {
                    panic!("expected a fresh admission");
                };
                let mut event = event_json(&admitted.plan);
                let lines = event["entries"][0]["lines"].as_array_mut().expect("lines");
                let debit = lines
                    .iter()
                    .position(|line| matches!(line["code"].as_i64(), Some(2120 | 2220)))
                    .expect("allocated debit");
                let credit = lines
                    .iter()
                    .position(|line| matches!(line["code"].as_i64(), Some(2130 | 2230)))
                    .expect("reserved credit");
                lines[debit]["amount"] =
                    json!(lines[debit]["amount"].as_i64().expect("amount") + 1000);
                lines[credit]["amount"] =
                    json!(lines[credit]["amount"].as_i64().expect("amount") - 1000);
                event
            },
        )
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E6 failed",
        },
    )
}

/// E7: an allocation entry that moves 1,000 more than the allocation's drawn amount grew by.
pub async fn e7(world: &World, service: &mut Service) -> Refusal {
    if let Err(error) = service
        .apply_purchase(purchase(payer(6), "fx_pay_6", 20_000_000))
        .await
    {
        return Err(failure("setup purchase", error));
    }
    let (event_id, entry_id, _) = event_ids(0x70);
    let request = AllocateRequest {
        event_id,
        entry_id,
        new_allocation: AllocationId::from_array([0x73; 16]),
        operation: op_key(0x70),
        payer: payer(6),
        amount_micro_dollars: 2_000_000,
        consent: world.consent(),
        now_ms: now_ms(),
    };
    refused(
        "an allocation that moves more than it draws",
        run_event(
            world,
            Some((
                "allocate",
                json!({"payer": payer(6).hex(), "session": world.session.hex()}),
            )),
            |snapshot| {
                let plan = plan_allocation(
                    &parse_allocate_snapshot(snapshot).expect("snapshot"),
                    &request,
                )
                .expect("an honest allocation plan");
                let mut event = event_json(&plan);
                let lines = event["entries"][0]["lines"].as_array_mut().expect("lines");
                let debit = lines
                    .iter()
                    .position(|l| l["amount"].as_i64() > Some(0))
                    .expect("debit");
                let credit = lines
                    .iter()
                    .position(|l| l["amount"].as_i64() < Some(0))
                    .expect("credit");
                lines[debit]["amount"] =
                    json!(lines[debit]["amount"].as_i64().expect("amount") + 1000);
                lines[credit]["amount"] =
                    json!(lines[credit]["amount"].as_i64().expect("amount") - 1000);
                event
            },
        )
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E7 failed",
        },
    )
}

/// E8: a trial entry that credits 15 dollars against a 10 dollar lot.
pub async fn e8(world: &World) -> Refusal {
    refused(
        "credit minted past its lot",
        forged_trial(world, 8, |event| {
            let lines = event["entries"][0]["lines"].as_array_mut().expect("lines");
            lines[0]["amount"] = json!(15_000_000);
            lines[1]["amount"] = json!(-15_000_000);
        })
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E8 failed",
        },
    )
}

/// The attack on equation `number` (1 to 8), set up from the shared fixture.
pub async fn equation(
    world: &World,
    service: &mut Service,
    fixture: &Fixture,
    number: u8,
) -> Refusal {
    match number {
        1 => e1(world, fixture).await,
        2 => e2(world, fixture).await,
        3 => e3(world, fixture).await,
        4 => e4(world, fixture).await,
        5 => e5(world, fixture).await,
        6 => e6(world, fixture).await,
        7 => e7(world, service).await,
        _ => e8(world).await,
    }
}

/// Dr 100 / Cr 90 on accounts no lot or row change touches: only the balance check stands in the way.
pub async fn unbalanced(world: &World) -> Refusal {
    refused(
        "an unbalanced entry",
        run_event(world, None, |_| fee_event(1, 100, 90)).await,
        Expect {
            codes: &["23000"],
            message: "not balanced",
        },
    )
}

/// The same entry from a session that first set every setting a memo of the check could trust.
pub async fn unbalanced_with_settings_preset(world: &World) -> Refusal {
    let event = fee_event(9, 100, 90);
    let entry_hex = event["entries"][0]["entry_id"]
        .as_str()
        .expect("entry id")
        .to_owned();
    let settings = vec![
        ("finance.epoch".to_owned(), "zz".to_owned()),
        (format!("finance.c_zz_e_{entry_hex}"), "1".to_owned()),
        (format!("finance.checked_e_{entry_hex}"), "1".to_owned()),
        (format!("finance.checked_lb_{entry_hex}"), "1".to_owned()),
    ];
    refused(
        "an unbalanced entry from a session with the check pre-set",
        run_event_with_settings(world, &settings, event).await,
        Expect {
            codes: &["23000"],
            message: "not balanced",
        },
    )
}

/// A revenue account in a fee entry (its key is made by the same event).
pub async fn forbidden_account(world: &World) -> Refusal {
    refused(
        "an account the kind may not post",
        run_event(world, None, |_| {
            let mut event = fee_event(2, 100, 100);
            event["entries"][0]["lines"][0]["code"] = json!(4110);
            event["changes"] = json!([
                {"kind": "key_create", "row": {"code": 4110, "currency": "USD"}},
                {"kind": "key_create", "row": {"code": 1100, "currency": "USD", "processor": "fake"}},
            ]);
            event["entries"][0]["lines"][0]["processor"] = Value::Null;
            event
        })
        .await,
        Expect {
            codes: &["FN003"],
            message: "may not post",
        },
    )
}

/// A second trial lot for an account that has one, with fresh ids, a fresh business key and no
/// trial_grant row: the plan for another payer, rewritten to name the victim.
pub async fn second_trial(world: &World, service: &mut Service) -> Refusal {
    let victim = payer(9);
    if let Err(error) = service.grant_trial(victim, [0x77; 32], CURRENCY).await {
        return Err(failure("setup trial", error));
    }
    refused(
        "a second trial grant for an account that has one",
        forged_trial(world, 10, |event| {
            let text = event.to_string().replace(&payer(10).hex(), &victim.hex());
            *event = serde_json::from_str(&text).expect("json");
            let changes = event["changes"].as_array_mut().expect("changes");
            changes.retain(|change| {
                change["kind"] != "payer_insert" && change["kind"] != "trial_grant_insert"
            });
            event["entries"][0]["business_key"] = json!("ab".repeat(32));
        })
        .await,
        Expect {
            codes: &["FN003"],
            message: "trial",
        },
    )
}

const NOT_LISTED: Expect = Expect {
    codes: &["FN003"],
    message: "may not set column",
};

/// A settlement of a fresh attempt whose lot update also carries `column`, which the settlement
/// never writes.
async fn settlement_with_a_lot_column(
    world: &World,
    service: &mut Service,
    (tag, salt): (u8, u8),
    name: &str,
    (column, value): (&str, Value),
) -> Refusal {
    let attempt = match sent_attempt(world, service, tag, 100_000, 100_000).await {
        Ok(attempt) => attempt,
        Err(error) => return Err(format!("setup attempt: {error}")),
    };
    refused(
        name,
        forged_settlement(world, attempt, salt, |event| {
            let change = event["changes"]
                .as_array_mut()
                .expect("changes")
                .iter_mut()
                .find(|change| change["kind"] == "funding_lot_update")
                .expect("a lot update");
            change["row"][column] = value;
        })
        .await,
        NOT_LISTED,
    )
}

/// A lot's original amount rewritten through a settlement's lot update.
pub async fn column_outside_the_allowlist(world: &World, service: &mut Service) -> Refusal {
    settlement_with_a_lot_column(
        world,
        service,
        (120, 0xb0),
        "an update of a column the change kind does not list",
        ("original_micro_dollars", json!(1)),
    )
    .await
}

/// A lot's state flipped by a settlement: the lot would leave `available` with its keys untouched.
pub async fn lot_state_flipped_by_a_settlement(world: &World, service: &mut Service) -> Refusal {
    settlement_with_a_lot_column(
        world,
        service,
        (121, 0xb1),
        "a lot state flipped by a settlement",
        ("state", json!("expired")),
    )
    .await
}

/// A credit whose lot update carries a column only a settlement writes: the change kind lists it,
/// the credit's own list does not.
pub async fn credit_writing_a_settlement_column(world: &World, fixture: &Fixture) -> Refusal {
    let (event_id, entry_id, _) = event_ids(0xb2);
    let request = CreditRequest {
        event_id,
        entry_id,
        operation: op_key(0xb2),
        amount_micro_dollars: 1_000,
        reason: CreditReason::TechnicalFailure,
        actor_id: [1; 16],
        case_ref: "fx-case".into(),
        approver_id: None,
        now_ms: now_ms(),
    };
    refused(
        "a credit that writes a column only a settlement writes",
        run_event(
            world,
            Some((
                "credit",
                json!({"usage_event": fixture.settled_usage.hex()}),
            )),
            |snapshot| {
                let snapshot = parse_credit_snapshot(snapshot).expect("snapshot");
                let plan = plan_credit(&snapshot, &request).expect("an honest credit plan");
                let mut event = event_json(&plan);
                let change = event["changes"]
                    .as_array_mut()
                    .expect("changes")
                    .iter_mut()
                    .find(|change| change["kind"] == "funding_lot_update")
                    .expect("a lot update");
                change["row"]["extinguished_micro_dollars"] = json!(1);
                event
            },
        )
        .await,
        NOT_LISTED,
    )
}

/// The session's running charged total reset to zero through the session update of an allocation,
/// which would give the session its ceiling back. Equation E9 refuses it first; with E9 gone the
/// allocation's column list for `session_funding_update` (which names `current_epoch` only) does.
pub async fn charged_total_reset_in_an_allocation(world: &World, service: &mut Service) -> Refusal {
    if let Err(error) = service
        .apply_purchase(purchase(payer(12), "fx_pay_12", 20_000_000))
        .await
    {
        return Err(failure("setup purchase", error));
    }
    let (event_id, entry_id, _) = event_ids(0xb3);
    let request = AllocateRequest {
        event_id,
        entry_id,
        new_allocation: AllocationId::from_array([0xb6; 16]),
        operation: op_key(0xb3),
        payer: payer(12),
        amount_micro_dollars: 2_000_000,
        consent: world.consent(),
        now_ms: now_ms(),
    };
    refused(
        "a session charged total reset in an allocation",
        run_event(
            world,
            Some((
                "allocate",
                json!({"payer": payer(12).hex(), "session": world.session.hex()}),
            )),
            |snapshot| {
                let plan = plan_allocation(
                    &parse_allocate_snapshot(snapshot).expect("snapshot"),
                    &request,
                )
                .expect("an honest allocation plan");
                let mut event = event_json(&plan);
                let change = event["changes"]
                    .as_array_mut()
                    .expect("changes")
                    .iter_mut()
                    .find(|change| change["kind"] == "session_funding_update")
                    .expect("the allocation updates the session's epoch");
                change["row"]["charged_total_micro_dollars"] = json!(0);
                event
            },
        )
        .await,
        Expect {
            codes: &["FN004"],
            message: "equation E9 failed",
        },
    )
}

/// A settlement entry that names another session's stream and a campaign that is not its own.
pub async fn forged_correlation(world: &World, fixture: &Fixture) -> Refusal {
    refused(
        "a settlement with forged correlation columns",
        forged_settlement(world, fixture.attempts[&7], 0x80, |event| {
            event["entries"][0]["campaign_id"] = json!("cd".repeat(16));
        })
        .await,
        Expect {
            codes: &["FN003"],
            message: "differs from its anchor",
        },
    )
}

/// A back-dated entry with no correction of a closed period behind it.
pub async fn back_dated(world: &World) -> Refusal {
    refused(
        "a back-dated entry",
        run_event(world, None, |_| {
            let mut event = fee_event(12, 100, 100);
            event["entries"][0]["effective_at_ms"] = json!(1000);
            event
        })
        .await,
        Expect {
            codes: &["FN003"],
            message: "correction of a closed period",
        },
    )
}

/// A system entry that names an actor.
pub async fn system_entry_with_an_actor(world: &World) -> Refusal {
    refused(
        "a system entry that names an actor",
        run_event(world, None, |_| {
            let mut event = fee_event(13, 100, 100);
            event["entries"][0]["actor_id"] = json!("ab".repeat(16));
            event
        })
        .await,
        Expect {
            codes: &["FN003", "23514"],
            message: "system entry",
        },
    )
}

/// An entry whose reason is free text.
pub async fn free_text_reason(world: &World) -> Refusal {
    refused(
        "an entry with a free-text reason",
        run_event(world, None, |_| {
            let mut event = fee_event(14, 100, 100);
            event["entries"][0]["reason"] = json!("the customer was nice to me");
            event
        })
        .await,
        Expect {
            codes: &["FN003"],
            message: "closed list",
        },
    )
}

/// An allocation of 15 dollars planned from a snapshot that claims 5 more than the payer holds.
pub async fn overdraw(world: &World, service: &mut Service) -> Refusal {
    if let Err(error) = service
        .apply_purchase(purchase(payer(7), "fx_pay_7", 10_000_000))
        .await
    {
        return Err(failure("setup purchase", error));
    }
    let (event_id, entry_id, _) = event_ids(0x90);
    let request = AllocateRequest {
        event_id,
        entry_id,
        new_allocation: AllocationId::from_array([0x93; 16]),
        operation: op_key(0x90),
        payer: payer(7),
        amount_micro_dollars: 15_000_000,
        consent: world.consent(),
        now_ms: now_ms(),
    };
    refused(
        "an overdraw of the payer's credit",
        run_event(
            world,
            Some((
                "allocate",
                json!({"payer": payer(7).hex(), "session": world.session.hex()}),
            )),
            |snapshot| {
                let mut forged = parse_allocate_snapshot(snapshot).expect("snapshot");
                for lot in &mut forged.payer_lots {
                    lot.available_micro_dollars += 5_000_000;
                }
                event_json(
                    &plan_allocation(&forged, &request).expect("a plan from the forged snapshot"),
                )
            },
        )
        .await,
        Expect {
            codes: &["23514"],
            message: "account_balance_natural_micro_doll",
        },
    )
}

/// A settlement whose lot update says 1,000 more was consumed than the keys lost.
pub async fn lot_totals(world: &World, fixture: &Fixture) -> Refusal {
    let _ = fixture;
    refused(
        "lot totals that do not add up",
        forged_settlement(world, fixture.attempts[&8], 0xa0, |event| {
            let change = event["changes"]
                .as_array_mut()
                .expect("changes")
                .iter_mut()
                .find(|change| change["kind"] == "funding_lot_update")
                .expect("a lot update");
            let consumed = change["row"]["consumed_micro_dollars"]
                .as_i64()
                .expect("consumed");
            change["row"]["consumed_micro_dollars"] = json!(consumed + 1000);
        })
        .await,
        Expect {
            codes: &["23000"],
            message: "does not conserve",
        },
    )
}

/// A posted entry edited in place by a superuser who has even opened an event marker.
pub async fn edit_posted_entry(world: &World) -> Refusal {
    plain(
        "a posted entry edited in place",
        world
            .admin
            .batch_execute(
                "INSERT INTO finance.event_marker (txid, event_id, event_kind) VALUES (txid_current(), decode(repeat('dd', 16), 'hex'), 'trial_granted'); \
                 UPDATE finance.journal_entry SET reason = 'correction'",
            )
            .await,
    )
}

/// A write that did not come through commit_event.
pub async fn direct_write(world: &World) -> Refusal {
    plain(
        "a write outside commit_event",
        world
            .admin
            .batch_execute("UPDATE finance.funding_lot SET revision = revision + 1")
            .await,
    )
}

/// The statement-level and row-level use of a real login that is only a member of `df_finance`.
pub async fn as_service_login(world: &World) -> Refusal {
    let client = service_client(world).await;
    plain(
        "a direct insert by the service role",
        client
            .batch_execute("INSERT INTO finance.journal_entry DEFAULT VALUES")
            .await,
    )
}

/// Every attack of the property scenario, in order; the log lines of the refused and the failures of
/// the accepted.
pub async fn run_all(world: &World, service: &mut Service) -> Result<Vec<String>, String> {
    let fixture = fixture(world, service).await?;
    let mut results: Vec<Refusal> = vec![
        unbalanced(world).await,
        unbalanced_with_settings_preset(world).await,
        forbidden_account(world).await,
        second_trial(world, service).await,
        column_outside_the_allowlist(world, service).await,
        lot_state_flipped_by_a_settlement(world, service).await,
        credit_writing_a_settlement_column(world, &fixture).await,
        charged_total_reset_in_an_allocation(world, service).await,
        forged_correlation(world, &fixture).await,
        back_dated(world).await,
        system_entry_with_an_actor(world).await,
        free_text_reason(world).await,
        overdraw(world, service).await,
        lot_totals(world, &fixture).await,
    ];
    for number in 1..=8 {
        results.push(equation(world, service, &fixture, number).await);
    }
    results.push(edit_posted_entry(world).await);
    results.push(direct_write(world).await);
    let (mut log, mut accepted) = (Vec::new(), Vec::new());
    for result in results {
        match result {
            Ok(line) => log.push(line),
            Err(failure) => accepted.push(failure),
        }
    }
    if accepted.is_empty() {
        Ok(log)
    } else {
        Err(accepted.join("; "))
    }
}
