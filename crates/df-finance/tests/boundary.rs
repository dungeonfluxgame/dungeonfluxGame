#![cfg(not(target_arch = "wasm32"))]

//! The database boundary under attack: the service role has no write privilege, writes outside
//! `commit_event` are refused for everyone, the allowlist and the equations refuse what no honest
//! planner would send, an update needs its lock and its revision, a closed period stays closed,
//! and the hash chain Rust computes is the one the database stores.

mod support;

use df_commerce::digest::entry_hash_v1;
use df_commerce::entry_rules::RULES;
use df_commerce::finance_ids::EventId;
use df_commerce::funding_flows::plan_trial;
use df_commerce::ledger::{PostingContext, canonical_v1};
use df_commerce::lifecycle::plan_quote_decision;
use df_commerce::plan::ChangeKind;
use df_finance::wire::{event_json, parse_quote_row, parse_trial_snapshot};
use serde_json::{Value, json};
use support::attack::{fee_event, forged_trial, run_event, service_client, trial_request};
use support::invariants;
use support::sqlstate;
use support::world::{CURRENCY, World, payer, purchase};

fn code_of(error: &tokio_postgres::Error) -> String {
    sqlstate(error).unwrap_or_else(|| format!("not a database error: {error:?}"))
}

#[test]
fn the_service_role_holds_no_write_privilege_and_cannot_take_the_definer_role() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        for sql in [
            "UPDATE finance.funding_lot SET revision = revision",
            "DELETE FROM finance.journal_line",
            "UPDATE finance.account_balance SET natural_micro_dollars = 0",
            "INSERT INTO finance.journal_entry DEFAULT VALUES",
            "DELETE FROM finance.event_marker",
            "INSERT INTO finance.check_mark VALUES (1, 'x')",
            "SELECT finance.already_checked('x')",
            "TRUNCATE finance.journal_entry",
            "SELECT finance.apply_change('trial_granted', '{}'::jsonb)",
            "ALTER FUNCTION finance.commit_event(jsonb) OWNER TO df_finance",
        ] {
            let client = service_client(&world).await;
            let error = client.batch_execute(sql).await.expect_err(sql);
            assert_eq!(code_of(&error), "42501", "{sql}");
        }
        // Neither service role is a member of the definer role or of the other.
        let members: i64 = world
            .admin
            .query_one(
                "SELECT count(*) FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid JOIN pg_roles u ON u.oid = m.member                   WHERE u.rolname IN ('df_finance', 'df_finance_replay', 'df_finance_definer')                     AND r.rolname IN ('df_finance', 'df_finance_replay', 'df_finance_definer')",
                &[],
            )
            .await
            .expect("memberships")
            .get(0);
        assert_eq!(members, 0);
        world.finish().await;
    });
}

