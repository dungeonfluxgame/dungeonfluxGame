use super::*;

fn fixture_base() -> std::path::PathBuf {
    // The fake runner validates the spec without launching it, so the
    // fixture must be absolute on every platform; the executable itself
    // need not exist. Production validation is unchanged.
    if let Some(dir) = std::env::var_os("TMPDIR")
        && !dir.is_empty()
    {
        return std::path::PathBuf::from(dir);
    }
    std::env::temp_dir()
}

fn spec() -> ProcessSpec {
    ProcessSpec {
        executable: fixture_base().join("df-codex-fake-codex.exe"),
        args: vec![std::ffi::OsString::from("app-server")],
        cwd: fixture_base().join("df-codex-fake-scratch"),
    }
}

#[test]
fn relative_executable_is_rejected() {
    let mut bad = spec();
    bad.executable = std::path::PathBuf::from("codex");
    assert_eq!(bad.validate().unwrap_err(), ProcessError::Configuration);
}

#[tokio::test]
async fn fake_session_records_sends_and_lines() {
    let runner = FakeRunner::new(vec![b"{\"a\":1}".to_vec()]);
    let limits = StreamLimits {
        max_stdout_bytes: 1024,
        max_stderr_bytes: 256,
        max_line_bytes: 512,
    };
    let mut session = runner.spawn(&spec(), &limits).await.unwrap();
    session.send(b"hello").await.unwrap();
    assert_eq!(
        session.next_line(512).await.unwrap(),
        Some(b"{\"a\":1}".to_vec())
    );
    assert_eq!(session.next_line(512).await.unwrap(), None);
    session.finish().await.unwrap();
    assert_eq!(runner.shared.sends.lock().unwrap().len(), 1);
    assert_eq!(
        runner
            .shared
            .kills
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn spawn_failure_reports_transport_without_a_session() {
    let runner = FakeRunner::new(Vec::new()).with_spawn_error(ProcessError::SpawnFailed {
        kind: std::io::ErrorKind::NotFound,
    });
    let limits = StreamLimits {
        max_stdout_bytes: 1024,
        max_stderr_bytes: 256,
        max_line_bytes: 512,
    };
    let error = runner.spawn(&spec(), &limits).await.unwrap_err();
    assert_eq!(
        error,
        ProcessError::SpawnFailed {
            kind: std::io::ErrorKind::NotFound,
        }
    );
    assert_eq!(
        runner
            .shared
            .spawns
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn oversize_line_is_rejected() {
    let runner = FakeRunner::new(vec![vec![b'x'; 64]]);
    let limits = StreamLimits {
        max_stdout_bytes: 4096,
        max_stderr_bytes: 256,
        max_line_bytes: 16,
    };
    let mut session = runner.spawn(&spec(), &limits).await.unwrap();
    assert_eq!(
        session.next_line(16).await.unwrap_err(),
        ProcessError::OutputTooLarge
    );
}

#[tokio::test]
async fn join_drain_timeout_reaps_a_stuck_task_without_detaching() {
    struct DropMark(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropMark {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mark = DropMark(flag.clone());
    let handle = tokio::spawn(async move {
        let _mark = mark;
        std::future::pending::<()>().await;
    });
    let mut slot = Some(handle);
    join_drain_timeout(&mut slot, std::time::Duration::from_millis(20)).await;
    assert!(slot.is_none(), "the drained slot is consumed");
    assert!(
        flag.load(std::sync::atomic::Ordering::SeqCst),
        "the stuck drain was aborted and reaped, never detached"
    );
    let mut empty: Option<tokio::task::JoinHandle<()>> = None;
    join_drain_timeout(&mut empty, std::time::Duration::from_millis(1)).await;
}

#[tokio::test]
async fn completed_drain_is_not_polled_twice() {
    // A handle that already finished is consumed without a second poll:
    // the old take-then-timeout-then-await panicked here.
    let finished = tokio::spawn(async {});
    let mut slot = Some(finished);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    join_drain_timeout(&mut slot, std::time::Duration::from_secs(1)).await;
    assert!(slot.is_none(), "the completed drain is consumed once");
}

#[tokio::test]
async fn cancelled_drain_wait_keeps_the_slot_for_cleanup() {
    struct DropMark(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropMark {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mark = DropMark(flag.clone());
    let handle = tokio::spawn(async move {
        let _mark = mark;
        std::future::pending::<()>().await;
    });
    let mut slot = Some(handle);
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(20),
        join_drain_timeout(&mut slot, std::time::Duration::from_secs(10)),
    )
    .await;
    assert!(
        slot.is_some(),
        "cancelling the wait must keep the owned handle"
    );
    assert!(
        !flag.load(std::sync::atomic::Ordering::SeqCst),
        "the stuck drain is still owned, not detached"
    );
    join_drain_timeout(&mut slot, std::time::Duration::from_millis(20)).await;
    assert!(slot.is_none(), "cleanup reaps the retained drain");
    assert!(
        flag.load(std::sync::atomic::Ordering::SeqCst),
        "the retained drain was aborted and reaped"
    );
}

#[tokio::test]
async fn helper_and_drain_tasks_do_not_survive_kill_or_drop() {
    let runner = FakeRunner::new(vec![b"{\"a\":1}".to_vec()]);
    let limits = StreamLimits {
        max_stdout_bytes: 1024,
        max_stderr_bytes: 256,
        max_line_bytes: 512,
    };
    let mut session = runner.spawn(&spec(), &limits).await.unwrap();
    assert!(
        runner
            .shared
            .helper_alive
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    session.kill().await.unwrap();
    assert!(
        !runner
            .shared
            .helper_alive
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(
        !runner
            .shared
            .drain_alive
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    let mut session = runner.spawn(&spec(), &limits).await.unwrap();
    let _ = session.next_line(512).await;
    drop(session);
    assert!(
        !runner
            .shared
            .helper_alive
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}
