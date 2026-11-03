//! The generator over a scripted backend: success, timeout, a login that is not there, a refusal
//! by the service, bytes that are not a picture, and storage through the staged upload.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tempfile::TempDir;

use super::fake::{FakeBackend, Script, picture, png_bytes};
use super::model::{
    BackendError, DEFAULT_DEADLINE, GeneratedImage, ImageRequest, JobState, JobView, RefusalCause,
};
use super::service::{Context, ImageGenerator, Refusal};
use crate::media_library::media_entries;

struct Rig {
    media: TempDir,
    provenance: TempDir,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn new() -> Self {
        Self {
            media: TempDir::new().expect("makes a directory"),
            provenance: TempDir::new().expect("makes a directory"),
            lines: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn context(&self) -> Context {
        self.context_in(self.media.path().to_path_buf())
    }

    fn context_in(&self, media_directory: PathBuf) -> Context {
        let lines = Arc::clone(&self.lines);
        Context {
            media_directory,
            record: Arc::new(move |line| {
                lines
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line);
            }),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn generator(&self, backend: Arc<FakeBackend>) -> Arc<ImageGenerator> {
        Arc::new(
            ImageGenerator::new(backend)
                .with_provenance_directory(self.provenance.path().to_path_buf()),
        )
    }

    fn stored_files(&self) -> Vec<String> {
        media_entries(self.media.path())
            .expect("lists the library")
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }
}

fn request(name: &str) -> ImageRequest {
    ImageRequest::new("a candlelit hall at dusk", name, Some("background"), None)
        .expect("a request")
}

async fn run_to_end(
    generator: &Arc<ImageGenerator>,
    context: &Context,
    request: ImageRequest,
) -> JobView {
    generator
        .start(context, request)
        .await
        .expect("starts")
        .finished
        .await
        .expect("the job finishes")
}

#[tokio::test]
async fn a_picture_is_stored_in_the_library_with_its_provenance() {
    let rig = Rig::new();
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let view = run_to_end(&generator, &rig.context(), request("hall")).await;

    assert_eq!(view.state, JobState::Done);
    assert_eq!(view.job, 1);
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
    let stored = std::fs::read(rig.media.path().join("hall.png")).expect("reads the file");
    assert_eq!(stored, png_bytes());

    let provenance = view.provenance.expect("provenance");
    assert_eq!(provenance.backend, "fake");
    assert_eq!(provenance.prompt, "a candlelit hall at dusk");
    assert!(provenance.sent_text.starts_with("a candlelit hall at dusk"));
    assert_eq!(provenance.media_file, "hall.png");
    assert_eq!((provenance.width_px, provenance.height_px), (1672, 941));
    assert_eq!(
        (
            provenance.requested_width_px,
            provenance.requested_height_px
        ),
        (1672, 941)
    );
    assert_eq!(provenance.duration_ms, 4200);
    assert_eq!(provenance.deadline_secs, DEFAULT_DEADLINE.as_secs());
    assert!(provenance.note.contains("PNG stays a PNG"));

    let sidecar = view.provenance_file.expect("a sidecar path");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("reads the sidecar"))
            .expect("is JSON");
    assert_eq!(written["prompt"], "a candlelit hall at dusk");
    assert_eq!(written["media_file"], "hall.png");

    let asked = backend.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1, DEFAULT_DEADLINE, "a large named deadline");
    assert!(asked[0].0.text.contains("background picture"));

    let lines = rig.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9")
    );
    assert!(lines[1].starts_with("IMAGEGEN stored job=1 name=hall file=hall.png bytes="));
    assert!(lines.iter().all(|line| !line.contains("candlelit")));
}

#[tokio::test]
async fn a_second_picture_of_one_name_replaces_the_first() {
    let rig = Rig::new();
    let generator = rig.generator(FakeBackend::new(Script::Picture(picture())));
    run_to_end(&generator, &rig.context(), request("hall")).await;
    let mut other = picture();
    other.bytes.extend_from_slice(b"second");
    let second = rig.generator(FakeBackend::new(Script::Picture(other.clone())));
    let view = run_to_end(&second, &rig.context(), request("hall")).await;
    assert_eq!(view.state, JobState::Done);
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
    assert_eq!(
        std::fs::read(rig.media.path().join("hall.png")).expect("reads"),
        other.bytes
    );
}

