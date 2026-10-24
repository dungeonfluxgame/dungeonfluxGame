//! Finding the game: its folder, the built server and browser files, and PostgreSQL. Each
//! location can be set in Settings; empty settings mean the automatic choice described here.
use std::path::{Path, PathBuf};

use crate::settings::Settings;

#[cfg(windows)]
const EXE: &str = ".exe";
#[cfg(not(windows))]
const EXE: &str = "";

#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
    pub server_exe: PathBuf,
    pub web_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub postgres_bin: PathBuf,
    pub play_root: PathBuf,
    pub env_file: PathBuf,
}

/// What is missing, written so the player knows what to do next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Missing(pub String);

impl Install {
    pub fn locate(settings: &Settings) -> Result<Self, Missing> {
        let root = game_folder(settings)?;
        let build = build_folder(settings, &root)?;
        let postgres_bin = if settings.postgres_folder.is_empty() {
            root.join("artifacts/build/postgres-g05/install/bin")
        } else {
            PathBuf::from(&settings.postgres_folder)
        };
        for tool in ["initdb", "pg_ctl", "createdb", "psql"] {
            if !postgres_bin.join(format!("{tool}{EXE}")).is_file() {
                return Err(Missing(format!(
                    "PostgreSQL was not found in {}. Set the PostgreSQL folder in Settings, under Developer.",
                    postgres_bin.display()
                )));
            }
        }
        let play_root = if settings.play_folder.is_empty() {
            root.join("artifacts/tmp/play")
        } else {
            PathBuf::from(&settings.play_folder)
        };
        Ok(Self {
            server_exe: build.join(format!("debug/df-transport-fixture{EXE}")),
            web_dir: build.join("play-web"),
            assets_dir: root.join("assets/ui"),
            env_file: root.join(".env"),
            root,
            postgres_bin,
            play_root,
        })
    }

    pub fn postgres_tool(&self, tool: &str) -> PathBuf {
        self.postgres_bin.join(format!("{tool}{EXE}"))
    }
}

fn is_game_folder(folder: &Path) -> bool {
    folder.join("content/harbor-journey/pack.json").is_file() && folder.join("assets/ui").is_dir()
}

/// The first folder above the launcher executable (or the current folder) that holds the game
/// content.
pub fn find_game_folder() -> Option<PathBuf> {
    let starts = [
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];
    starts.into_iter().flatten().find_map(|start| {
        start
            .ancestors()
            .find(|folder| is_game_folder(folder))
            .map(Path::to_path_buf)
    })
}

/// The configured folder, else [`find_game_folder`].
fn game_folder(settings: &Settings) -> Result<PathBuf, Missing> {
    if !settings.game_folder.is_empty() {
        let folder = PathBuf::from(&settings.game_folder);
        return if is_game_folder(&folder) {
            Ok(folder)
        } else {
            Err(Missing(format!(
                "{} does not hold the DungeonFlux content folder. Correct the game folder in Settings, under Developer.",
                folder.display()
            )))
        };
    }
    find_game_folder().ok_or_else(|| {
        Missing(
            "The DungeonFlux game folder was not found. Set it in Settings, under Developer."
                .to_owned(),
        )
    })
}

pub fn is_build(folder: &Path) -> bool {
    folder
        .join(format!("debug/df-transport-fixture{EXE}"))
        .is_file()
        && folder.join("play-web/df_tools_bg.wasm").is_file()
        && folder.join("play-web/df_tools.js").is_file()
}

/// The configured build, else `artifacts/build/play` (where `run-gameplay.sh` builds), else
/// the most recently built complete build under `artifacts/build`.
fn build_folder(settings: &Settings, root: &Path) -> Result<PathBuf, Missing> {
    if !settings.build_folder.is_empty() {
        let folder = PathBuf::from(&settings.build_folder);
        return if is_build(&folder) {
            Ok(folder)
        } else {
            Err(Missing(format!(
                "{} is not a complete game build (it needs debug/df-transport-fixture{EXE} and play-web). Correct the build folder in Settings, under Developer.",
                folder.display()
            )))
        };
    }
    let standard = root.join("artifacts/build/play");
    if is_build(&standard) {
        return Ok(standard);
    }
    newest_build(&root.join("artifacts/build")).ok_or_else(|| {
        Missing(
            "The game has not been built yet. Build it once with development/run-gameplay.sh, or set a build folder in Settings, under Developer."
                .to_owned(),
        )
    })
}

fn newest_build(parent: &Path) -> Option<PathBuf> {
    std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|folder| is_build(folder))
        .filter_map(|folder| {
            let built = folder
                .join(format!("debug/df-transport-fixture{EXE}"))
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()?;
            Some((built, folder))
        })
        .max_by_key(|(built, _)| *built)
        .map(|(_, folder)| folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_build(folder: &Path) {
        std::fs::create_dir_all(folder.join("debug")).unwrap();
        std::fs::create_dir_all(folder.join("play-web")).unwrap();
        std::fs::write(
            folder.join(format!("debug/df-transport-fixture{EXE}")),
            b"x",
        )
        .unwrap();
        std::fs::write(folder.join("play-web/df_tools_bg.wasm"), b"x").unwrap();
        std::fs::write(folder.join("play-web/df_tools.js"), b"x").unwrap();
    }

    #[test]
    fn locate_prefers_the_play_build_then_the_newest_complete_build() {
        let root = std::env::temp_dir().join(format!("df-launcher-install-{}", std::process::id()));
        std::fs::create_dir_all(root.join("content/harbor-journey")).unwrap();
        std::fs::create_dir_all(root.join("assets/ui")).unwrap();
        std::fs::write(root.join("content/harbor-journey/pack.json"), b"{}").unwrap();
        let bin = root.join("artifacts/build/postgres-g05/install/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for tool in ["initdb", "pg_ctl", "createdb", "psql"] {
            std::fs::write(bin.join(format!("{tool}{EXE}")), b"x").unwrap();
        }
        let settings = Settings {
            game_folder: root.display().to_string(),
            ..Settings::default()
        };
        assert!(
            Install::locate(&settings)
                .unwrap_err()
                .0
                .contains("not been built")
        );
        std::fs::create_dir_all(root.join("artifacts/build/incomplete/debug")).unwrap();
        fake_build(&root.join("artifacts/build/lane"));
        let found = Install::locate(&settings).unwrap();
        assert!(
            found
                .server_exe
                .starts_with(root.join("artifacts/build/lane"))
        );
        fake_build(&root.join("artifacts/build/play"));
        let found = Install::locate(&settings).unwrap();
        assert!(
            found
                .server_exe
                .starts_with(root.join("artifacts/build/play"))
        );
        assert_eq!(found.play_root, root.join("artifacts/tmp/play"));
        let wrong = Settings {
            build_folder: root
                .join("artifacts/build/incomplete")
                .display()
                .to_string(),
            ..settings
        };
        assert!(
            Install::locate(&wrong)
                .unwrap_err()
                .0
                .contains("not a complete game build")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
