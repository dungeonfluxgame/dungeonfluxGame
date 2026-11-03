//! The request words, the shapes, and the lines of the audit log.

use std::time::Duration;

use super::events::ImageGenEvent;
use super::model::{
    Aspect, ImageRequest, MAXIMUM_NAME_CHARS, MAXIMUM_PROMPT_CHARS, Purpose, RefusalCause,
    RequestProblem,
};
use crate::command::{Command, CommandError, MediaGenerate, parse, split_words};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("splits")
}

fn parsed(line: &str) -> Result<Command, CommandError> {
    parse(&words(line))
}

#[test]
fn a_wide_aspect_asks_for_the_size_codex_gives_a_wide_picture() {
    assert_eq!(Aspect::WIDE.requested_size(), (1672, 941));
    assert_eq!(
        Aspect::parse("3:4").expect("a shape").requested_size(),
        (1254, 1672)
    );
    assert_eq!(
        Aspect::parse("1:1").expect("a shape").requested_size(),
        (1672, 1672)
    );
}

#[test]
fn a_shape_is_two_whole_numbers_within_four_to_one() {
    for text in ["16:9", "1:4", "4:1", "64:64", "3:4"] {
        assert!(Aspect::parse(text).is_some(), "{text}");
    }
    for text in [
        "", "16", "16:", ":9", "0:9", "16:0", "a:b", "1:5", "5:1", "65:64", "16:9:1", "-1:2",
    ] {
        assert!(Aspect::parse(text).is_none(), "{text}");
    }
}

#[test]
fn a_request_checks_every_word() {
    let request = ImageRequest::new("a hall", "hall-1", None, None).expect("a request");
    assert_eq!(request.purpose, Purpose::Background);
    assert_eq!(request.aspect, Aspect::WIDE);
    let portrait = ImageRequest::new("a face", "face", Some("portrait"), None).expect("a request");
    assert_eq!(portrait.aspect.label(), "3:4");
    let problem = |prompt: &str, name: &str, purpose: Option<&str>, aspect: Option<&str>| {
        ImageRequest::new(prompt, name, purpose, aspect).expect_err("refused")
    };
    assert_eq!(problem("  ", "a", None, None), RequestProblem::EmptyPrompt);
    assert!(matches!(
        problem(&"x".repeat(MAXIMUM_PROMPT_CHARS + 1), "a", None, None),
        RequestProblem::PromptTooLong { .. }
    ));
    for name in [
        "",
        "a b",
        "a.png",
        "../a",
        "a/b",
        "é",
        &"n".repeat(MAXIMUM_NAME_CHARS + 1),
    ] {
        assert!(
            matches!(
                problem("p", name, None, None),
                RequestProblem::BadName { .. }
            ),
            "{name}"
        );
    }
    assert!(matches!(
        problem("p", "a", Some("poster"), None),
        RequestProblem::BadPurpose { .. }
    ));
    assert!(matches!(
        problem("p", "a", None, Some("wide")),
        RequestProblem::BadAspect { .. }
    ));
}

#[test]
fn the_backend_reads_the_prompt_first_then_the_purpose() {
    let request = ImageRequest::new("a candlelit hall", "hall", Some("plate"), None).expect("ok");
    let text = request.sent_text();
    assert!(text.starts_with("a candlelit hall"));
    assert!(text.contains("scene plate"));
}

#[test]
fn an_audit_line_never_holds_the_prompt() {
    let secret = "the dragon's private secret plan";
    let request = ImageRequest::new(secret, "hall", None, None).expect("ok");
    let line = ImageGenEvent::Requested {
        job: 1,
        name: request.name.clone(),
        purpose: request.purpose,
        aspect: request.aspect,
        prompt_chars: request.prompt.chars().count(),
        deadline: Duration::from_secs(900),
        backend: "fake",
    }
    .line();
    assert_eq!(
        line,
        format!(
            "IMAGEGEN requested job=1 name=hall purpose=background aspect=16:9 prompt_chars={} deadline_s=900 backend=fake",
            secret.chars().count()
        )
    );
    assert!(!line.contains("dragon"));
}

#[test]
fn a_refused_request_cannot_break_the_shape_of_its_line() {
    let line = ImageGenEvent::refused(
        "bad name\nIMAGEGEN stored",
        RefusalCause::BadRequest,
        "fake",
    )
    .line();
    assert_eq!(line.lines().count(), 1);
    assert_eq!(
        line,
        "IMAGEGEN refused name=bad_name_IMAGEGEN_stored cause=bad_request backend=fake"
    );
    assert_eq!(
        ImageGenEvent::refused("", RefusalCause::NotConfigured, "none").line(),
        "IMAGEGEN refused name=_ cause=not_configured backend=none"
    );
}

#[test]
fn the_stored_and_failed_lines_carry_ids_sizes_and_milliseconds() {
    let stored = ImageGenEvent::Stored {
        job: 2,
        name: "hall".to_owned(),
        file: "hall.png".to_owned(),
        bytes: 99,
        width_px: 1672,
        height_px: 941,
        duration: Duration::from_millis(1500),
        backend: "fake",
        sha256_prefix: "0123456789abcdef".to_owned(),
    };
    assert_eq!(
        stored.line(),
        "IMAGEGEN stored job=2 name=hall file=hall.png bytes=99 size=1672x941 duration_ms=1500 backend=fake sha256=0123456789abcdef"
    );
    let failed = ImageGenEvent::Failed {
        job: 2,
        name: "hall".to_owned(),
        cause: RefusalCause::Timeout,
        duration: Duration::from_secs(2),
        backend: "fake",
    };
    assert_eq!(
        failed.line(),
        "IMAGEGEN failed job=2 name=hall cause=timeout duration_ms=2000 backend=fake"
    );
}

#[test]
fn media_generate_parses_a_prompt_with_options() {
    let command = parsed(
        "media generate a candlelit hall, at dusk: calm as=hall purpose=plate aspect=16:9 wait=true",
    )
    .expect("parses");
    assert_eq!(
        command,
        Command::MediaGenerate(MediaGenerate::Generate {
            prompt: "a candlelit hall, at dusk: calm".to_owned(),
            name: "hall".to_owned(),
            purpose: Some("plate".to_owned()),
            aspect: Some("16:9".to_owned()),
            wait: true,
        })
    );
    let quoted = parsed("media generate \"one word=prompt\" as=x").expect("parses");
    assert_eq!(
        quoted,
        Command::MediaGenerate(MediaGenerate::Generate {
            prompt: "one word=prompt".to_owned(),
            name: "x".to_owned(),
            purpose: None,
            aspect: None,
            wait: false,
        })
    );
    assert_eq!(
        parsed("media generate status").expect("parses"),
        Command::MediaGenerate(MediaGenerate::Status)
    );
}

#[test]
fn media_generate_refuses_what_it_cannot_send() {
    assert!(matches!(
        parsed("media generate a hall"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parsed("media generate as=hall"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parsed("media generate"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parsed("media generate a as=b as=c"),
        Err(CommandError::DuplicateKey(key)) if key == "as"
    ));
    for line in [
        "media generate a as=hall.png",
        "media generate a as=hall purpose=poster",
        "media generate a as=hall aspect=wide",
        "media generate a as=hall wait=maybe",
    ] {
        assert!(
            matches!(parsed(line), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
}

#[test]
fn media_generate_is_in_the_help() {
    let help = crate::command::usage_text();
    assert!(help.contains("media generate <prompt...> as=NAME"));
}
