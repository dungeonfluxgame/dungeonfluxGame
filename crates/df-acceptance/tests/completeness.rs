//! The suite cannot silently lose a release gate: every section 17 gate and every section 15.4
//! invariant of the master launch plan must have a scenario here, real or blocked.
//!
//! The gate ids and the invariant count come from the plan itself (`include_str!`), so adding a
//! row to the plan fails this test until a scenario is written. A scenario is found by its test
//! name: `<prefix>_<number>_...` for a gate, and the listed name prefix for an invariant. A real
//! test replacing a blocked scenario keeps its name, so this file never needs editing for that.

const PLAN: &str = include_str!("../../../planning/master-launch-plan.md");

const SOURCES: [&str; 12] = [
    include_str!("auth.rs"),
    include_str!("file.rs"),
    include_str!("invariants.rs"),
    include_str!("job.rs"),
    include_str!("ops.rs"),
    include_str!("pay.rs"),
    include_str!("pot.rs"),
    include_str!("provider_outcome.rs"),
    include_str!("refund.rs"),
    include_str!("save.rs"),
    include_str!("time.rs"),
    include_str!("ux.rs"),
];

const GATE_PREFIXES: [&str; 10] = [
    "PAY", "POT", "JOB", "TIME", "REF", "AUTH", "SAVE", "FILE", "OPS", "UX",
];

/// One scenario name prefix per invariant of plan section 15.4, in the plan's order.
const INVARIANT_SCENARIOS: [&str; 10] = [
    "invariant_available_funds",
    "invariant_every_settled_customer_charge",
    "invariant_every_sent_supplier_attempt",
    "invariant_every_monetary_journal",
    "invariant_a_stale_job_completion",
    "invariant_a_revoked_member",
    "invariant_every_acknowledged_game_decision",
    "invariant_a_save_branch_or_restore",
    "invariant_telemetry_failure",
    "invariant_a_saved_or_paid_message",
];

/// Whether `source` declares a scenario function whose name starts with `name_prefix`: a line,
/// outside a `//` comment, that begins (after indentation) with `fn <name_prefix>`.
fn defines_scenario(source: &str, name_prefix: &str) -> bool {
    let declaration = format!("fn {name_prefix}");
    source
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//"))
        .any(|line| line.starts_with(&declaration))
}

fn has_scenario(name_prefix: &str) -> bool {
    SOURCES
        .iter()
        .any(|source| defines_scenario(source, name_prefix))
}

fn plan_gate_ids() -> Vec<String> {
    PLAN.lines()
        .filter_map(|line| line.strip_prefix("| "))
        .filter_map(|row| row.split(" |").next())
        .filter(|cell| {
            let Some((prefix, number)) = cell.split_once('-') else {
                return false;
            };
            GATE_PREFIXES.contains(&prefix)
                && number.len() == 2
                && number.bytes().all(|byte| byte.is_ascii_digit())
        })
        .map(str::to_owned)
        .collect()
}

fn plan_invariant_count() -> usize {
    PLAN.split("## 15.4 Mandatory behavioral invariants")
        .nth(1)
        .and_then(|rest| rest.split("\n# ").next())
        .map_or(0, |section| {
            section
                .lines()
                .filter(|line| line.starts_with("- "))
                .count()
        })
}

#[test]
fn the_plan_still_lists_forty_seven_release_gates() {
    assert_eq!(plan_gate_ids().len(), 47);
}

#[test]
fn every_release_gate_of_the_plan_has_a_scenario() {
    let missing: Vec<String> = plan_gate_ids()
        .into_iter()
        .filter(|id| !has_scenario(&format!("{}_", id.to_lowercase().replace('-', "_"))))
        .collect();

    assert!(missing.is_empty(), "gates without a scenario: {missing:?}");
}

#[test]
fn the_plan_still_lists_ten_invariants() {
    assert_eq!(plan_invariant_count(), INVARIANT_SCENARIOS.len());
}

#[test]
fn every_invariant_of_the_plan_has_a_scenario() {
    let missing: Vec<&str> = INVARIANT_SCENARIOS
        .into_iter()
        .filter(|prefix| !has_scenario(prefix))
        .collect();

    assert!(
        missing.is_empty(),
        "invariants without a scenario: {missing:?}"
    );
}

#[test]
fn a_name_that_appears_only_in_a_comment_or_a_string_is_not_a_scenario() {
    let source = concat!(
        "// fn pay_99_in_a_comment\n",
        "    /// fn pay_99_in_a_doc_comment\n",
        "let text = \"fn pay_99_in_a_string\";\n",
        "let helper = other_fn pay_99_after_other_text;\n",
    );

    assert!(!defines_scenario(source, "pay_99_"));
}

#[test]
fn an_indented_test_function_and_a_scenario_macro_name_both_count() {
    let source = concat!(
        "#[test]\n",
        "fn pay_01_a_real_test() {}\n",
        "blocked_scenario! {\n",
        "    fn pay_02_blocked,\n",
        "}\n",
    );

    assert!(defines_scenario(source, "pay_01_"));
    assert!(defines_scenario(source, "pay_02_"));
}
