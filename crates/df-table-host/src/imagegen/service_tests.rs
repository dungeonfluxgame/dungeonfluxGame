//! The generator over a scripted backend: success, timeout, a login that is not there, a refusal
//! by the service, bytes that are not a picture, and storage through the staged upload.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use df_table::testkit::ManualClock;
use tempfile::TempDir;

use df_providers::{FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS, ForgeClient, ForgeConfig};

use super::backend::{BackendError, ForgeImageBackend};
use super::fixtures::{FakeBackend, Script, picture, png_bytes};
use super::limits::DEFAULT_DEADLINE;
use super::model::{GeneratedImage, ImageRequest, Purpose};
use super::request::{GenerateError, JobState, JobView, PictureCause, Provenance, Sidecar};
use super::service::{
    DEADLINE_SECONDS_VARIABLE, DeadlineSettingError, ImageGenerator, deadline_or_default,
    deadline_setting, forget_oldest_finished, shape_kept,
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
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].starts_with("IMAGEGEN check backend=fake duration_ms="));
    assert!(lines[0].ends_with(" result=ok"));
    assert!(
        lines[1].starts_with("IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9")
    );
    assert!(lines[1].ends_with(" running=1"), "{lines:?}");
    assert!(lines[2].starts_with("IMAGEGEN started job=1 wait_ms="));
    assert!(lines[3].starts_with("IMAGEGEN stored job=1 name=hall file=hall.png bytes="));
    assert!(
        lines[3].ends_with(" tokens_in=10 tokens_out=20"),
        "{lines:?}"
    );
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
    let lines = rig.lines();
    assert_eq!(
        lines.len(),
        1,
        "only the check; the caller records a refusal: {lines:?}"
    );
    assert!(lines[0].ends_with(" result=no_login"), "{lines:?}");
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
async fn references_reach_the_backend_in_order_after_the_base() {
    let rig = Rig::new().await;
    std::fs::create_dir_all(rig.media()).expect("makes the library");
    let (first, second) = (flat_png(2, 2, [9, 9, 9]), flat_png(2, 3, [8, 8, 8]));
    for (name, bytes) in [
        ("door.png", b"base".as_slice()),
        ("ref-a.png", first.as_slice()),
        ("ref-b.png", second.as_slice()),
    ] {
        std::fs::write(rig.media().join(name), bytes).expect("writes");
    }
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let request = request("hall")
        .editing("door.png")
        .and_then(|request| request.following(&["ref-a.png".to_owned(), "ref-b.png".to_owned()]))
        .expect("a request")
        .leaving_out(&["ref-c.png".to_owned()]);
    let view = rig.run_to_end(&generator, request).await.expect("stored");
    // The audit line says what the job followed and what its caller left out for want of room;
    // a job with no pictures writes none (the other tests count their lines).
    let lines = rig.lines();
    let references = lines
        .iter()
        .find(|line| line.starts_with("IMAGEGEN references "))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        references.starts_with(concat!(
            "IMAGEGEN references job=1 name=hall base=door.png ",
            "attached=ref-a.png,ref-b.png count=2 maximum="
        )) && references.ends_with(" left_out=ref-c.png"),
        "{references}"
    );
    let asked = backend.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0.base.as_deref(), Some(b"base".as_slice()));
    assert_eq!(asked[0].0.references, [first, second]);
    let (provenance, _) = stored(&view);
    assert!(
        provenance
            .sent_text
            .contains("The reference pictures of people")
    );
}

