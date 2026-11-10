//! The grammar of `media generate`: a prompt in words, then `key=value` options.

use super::cli::{MEDIA_BACKEND_USAGE, MEDIA_GENERATE_USAGE, MediaGenerate, parse};
use super::limits::MAXIMUM_SENT_TEXT_CHARS;
use super::model::{Aspect, InferenceSteps, Priority, Purpose};
use super::request::{BackendSwitch, GenerateRequest};
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
        "wait=true look=none"
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
            references: Vec::new(),
            things: Vec::new(),
            avoid: Vec::new(),
            left_out: Vec::new(),
            priority: Priority::Normal,
            sheet: None,
        }))
    );
    let quoted = parsed("media generate \"one word=prompt\" as=x look=none").expect("parses");
    assert_eq!(
        quoted,
        Command::MediaGenerate(MediaGenerate::Generate(GenerateRequest {
            prompt: "one word=prompt".to_owned(),
            name: "x".to_owned(),
            purpose: None,
            aspect: None,
            wait: false,
            base: None,
            references: Vec::new(),
            things: Vec::new(),
            avoid: Vec::new(),
            left_out: Vec::new(),
            priority: Priority::Normal,
            sheet: None,
        }))
    );
    let urgent =
        parsed("media generate one door as=x priority=emergency look=none").expect("parses");
    assert!(matches!(
        urgent,
        Command::MediaGenerate(MediaGenerate::Generate(GenerateRequest {
            priority: Priority::Emergency,
            ..
        }))
    ));
    assert!(parsed("media generate one door as=x priority=urgent").is_err());
    assert_eq!(
        parsed("media generate status").expect("parses"),
        Command::MediaGenerate(MediaGenerate::Status)
    );
}

#[test]
fn media_generate_reads_references_in_order() {
    let Ok(Command::MediaGenerate(MediaGenerate::Generate(request))) = parsed(concat!(
        "media generate the two at the door as=door look=none ",
        "refs=ref-warrior.png,ref-mara_adventurer.png"
    )) else {
        panic!("parses");
    };
    assert_eq!(
        request.references,
        ["ref-warrior.png", "ref-mara_adventurer.png"]
    );
    assert_eq!(request.prompt, "the two at the door");
    for line in [
        "media generate a as=hall refs=ref-warrior",
        "media generate a as=hall refs=ref-warrior.png,",
        "media generate a as=hall refs=../ref.png",
        "media generate a as=hall base=b.png refs=1.png,2.png,3.png,4.png,5.png,6.png,7.png,8.png",
    ] {
        assert!(
            matches!(parsed(line), Err(CommandError::BadChoice { key, .. }) if key == "refs"),
            "{line}"
        );
    }
}

#[test]
fn media_generate_sends_the_prompt_inside_the_house_look_unless_told_not_to() {
    let style = crate::movement::brief::BriefStyle::default();
    let Ok(Command::MediaGenerate(MediaGenerate::Generate(request))) =
        parsed("media generate a plain town hall as=hall")
    else {
        panic!("parses");
    };
    assert!(request.prompt.starts_with(&style.direction));
    assert!(request.prompt.contains("a plain town hall"));
    assert!(request.prompt.ends_with(&style.writing));
    assert_eq!(request.avoid, style.avoid);
    assert!(matches!(
        parsed("media generate a hall as=hall look=painted"),
        Err(CommandError::BadChoice { .. })
    ));
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

#[test]
fn media_backend_shows_or_switches_the_image_backend() {
    let switch = |line: &str| match parsed(line) {
        Ok(Command::MediaGenerate(MediaGenerate::SwitchBackend(switch))) => switch,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(
        parsed("media backend").expect("parses"),
        Command::MediaGenerate(MediaGenerate::Backend)
    );
    assert_eq!(
        switch("media backend codex-cli"),
        BackendSwitch::CodexCli {}
    );
    assert_eq!(
        switch("media backend forge"),
        BackendSwitch::Forge {
            url: None,
            project: None,
            size: None,
            steps: None
        }
    );
    assert_eq!(
        switch("media backend forge url=http://127.0.0.1:5174 project=abc"),
        BackendSwitch::Forge {
            url: Some("http://127.0.0.1:5174".to_owned()),
            project: Some("abc".to_owned()),
            size: None,
            steps: None
        }
    );
    // The size and steps are set with the switch, in either order.
    let tuned = BackendSwitch::Forge {
        url: None,
        project: None,
        size: Some("512x512".parse().expect("a size")),
        steps: Some(InferenceSteps::new(20).expect("steps")),
    };
    assert_eq!(switch("media backend forge size=512x512 steps=20"), tuned);
    assert_eq!(switch("media backend forge steps=20 size=512x512"), tuned);
    for line in [
        "media backend forge size=512",
        "media backend forge size=0x512",
        "media backend forge size=512x512x3",
        "media backend forge steps=0",
        "media backend forge steps=101",
        "media backend forge steps=many",
    ] {
        assert!(
            matches!(
                parsed(line),
                Err(CommandError::BadChoice { ref key, .. }) if key == "size" || key == "steps"
            ),
            "{line}: {:?}",
            parsed(line)
        );
    }
    assert!(matches!(
        parsed("media backend forge size=512x512 size=64x64"),
        Err(CommandError::DuplicateKey(key)) if key == "size"
    ));
    // Codex takes neither.
    assert!(matches!(
        parsed("media backend codex-cli size=512x512"),
        Err(CommandError::Usage(MEDIA_BACKEND_USAGE))
    ));
    assert!(matches!(
        parsed("media backend dall-e"),
        Err(CommandError::BadChoice { key, .. }) if key == "backend"
    ));
    for line in [
        "media backend codex-cli url=http://127.0.0.1:5173",
        "media backend forge seed=3",
    ] {
        assert!(
            matches!(parsed(line), Err(CommandError::Usage(MEDIA_BACKEND_USAGE))),
            "{line}"
        );
    }
    assert!(matches!(
        parsed("media backend forge project=a project=b"),
        Err(CommandError::DuplicateKey(key)) if key == "project"
    ));
    assert!(command::usage_text().contains(MEDIA_BACKEND_USAGE));
    for switch in [
        BackendSwitch::CodexCli {},
        BackendSwitch::Forge {
            url: None,
            project: None,
            size: None,
            steps: None,
        },
        tuned,
    ] {
        assert_eq!(
            serde_json::to_value(&switch).expect("writes")["backend"],
            switch.key()
        );
    }
}

#[test]
fn media_backend_refs_sets_the_soft_cap_on_the_pictures_of_a_plate_request() {
    use super::request::ReferenceCap;
    let command = parsed("media backend refs=3").expect("parses");
    assert_eq!(
        command,
        Command::MediaGenerate(MediaGenerate::ReferenceCap(ReferenceCap {
            maximum: 3.try_into().unwrap()
        }))
    );
    for odd in [
        "media backend refs=0",
        "media backend refs=many",
        "media backend refs=3 forge",
    ] {
        assert!(parsed(odd).is_err(), "{odd}");
    }
}
