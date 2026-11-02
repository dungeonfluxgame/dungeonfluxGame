use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::repl::{LineResult, run_line, run_lines};
use crate::test_support::{STARTER_FILES, closed_port, start};

#[tokio::test]
async fn blank_lines_and_comments_are_skipped_and_quit_ends_the_session() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(run_line(running.port, "").await, LineResult::Skipped);
    assert_eq!(
        run_line(running.port, "   # a note").await,
        LineResult::Skipped
    );
    assert_eq!(run_line(running.port, "quit").await, LineResult::Quit);
    assert_eq!(run_line(running.port, "exit").await, LineResult::Quit);
}

#[tokio::test]
async fn a_line_is_an_ordinary_command_with_its_answer() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        run_line(running.port, "spawn marker m1 x=10 y=10").await,
        LineResult::Ok("ok (revision 1)".to_owned())
    );
    assert!(matches!(
        run_line(running.port, "kinds").await,
        LineResult::Ok(text) if text.contains("marker")
    ));
}

#[tokio::test]
async fn a_failing_line_is_reported_and_the_session_goes_on() {
    let running = start(&STARTER_FILES).await;
    assert!(
        matches!(run_line(running.port, "dance").await, LineResult::Failed(text) if text.contains("unknown command"))
    );
    assert!(matches!(
        run_line(running.port, "spawn dragon smaug").await,
        LineResult::Failed(_)
    ));
    assert!(
        matches!(run_line(running.port, "spawn marker \"unclosed").await, LineResult::Failed(text) if text.contains("quote"))
    );
}

#[tokio::test]
async fn script_and_repl_cannot_be_started_from_inside_a_session() {
    let running = start(&STARTER_FILES).await;
    for line in ["script a.txt", "repl"] {
        assert!(
            matches!(run_line(running.port, line).await, LineResult::Failed(text) if text.contains("cannot be used")),
            "{line}"
        );
    }
}

#[tokio::test]
async fn with_nothing_serving_each_line_says_how_to_start_it() {
    let port = closed_port().await;
    assert!(
        matches!(run_line(port, "canvas").await, LineResult::Failed(text) if text.contains("df-table serve"))
    );
}

#[tokio::test]
async fn a_session_runs_every_line_prints_answers_and_failures_apart_and_counts_failures() {
    let running = start(&STARTER_FILES).await;
    let (sender, receiver) = mpsc::unbounded_channel();
    for line in [
        "spawn marker a",
        "# note",
        "spawn marker a",
        "spawn marker b",
        "quit",
        "spawn marker never",
    ] {
        sender.send(line.to_owned()).expect("sends");
    }
    let answers = Arc::new(Mutex::new(Vec::new()));
    let failures_seen = Arc::new(Mutex::new(Vec::new()));
    let (out, err) = (Arc::clone(&answers), Arc::clone(&failures_seen));
    let failures = run_lines(
        running.port,
        receiver,
        move |text| out.lock().expect("not poisoned").push(text.to_owned()),
        move |text| err.lock().expect("not poisoned").push(text.to_owned()),
    )
    .await;
    assert_eq!(failures, 1);
    assert_eq!(answers.lock().expect("not poisoned").len(), 2);
    assert!(failures_seen.lock().expect("not poisoned")[0].contains("already exists"));
    assert_eq!(running.state.canvas().len(), 2, "nothing after quit ran");
}

#[tokio::test]
async fn a_session_ends_when_its_input_does() {
    let running = start(&STARTER_FILES).await;
    let (sender, receiver) = mpsc::unbounded_channel();
    sender.send("spawn marker a".to_owned()).expect("sends");
    drop(sender);
    assert_eq!(run_lines(running.port, receiver, |_| {}, |_| {}).await, 0);
}

#[tokio::test]
async fn a_command_in_a_session_is_fast_enough_to_move_things_at_inference_speed() {
    let running = start(&STARTER_FILES).await;
    run_line(running.port, "spawn marker m1 x=10 y=10").await;
    let started = Instant::now();
    for step in 0..100 {
        let line = format!("move m1 {} 20", 5 + step % 90);
        assert!(matches!(
            run_line(running.port, &line).await,
            LineResult::Ok(_)
        ));
    }
    let each = started.elapsed() / 100;
    // The bound is generous for a loaded machine; a process per command costs about 58 ms.
    assert!(
        each < std::time::Duration::from_millis(15),
        "{each:?} per command"
    );
}