#[tokio::test]
async fn a_missing_reference_ends_the_job_with_its_name_and_asks_no_backend() {
    let rig = Rig::new().await;
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let request = request("hall")
        .following(&["ref-gone.png".to_owned()])
        .expect("a request");
    let error = rig
        .run_to_end(&generator, request)
        .await
        .expect_err("no reference");
    assert_eq!(
        error,
        GenerateError::ReferenceUnreadable {
            job: 1,
            reference: "ref-gone.png".to_owned(),
            cause: crate::io_cause::IoCause::from_kind(std::io::ErrorKind::NotFound),
        }
    );
    assert!(backend.asked().is_empty());
    assert!(rig.stored_files().is_empty());
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

#[tokio::test]
async fn a_check_and_a_started_line_take_their_times_from_the_injected_clock() {
    let rig = Rig::new().await;
    let clock = Arc::new(ManualClock::starting_at(1_000));
    let slow = FakeBackend::checking_for(Script::Picture(picture()), clock.clone(), 250);
    let generator = Arc::new(ImageGenerator::new(slow).with_clock(clock.clone()));
    let view = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    assert_eq!(view.job, 1);

    let lines = rig.lines();
    assert_eq!(
        lines[..3],
        [
            "IMAGEGEN check backend=fake duration_ms=250 result=ok".to_owned(),
            format!(
                concat!(
                    "IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9 ",
                    "size=1672x941 prompt_chars=24 deadline_ms={} backend=fake running=1"
                ),
                DEFAULT_DEADLINE.as_millis()
            ),
            "IMAGEGEN started job=1 wait_ms=250".to_owned(),
        ],
        "{lines:?}"
    );
    assert_eq!(generator.last_outcome(), Some("stored"));
}

#[tokio::test]
async fn a_requested_line_counts_the_jobs_running_this_one_included() {
    let rig = Rig::new().await;
    let slow = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(300));
    let generator = rig.generator(slow);
    let first = generator
        .start(&rig.running.state, request("hall"))
        .await
        .expect("starts");
    let second = generator
        .start(&rig.running.state, request("yard"))
        .await
        .expect("starts");
    assert_eq!(generator.running(), 2);
    first.finished.await.expect("finishes").expect("stored");
    second.finished.await.expect("finishes").expect("stored");
    assert_eq!(generator.running(), 0);

    let requested: Vec<String> = rig
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("IMAGEGEN requested"))
        .collect();
    assert_eq!(requested.len(), 2, "{requested:?}");
    assert!(requested[0].ends_with(" running=1"), "{requested:?}");
    assert!(requested[1].ends_with(" running=2"), "{requested:?}");
}

#[tokio::test]
async fn the_last_outcome_is_the_key_of_the_job_that_ended_last() {
    let rig = Rig::new().await;
    let generator = rig.generator(FakeBackend::new(Script::Fail(BackendError::Timeout)));
    assert_eq!(generator.last_outcome(), None);
    let ended = rig.run_to_end(&generator, request("hall")).await;
    assert!(ended.is_err());
    assert_eq!(generator.last_outcome(), Some("backend"));
}

