//! Shared store fixtures: the parent every fixture-only `Store::open` root lives under, and the
//! guard that removes one root when its test ends, so test runs leave no stores behind.
use std::{
    fs,
    io::ErrorKind,
    ops::Deref,
    path::{Path, PathBuf},
};

/// Create (if needed) and return the workspace fixture parent `Store::open` accepts.
pub fn fixture_parent() -> PathBuf {
    let parent = df_telemetry::fixture_root_parent();
    fs::create_dir_all(&parent).unwrap();
    parent
}

/// A fixture root owned by one test and removed with its contents when dropped. A failing test
/// keeps its root and prints the path, so the evidence can be inspected.
pub struct FixtureRoot(PathBuf);

impl FixtureRoot {
    /// Take ownership of `path`; whoever opens the store creates the directory.
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Deref for FixtureRoot {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for FixtureRoot {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("failed test keeps its fixture root: {}", self.0.display());
            return;
        }
        match fs::remove_dir_all(&self.0) {
            Ok(()) => (),
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(error) => panic!("fixture root {} was not removed: {error}", self.0.display()),
        }
    }
}
