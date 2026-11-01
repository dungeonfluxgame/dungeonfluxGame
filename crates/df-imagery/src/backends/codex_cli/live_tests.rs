//! One real Codex CLI image turn through the same `execute` path the service uses.
//! Runs only with `DF_LIVE_CODEX=1`, because it spends Codex quota. `DF_CODEX_EXE` may name the
//! binary; the default is the Desktop install.

use super::*;
use crate::backends::codex_cli::process::TokioProcessRunner;
use crate::output::{Dimensions, OutputFormat};
use crate::request::TextPrompt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[tokio::test]
async fn live_codex_generates_one_png() {
    if std::env::var("DF_LIVE_CODEX").as_deref() != Ok("1") {
        eprintln!("skipped: set DF_LIVE_CODEX=1 to spend Codex quota on a real image");
        return;
    }
    let executable = std::env::var("DF_CODEX_EXE").unwrap_or_else(|_| {
        r"C:\Users\mreca\AppData\Local\Programs\OpenAI\Codex\bin\codex.exe".to_owned()
    });
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("artifacts")
        .join("tmp")
        .join("live-codex")
        .join(stamp.to_string());
    for directory in ["scratch", "inputs", "outputs"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    // The same limits the service applies to every Codex dispatch, from its default image config.
    let defaults = crate::config::ImageConfig::defaults().unwrap();
    let config = CodexCliConfig::new(PathBuf::from(executable))
        .unwrap()
        .with_request_limits(&defaults.request_limits)
        .unwrap()
        .with_max_artifact_bytes(defaults.transforms.max_output_bytes)
        .unwrap()
        .with_max_message_bytes(defaults.codex.max_message_bytes)
        .unwrap()
        .with_max_stdout_bytes(defaults.codex.max_stdout_bytes)
        .unwrap()
        .with_max_stderr_bytes(defaults.codex.max_stderr_bytes)
        .unwrap();
    let request = CodexImageRequest::new(
        TextPrompt::new(
            "A plain solid red square on a white background, 1024 by 1024 pixels.",
            1,
        )
        .unwrap(),
        ImageKind::Generate,
        Vec::new(),
        Dimensions::new(1024, 1024).unwrap(),
        AlphaRequirement::Either,
        OutputFormat::Png,
    );
    let attempt = AttemptDirs::new(
        root.join("scratch"),
        root.join("inputs"),
        root.join("outputs"),
    );
    let outcome = execute(
        &config,
        &request,
        &attempt,
        &TokioProcessRunner,
        Duration::from_secs(600),
        None,
    )
    .await;
    match outcome {
        CodexOutcome::Ready { receipt, usage } => {
            let bytes = std::fs::read(receipt.path()).unwrap();
            let copy = root.join("result.png");
            std::fs::write(&copy, &bytes).unwrap();
            eprintln!(
                "LIVE OK: {}x{} {} {} bytes, usage={:?}, saved to {}",
                receipt.width_px(),
                receipt.height_px(),
                receipt.mime(),
                receipt.byte_len(),
                usage,
                copy.display()
            );
            // The image tool picks its own size; the worker resizes to the delivered profile.
            assert!(receipt.width_px() > 0 && receipt.height_px() > 0);
            assert_eq!(receipt.mime(), "image/png");
            assert_eq!(receipt.byte_len(), bytes.len());
        }
        other => panic!("live Codex turn did not produce an image: {other:?}"),
    }
}
