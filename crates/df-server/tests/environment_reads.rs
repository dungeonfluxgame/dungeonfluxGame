//! Hosted runtime design 8.4: the process environment is read in exactly two places of the
//! hosted binaries (`df-server`'s `environment.rs` with `var_os(` and `command_line.rs` with
//! `args_os(`), child processes start only through the runner's cleared-environment helper, and
//! the game library's existing reads can only shrink. The scan covers non-test source (and
//! `build.rs`) of df-server, df-runner and every workspace crate in df-server's dependency graph
//! (normal and build dependencies, `workspace = true` included), plus df-tools, which joins that
//! graph with the runtime seam (D15). `tests/environment_inventory.txt` records df-tools-era
//! reads by file; delete a line when its reads move into configuration fields.

#![cfg(not(target_arch = "wasm32"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Reads of the process environment, the temporary directory and the working directory. A
/// substring match, so a bare `temp_dir(` after `use std::env::temp_dir` is found too.
const PATTERNS: [&str; 18] = [
    "env::var(",
    "var_os(",
    "env::vars(",
    "vars_os(",
    "env::args(",
    "args_os(",
    "temp_dir(",
    "tempdir(",
    "NamedTempFile::new(",
    "tempfile::tempfile(",
    "TempDir::new(",
    "home_dir(",
    "current_dir(",
    "current_exe(",
    "env::set_var(",
    "env::remove_var(",
    "env::{",
    "from_env(",
];

/// Starting a child process, including through an alias.
const SPAWN_PATTERNS: [&str; 2] = ["Command::new(", "Command as "];

/// Joined to the dependency graph by the runtime seam.
const JOINS_WITH_THE_SEAM: [&str; 1] = ["df-tools"];

/// The only hosted-binary files that may read the environment, and the one pattern each may use.
const HOSTED_ALLOWLIST: [(&str, &str); 2] = [
    ("crates/df-server/src/environment.rs", "var_os("),
    ("crates/df-server/src/command_line.rs", "args_os("),
];

fn crates_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The workspace dependency table of the root manifest, for `workspace = true` entries.
fn workspace_dependencies() -> toml::Table {
    let root = crates_directory().join("..").join("Cargo.toml");
    let Ok(text) = fs::read_to_string(root) else {
        return toml::Table::new();
    };
    let table: toml::Table = text.parse().expect("root manifest parses");
    table
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default()
}

/// Names of workspace crates `crate_name` depends on by path in its normal and build tables.
fn path_dependencies(crate_name: &str) -> Vec<String> {
    let manifest = crates_directory().join(crate_name).join("Cargo.toml");
    let Ok(text) = fs::read_to_string(&manifest) else {
        return Vec::new();
    };
    let table: toml::Table = text.parse().expect("manifest parses");
    let shared = workspace_dependencies();
    let mut tables = Vec::new();
    for section in ["dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get(section).and_then(toml::Value::as_table) {
            tables.push(dependencies);
        }
    }
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            for section in ["dependencies", "build-dependencies"] {
                if let Some(dependencies) = target.get(section).and_then(toml::Value::as_table) {
                    tables.push(dependencies);
                }
            }
        }
    }
    tables
        .into_iter()
        .flat_map(|dependencies| dependencies.iter())
        .filter_map(|(name, dependency)| {
            let declared =
                if dependency.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    shared.get(name)?
                } else {
                    dependency
                };
            declared.get("path")?.as_str()
        })
        .filter_map(|path| path.strip_prefix("../"))
        .map(str::to_string)
        .collect()
}

fn scanned_crates() -> BTreeSet<String> {
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut pending: Vec<String> = ["df-server", "df-runner"]
        .into_iter()
        .chain(JOINS_WITH_THE_SEAM)
        .map(str::to_string)
        .collect();
    while let Some(next) = pending.pop() {
        if found.insert(next.clone()) {
            pending.extend(path_dependencies(&next));
        }
    }
    found
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with("_tests.rs")
        || name == "tests.rs"
        || path.components().any(|part| part.as_os_str() == "tests")
}

