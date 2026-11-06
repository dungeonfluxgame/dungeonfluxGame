//! The generator over a scripted backend: success, timeout, a login that is not there, a refusal
//! by the service, bytes that are not a picture, and storage through the staged upload.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use df_table::testkit::ManualClock;
use tempfile::TempDir;

use super::backend::BackendError;
use super::fixtures::{FakeBackend, Script, picture, png_bytes};
use super::limits::DEFAULT_DEADLINE;
use super::model::{GeneratedImage, ImageRequest, Purpose};
use super::request::{GenerateError, JobState, JobView, PictureCause, Provenance, Sidecar};
use super::service::{
    DEADLINE_SECONDS_VARIABLE, DeadlineSettingError, ImageGenerator, deadline_or_default,
    deadline_setting, forget_oldest_finished,
};
use crate::media_library::media_entries;
use crate::test_support::{Running, start};

struct Rig {
    running: Running,
    provenance: TempDir,
}

impl Rig {
    async fn new() -> Self {
        Self {
            running: start(&[]).await,
            provenance: TempDir::new().expect("makes a directory"),
        }
    }

    fn media(&self) -> PathBuf {
        self.running.playground_path("assets/media")
    }

    fn lines(&self) -> Vec<String> {
        self.running
            .state
            .log()
            .newest(100)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("IMAGEGEN"))
            .collect()
    }

    fn generator(&self, backend: Arc<FakeBackend>) -> Arc<ImageGenerator> {
        Arc::new(
            ImageGenerator::new(backend)
                .with_provenance_directory(self.provenance.path().to_path_buf()),
        )
    }

    fn stored_files(&self) -> Vec<String> {
        media_entries(&self.media())
            .expect("lists the library")
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    async fn run_to_end(
        &self,
        generator: &Arc<ImageGenerator>,
        request: ImageRequest,
    ) -> Result<JobView, GenerateError> {
        generator
            .start(&self.running.state, request)
            .await
            .expect("starts")
            .finished
            .await
            .expect("the job finishes")
    }

    async fn refused_at_start(
        &self,
        generator: &Arc<ImageGenerator>,
        request: ImageRequest,
    ) -> GenerateError {
        generator
            .start(&self.running.state, request)
            .await
            .map(|_| ())
            .expect_err("refused")
    }
}

fn request(name: &str) -> ImageRequest {
    ImageRequest::new(
        "a candlelit hall at dusk",
        name,
        Some(Purpose::Background),
        None,
    )
    .expect("a request")
}

fn stored(view: &JobView) -> (&Provenance, Option<&Sidecar>) {
    let JobState::Stored {
        provenance,
        sidecar,
    } = &view.state
    else {
        panic!("expected a stored picture: {view:?}");
    };
    (provenance, sidecar.as_ref())
}

#[tokio::test]
async fn a_picture_is_stored_in_the_library_with_its_provenance() {
    let rig = Rig::new().await;
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let view = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect("stored");

    assert_eq!(view.job, 1);
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
    let bytes = std::fs::read(rig.media().join("hall.png")).expect("reads the file");
    assert_eq!(bytes, png_bytes());

    let (provenance, sidecar) = stored(&view);
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
    assert_eq!(
        u128::from(provenance.deadline_ms),
        DEFAULT_DEADLINE.as_millis()
    );
    assert!(provenance.note.contains("PNG stays a PNG"));

    let Some(Sidecar::Written { path }) = sidecar else {
        panic!("expected a written sidecar: {sidecar:?}");
    };
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("reads the sidecar"))
            .expect("is JSON");
    assert_eq!(written["prompt"], "a candlelit hall at dusk");
    assert_eq!(written["media_file"], "hall.png");
    assert_eq!(written["aspect"], "16:9");

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
    let rig = Rig::new().await;
    let generator = rig.generator(FakeBackend::new(Script::Picture(picture())));
    rig.run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    let mut other = picture();
    other.bytes.extend_from_slice(b"second");
    let second = rig.generator(FakeBackend::new(Script::Picture(other.clone())));
    rig.run_to_end(&second, request("hall"))
        .await
        .expect("stored");
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
    assert_eq!(
        std::fs::read(rig.media().join("hall.png")).expect("reads"),
        other.bytes
    );
}

