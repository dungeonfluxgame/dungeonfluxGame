//! The crate stays pure: no async, no clock, no randomness, no files, no network, no threads, no
//! channels, no terminal output, no environment or process access, no panicking shortcut in
//! production code, and no other dependency than serde and serde_json (O.2, O.7, question 87).
//! The scan reads the crate's own non-test source with comments and string literals blanked out,
//! so prose may say "async" while code may not use it.

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN_IDENTIFIERS: [&str; 6] =
    ["tokio", "async", "await", "Instant", "SystemTime", "rand"];
const FORBIDDEN_IDENTIFIER_FRAGMENTS: [&str; 1] = ["unbounded"];
const FORBIDDEN_PATHS: [&str; 13] = [
    "std::fs",
    "std::net",
    "std::env",
    "std::process",
    "std::io",
    "std::thread",
    "thread::",
    "mpsc::channel",
    "println!",
    "eprintln!",
    "print!",
    "eprint!",
    "dbg!",
];
const FORBIDDEN_STD_GROUP_MEMBERS: [&str; 6] = ["fs", "net", "env", "process", "io", "thread"];
const PANICKING_SHORTCUTS: [&str; 6] = [
    ".unwrap()",
    ".expect(",
    "panic!",
    "unreachable!",
    "todo!",
    "unimplemented!",
];
/// Production files whose existing `expect` calls predate C0, with how many each may keep. The
/// count may only go down. `testkit.rs` is test support that panics by design and is not scanned.
const PANICKING_SHORTCUT_ALLOWANCE: [(&str, usize); 1] = [("ecs.rs", 3)];
const TEST_SUPPORT_FILE: &str = "testkit.rs";
const ALLOWED_DEPENDENCIES: [&str; 2] = ["serde", "serde_json"];