/// Opening and closing braces of a line, not counting those inside string or character literals.
fn braces(line: &str) -> (i32, i32) {
    let (mut open, mut close) = (0, 0);
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                while let Some(inner) = characters.next() {
                    match inner {
                        '\\' => {
                            characters.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '\'' => {
                // A character literal closes within three characters; a lifetime never does.
                let rest: String = characters.clone().take(3).collect();
                let closes_at = if rest.starts_with('\\') {
                    rest.find('\'').filter(|at| *at > 1)
                } else {
                    rest.find('\'').filter(|at| *at == 1)
                };
                if let Some(at) = closes_at {
                    for _ in 0..=at {
                        characters.next();
                    }
                }
            }
            '{' => open += 1,
            '}' => close += 1,
            _ => {}
        }
    }
    (open, close)
}

/// The code lines of a file with `#[cfg(test)]` items and comment lines removed.
fn production_lines(text: &str) -> Vec<&str> {
    let mut kept = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            let mut depth = 0i32;
            let mut opened = false;
            for skipped in lines.by_ref() {
                let (open, close) = braces(skipped);
                depth += open - close;
                opened |= open > 0;
                if (opened && depth <= 0) || (!opened && skipped.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        if !trimmed.starts_with("//") {
            kept.push(line);
        }
    }
    kept
}

#[derive(Default)]
struct Sites {
    /// Occurrences per environment pattern.
    environment: BTreeMap<&'static str, usize>,
    spawns: usize,
}

impl Sites {
    fn environment_total(&self) -> usize {
        self.environment.values().sum()
    }
}

fn scan_text(text: &str) -> Sites {
    let mut found = Sites::default();
    for line in production_lines(text) {
        for pattern in PATTERNS {
            let count = line.matches(pattern).count();
            if count > 0 {
                *found.environment.entry(pattern).or_default() += count;
            }
        }
        found.spawns += SPAWN_PATTERNS
            .iter()
            .map(|pattern| line.matches(pattern).count())
            .sum::<usize>();
    }
    found
}

fn scan(crate_name: &str) -> BTreeMap<String, Sites> {
    let root = crates_directory().join(crate_name);
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    files.push(root.join("build.rs"));
    let mut sites = BTreeMap::new();
    for path in files.into_iter().filter(|path| !is_test_file(path)) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let found = scan_text(&text);
        if found.environment_total() + found.spawns > 0 {
            let relative = path
                .strip_prefix(crates_directory())
                .expect("inside crates");
            sites.insert(
                format!("crates/{}", relative.to_string_lossy().replace('\\', "/")),
                found,
            );
        }
    }
    sites
}

#[test]
fn the_hosted_binaries_read_the_environment_only_in_their_two_designated_files_with_one_pattern_each()
 {
    for crate_name in ["df-server", "df-runner"] {
        for (file, sites) in scan(crate_name) {
            for (pattern, count) in &sites.environment {
                assert!(
                    HOSTED_ALLOWLIST.contains(&(file.as_str(), *pattern)),
                    "{file} uses `{pattern}` ({count} sites); move it into configuration, or the \
                     two allowed files with their one pattern each"
                );
            }
        }
    }
}

#[test]
fn child_processes_start_only_through_the_runners_cleared_environment_helper() {
    let helper = "crates/df-runner/src/process.rs";
    for crate_name in ["df-server", "df-runner"] {
        for (file, sites) in scan(crate_name) {
            assert!(
                sites.spawns == 0 || file == helper,
                "{file} starts a child process directly ({} sites); use the cleared-environment helper",
                sites.spawns
            );
        }
    }
}

fn inventory() -> BTreeMap<String, usize> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/environment_inventory.txt");
    let text = fs::read_to_string(path).expect("the inventory is checked in");
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (file, count) = line.rsplit_once(' ').expect("`path count`");
            (file.to_string(), count.parse().expect("count"))
        })
        .collect()
}

#[test]
fn the_game_librarys_environment_reads_and_spawns_can_only_shrink() {
    let recorded = inventory();
    let mut current = BTreeMap::new();
    for crate_name in scanned_crates() {
        if crate_name == "df-server" || crate_name == "df-runner" {
            continue;
        }
        for (file, sites) in scan(&crate_name) {
            current.insert(file, sites.environment_total() + sites.spawns);
        }
    }
    let grown: Vec<String> = current
        .iter()
        .filter(|(file, count)| recorded.get(*file).is_none_or(|allowed| allowed < count))
        .map(|(file, count)| format!("{file} {count}"))
        .collect();
    assert!(
        grown.is_empty(),
        "new environment reads or spawns outside the inventory (move them into configuration, or \
         record them in tests/environment_inventory.txt with an owner's reason):\n{}",
        grown.join("\n")
    );
    assert!(
        !current.is_empty(),
        "the scan must find the known reads of df-tools"
    );
}

#[test]
fn the_scan_reaches_the_game_library_and_persistence_so_it_cannot_pass_by_scanning_nothing() {
    let crates = scanned_crates();
    for expected in ["df-tools", "df-persistence", "df-session", "df-observe"] {
        assert!(
            crates.contains(expected),
            "{expected} must be in the scanned graph: {crates:?}"
        );
    }
    let sites = scan("df-tools");
    let total: usize = sites.values().map(Sites::environment_total).sum();
    assert!(
        total > 30,
        "df-tools reads the environment in dozens of places, found {total}"
    );
}

#[test]
fn production_lines_drop_test_items_and_comments_but_keep_everything_else() {
    let source = "\
fn read() { std::env::var(\"A\"); }
// std::env::var(\"commented\")
#[cfg(test)]
mod tests {
    fn read() { std::env::var(\"B\"); }
}
#[cfg(test)]
mod elsewhere;
fn after() { std::env::var(\"C\"); }
";
    assert_eq!(
        production_lines(source),
        [
            "fn read() { std::env::var(\"A\"); }",
            "fn after() { std::env::var(\"C\"); }"
        ]
    );
}

#[test]
fn a_lone_brace_in_a_string_or_character_inside_a_test_item_does_not_swallow_later_code() {
    let source = "\
#[cfg(test)]
mod tests {
    const OPEN: &str = \"{\";
    const CHARACTER: char = '{';
    fn lifetime<'a>(value: &'a str) -> &'a str { value }
}
fn after() { std::env::var(\"C\"); }
";
    assert_eq!(
        production_lines(source),
        ["fn after() { std::env::var(\"C\"); }"]
    );
}

#[test]
fn the_patterns_find_aliased_imports_temporary_directories_and_working_directories() {
    for (source, pattern) in [
        ("use std::env::temp_dir;\nlet p = temp_dir();", "temp_dir("),
        ("let t = tempfile::tempdir();", "tempdir("),
        ("let f = NamedTempFile::new();", "NamedTempFile::new("),
        ("let d = std::env::current_dir();", "current_dir("),
        ("let e = std::env::current_exe();", "current_exe("),
        ("use std::env::{var, args};", "env::{"),
        ("let v = var_os(\"X\");", "var_os("),
        ("let c = SdkConfig::from_env();", "from_env("),
    ] {
        let found = scan_text(source);
        assert!(
            found.environment.contains_key(pattern),
            "`{pattern}` in `{source}`"
        );
    }
    assert_eq!(scan_text("use std::process::Command as Spawner;").spawns, 1);
    assert_eq!(
        scan_text("let c = tokio::process::Command::new(\"x\");").spawns,
        1
    );
}
