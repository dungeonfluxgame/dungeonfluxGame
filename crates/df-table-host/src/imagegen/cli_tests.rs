//! The grammar of `media generate`: a prompt in words, then `key=value` options.

use super::cli::{MEDIA_GENERATE_USAGE, MediaGenerate, parse};
use super::limits::MAXIMUM_SENT_TEXT_CHARS;
use super::model::{Aspect, Purpose};
use super::request::GenerateRequest;
use crate::command::{self, Command, CommandError, split_words};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("splits")
}

/// A line of the whole command line, from `media generate` on.
fn parsed(line: &str) -> Result<Command, CommandError> {
    command::parse(&words(line))
}

#[test]
fn media_generate_parses_a_prompt_with_options() {
    let command = parsed(concat!(
        "media generate a candlelit hall, at dusk: calm as=hall purpose=plate aspect=16:9 ",
        "wait=true"
    ))
    .expect("parses");
    assert_eq!(
        command,
        Command::MediaGenerate(MediaGenerate::Generate(GenerateRequest {
            prompt: "a candlelit hall, at dusk: calm".to_owned(),
            name: "hall".to_owned(),
            purpose: Some(Purpose::Plate),
            aspect: Some(Aspect::WIDE),
            wait: true,
            base: None,
        }))
    );
    let quoted = parsed("media generate \"one word=prompt\" as=x").expect("parses");
    assert_eq!(
        quoted,
        Command::MediaGenerate(MediaGenerate::Generate(GenerateRequest {
            prompt: "one word=prompt".to_owned(),
            name: "x".to_owned(),
            purpose: None,
            aspect: None,
            wait: false,
            base: None,
        }))
    );
    assert_eq!(
        parsed("media generate status").expect("parses"),
        Command::MediaGenerate(MediaGenerate::Status)
    );
}

#[test]
fn media_generate_refuses_what_it_cannot_send() {
    for line in [
        "media generate a hall",
        "media generate as=hall",
        "media generate",
    ] {
        assert!(
            matches!(parsed(line), Err(CommandError::Usage(MEDIA_GENERATE_USAGE))),
            "{line}"
        );
    }
    assert!(matches!(
        parsed("media generate a as=b as=c"),
        Err(CommandError::DuplicateKey(key)) if key == "as"
    ));
    for (line, refused_key) in [
        ("media generate a as=hall.png", "as"),
        ("media generate a as=hall purpose=poster", "purpose"),
        ("media generate a as=hall aspect=wide", "aspect"),
        ("media generate a as=hall aspect=1:9", "aspect"),
        ("media generate a as=hall wait=maybe", "wait"),
    ] {
        assert!(
            matches!(parsed(line), Err(CommandError::BadChoice { key, .. }) if key == refused_key),
            "{line}"
        );
    }
}

#[test]
fn media_generate_refuses_a_prompt_the_backend_could_not_take() {
    let prompt = "x".repeat(MAXIMUM_SENT_TEXT_CHARS);
    let refused = parse(&[prompt, "as=hall".to_owned()]).expect_err("too long");
    assert!(
        matches!(&refused, CommandError::BadChoice { key, .. } if key == "prompt"),
        "{refused:?}"
    );
}

#[test]
fn media_generate_is_in_the_help() {
    let help = command::usage_text();
    assert!(help.contains("media generate <prompt...> as=NAME"));
    assert!(help.contains(MEDIA_GENERATE_USAGE));
}
