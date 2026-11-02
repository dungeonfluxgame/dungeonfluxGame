//! Loading kind files from the kind directory.
//!
//! It owns reading and parsing, with bounds on how many files and how large, and reports every
//! problem in every file at once. It never keeps a half-loaded set: either all the kinds are
//! valid and returned, or none is. Reading is blocking file work, so the async entry point runs
//! it on the blocking pool.

use std::fmt;
use std::path::{Path, PathBuf};

use df_table::kind::KindRegistry;
use df_table::kind_file::{KindError, KindFile};

use crate::limits::{MAXIMUM_KIND_FILE_BYTES, MAXIMUM_KIND_FILES};

/// One thing wrong with the kind directory or one of its files.
#[derive(Debug, Clone, PartialEq)]
pub enum KindFileProblem {
    UnreadableDirectory {
        directory: PathBuf,
        message: String,
    },
    UnreadableEntry {
        message: String,
    },
    UnreadableFile {
        file: String,
        message: String,
    },
    FileTooLarge {
        file: String,
        bytes: u64,
    },
    TooManyFiles {
        found: usize,
    },
    Parse {
        file: String,
        message: String,
    },
    Kind(KindError),
    /// The blocking task that reads the files did not finish.
    ReadingStopped,
}

impl fmt::Display for KindFileProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnreadableDirectory { directory, message } => {
                write!(formatter, "cannot read {}: {message}", directory.display())
            }
            Self::UnreadableEntry { message } => {
                write!(formatter, "cannot read an entry: {message}")
            }
            Self::UnreadableFile { file, message } => write!(formatter, "{file}: {message}"),
            Self::FileTooLarge { file, bytes } => write!(
                formatter,
                "{file}: {bytes} bytes; a kind file is at most {MAXIMUM_KIND_FILE_BYTES}"
            ),
            Self::TooManyFiles { found } => write!(
                formatter,
                "{found} kind files; at most {MAXIMUM_KIND_FILES} are read"
            ),
            Self::Parse { file, message } => write!(formatter, "{file}: {message}"),
            Self::Kind(error) => write!(formatter, "{error}"),
            Self::ReadingStopped => formatter.write_str("reading the kind files was interrupted"),
        }
    }
}

/// Every problem found while loading the kind directory.
#[derive(Debug, Clone, PartialEq)]
pub struct KindFileProblems(pub Vec<KindFileProblem>);

impl KindFileProblems {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// One plain sentence per problem.
    pub fn messages(&self) -> Vec<String> {
        self.0.iter().map(ToString::to_string).collect()
    }
}

impl fmt::Display for KindFileProblems {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.messages().join("\n"))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

fn is_kind_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

/// Reads every `*.json` file in `directory` (in name order) and checks the kinds they define.
pub fn load_kind_files(directory: &Path) -> Result<KindRegistry, KindFileProblems> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        KindFileProblems(vec![KindFileProblem::UnreadableDirectory {
            directory: directory.to_path_buf(),
            message: error.to_string(),
        }])
    })?;
    let mut problems = Vec::new();
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) if is_kind_file(&entry.path()) => paths.push(entry.path()),
            Ok(_) => {}
            Err(error) => problems.push(KindFileProblem::UnreadableEntry {
                message: error.to_string(),
            }),
        }
    }
    paths.sort();
    if paths.len() > MAXIMUM_KIND_FILES {
        problems.push(KindFileProblem::TooManyFiles { found: paths.len() });
        paths.truncate(MAXIMUM_KIND_FILES);
    }

    let mut files = Vec::new();
    for path in &paths {
        let file = file_name(path);
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.len() > MAXIMUM_KIND_FILE_BYTES => {
                problems.push(KindFileProblem::FileTooLarge {
                    file,
                    bytes: metadata.len(),
                });
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                problems.push(KindFileProblem::UnreadableFile {
                    file,
                    message: error.to_string(),
                });
                continue;
            }
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<KindFile>(&text) {
                Ok(parsed) => files.push(parsed),
                Err(error) => problems.push(KindFileProblem::Parse {
                    file,
                    message: error.to_string(),
                }),
            },
            Err(error) => problems.push(KindFileProblem::UnreadableFile {
                file,
                message: error.to_string(),
            }),
        }
    }

    match KindRegistry::from_files(&files) {
        Ok(registry) if problems.is_empty() => Ok(registry),
        Ok(_) => Err(KindFileProblems(problems)),
        Err(errors) => {
            problems.extend(errors.into_iter().map(KindFileProblem::Kind));
            Err(KindFileProblems(problems))
        }
    }
}

/// `load_kind_files` on the blocking pool, so a slow disk never stalls the async workers.
pub async fn load_kind_files_async(directory: PathBuf) -> Result<KindRegistry, KindFileProblems> {
    tokio::task::spawn_blocking(move || load_kind_files(&directory))
        .await
        .unwrap_or_else(|_| Err(KindFileProblems(vec![KindFileProblem::ReadingStopped])))
}