#[tokio::test]
async fn a_scratch_folder_left_behind_is_recorded_and_the_picture_still_stored() {
    let rig = Rig::new().await;
    let left_behind = GeneratedImage {
        cleanup: Some(df_imagery::oneshot::CleanupCause::ResourceBusy),
        ..picture()
    };
    let generator = rig.generator(FakeBackend::new(Script::Picture(left_behind)));
    let view = rig
        .run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    assert!(matches!(view.state, JobState::Stored { .. }), "{view:?}");
    let lines = rig.lines();
    assert!(
        lines.contains(&"IMAGEGEN cleanup_failed job=1 kind=resource_busy".to_owned()),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_switched_backend_takes_new_jobs_while_a_running_one_ends_on_its_own() {
    let rig = Rig::new().await;
    let first = FakeBackend::slow(Script::Picture(picture()), Duration::from_millis(300));
    let generator = rig.generator(Arc::clone(&first));
    let started = generator
        .start(&rig.running.state, request("hall"))
        .await
        .expect("starts");
    let second = FakeBackend::new(Script::Picture(picture()));
    let view = generator
        .switch_backend(&rig.running.state, Arc::clone(&second) as _)
        .await
        .expect("switches");
    assert_eq!(view.running, 1);
    started
        .finished
        .await
        .expect("the job finishes")
        .expect("stored");
    rig.run_to_end(&generator, request("yard"))
        .await
        .expect("stored");
    assert_eq!((first.asked().len(), second.asked().len()), (1, 1));
    assert!(
        rig.lines()
            .iter()
            .any(|line| line == "IMAGEGEN switched from=fake to=fake running=1"),
        "{:?}",
        rig.lines()
    );
    // A backend whose preconditions do not hold is refused, and new jobs stay where they were.
    let unset = FakeBackend::failing_check(BackendError::NoCodexHome);
    assert_eq!(
        generator
            .switch_backend(&rig.running.state, unset as _)
            .await
            .map(|_| ()),
        Err(GenerateError::Backend {
            job: None,
            backend: BackendError::NoCodexHome
        })
    );
    rig.run_to_end(&generator, request("gate"))
        .await
        .expect("stored");
    assert_eq!(second.asked().len(), 2);
    // The limits are the backend's new jobs start on.
    let config = ForgeConfig::new("http://127.0.0.1:9", "41fbb3c9-ac91-4897-b890-7a056b4ae371")
        .expect("a loopback origin and a project id");
    let forge = ForgeImageBackend::new(Ok(ForgeClient::new(config).expect("builds")));
    let view = generator
        .switch_backend(&rig.running.state, Arc::new(forge))
        .await
        .expect("switches");
    assert_eq!(view.backend, "forge");
    assert_eq!(
        generator.backend_limits().maximum_text_characters,
        Some(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS)
    );
}

#[tokio::test]
async fn an_emergency_picture_reaches_the_backend_as_one_and_says_so_in_the_log() {
    use super::model::Priority;
    let rig = Rig::new().await;
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    rig.run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    rig.run_to_end(&generator, request("door").prioritised(Priority::Emergency))
        .await
        .expect("stored");
    let asked = backend.asked();
    assert_eq!(asked[0].0.priority, Priority::Normal);
    assert_eq!(
        asked[1].0.priority,
        Priority::Emergency,
        "the backend that has a queue of its own puts it first"
    );
    let lines = rig.lines();
    let priorities: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("IMAGEGEN priority "))
        .collect();
    assert_eq!(
        priorities,
        ["IMAGEGEN priority job=2 name=door level=emergency backend=fake"],
        "a normal request writes none: {lines:?}"
    );
}

#[tokio::test]
async fn a_picture_of_another_shape_than_asked_is_said_so_before_its_stored_line() {
    let rig = Rig::new().await;
    let square = GeneratedImage {
        width_px: 512,
        height_px: 512,
        ..picture()
    };
    let generator = rig.generator(FakeBackend::new(Script::Picture(square)));
    rig.run_to_end(&generator, request("hall"))
        .await
        .expect("stored");
    let lines = rig.lines();
    assert!(
        lines[1].contains(" aspect=16:9 size=1672x941 "),
        "the request says the size asked: {lines:?}"
    );
    assert_eq!(
        lines[3],
        "IMAGEGEN reshaped job=1 name=hall asked=1672x941 delivered=512x512 backend=fake"
    );
    assert!(lines[4].starts_with("IMAGEGEN stored job=1 "), "{lines:?}");
}

#[test]
fn a_backend_s_rounded_sides_keep_the_shape_and_a_square_for_a_wide_one_does_not() {
    assert!(shape_kept((1672, 941), (1664, 928)));
    assert!(shape_kept((512, 512), (512, 512)));
    assert!(!shape_kept((1672, 941), (512, 512)));
    assert!(!shape_kept((512, 512), (1672, 941)));
}

/// A real PNG of `width` by `height` whose pixels are all `rgb`: stored (uncompressed) blocks, so
/// the test needs no encoder.
fn flat_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    }
    // Each row starts with its filter byte (none), then the pixels.
    let row: Vec<u8> = std::iter::once(0)
        .chain(rgb.iter().copied().cycle().take(rgb.len() * width as usize))
        .collect();
    let raw: Vec<u8> = row.repeat(height as usize);
    let (mut low, mut high) = (1_u32, 0_u32);
    for byte in &raw {
        low = (low + u32::from(*byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    let mut zlib = vec![0x78, 0x01, 1];
    let length = u16::try_from(raw.len()).unwrap();
    zlib.extend_from_slice(&length.to_le_bytes());
    zlib.extend_from_slice(&(!length).to_le_bytes());
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&((high << 16) | low).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn size_of_png(png: &[u8]) -> (u32, u32) {
    let side = |from: usize| u32::from_be_bytes(png[from..from + 4].try_into().unwrap());
    (side(16), side(20))
}

#[tokio::test]
async fn an_edit_is_shown_the_front_view_of_a_multi_view_sheet_and_a_fresh_picture_the_whole() {
    let rig = Rig::new().await;
    std::fs::create_dir_all(rig.media()).expect("makes the library");
    let sheet = flat_png(9, 3, [200, 10, 10]);
    let portrait = flat_png(3, 6, [10, 200, 10]);
    for (name, bytes) in [
        ("door.png", b"base".to_vec()),
        ("ref-wide.png", sheet.clone()),
        ("ref-tall.png", portrait.clone()),
    ] {
        std::fs::write(rig.media().join(name), bytes).expect("writes");
    }
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let people = ["ref-wide.png".to_owned(), "ref-tall.png".to_owned()];
    let edit = request("hall")
        .editing("door.png")
        .and_then(|request| request.following(&people))
        .expect("a request");
    rig.run_to_end(&generator, edit).await.expect("stored");
    let fresh = request("lane").following(&people).expect("a request");
    rig.run_to_end(&generator, fresh).await.expect("stored");
    let asked = backend.asked();
    assert_eq!(
        size_of_png(&asked[0].0.references[0]),
        (3, 3),
        "the edit is shown the front third of the sheet, not three people side by side"
    );
    assert_eq!(
        asked[0].0.references[1], portrait,
        "a sheet with one view is shown as it is"
    );
    assert_eq!(
        asked[1].0.references,
        [sheet, portrait],
        "a picture painted anew is shown the whole sheet"
    );
    let views: Vec<String> = rig
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("IMAGEGEN sheet_view "))
        .collect();
    assert_eq!(
        views,
        ["IMAGEGEN sheet_view job=1 name=hall picture=ref-wide.png view=front"]
    );
}

#[tokio::test]
async fn a_picture_that_asks_for_the_full_length_view_is_shown_it_whatever_its_priority() {
    use super::model::SheetPick;
    let rig = Rig::new().await;
    std::fs::create_dir_all(rig.media()).expect("makes the library");
    std::fs::write(
        rig.media().join("ref-wide.png"),
        flat_png(9, 3, [200, 10, 10]),
    )
    .expect("writes");
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let people = ["ref-wide.png".to_owned()];
    let request = request("door")
        .following(&people)
        .expect("a request")
        .showing_sheets(Some(SheetPick::FullLength));
    rig.run_to_end(&generator, request).await.expect("stored");
    assert_eq!(size_of_png(&backend.asked()[0].0.references[0]), (3, 3));
    let views: Vec<String> = rig
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("IMAGEGEN sheet_view "))
        .collect();
    assert_eq!(
        views,
        ["IMAGEGEN sheet_view job=1 name=door picture=ref-wide.png view=full_length"]
    );
}

/// The pictures the fake backend was shown and the `sheet_view` lines, for a request on a 9 by 3
/// sheet (three 3 by 3 views).
async fn views_of(
    make: impl FnOnce(super::model::ImageRequest) -> super::model::ImageRequest,
) -> (Vec<(u32, u32)>, Vec<String>) {
    let rig = Rig::new().await;
    std::fs::create_dir_all(rig.media()).expect("makes the library");
    std::fs::write(
        rig.media().join("ref-wide.png"),
        flat_png(9, 3, [200, 10, 10]),
    )
    .expect("writes");
    std::fs::write(rig.media().join("base.png"), flat_png(4, 4, [1, 2, 3])).expect("writes");
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let people = ["ref-wide.png".to_owned()];
    let request = make(request("door").following(&people).expect("a request"));
    rig.run_to_end(&generator, request).await.expect("stored");
    let shown = backend.asked()[0]
        .0
        .references
        .iter()
        .map(|bytes| size_of_png(bytes))
        .collect();
    let lines = rig
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("IMAGEGEN sheet_view "))
        .collect();
    (shown, lines)
}