#[test]
fn a_write_outside_commit_event_is_refused_even_for_the_superuser() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), [1; 32], CURRENCY)
            .await
            .expect("trial")
            .expect("granted");
        let entries = world.count("journal_entry").await;
        for sql in [
            "UPDATE finance.funding_lot SET revision = revision + 1",
            "INSERT INTO finance.account_key (code, currency) VALUES (1000, 'USD')",
            "UPDATE finance.account_balance SET natural_micro_dollars = natural_micro_dollars + 1",
            "UPDATE finance.journal_entry SET reason = 'edited'",
            "DELETE FROM finance.journal_line",
            "UPDATE finance.journal_line SET amount_micro_dollars = amount_micro_dollars",
            "TRUNCATE finance.journal_entry CASCADE",
        ] {
            let error = world.admin.batch_execute(sql).await.expect_err(sql);
            assert!(sqlstate(&error).is_some(), "{sql}: {error:?}");
        }
        assert_eq!(world.count("journal_entry").await, entries);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn an_unbalanced_entry_is_refused_and_leaves_nothing_behind() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        run_event(&world, None, |_| fee_event(1, 100, 100))
            .await
            .expect("a balanced fee entry");
        let before = (
            world.count("journal_entry").await,
            world.count("journal_line").await,
            world.count("event_marker").await,
        );
        let error = run_event(&world, None, |_| fee_event(2, 100, 90))
            .await
            .expect_err("unbalanced");
        assert!(sqlstate(&error).is_some(), "{error:?}");
        assert_eq!(
            (
                world.count("journal_entry").await,
                world.count("journal_line").await,
                world.count("event_marker").await
            ),
            before
        );
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn the_allowlist_and_the_envelope_refuse_what_no_honest_planner_sends() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        type Mutation = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Mutation)> = vec![
            (
                "a revenue account in a fee entry",
                Box::new(|event| {
                    event["entries"][0]["lines"][0]["code"] = json!(4110);
                    event["changes"].as_array_mut().expect("changes").push(json!({"kind": "key_create", "row": {"code": 4110, "currency": "USD", "processor": "fake"}}));
                    event["entries"][0]["lines"][0]["processor"] = Value::Null;
                    event["changes"][2]["row"]
                        .as_object_mut()
                        .expect("row")
                        .remove("processor");
                }),
            ),
            (
                "an entry without a reason",
                Box::new(|event| event["entries"][0]["reason"] = Value::Null),
            ),
            (
                "an operator posting a system kind",
                Box::new(|event| event["entries"][0]["actor_kind"] = json!("operator")),
            ),
            (
                "the wrong stream",
                Box::new(|event| event["entries"][0]["stream_kind"] = json!("payer")),
            ),
            (
                "an entry kind the event kind does not carry",
                Box::new(|event| event["entries"][0]["kind"] = json!("trial_grant")),
            ),
            (
                "a missing external reference",
                Box::new(|event| event["entries"][0]["external_ref"] = Value::Null),
            ),
            (
                "the wrong external reference kind",
                Box::new(|event| event["entries"][0]["external_ref_kind"] = json!("none")),
            ),
            (
                "an unknown entry kind",
                Box::new(|event| event["entries"][0]["kind"] = json!("mint")),
            ),
            (
                "an unknown event kind",
                Box::new(|event| event["event_kind"] = json!("anything_goes")),
            ),
            (
                "an unsupported payload schema",
                Box::new(|event| event["payload_schema"] = json!(2)),
            ),
            (
                "an event kind that carries entries with none",
                Box::new(|event| event["entries"] = json!([])),
            ),
            (
                "a zero line",
                Box::new(|event| event["entries"][0]["lines"][0]["amount"] = json!(0)),
            ),
            (
                "a change the event kind does not carry",
                Box::new(|event| {
                    event["changes"] = json!([{"kind": "funding_lot_insert", "row": {}}]);
                }),
            ),
        ];
        for (name, mutate) in cases {
            let error = run_event(&world, None, |_| {
                let mut event = fee_event(7, 100, 100);
                mutate(&mut event);
                event
            })
            .await
            .expect_err(name);
            assert_eq!(code_of(&error), "FN003", "{name}: {error:?}");
        }
        assert_eq!(world.count("journal_entry").await, 0);
        // A key the entry names but nobody created is refused, not made up.
        let error = run_event(&world, None, |_| {
            let mut event = fee_event(8, 100, 100);
            event["changes"] = json!([]);
            event
        })
        .await
        .expect_err("no keys");
        assert_eq!(code_of(&error), "FN005", "{error:?}");
        world.finish().await;
    });
}

#[test]
fn a_manual_adjustment_cannot_touch_customer_credit() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        let lot = world.lots_of(payer(1)).await[0];
        let entries = world.count("journal_entry").await;
        let manual = |credit_code: i64, credit_extra: Value| {
            let mut credit = json!({
                "code": credit_code, "currency": "USD", "payer": null, "lot": null, "allocation": null,
                "provider": null, "processor": null, "loss_class": null, "jurisdiction": null,
                "promo_source": null, "amount": -1000,
            });
            for (key, value) in credit_extra.as_object().expect("object") {
                credit[key] = value.clone();
            }
            json!({
                "event_id": "aa000000000000000000000000000001",
                "event_kind": "manual_adjustment_posted",
                "payload_schema": 1,
                "changes": [],
                "entries": [{
                    "entry_id": "aa000000000000000000000000000002",
                    "kind": "manual_adjustment",
                    "business_key": "aa".repeat(32),
                    "external_ref_kind": "support_case",
                    "external_ref": "case-9",
                    "stream_kind": "platform",
                    "stream_id": "99".repeat(16),
                    "effective_at_ms": null,
                    "reason": "manual_adjustment",
                    "actor_kind": "operator",
                    "actor_id": "bb".repeat(16),
                    "approver_id": null,
                    "case_ref": "case-9",
                    "campaign_id": null, "session_id": null, "attempt_id": null, "reservation_id": null,
                    "price_version_id": null,
                    "lines": [
                        {"code": 4110, "currency": "USD", "payer": null, "lot": null, "allocation": null,
                         "provider": null, "processor": null, "loss_class": null, "jurisdiction": null,
                         "promo_source": null, "amount": 1000},
                        credit,
                    ],
                }],
            })
        };
        // Creating spendable credit out of nothing: Dr revenue, Cr the payer's available credit.
        let error = run_event(&world, None, |_| {
            manual(2110, json!({"payer": payer(1).hex(), "lot": lot.hex()}))
        })
        .await
        .expect_err("customer credit");
        assert_eq!(code_of(&error), "FN003", "{error:?}");
        assert_eq!(world.count("journal_entry").await, entries);
        world.finish().await;
    });
}