/// Replaces comments, string literals and character literals with spaces, keeping everything else.
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
        } else if let Some(end) = raw_string_end(&characters, index) {
            index = end;
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

/// The index just past a raw string that starts at `start`, or `None` when none starts there.
fn raw_string_end(characters: &[char], start: usize) -> Option<usize> {
    if characters[start] != 'r' || (start > 0 && is_identifier_character(characters[start - 1])) {
        return None;
    }
    let mut cursor = start + 1;
    let mut hashes = 0;
    while characters.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    if characters.get(cursor) != Some(&'"') {
        return None;
    }
    cursor += 1;
    while cursor < characters.len() {
        let closes = characters[cursor] == '"'
            && (1..=hashes).all(|offset| characters.get(cursor + offset) == Some(&'#'));
        if closes {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Some(characters.len())
}

/// A quote after an apostrophe is a character literal, not a lifetime, when it closes at once or
/// starts with a backslash.
fn is_character_literal(characters: &[char], start: usize) -> bool {
    match characters.get(start + 1) {
        Some('\\') => true,
        Some(_) => characters.get(start + 2) == Some(&'\''),
        None => false,
    }
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// What a source breaks, as readable findings; empty when it is clean.
fn violations_in(source: &str) -> Vec<String> {
    let code = blank_comments_and_literals(source);
    let mut findings = Vec::new();
    for token in code
        .split(|character: char| !is_identifier_character(character))
        .filter(|token| !token.is_empty())
    {
        if FORBIDDEN_IDENTIFIERS.contains(&token) {
            findings.push(format!("identifier `{token}`"));
        }
        for fragment in FORBIDDEN_IDENTIFIER_FRAGMENTS {
            if token.contains(fragment) {
                findings.push(format!("identifier `{token}` mentions `{fragment}`"));
            }
        }
    }
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let tidy = tidy_punctuation(&code);
    for path in FORBIDDEN_PATHS {
        if compact.contains(path) {
            findings.push(format!("`{path}`"));
        }
    }
    for group in tidy.split("std::{").skip(1) {
        let members = group.split('}').next().unwrap_or_default();
        for member in members.split(',') {
            let name: String = member
                .chars()
                .take_while(|character| is_identifier_character(*character))
                .collect();
            if FORBIDDEN_STD_GROUP_MEMBERS.contains(&name.as_str()) {
                findings.push(format!("`std::{{{name}}}` import"));
            }
        }
    }
    findings
}

/// Collapses whitespace to single spaces and drops the ones next to `:`, `{`, `}` and `,`, so
/// `std :: { fs as files }` reads as `std::{fs as files}` with its members still separate.
fn tidy_punctuation(code: &str) -> String {
    let collapsed: Vec<char> = code
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let glued = |character: char| matches!(character, ':' | '{' | '}' | ',');
    let mut tidy = String::with_capacity(collapsed.len());
    for (index, character) in collapsed.iter().enumerate() {
        let before = index.checked_sub(1).and_then(|at| collapsed.get(at));
        let after = collapsed.get(index + 1);
        let beside_glue = before.is_some_and(|c| glued(*c)) || after.is_some_and(|c| glued(*c));
        if *character == ' ' && beside_glue {
            continue;
        }
        tidy.push(*character);
    }
    tidy
}

/// Calls that can panic on a runtime path, as one finding per call.
fn panicking_shortcuts_in(source: &str) -> Vec<String> {
    let compact: String = blank_comments_and_literals(source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    PANICKING_SHORTCUTS
        .iter()
        .flat_map(|shortcut| {
            compact
                .matches(shortcut)
                .map(move |_| format!("`{shortcut}`"))
        })
        .collect()
}

fn non_test_sources(directory: &Path, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).expect("the source directory is readable");
    for entry in entries {
        let path = entry.expect("a directory entry reads").path();
        if path.is_dir() {
            non_test_sources(&path, found);
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

#[test]
fn the_crate_source_uses_no_async_clock_randomness_file_network_or_print() {
    let mut sources = Vec::new();
    non_test_sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(
        sources.len() > 10,
        "the scan found only {} files",
        sources.len()
    );
    let mut report = Vec::new();
    for path in &sources {
        let text = fs::read_to_string(path).expect("a source file reads");
        for finding in violations_in(&text) {
            report.push(format!("{}: {finding}", path.display()));
        }
    }
    assert!(
        report.is_empty(),
        "forbidden use in df-table:\n{}",
        report.join("\n")
    );
}

/// The names declared under each dependency table of a manifest, as `section: name`.
fn declared_dependencies(manifest: &str) -> Vec<String> {
    let mut section = String::new();
    let mut declared = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = header.to_owned();
            if section.contains("dependencies") && section != "dependencies" {
                declared.push(format!("{section}: (table)"));
            }
        } else if section == "dependencies" && !line.is_empty() && !line.starts_with('#') {
            let name = line
                .split(|c: char| c == '=' || c == '.' || c.is_whitespace())
                .next()
                .unwrap_or_default();
            declared.push(format!("dependencies: {name}"));
        }
    }
    declared
}

fn features_that_enable_something(manifest: &str) -> Vec<String> {
    let mut in_features = false;
    let mut enabling = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_features = line == "[features]";
        } else if in_features && line.contains('=') && !line.starts_with('#') {
            let value = line.split_once('=').map(|(_, value)| value.trim());
            if value != Some("[]") {
                enabling.push(line.to_owned());
            }
        }
    }
    enabling
}

#[test]
fn the_manifest_depends_on_serde_and_serde_json_only() {
    let manifest = include_str!("../Cargo.toml");
    let mut declared = declared_dependencies(manifest);
    declared.sort();
    let expected: Vec<String> = ALLOWED_DEPENDENCIES
        .iter()
        .map(|name| format!("dependencies: {name}"))
        .collect();
    assert_eq!(declared, expected);
    assert!(
        features_that_enable_something(manifest).is_empty(),
        "a feature may not pull in anything: the testkit feature is an empty list"
    );
}

#[test]
fn the_scan_finds_each_forbidden_thing_in_code() {
    let cases = [
        ("tokio::spawn(task);", "tokio"),
        ("async fn run() {}", "async"),
        ("let now = std::time::Instant::now();", "Instant"),
        ("let now = SystemTime::now();", "SystemTime"),
        ("let n: u8 = rand::random();", "rand"),
        ("let text = std::fs::read_to_string(p);", "std::fs"),
        ("use std::net::TcpStream;", "std::net"),
        ("use std::{fs, io};", "std::{fs}"),
        ("use std::{io, net::TcpStream};", "std::{net}"),
        ("let (a, b) = unbounded_channel();", "unbounded"),
        ("println!(\"hello\");", "println!"),
        ("eprintln!(\"hello\");", "eprintln!"),
        ("value.await;", "await"),
        (
            "let (sender, receiver) = std::sync::mpsc::channel();",
            "mpsc::channel",
        ),
        ("std::thread::spawn(|| ());", "std::thread"),
        ("thread::sleep(pause);", "thread::"),
        ("print!(\"x\");", "print!"),
        ("eprint!(\"x\");", "eprint!"),
        ("dbg!(value);", "dbg!"),
        ("let home = std::env::var(name);", "std::env"),
        ("let child = std::process::Command::new(p);", "std::process"),
        ("let mut out = std::io::stdout();", "std::io"),
        ("use std::{fs as files};", "std::{fs}"),
        ("use std :: { io , env as e };", "std::{io}"),
        ("use std::{env, process};", "std::{env}"),
        ("use std::{thread, io};", "std::{thread}"),
        ("use std::{net as n, io::Read};", "std::{net}"),
    ];
    for (source, expected) in cases {
        let findings = violations_in(source);
        assert!(
            findings.iter().any(|finding| finding.contains(expected)),
            "{source:?} was not caught as {expected:?}: {findings:?}"
        );
    }
}

#[test]
fn the_scan_ignores_prose_strings_and_look_alike_words() {
    let clean = [
        "// no async here, and no tokio or Instant::now",
        "/// the module never reads std::fs or the rand crate",
        "/* a block comment about SystemTime */ let value = 1;",
        "let text = \"tokio async Instant rand std::fs println!\";",
        "let text = r#\"std::net and \"async\" inside a raw string\"#;",
        "let quote = '\"'; let operand = 1; let random_choice = 2; let instantly = 3;",
        "fn lifetime<'a>(value: &'a str) -> &'a str { value }",
        "use std::{collections, fmt};",
    ];
    for source in clean {
        assert_eq!(violations_in(source), Vec::<String>::new(), "{source:?}");
    }
}

#[test]
fn production_code_has_no_unwrap_expect_or_panic_beyond_the_named_allowance() {
    let mut sources = Vec::new();
    non_test_sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    let mut report = Vec::new();
    for path in &sources {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name == TEST_SUPPORT_FILE {
            continue;
        }
        let text = fs::read_to_string(path).expect("a source file reads");
        let findings = panicking_shortcuts_in(&text);
        let allowed = PANICKING_SHORTCUT_ALLOWANCE
            .iter()
            .find(|(allowed_name, _)| *allowed_name == name)
            .map_or(0, |(_, count)| *count);
        if findings.len() > allowed {
            report.push(format!(
                "{name}: {} panicking calls, {allowed} allowed: {findings:?}",
                findings.len()
            ));
        }
    }
    assert!(
        report.is_empty(),
        "panicking shortcuts:\n{}",
        report.join("\n")
    );
}

#[test]
fn the_panic_scan_finds_each_shortcut_and_ignores_prose_and_look_alikes() {
    let cases = [
        ("let x = value.unwrap();", ".unwrap()"),
        ("let x = value . expect(\"why\");", ".expect("),
        ("panic!(\"no\");", "panic!"),
        ("unreachable!();", "unreachable!"),
        ("todo!();", "todo!"),
        ("unimplemented!();", "unimplemented!"),
    ];
    for (source, expected) in cases {
        let findings = panicking_shortcuts_in(source);
        assert!(
            findings.iter().any(|finding| finding.contains(expected)),
            "{source:?} was not caught as {expected:?}: {findings:?}"
        );
    }
    let clean = [
        "// value.unwrap() would panic",
        "let text = \"call .expect( or panic!\";",
        "let x = value.unwrap_or_default().unwrap_or_else(|| 1);",
        "fn expectation() {}",
    ];
    for source in clean {
        assert_eq!(
            panicking_shortcuts_in(source),
            Vec::<String>::new(),
            "{source:?}"
        );
    }
}

#[test]
fn the_manifest_scan_flags_another_dependency_and_a_dev_dependency_table() {
    let extra = "[dependencies]\nserde = \"1\"\nserde_json = \"1\"\ntokio = \"1\"\n";
    assert!(declared_dependencies(extra).contains(&"dependencies: tokio".to_owned()));
    let dev = "[dependencies]\nserde = \"1\"\n[dev-dependencies]\ntempfile = \"3\"\n";
    assert!(declared_dependencies(dev).contains(&"dev-dependencies: (table)".to_owned()));
    let enabling = "[features]\ntestkit = [\"dep:tokio\"]\n";
    assert_eq!(features_that_enable_something(enabling).len(), 1);
}