#[tokio::test]
async fn the_view_of_a_sheet_follows_the_kind_of_request_not_its_priority() {
    use super::model::{Priority, SheetPick};
    // Painted anew and saying nothing: the whole sheet, emergency or not.
    for priority in [Priority::Normal, Priority::Emergency] {
        let (shown, lines) = views_of(|request| request.prioritised(priority)).await;
        assert_eq!(shown, [(9, 3)], "{priority:?}");
        assert!(lines.is_empty(), "{lines:?}");
    }
    // An edit saying nothing: the front view.
    let (shown, lines) = views_of(|request| request.editing("base.png").expect("an edit")).await;
    assert_eq!(shown, [(3, 3)]);
    assert_eq!(
        lines,
        ["IMAGEGEN sheet_view job=1 name=door picture=ref-wide.png view=front"]
    );
    // What the caller asks for wins over the kind: the full-length view of a picture painted anew
    // (a walking figure), the whole sheet of an edit, the front view of a picture painted anew.
    let (shown, lines) =
        views_of(|request| request.showing_sheets(Some(SheetPick::FullLength))).await;
    assert_eq!(shown, [(3, 3)]);
    assert_eq!(
        lines,
        ["IMAGEGEN sheet_view job=1 name=door picture=ref-wide.png view=full_length"]
    );
    let (shown, _) = views_of(|request| {
        request
            .editing("base.png")
            .expect("an edit")
            .showing_sheets(Some(SheetPick::Whole))
    })
    .await;
    assert_eq!(shown, [(9, 3)]);
    let (shown, lines) = views_of(|request| request.showing_sheets(Some(SheetPick::Front))).await;
    assert_eq!(shown, [(3, 3)]);
    assert!(lines[0].ends_with("view=front"), "{lines:?}");
}

