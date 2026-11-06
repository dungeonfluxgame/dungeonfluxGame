//! The purpose, the shape and the checks of a request.

use super::limits::{MAXIMUM_NAME_CHARS, MAXIMUM_SENT_TEXT_CHARS};
use super::model::{
    Aspect, AspectError, GenerateRequestError, ImageRequest, Purpose, maximum_prompt_chars,
};

fn shape(text: &str) -> Aspect {
    text.parse().expect("a shape")
}

#[test]
fn a_wide_aspect_asks_for_the_size_codex_gives_a_wide_picture() {
    assert_eq!(Aspect::WIDE.requested_size(), (1672, 941));
    assert_eq!(shape("3:4").requested_size(), (1254, 1672));
    assert_eq!(shape("1:1").requested_size(), (1672, 1672));
}

#[test]
fn a_shape_is_two_whole_numbers_within_four_to_one() {
    for text in ["16:9", "1:4", "4:1", "64:64", "3:4"] {
        assert_eq!(shape(text).to_string(), text);
    }
    for (text, error) in [
        ("", AspectError::NotWidthByHeight),
        ("16", AspectError::NotWidthByHeight),
        ("16:", AspectError::NotWidthByHeight),
        (":9", AspectError::NotWidthByHeight),
        ("a:b", AspectError::NotWidthByHeight),
        ("16:9:1", AspectError::NotWidthByHeight),
        ("-1:2", AspectError::NotWidthByHeight),
        ("0:9", AspectError::PartOutOfRange),
        ("16:0", AspectError::PartOutOfRange),
        ("65:64", AspectError::PartOutOfRange),
        ("1:5", AspectError::TooElongated),
        ("5:1", AspectError::TooElongated),
    ] {
        assert_eq!(text.parse::<Aspect>(), Err(error), "{text}");
    }
}

#[test]
fn a_shape_is_text_on_the_wire_and_is_checked_when_read() {
    assert_eq!(
        serde_json::to_string(&Aspect::WIDE).expect("writes"),
        "\"16:9\""
    );
    assert_eq!(
        serde_json::from_str::<Aspect>("\"3:4\"").expect("reads"),
        Aspect::PORTRAIT
    );
    assert!(serde_json::from_str::<Aspect>("\"1:9\"").is_err());
    assert!(serde_json::from_str::<Aspect>(r#"{"width":1,"height":9}"#).is_err());
}

#[test]
fn a_purpose_key_is_its_serde_name() {
    for purpose in Purpose::ALL {
        let json = serde_json::to_value(purpose).expect("serializes");
        assert_eq!(json, serde_json::Value::from(purpose.key()));
        assert_eq!(Purpose::from_key(purpose.key()), Some(purpose));
    }
    assert_eq!(Purpose::from_key("poster"), None);
}

#[test]
fn a_request_without_purpose_or_shape_is_a_wide_background() {
    let request = ImageRequest::new("a hall", "hall-1", None, None).expect("a request");
    assert_eq!(request.purpose(), Purpose::Background);
    assert_eq!(request.aspect(), Aspect::WIDE);
    let portrait =
        ImageRequest::new("a face", "face", Some(Purpose::Portrait), None).expect("a request");
    assert_eq!(portrait.aspect(), Aspect::PORTRAIT);
}

#[test]
fn a_request_checks_its_prompt_and_name() {
    let refused = |prompt: &str, name: &str| {
        ImageRequest::new(prompt, name, None, None).expect_err("refused")
    };
    assert_eq!(refused("  ", "a"), GenerateRequestError::EmptyPrompt);
    for name in [
        "",
        "a b",
        "a.png",
        "../a",
        "a/b",
        "é",
        &"n".repeat(MAXIMUM_NAME_CHARS + 1),
    ] {
        assert_eq!(
            refused("p", name),
            GenerateRequestError::InvalidName {
                name: name.to_owned()
            },
            "{name}"
        );
    }
    ImageRequest::new("p", &"n".repeat(MAXIMUM_NAME_CHARS), None, None)
        .expect("the longest name is a name");
}

#[test]
fn a_prompt_too_long_for_the_backend_with_its_direction_is_refused_before_any_job() {
    // Before, a prompt of exactly the backend's maximum passed the host's check, started a job and
    // then failed in the backend, because the purpose's direction is added to it.
    let at_backend_limit = "x".repeat(MAXIMUM_SENT_TEXT_CHARS);
    assert_eq!(
        ImageRequest::new(&at_backend_limit, "a", None, None),
        Err(GenerateRequestError::PromptTooLong {
            chars: MAXIMUM_SENT_TEXT_CHARS,
            maximum_chars: maximum_prompt_chars(Purpose::Background),
        })
    );
    for purpose in Purpose::ALL {
        let longest = "x".repeat(maximum_prompt_chars(purpose));
        let request =
            ImageRequest::new(&longest, "a", Some(purpose), None).expect("the longest prompt");
        assert_eq!(
            request.sent_text().chars().count(),
            MAXIMUM_SENT_TEXT_CHARS,
            "{purpose:?}"
        );
        let over = "x".repeat(maximum_prompt_chars(purpose) + 1);
        assert!(matches!(
            ImageRequest::new(&over, "a", Some(purpose), None),
            Err(GenerateRequestError::PromptTooLong { .. })
        ));
    }
}

#[test]
fn the_backend_reads_the_prompt_first_then_the_purpose() {
    let request =
        ImageRequest::new("a candlelit hall", "hall", Some(Purpose::Plate), None).expect("ok");
    let text = request.sent_text();
    assert!(text.starts_with("a candlelit hall"));
    assert!(text.ends_with(Purpose::Plate.direction()));
}
