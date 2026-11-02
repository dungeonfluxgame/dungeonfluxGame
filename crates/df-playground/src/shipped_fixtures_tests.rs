//! The fixtures shipped in `assets/playground/fixtures` must read, and all but the one that exists
//! to break the rules must satisfy the choice-list contract.

use std::path::PathBuf;

use crate::choices::build;
use crate::fixture::Fixture;

/// The fixture that breaks the contract on purpose, so the warnings have something to show.
const DELIBERATELY_BROKEN: &str = "contract_breaks.json";

fn fixtures() -> Vec<(String, Fixture)> {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/fixtures");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("the fixture directory exists") {
        let path = entry.expect("reads an entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let text = std::fs::read_to_string(&path).expect("reads the fixture");
            let name = path
                .file_name()
                .expect("has a name")
                .to_string_lossy()
                .into_owned();
            let fixture = Fixture::parse(&text).unwrap_or_else(|error| {
                panic!(
                    "{name}, line {}, column {}: {}",
                    error.line, error.column, error.message
                )
            });
            found.push((name, fixture));
        }
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    found
}

#[test]
fn every_shipped_fixture_reads() {
    assert!(fixtures().len() >= 4);
}

#[test]
fn every_shipped_fixture_but_the_deliberate_one_follows_the_choice_rules() {
    for (name, fixture) in fixtures() {
        let warnings = build(&fixture.phone, true).warnings;
        if name == DELIBERATELY_BROKEN {
            assert!(!warnings.is_empty(), "{name} should break the rules");
        } else {
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
        }
    }
}