#[test]
fn the_event_cannot_be_replayed_and_a_business_key_is_posted_once() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        run_event(&world, None, |_| fee_event(3, 100, 100))
            .await
            .expect("first");
        let replay = run_event(&world, None, |_| fee_event(3, 100, 100))
            .await
            .expect_err("replay");
        assert_eq!(code_of(&replay), "23505", "{replay:?}");
        // Another event, the same business key: the same economic fact twice.
        let twice = run_event(&world, None, |_| {
            let mut event = fee_event(3, 100, 100);
            event["event_id"] = json!("cc000000000000000000000000000001");
            event["entries"][0]["entry_id"] = json!("cc000000000000000000000000000002");
            event
        })
        .await
        .expect_err("business key");
        assert_eq!(code_of(&twice), "23505", "{twice:?}");
        assert_eq!(world.count("journal_entry").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_semantic_mint_is_refused_by_the_lot_conservation_checks() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        // A trial grant whose entry says 15 dollars of credit while its lot says 10.
        let error = forged_trial(&world, 1, |event| {
            let lines = event["entries"][0]["lines"].as_array_mut().expect("lines");
            lines[0]["amount"] = json!(15_000_000);
            lines[1]["amount"] = json!(-15_000_000);
        })
        .await
        .expect_err("mint");
        assert!(sqlstate(&error).is_some(), "{error:?}");
        assert_eq!(world.count("journal_entry").await, 0);
        assert_eq!(world.count("funding_lot").await, 0);
        world.finish().await;
    });
}

#[test]
fn an_update_needs_the_flow_lock_and_the_revision_it_read() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        let command = world.admit_command(1, 1000, 1000);
        let quote = service
            .create_quote(
                &world.version,
                df_finance::held::QuoteCommand {
                    session: world.session,
                    route: command.route.clone(),
                    work_class: command.work_class,
                    maximum_units: command.maximum_units.clone(),
                    estimate_units: command.estimate_units.clone(),
                    request_digest: command.request_digest,
                    lifetime_ms: 600_000,
                },
            )
            .await
            .expect("quote");
        let now_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_millis(),
        )
        .expect("ms");
        // The decision as planned from a snapshot read under the quote's lock.
        let stale_event: Value;
        let args = json!({"session": world.session.hex(), "quote": quote.hex()});
        {
            let mut client = service_client(&world).await;
            let transaction = client.transaction().await.expect("begin");
            let row = transaction
                .query_one(
                    "SELECT finance.lock_flow('quote', $1::text::jsonb)::text",
                    &[&args.to_string()],
                )
                .await
                .expect("lock");
            let snapshot: Value = serde_json::from_str(&row.get::<_, String>(0)).expect("json");
            let quote_row = parse_quote_row(&snapshot["quote"])
                .expect("wire")
                .expect("a quote");
            let plan = plan_quote_decision(
                EventId::from_array([0xd1; 16]),
                world.session,
                &quote_row,
                true,
                now_ms,
            )
            .expect("plan");
            stale_event = event_json(&plan);
            transaction.rollback().await.expect("rollback");
        }
        // Without the lock set of this transaction, the update is refused.
        let mut client = service_client(&world).await;
        let transaction = client.transaction().await.expect("begin");
        let error = transaction
            .query_one(
                "SELECT finance.commit_event($1::text::jsonb)::text",
                &[&stale_event.to_string()],
            )
            .await
            .expect_err("no lock set");
        assert_eq!(code_of(&error), "FN002", "{error:?}");
        drop(transaction);
        // Someone else decides first; the old plan now carries a revision that is gone.
        service
            .decide_quote(world.session, quote, true)
            .await
            .expect("decided");
        let error = run_event(&world, Some(("quote", args)), |_| stale_event.clone())
            .await
            .expect_err("stale");
        assert_eq!(code_of(&error), "FN001", "{error:?}");
        world.finish().await;
    });
}