#[tokio::test]
async fn the_soft_cap_holds_the_pictures_a_plate_carries_below_the_backend_and_never_above_it() {
    let rig = Rig::new().await;
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    let limits = |generator: &ImageGenerator| generator.backend_limits().maximum_pictures;
    let backend_maximum = super::backend::ImageBackend::limits(backend.as_ref()).maximum_pictures;
    assert!(backend_maximum > 3);
    assert_eq!(
        limits(&generator),
        3,
        "three pictures unless the owner sets another"
    );
    generator.set_reference_cap(&rig.running.state, 2);
    assert_eq!(limits(&generator), 2);
    assert_eq!(generator.backend_view().reference_cap, 2);
    generator.set_reference_cap(&rig.running.state, 50);
    assert_eq!(
        limits(&generator),
        backend_maximum,
        "never above what the backend takes"
    );
    assert_eq!(generator.backend_view().reference_cap, 50);
    let lines = rig.lines();
    assert!(
        lines.contains(&format!(
            "IMAGEGEN reference_cap maximum=2 backend_maximum={backend_maximum}"
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&format!(
            "IMAGEGEN reference_cap maximum={backend_maximum} backend_maximum={backend_maximum}"
        )),
        "{lines:?}"
    );
}

#[tokio::test]
async fn the_references_line_says_the_soft_cap_and_the_backend_maximum() {
    let rig = Rig::new().await;
    std::fs::create_dir_all(rig.media()).expect("makes the library");
    std::fs::write(rig.media().join("ref-a.png"), flat_png(2, 2, [1, 1, 1])).expect("writes");
    let backend = FakeBackend::new(Script::Picture(picture()));
    let generator = rig.generator(Arc::clone(&backend));
    generator.set_reference_cap(&rig.running.state, 2);
    let edit = request("hall")
        .following(&["ref-a.png".to_owned()])
        .expect("a request")
        .leaving_out(&["item-danny-brushes.png".to_owned()]);
    rig.run_to_end(&generator, edit).await.expect("stored");
    let backend_maximum = super::backend::ImageBackend::limits(backend.as_ref()).maximum_pictures;
    let lines = rig.lines();
    let references = lines
        .iter()
        .find(|line| line.starts_with("IMAGEGEN references "))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        references.contains(&format!(
            " count=1 maximum=2 backend_maximum={backend_maximum} left_out=item-danny-brushes.png"
        )),
        "{references}"
    );
}
