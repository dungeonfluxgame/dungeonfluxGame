//! The flexibility rules of O.5, enforced over every module that implements `Machine`: a machine
//! imports no other machine, and it never names a port that reads time, draws dice, calls a model
//! or the network, or writes events. The scan finds machines by `impl Machine for`, so a new
//! machine is covered the moment it is written, and it reads the source with comments and string
//! literals blanked, as the architecture scan does, so prose and machine ids do not count.

use std::fs;
use std::path::{Path, PathBuf};

/// Port traits and port calls a machine is never given. `Dice` and `RulesOwner` are allowed only
/// as `MachineInput::Dice` and `PortName::RulesOwner`, the value a machine receives and the name
/// it declares.
const FORBIDDEN_NAMES: [&str; 9] = [
    "Clock",
    "ManualClock",
    "Broker",
    "Interpreter",
    "Narrator",
    "EventSink",
    "KnowledgeView",
    "prepare",
    "legal_choices",
];

fn blank_comments_and_literals(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut blanked = String::with_capacity(source.len());
    let mut index = 0;
    while index < characters.len() {
        let current = characters[index];
        let next = characters.get(index + 1).copied();
        if current == '/' && next == Some('/') {
            while index < characters.len() && characters[index] != '\n' {
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            index += 2;
            while index + 1 < characters.len()
                && !(characters[index] == '*' && characters[index + 1] == '/')
            {
                index += 1;
            }
            index += 2;
            blanked.push(' ');
        } else if current == 'r'
            && matches!(next, Some('"' | '#'))
            && is_raw_string(&characters, index)
        {
            index = raw_string_end(&characters, index);
            blanked.push(' ');
        } else if current == '"' {
            index += 1;
            while index < characters.len() && characters[index] != '"' {
                index += if characters[index] == '\\' { 2 } else { 1 };
            }
            index += 1;
            blanked.push(' ');
        } else if current == '\'' && is_character_literal(&characters, index) {
            index += 1;
            while index < characters.len() && characters[index] != '\'' {
                index += if characters[index] == '\\' { 2 } else { 1 };
            }
            index += 1;
            blanked.push(' ');
        } else {
            blanked.push(current);
            index += 1;
        }
    }
    blanked
}

fn is_raw_string(characters: &[char], start: usize) -> bool {
    let previous_is_word =
        start > 0 && (characters[start - 1].is_alphanumeric() || characters[start - 1] == '_');
    let mut cursor = start + 1;
    while characters.get(cursor) == Some(&'#') {
        cursor += 1;
    }
    !previous_is_word && characters.get(cursor) == Some(&'"')
}

fn raw_string_end(characters: &[char], start: usize) -> usize {
    let mut cursor = start + 1;
    let mut hashes = 0;
    while characters.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    cursor += 1;
    while cursor < characters.len() {
        let closes = characters[cursor] == '"'
            && (1..=hashes).all(|offset| characters.get(cursor + offset) == Some(&'#'));
        if closes {
            return cursor + 1 + hashes;
        }
        cursor += 1;
    }
    characters.len()
}

fn is_character_literal(characters: &[char], start: usize) -> bool {
    match characters.get(start + 1) {
        Some('\\') => true,
        Some(_) => characters.get(start + 2) == Some(&'\''),
        None => false,
    }
}

fn tokens(code: &str) -> Vec<&str> {
    code.split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .collect()
}

/// Whether the source implements `Machine`.
fn implements_machine(source: &str) -> bool {
    let code = blank_comments_and_literals(source);
    tokens(&code)
        .windows(3)
        .any(|window| window == ["impl", "Machine", "for"])
}

/// What a machine module may not do, as readable findings; empty when it is clean. `own` is the
/// module's name and `machines` the names of every machine module.
fn violations(own: &str, machines: &[String], source: &str) -> Vec<String> {
    let code = blank_comments_and_literals(source);
    let words = tokens(&code);
    let mut findings = Vec::new();
    let is_other_machine =
        |word: &str| word != own && machines.iter().any(|machine| machine == word);
    for statement in code.split(';') {
        let statement_words = tokens(statement);
        if statement_words.first() == Some(&"use") {
            for word in statement_words.iter().filter(|word| is_other_machine(word)) {
                findings.push(format!("imports the machine module `{word}`"));
            }
        }
    }
    for (index, word) in words.iter().enumerate() {
        let after = |expected: &str| index > 0 && words[index - 1] == expected;
        let in_path = index > 0 && matches!(words[index - 1], "crate" | "super" | "self");
        if in_path && is_other_machine(word) {
            findings.push(format!("names the machine module `{word}` by path"));
        }
        if FORBIDDEN_NAMES.contains(word) {
            findings.push(format!("names `{word}`"));
        }
        if *word == "Dice" && !after("MachineInput") {
            findings.push("names the `Dice` port".to_owned());
        }
        if *word == "RulesOwner" && !after("PortName") {
            findings.push("names the `RulesOwner` port".to_owned());
        }
    }
    findings
}

fn source_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).expect("the source directory is readable");
    for entry in entries {
        let path = entry.expect("a directory entry reads").path();
        if path.is_dir() {
            source_files(&path, found);
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
            found.push(path);
        }
    }
}