#[tokio::test]
async fn a_deadline_ends_the_job_as_a_timeout_and_stores_nothing() {
    let rig = Rig::new().await;
    let backend = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(600));
    let generator = Arc::new(
        ImageGenerator::new(backend)
            .with_deadline(Duration::from_millis(20))
            .with_grace(Duration::from_millis(20)),
    );
    let error = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect_err("timed out");
    assert_eq!(
        error,
        GenerateError::Backend {
            job: Some(1),
            backend: BackendError::Timeout
        }
    );
    assert!(rig.stored_files().is_empty());
    let lines = rig.lines();
    assert!(
        lines
            .last()
            .is_some_and(|line| line
                .starts_with("IMAGEGEN failed job=1 name=hall error=backend cause=timeout")),
        "{lines:?}"
    );
    let views = generator.views();
    assert!(
        matches!(&views[0].state, JobState::Failed { error: failed, .. } if *failed == error),
        "{views:?}"
    );
}

#[tokio::test]
async fn a_missing_login_is_refused_before_any_job_starts() {
    let rig = Rig::new().await;
    let home = PathBuf::from("codex-home");
    let backend = FakeBackend::failing_check(BackendError::NoLogin { home: home.clone() });
    let generator = rig.generator(Arc::clone(&backend));
    let error = rig.refused_at_start(&generator, request("hall")).await;
    assert_eq!(
        error,
        GenerateError::Backend {
            job: None,
            backend: BackendError::NoLogin { home }
        }
    );
    assert!(backend.asked().is_empty(), "the backend was never asked");
    assert!(generator.views().is_empty(), "no job was made");
    assert!(rig.lines().is_empty(), "the caller records a refusal");
}

#[tokio::test]
async fn a_refusal_by_the_service_ends_the_job_with_its_typed_error() {
    let rig = Rig::new().await;
    let refused = BackendError::Refused {
        detail: "content policy".to_owned(),
    };
    let generator = rig.generator(FakeBackend::new(Script::Fail(refused.clone())));
    let error = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect_err("refused");
    assert_eq!(
        error,
        GenerateError::Backend {
            job: Some(1),
            backend: refused
        }
    );
    assert!(rig.stored_files().is_empty());
    assert_eq!(generator.views().len(), 1);
}

#[tokio::test]
async fn bytes_that_are_not_a_picture_are_refused_and_not_stored() {
    let rig = Rig::new().await;
    let cases: Vec<(GeneratedImage, PictureCause)> = vec![
        (
            GeneratedImage {
                bytes: b"this is not a picture".to_vec(),
                ..picture()
            },
            PictureCause::NotOfType {
                mime: "image/png".to_owned(),
            },
        ),
        (
            GeneratedImage {
                bytes: Vec::new(),
                ..picture()
            },
            PictureCause::Empty,
        ),
        (
            GeneratedImage {
                mime: "image/webp".to_owned(),
                ..picture()
            },
            PictureCause::NotOfType {
                mime: "image/webp".to_owned(),
            },
        ),
        (
            GeneratedImage {
                width_px: 0,
                ..picture()
            },
            PictureCause::NoSize,
        ),
        (
            GeneratedImage {
                mime: "text/plain".to_owned(),
                ..picture()
            },
            PictureCause::NotOfType {
                mime: "text/plain".to_owned(),
            },
        ),
    ];
    for (image, cause) in cases {
        let generator = rig.generator(FakeBackend::new(Script::Picture(image)));
        let error = rig
            .run_to_end(&generator, request("hall"))
            .await
            .expect_err("not a picture");
        assert_eq!(
            error,
            GenerateError::InvalidPicture {
                job: 1,
                cause: cause.clone()
            }
        );
        assert!(rig.stored_files().is_empty(), "{cause:?}");
    }
}

