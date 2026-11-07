use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use crate::blocking::run_owned;

#[tokio::test]
async fn owned_work_finishes_after_the_request_that_waited_for_it_is_dropped() {
    let gate = Arc::new(Notify::new());
    let finished = Arc::new(AtomicBool::new(false));
    let work = {
        let (gate, finished) = (Arc::clone(&gate), Arc::clone(&finished));
        async move {
            gate.notified().await;
            finished.store(true, Ordering::SeqCst);
            Ok::<_, &'static str>(7)
        }
    };
    // The client leaves: the request future is dropped while the work is still waiting.
    let waited = tokio::time::timeout(Duration::from_millis(20), run_owned(work, "stopped")).await;
    assert!(
        waited.is_err(),
        "the request was still waiting when it was dropped"
    );
    assert!(!finished.load(Ordering::SeqCst));
    gate.notify_one();
    for _ in 0..100 {
        if finished.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the owned work was cancelled with its request");
}

#[tokio::test]
async fn owned_work_that_panics_is_reported_as_stopped() {
    let result = run_owned(
        async { panic!("the work failed") as Result<u8, &'static str> },
        "stopped",
    )
    .await;
    assert_eq!(result, Err("stopped"));
}