fn machine_modules() -> Vec<(String, String)> {
    let mut files = Vec::new();
    source_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut machines = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("a source file reads");
        if implements_machine(&text) {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned();
            machines.push((stem, text));
        }
    }
    machines
}

#[test]
fn the_scan_finds_the_lobby_and_explore_and_any_machine_added_later() {
    let names: Vec<String> = machine_modules()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(names.contains(&"lobby".to_owned()), "{names:?}");
    assert!(names.contains(&"explore".to_owned()), "{names:?}");
}

#[test]
fn no_machine_module_imports_another_machine_or_names_a_port_it_is_not_given() {
    let machines = machine_modules();
    let names: Vec<String> = machines.iter().map(|(name, _)| name.clone()).collect();
    for (name, source) in &machines {
        let findings = violations(name, &names, source);
        assert!(findings.is_empty(), "{name}.rs {findings:?}");
    }
}

#[test]
fn the_scan_catches_each_way_a_machine_can_break_the_rules() {
    let machines = vec!["lobby".to_owned(), "explore".to_owned(), "rest".to_owned()];
    let breaking = [
        ("use crate::{lobby::LobbyMachine};", "lobby"),
        ("use super::explore;", "explore"),
        ("use crate::explore::AuthoredBeat;", "explore"),
        ("fn roll(dice: &dyn Dice) {}", "Dice"),
        ("use crate::ports::{Clock, Dice};", "Clock"),
        ("fn ask(rules: &dyn RulesOwner) {}", "RulesOwner"),
        ("fn f(b: &dyn Broker) {}", "Broker"),
        ("rules.prepare(a, b, c);", "prepare"),
    ];
    for (source, expected) in breaking {
        let findings = violations("rest", &machines, source);
        assert!(
            findings.iter().any(|finding| finding.contains(expected)),
            "{source:?} was not caught as {expected:?}: {findings:?}"
        );
    }
}

#[test]
fn the_scan_ignores_prose_strings_machine_ids_and_the_allowed_names() {
    let machines = vec!["lobby".to_owned(), "explore".to_owned(), "rest".to_owned()];
    let clean = [
        "// a lobby, an explore and a Dice and a Clock, all in prose",
        "/// docs: `use crate::lobby`, never the Broker",
        "let next = \"explore\"; let text = \"Dice Clock prepare\";",
        "let text = r#\"use crate::lobby::LobbyMachine\"#;",
        "let input = MachineInput::Dice { pending, draws };",
        "let port = PortName::RulesOwner;",
        "let lobby_machine = LobbyMachine::new();",
    ];
    for source in clean {
        assert_eq!(
            violations("rest", &machines, source),
            Vec::<String>::new(),
            "{source:?}"
        );
    }
    assert!(implements_machine("impl Machine for Rest { }"));
    assert!(!implements_machine(
        "// impl Machine for Rest\nlet s = \"impl Machine for\";"
    ));
}