#[tokio::test]
async fn a_deadline_ends_the_job_as_a_timeout_and_stores_nothing() {
    let rig = Rig::new();
    let backend = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(600));
    let generator = Arc::new(
        ImageGenerator::new(backend)
            .with_deadline(Duration::from_millis(20))
            .with_grace(Duration::from_millis(20)),
    );
    let view = run_to_end(&generator, &rig.context(), request("hall")).await;
    assert_eq!(view.state, JobState::Refused);
    assert_eq!(view.cause, Some(RefusalCause::Timeout));
    assert!(rig.stored_files().is_empty());
    let lines = rig.lines();
    assert!(
        lines
            .last()
            .is_some_and(|line| line.starts_with("IMAGEGEN failed job=1 name=hall cause=timeout")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_backend_that_reports_a_timeout_is_a_timeout() {
    let rig = Rig::new();
    let generator = rig.generator(FakeBackend::new(Script::Fail(BackendError::Timeout)));
    let view = run_to_end(&generator, &rig.context(), request("hall")).await;
    assert_eq!(view.cause, Some(RefusalCause::Timeout));
    assert!(view.message.expect("a message").contains("deadline"));
}

#[tokio::test]
async fn a_missing_login_is_refused_before_any_job_starts() {
    let rig = Rig::new();
    let backend = FakeBackend::failing_check(BackendError::NotSignedIn(
        "the Codex home has no login: sign in with `CODEX_HOME=<home> codex login`".to_owned(),
    ));
    let generator = rig.generator(Arc::clone(&backend));
    let refusal: Refusal = generator
        .start(&rig.context(), request("hall"))
        .await
        .map(|_| ())
        .expect_err("refused");
    assert_eq!(refusal.cause, RefusalCause::NotSignedIn);
    assert_eq!(refusal.job, None);
    assert!(refusal.message.contains("codex login"));
    assert!(backend.asked().is_empty(), "the backend was never asked");
    assert!(generator.views().is_empty(), "no job was made");
    assert_eq!(
        rig.lines(),
        vec!["IMAGEGEN refused name=hall cause=not_signed_in backend=fake".to_owned()]
    );
}

#[tokio::test]
async fn a_backend_that_is_not_set_up_is_refused_with_its_own_cause() {
    let rig = Rig::new();
    for (error, cause) in [
        (
            BackendError::NotConfigured("not switched on".to_owned()),
            RefusalCause::NotConfigured,
        ),
        (
            BackendError::CodexMissing("no codex".to_owned()),
            RefusalCause::CodexMissing,
        ),
    ] {
        let generator = rig.generator(FakeBackend::failing_check(error));
        let refusal = generator
            .start(&rig.context(), request("hall"))
            .await
            .map(|_| ())
            .expect_err("refused");
        assert_eq!(refusal.cause, cause);
    }
}

#[tokio::test]
async fn a_refusal_by_the_service_is_recorded_as_refused() {
    let rig = Rig::new();
    let generator = rig.generator(FakeBackend::new(Script::Fail(BackendError::Refused(
        "the image service refused the request (content policy)".to_owned(),
    ))));
    let view = run_to_end(&generator, &rig.context(), request("hall")).await;
    assert_eq!(view.state, JobState::Refused);
    assert_eq!(view.cause, Some(RefusalCause::Refused));
    assert!(view.message.expect("a message").contains("content policy"));
    assert!(rig.stored_files().is_empty());
    assert_eq!(generator.views().len(), 1);
}

#[tokio::test]
async fn bytes_that_are_not_a_picture_are_refused_and_not_stored() {
    let rig = Rig::new();
    let cases: Vec<(&str, GeneratedImage)> = vec![
        (
            "text",
            GeneratedImage {
                bytes: b"this is not a picture".to_vec(),
                ..picture()
            },
        ),
        (
            "empty",
            GeneratedImage {
                bytes: Vec::new(),
                ..picture()
            },
        ),
        (
            "wrong type",
            GeneratedImage {
                mime: "image/webp".to_owned(),
                ..picture()
            },
        ),
        (
            "no size",
            GeneratedImage {
                width_px: 0,
                ..picture()
            },
        ),
        (
            "unknown type",
            GeneratedImage {
                mime: "text/plain".to_owned(),
                ..picture()
            },
        ),
    ];
    for (label, image) in cases {
        let generator = rig.generator(FakeBackend::new(Script::Picture(image)));
        let view = run_to_end(&generator, &rig.context(), request("hall")).await;
        assert_eq!(view.cause, Some(RefusalCause::BadImage), "{label}");
        assert!(rig.stored_files().is_empty(), "{label}");
    }
}

#[tokio::test]
async fn a_library_that_cannot_take_the_file_is_a_typed_store_failure() {
    let rig = Rig::new();
    // A path below a plain file can never be made into a directory.
    let blocker = rig.media.path().join("blocker");
    std::fs::write(&blocker, b"a file").expect("writes");
    let generator = rig.generator(FakeBackend::new(Script::Picture(picture())));
    let context = rig.context_in(blocker.join("media"));
    let view = run_to_end(&generator, &context, request("hall")).await;
    assert_eq!(view.state, JobState::Refused);
    assert_eq!(view.cause, Some(RefusalCause::StoreFailed));
}

#[tokio::test]
async fn one_name_cannot_be_generated_twice_at_once() {
    let rig = Rig::new();
    let backend = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(300));
    let generator = rig.generator(backend);
    let context = rig.context();
    let first = generator
        .start(&context, request("hall"))
        .await
        .expect("starts");
    let second = generator
        .start(&context, request("hall"))
        .await
        .map(|_| ())
        .expect_err("refused while the first runs");
    assert_eq!(second.cause, RefusalCause::NameBusy);
    let other = generator
        .start(&context, request("other"))
        .await
        .expect("another name is fine");
    assert_eq!(other.job, 2);
    let done = first.finished.await.expect("finishes");
    assert_eq!(done.state, JobState::Done);
    other.finished.await.expect("finishes");
    let again = run_to_end(&generator, &context, request("hall")).await;
    assert_eq!(again.state, JobState::Done, "free again once it ended");
    assert_eq!(generator.views().len(), 3);
}

#[tokio::test]
async fn a_sidecar_that_cannot_be_written_never_fails_the_job() {
    let rig = Rig::new();
    let blocker = rig.provenance.path().join("blocker");
    std::fs::write(&blocker, b"a file").expect("writes");
    let generator = Arc::new(
        ImageGenerator::new(FakeBackend::new(Script::Picture(picture())))
            .with_provenance_directory(blocker.join("out")),
    );
    let view = run_to_end(&generator, &rig.context(), request("hall")).await;
    assert_eq!(view.state, JobState::Done);
    assert!(
        view.provenance_file
            .expect("says where")
            .contains("not written")
    );
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
}