#[test]
fn a_closed_period_refuses_posting_and_is_never_reopened() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let period: i32 = world
            .admin
            .query_one("SELECT finance.period_for(finance.now_ms())", &[])
            .await
            .expect("period")
            .get(0);
        world
            .admin
            .execute(
                "UPDATE finance.accounting_period SET status = 'closed', closed_at_ms = 1, digest = decode(repeat('ab', 32), 'hex') WHERE period_id = $1",
                &[&period],
            )
            .await
            .expect("close the period");
        let error = run_event(&world, None, |_| fee_event(4, 100, 100))
            .await
            .expect_err("closed");
        assert_eq!(code_of(&error), "FN006", "{error:?}");
        for sql in [
            "UPDATE finance.accounting_period SET status = 'open', closed_at_ms = NULL, digest = NULL",
            "DELETE FROM finance.accounting_period",
        ] {
            let error = world.admin.batch_execute(sql).await.expect_err(sql);
            assert!(sqlstate(&error).is_some(), "{sql}");
        }
        let status: String = world
            .admin
            .query_one(
                "SELECT status FROM finance.accounting_period WHERE period_id = $1",
                &[&period],
            )
            .await
            .expect("period")
            .get(0);
        assert_eq!(status, "closed");
        world.finish().await;
    });
}

#[test]
fn the_hash_chain_rust_computes_is_the_one_the_database_stores() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let request = trial_request(1);
        let mut planned = None;
        run_event(
            &world,
            Some(("trial", json!({"payer": payer(1).hex()}))),
            |snapshot| {
                let plan = plan_trial(parse_trial_snapshot(snapshot), request).expect("plan");
                planned = Some(plan.entries[0].clone());
                event_json(&plan)
            },
        )
        .await
        .expect("trial");
        let entry = planned.expect("entry");
        let row = world
            .admin
            .query_one(
                "SELECT stream_seq, recorded_at_ms, period_id, prev_hash, entry_hash, entry_hash_version \
                 FROM finance.journal_entry WHERE entry_id = $1",
                &[&entry.entry_id.as_bytes().to_vec()],
            )
            .await
            .expect("stored entry");
        let (seq, recorded, period): (i64, i64, i32) = (row.get(0), row.get(1), row.get(2));
        let prev: Vec<u8> = row.get(3);
        let stored: Vec<u8> = row.get(4);
        assert_eq!(row.get::<_, i16>(5), 1);
        assert_eq!(
            prev,
            vec![0_u8; 32],
            "the first entry of a stream chains to the zero hash"
        );
        let canonical = canonical_v1(
            &entry,
            PostingContext {
                event_id: request.event_id,
                stream_seq: seq,
                recorded_at_ms: recorded,
                period_id: i64::from(period),
            },
        );
        let in_database: Vec<u8> = world
            .admin
            .query_one(
                "SELECT finance.entry_canonical_v1(finance.entry_json($1))",
                &[&entry.entry_id.as_bytes().to_vec()],
            )
            .await
            .expect("canonical")
            .get(0);
        assert_eq!(
            canonical, in_database,
            "the canonical bytes are the same in Rust and in SQL"
        );
        let prev32: [u8; 32] = prev.try_into().expect("32 bytes");
        assert_eq!(
            entry_hash_v1(&prev32, &canonical).to_vec(),
            stored,
            "the stored hash is the Rust hash"
        );
        // And a second entry in the same stream chains to the first.
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        invariants::chains(&world.admin).await.expect("chains");
        let linked: i64 = world
            .admin
            .query_one(
                "SELECT count(*) FROM finance.journal_entry b JOIN finance.journal_entry a \
                   ON a.stream_kind = b.stream_kind AND a.stream_id = b.stream_id AND a.stream_seq = b.stream_seq - 1 \
                  WHERE b.prev_hash = a.entry_hash",
                &[],
            )
            .await
            .expect("links")
            .get(0);
        assert_eq!(linked, 1);
        world.finish().await;
    });
}

