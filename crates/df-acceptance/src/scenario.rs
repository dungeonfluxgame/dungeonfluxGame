/// Declares a section 17 scenario whose owning API does not exist yet.
///
/// The test is ignored with the missing API as the reason. Running it with `--ignored` fails and
/// prints the setup, steps and required outcome, so the owner sees exactly what will be checked.
/// Replace the whole invocation with a real test when the API lands; never edit the outcome text
/// to make a scenario easier. `outcome` is the plan's "Required outcome" column.
#[macro_export]
macro_rules! blocked_scenario {
    (
        fn $name:ident,
        blocked_on: $reason:literal,
        setup: [$($setup:literal),+ $(,)?],
        steps: [$($step:literal),+ $(,)?],
        outcome: $outcome:literal $(,)?
    ) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            panic!(
                "{}",
                concat!(
                    "scenario not implemented, ", $reason, "\n",
                    "setup:\n", $("  - ", $setup, "\n",)+
                    "steps:\n", $("  - ", $step, "\n",)+
                    "required outcome: ", $outcome,
                )
            );
        }
    };
}
