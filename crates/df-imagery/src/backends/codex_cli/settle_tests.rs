use super::*;
use crate::output::OutputFormat;

fn scratch_root(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir();
    let root = base.join(format!("df-codex-settle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("outputs")).unwrap();
    root
}

fn write_png(path: &Path, width: u32, height: u32) {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        pixels.extend_from_slice(&[200u8, 200u8, 200u8, 255u8]);
    }
    let mut encoded = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    std::fs::write(path, encoded).unwrap();
}

fn item(id: &str) -> ImageItem {
    ImageItem {
        id: id.to_owned(),
        status: "completed".to_owned(),
        file_id: None,
        has_result: false,
        result_len: 0,
        saved_path: None,
        failure: None,
        transparent_background: None,
        revised_prompt_present: false,
        input_images: None,
    }
}

#[tokio::test]
async fn started_blocking_work_is_drained_not_detached_at_deadline() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let done = Arc::new(AtomicBool::new(false));
    let mark = done.clone();
    let (started, did_start) = tokio::sync::oneshot::channel();
    let mut slot = Some(tokio::task::spawn_blocking(move || {
        started.send(()).expect("started work observed");
        std::thread::sleep(std::time::Duration::from_millis(100));
        mark.store(true, Ordering::SeqCst);
    }));
    did_start.await.expect("blocking work started");
    let deadline_at = Instant::now();
    let result = join_blocking(&mut slot, deadline_at).await;
    assert!(result.is_none(), "the expired wait reports no value");
    assert!(slot.is_none(), "the aborted handle is rejoined");
    assert!(
        done.load(Ordering::SeqCst),
        "started blocking work finishes before return, never late"
    );
}

#[tokio::test]
async fn expired_import_creates_no_file_and_keeps_collisions() {
    let root = scratch_root("expired");
    let outputs = root.join("outputs");
    let original = outputs.join("original.png");
    write_png(&original, 8, 4);
    let verified = verify_artifact(
        &original.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        false,
    )
    .unwrap();
    let collision = outputs.join("item-1.png");
    std::fs::write(&collision, b"pre-existing collision").unwrap();
    let deadline_at = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(1));
    let error = import_verified_bytes(
        verified,
        &outputs,
        &item("item-1"),
        1024 * 1024,
        deadline_at,
    )
    .await
    .unwrap_err();
    assert_eq!(error, ArtifactError::Unreadable);
    assert_eq!(
        std::fs::read(&collision).unwrap(),
        b"pre-existing collision",
        "an expired import never deletes a collision"
    );
    assert!(
        !outputs.join("item-1-1.png").exists(),
        "no late orphan appears after the expired return"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn reread_failure_removes_only_the_owned_import() {
    let root = scratch_root("reread-cleanup");
    let outputs = root.join("outputs");
    let original = outputs.join("original.png");
    write_png(&original, 8, 4);
    let verified = verify_artifact(
        &original.to_string_lossy(),
        &outputs,
        1024 * 1024,
        OutputFormat::Png,
        false,
    )
    .unwrap();
    let collision = outputs.join("item-1.png");
    std::fs::write(&collision, b"unrelated collision bytes").unwrap();
    let error = import_verified_bytes(
        verified,
        &outputs,
        &item("item-1"),
        10,
        Instant::now() + std::time::Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    assert_eq!(error, ArtifactError::Unreadable);
    assert_eq!(
        std::fs::read(&collision).unwrap(),
        b"unrelated collision bytes",
        "the pre-existing collision is never deleted"
    );
    assert!(
        !outputs.join("item-1-1.png").exists(),
        "the owned partial import is removed"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn late_open_does_not_leave_an_orphan_after_deadline() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = scratch_root("late-open");
        let outputs = root.join("outputs");
        let original = outputs.join("original.png");
        write_png(&original, 8, 4);
        let verified = verify_artifact(
            &original.to_string_lossy(),
            &outputs,
            1024 * 1024,
            OutputFormat::Png,
            false,
        )
        .unwrap();
        let item = item("item-1");
        let expected = outputs.join("item-1.png");
        let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = started.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let blocker = tokio::task::spawn_blocking(move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = release_rx.blocking_recv();
        });
        while !started.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        let deadline_at = Instant::now() + std::time::Duration::from_millis(50);
        let outputs_clone = outputs.clone();
        let import = tokio::spawn(async move {
            import_verified_bytes(verified, &outputs_clone, &item, 1024 * 1024, deadline_at).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = release_tx.send(());
        let result = import.await.unwrap();
        assert_eq!(result.unwrap_err(), ArtifactError::Unreadable);
        assert!(
            !expected.exists(),
            "the late open is awaited then cleaned, never orphaned"
        );
        let _ = blocker.await;
        tokio::task::yield_now().await;
        assert!(
            !expected.exists(),
            "no zero-byte file appears after the workers finish"
        );
        let _ = std::fs::remove_dir_all(&root);
    });
}

#[test]
fn generated_images_live_under_the_configured_codex_home_only() {
    let home = std::env::temp_dir().join("df-codex-settle-home");
    assert_eq!(
        generated_images_root(Some(&home), "thread-1"),
        Some(home.join("generated_images").join("thread-1"))
    );
    assert_eq!(generated_images_root(None, "thread-1"), None);
    assert_eq!(generated_images_root(Some(&home), "../thread"), None);
    assert_eq!(generated_images_root(Some(&home), ""), None);
}