#[test]
fn the_entry_kind_seed_in_the_database_is_the_table_in_rust() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let rows = world
            .admin
            .query(
                "SELECT kind, stream_rule, reversible FROM finance.entry_kind ORDER BY kind",
                &[],
            )
            .await
            .expect("kinds");
        assert_eq!(rows.len(), RULES.len());
        let seeded: Vec<(String, i32, i16)> = world
            .admin
            .query("SELECT kind, code, side FROM finance.entry_kind_rule", &[])
            .await
            .expect("rules")
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>(0),
                    i32::from(row.get::<_, i16>(1)),
                    row.get::<_, i16>(2),
                )
            })
            .collect();
        for rule in &RULES {
            let kind = rule.kind.label();
            let stream = rows
                .iter()
                .find(|row| row.get::<_, String>(0) == kind)
                .unwrap_or_else(|| panic!("kind {kind} is not seeded"));
            let want_stream = rule.stream.map_or("original", |stream| stream.label());
            assert_eq!(stream.get::<_, String>(1), want_stream, "{kind}: stream");
            assert_eq!(
                stream.get::<_, bool>(2),
                rule.reversible,
                "{kind}: reversible"
            );
            let codes = |side: i16| -> Vec<i32> {
                let mut codes: Vec<i32> = seeded
                    .iter()
                    .filter(|(k, _, s)| k == kind && *s == side)
                    .map(|(_, code, _)| *code)
                    .collect();
                codes.sort_unstable();
                codes
            };
            let mut debits: Vec<i32> = rule
                .debits
                .iter()
                .map(|code| i32::from(code.number()))
                .collect();
            let mut credits: Vec<i32> = rule
                .credits
                .iter()
                .map(|code| i32::from(code.number()))
                .collect();
            debits.sort_unstable();
            credits.sort_unstable();
            if rule.kind != df_commerce::ledger::EntryKind::Reversal {
                assert_eq!(codes(1), debits, "{kind}: debit accounts");
                assert_eq!(codes(-1), credits, "{kind}: credit accounts");
            }
        }
        world.finish().await;
    });
}

#[test]
fn the_reason_codes_in_the_database_are_the_codes_in_rust() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut stored: Vec<String> = world
            .admin
            .query("SELECT code FROM finance.entry_reason", &[])
            .await
            .expect("reasons")
            .iter()
            .map(|row| row.get(0))
            .collect();
        let mut known: Vec<String> = df_commerce::ledger::EntryReason::ALL
            .iter()
            .map(|reason| reason.label().to_owned())
            .collect();
        stored.sort();
        known.sort();
        assert_eq!(stored, known);
        world.finish().await;
    });
}

#[test]
fn the_change_kinds_in_rust_are_the_change_kinds_in_the_registry() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut database: Vec<String> = world
            .admin
            .query("SELECT kind FROM finance.change_kind", &[])
            .await
            .expect("registry")
            .iter()
            .map(|row| row.get(0))
            .collect();
        database.sort();
        let mut planner: Vec<String> = ChangeKind::ALL
            .iter()
            .map(|kind| kind.label().to_owned())
            .collect();
        planner.sort();
        assert_eq!(
            planner, database,
            "a kind the planner can name is in the registry, and the registry names none the planner cannot"
        );
        world.finish().await;
    });
}

/// Every generated `apply_*` function is called with a minimal row. The call may fail on a
/// constraint, but never at planning: a column list the table rejects (an identity or generated
/// column, a name that does not exist) is an error of class 42 the first time the statement is
/// planned, which is when a flow would meet it in production.
#[test]
fn every_generated_apply_function_plans_with_a_minimal_row() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let kinds = world
            .admin
            .query(
                "SELECT kind, op FROM finance.change_kind WHERE op IN ('insert', 'update') ORDER BY kind",
                &[],
            )
            .await
            .expect("registry");
        assert!(kinds.len() > 20, "the registry has its generated kinds");
        let mut broken = Vec::new();
        for row in &kinds {
            let kind: String = row.get(0);
            let op: String = row.get(1);
            let exists: bool = world
                .admin
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace                       WHERE n.nspname = 'finance' AND p.proname = 'apply_' || $1)",
                    &[&kind],
                )
                .await
                .expect("catalog")
                .get(0);
            if !exists {
                broken.push(format!("{kind}: no generated function"));
                continue;
            }
            assert!(
                kind.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
                "a change kind is a plain identifier: {kind}"
            );
            world.admin.batch_execute("BEGIN").await.expect("begin");
            let outcome = if op == "insert" {
                world
                    .admin
                    .query(&format!("SELECT finance.apply_{kind}('{{}}'::jsonb)"), &[])
                    .await
                    .map(|_| ())
            } else {
                world
                    .admin
                    .query(
                        &format!("SELECT finance.apply_{kind}('{{}}'::jsonb, 0)"),
                        &[],
                    )
                    .await
                    .map(|_| ())
            };
            world
                .admin
                .batch_execute("ROLLBACK")
                .await
                .expect("rollback");
            if let Err(error) = outcome {
                let code = code_of(&error);
                if !(code.starts_with("23") || code.starts_with("FN") || code == "42501") {
                    broken.push(format!("{kind}: {code} {error}"));
                }
            }
        }
        assert!(
            broken.is_empty(),
            "generated functions that do not plan: {broken:#?}"
        );
        world.finish().await;
    });
}