#[tokio::test]
async fn a_library_that_cannot_take_the_file_is_a_typed_store_failure() {
    let rig = Rig::new().await;
    // A plain file where the library's folder belongs can never be made into a directory.
    std::fs::create_dir_all(rig.running.playground_path("assets")).expect("makes a directory");
    std::fs::write(rig.media(), b"a file").expect("writes");
    let generator = rig.generator(FakeBackend::new(Script::Picture(picture())));
    let error = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect_err("not stored");
    assert!(
        matches!(error, GenerateError::StoreFailed { job: 1, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn one_name_cannot_be_generated_twice_at_once() {
    let rig = Rig::new().await;
    let backend = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(300));
    let generator = rig.generator(backend);
    let first = generator
        .start(&rig.running.state, request("hall"))
        .await
        .expect("starts");
    let second = rig.refused_at_start(&generator, request("hall")).await;
    assert_eq!(
        second,
        GenerateError::NameBusy {
            name: "hall".to_owned()
        }
    );
    let other = generator
        .start(&rig.running.state, request("other"))
        .await
        .expect("another name is fine");
    assert_eq!(other.view.job, 2);
    first.finished.await.expect("finishes").expect("stored");
    other.finished.await.expect("finishes").expect("stored");
    rig.run_to_end(&generator, request("hall"))
        .await
        .expect("free again once it ended");
    assert_eq!(generator.views().len(), 3);
}

#[tokio::test]
async fn a_sidecar_that_cannot_be_written_never_fails_the_job() {
    let rig = Rig::new().await;
    let blocker = rig.provenance.path().join("blocker");
    std::fs::write(&blocker, b"a file").expect("writes");
    let generator = Arc::new(
        ImageGenerator::new(FakeBackend::new(Script::Picture(picture())))
            .with_provenance_directory(blocker.join("out")),
    );
    let view = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    let (_, sidecar) = stored(&view);
    assert!(
        matches!(sidecar, Some(Sidecar::NotWritten { path, .. })
            if *path == blocker.join("out").join("hall.provenance.json")),
        "{sidecar:?}"
    );
    assert_eq!(rig.stored_files(), vec!["hall.png".to_owned()]);
}

#[tokio::test]
async fn a_running_job_shows_the_time_from_the_injected_clock() {
    let rig = Rig::new().await;
    let clock = Arc::new(ManualClock::starting_at(1_000));
    let backend = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(300));
    let generator = Arc::new(ImageGenerator::new(backend).with_clock(clock.clone()));
    let started = generator
        .start(&rig.running.state, request("hall"))
        .await
        .expect("starts");
    assert_eq!(started.view.state, JobState::Running);
    assert_eq!(started.view.elapsed_ms, 0);
    clock.advance_ms(2_500);
    assert_eq!(generator.views()[0].elapsed_ms, 2_500);
    clock.advance_ms(500);
    let done = started.finished.await.expect("finishes").expect("stored");
    assert_eq!(done.elapsed_ms, 3_000);
    clock.advance_ms(10_000);
    assert_eq!(
        generator.views()[0].elapsed_ms,
        3_000,
        "an ended job keeps its time"
    );
}

#[test]
fn the_deadline_setting_is_whole_seconds_above_zero_or_the_default() {
    assert_eq!(deadline_setting(None), Ok(DEFAULT_DEADLINE));
    assert_eq!(deadline_setting(Some("90")), Ok(Duration::from_secs(90)));
    for text in ["0", "-5", "1.5", "soon"] {
        assert_eq!(
            deadline_setting(Some(text)),
            Err(DeadlineSettingError {
                text: text.to_owned()
            })
        );
    }
}

#[test]
fn the_oldest_finished_jobs_are_forgotten_first_and_running_ones_never() {
    // (job, running)
    let mut jobs = vec![(1, false), (2, true), (3, false), (4, false)];
    forget_oldest_finished(&mut jobs, 3, |(_, running)| *running);
    assert_eq!(jobs, vec![(2, true), (4, false)]);

    let mut all_running = vec![(1, true), (2, true)];
    forget_oldest_finished(&mut all_running, 1, |(_, running)| *running);
    assert_eq!(all_running, vec![(1, true), (2, true)]);
}

#[tokio::test]
async fn a_deadline_setting_that_does_not_read_is_an_audit_line_and_the_default() {
    let rig = Rig::new().await;
    assert_eq!(
        deadline_or_default(&rig.running.state, Some("90")),
        Duration::from_secs(90)
    );
    assert!(rig.lines().is_empty(), "{:?}", rig.lines());
    assert_eq!(
        deadline_or_default(&rig.running.state, Some("soon")),
        DEFAULT_DEADLINE
    );
    assert_eq!(
        rig.lines(),
        vec![format!(
            "IMAGEGEN setting variable={DEADLINE_SECONDS_VARIABLE} status=invalid default_ms={}",
            DEFAULT_DEADLINE.as_millis()
        )]
    );
}
