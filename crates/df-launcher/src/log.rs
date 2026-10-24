//! The launcher's log: one line per event, `<time> pid=<pid> <EVENT> key=value ...`, in the
//! style of the game server's audit lines, written to `<data>/logs/launcher.log` (rotated at
//! 4 MiB, one previous file kept). Agents read it with `dungeonflux logs`. Callers never pass
//! key values, form bodies or query strings; `value` quotes text so lines stay one per event.
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const ROTATE_BYTES: u64 = 4 * 1024 * 1024;

static SINK: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn path(data: &Path) -> PathBuf {
    data.join("logs").join("launcher.log")
}

/// Opens the log for this process. Failure to open is not fatal: events are then dropped.
pub fn init(data: &Path) {
    let file = path(data);
    let _ = PATH.set(file.clone());
    let opened = (|| {
        std::fs::create_dir_all(file.parent()?).ok()?;
        if std::fs::metadata(&file).is_ok_and(|meta| meta.len() > ROTATE_BYTES) {
            let _ = std::fs::rename(&file, file.with_extension("log.1"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .ok()
    })();
    let _ = SINK.set(Mutex::new(opened));
}

/// Writes one event. `fields` is already formatted `key=value` text.
pub fn event(name: &str, fields: impl AsRef<str>) {
    let Some(sink) = SINK.get() else {
        return;
    };
    let line = format!(
        "{} pid={} {name} {}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
        std::process::id(),
        fields.as_ref()
    );
    if let Ok(mut file) = sink.lock()
        && let Some(file) = file.as_mut()
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Quotes a value when it holds spaces, quotes or line breaks, so each event stays one line.
pub fn value(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    if !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_whitespace() && c != '"' && !c.is_control())
    {
        text.to_owned()
    } else {
        format!("{text:?}")
    }
}

/// The last `lines` lines of this process's log file, for `dungeonflux logs`.
pub fn tail(data: &Path, lines: usize) -> String {
    let file = PATH.get().cloned().unwrap_or_else(|| path(data));
    crate::files::tail(&file, lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_stay_on_one_line() {
        assert_eq!(value("plain"), "plain");
        assert_eq!(value("The Iron Map"), "\"The Iron Map\"");
        assert_eq!(value("two\nlines"), "\"two\\nlines\"");
        assert_eq!(value(""), "\"\"");
    }
}
