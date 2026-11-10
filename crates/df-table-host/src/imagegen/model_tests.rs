//! The purpose, the shape and the checks of a request.

use super::backend::BackendLimits;
use super::limits::{MAXIMUM_NAME_CHARS, MAXIMUM_SENT_TEXT_CHARS};
use super::model::{
    Aspect, AspectError, GenerateRequestError, ImageRequest, InferenceSteps, PictureSize,
    PictureSizeError, Purpose, ThingReference, maximum_prompt_chars,
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
        let request = ImageRequest::new(&longest, "a", Some(purpose), None)
            .and_then(|request| request.following(&["ref-a.png".to_owned()]))
            .expect("the longest prompt, with references");
        assert_eq!(
            request.sent_text(&BackendLimits::REQUEST).chars().count(),
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
fn references_are_library_pictures_and_add_their_line_before_the_purpose() {
    let plain = ImageRequest::new("the door", "door", Some(Purpose::Plate), None).expect("ok");
    let references = ["ref-warrior.png".to_owned(), "ref-sella.png".to_owned()];
    let following = plain.clone().following(&references).expect("follows");
    assert_eq!(following.references(), references);
    let text = following.sent_text(&BackendLimits::REQUEST);
    assert!(text.starts_with("the door\n\nThe reference pictures of people"));
    assert!(text.ends_with(Purpose::Plate.direction()));
    assert!(
        !plain
            .sent_text(&BackendLimits::REQUEST)
            .contains("reference")
    );
    assert_eq!(
        plain.clone().following(&["ref-warrior".to_owned()]),
        Err(GenerateRequestError::InvalidReference {
            reference: "ref-warrior".to_owned()
        })
    );
    let eight: Vec<String> = (1..=8).map(|index| format!("ref-{index}.png")).collect();
    assert!(plain.clone().following(&eight).is_ok());
    assert_eq!(
        plain
            .editing("door.png")
            .and_then(|request| request.following(&eight)),
        Err(GenerateRequestError::TooManyPictures {
            count: 9,
            maximum: 8
        })
    );
}

#[test]
fn the_backend_reads_the_prompt_first_then_the_purpose() {
    let request =
        ImageRequest::new("a candlelit hall", "hall", Some(Purpose::Plate), None).expect("ok");
    let text = request.sent_text(&BackendLimits::REQUEST);
    assert!(text.starts_with("a candlelit hall"));
    assert!(text.ends_with(Purpose::Plate.direction()));
}

#[test]
fn a_thing_s_picture_is_named_by_its_number_among_the_pictures_attached() {
    let wheel = ThingReference {
        picture: "wheel-close.png".to_owned(),
        shows: "Brass cipher wheel".to_owned(),
    };
    let request = ImageRequest::new("the scriptorium", "room", Some(Purpose::Plate), None)
        .and_then(|request| request.editing("room-1.png"))
        .and_then(|request| request.following(&["ref-jory.png".to_owned()]))
        .and_then(|request| request.showing(std::slice::from_ref(&wheel)))
        .expect("a valid request");
    assert_eq!(request.things(), [wheel]);
    let text = request.sent_text(&BackendLimits::REQUEST);
    // The picture to edit is the first attached and the person's sheet the second.
    assert!(
        text.contains("\n\nReference 3: Brass cipher wheel as it looks.\n\n"),
        "{text}"
    );
    assert_eq!(
        request.check_pictures(2),
        Err(GenerateRequestError::TooManyPictures {
            count: 3,
            maximum: 2
        })
    );
    let unnamed = ThingReference {
        picture: "wheel".to_owned(),
        shows: "a wheel".to_owned(),
    };
    assert!(matches!(
        request.showing(&[unnamed]),
        Err(GenerateRequestError::InvalidReference { .. })
    ));
}

#[test]
fn what_a_picture_must_not_show_is_text_or_a_negative_prompt_as_the_backend_takes_it() {
    let avoid = ["painting".to_owned(), "readable text".to_owned()];
    let request = ImageRequest::new("a candlelit hall", "hall", Some(Purpose::Plate), None)
        .expect("ok")
        .avoiding(&avoid);
    // A backend with no negative prompt (Codex) reads them in the text.
    let in_text = request.sent_text(&BackendLimits::REQUEST);
    assert!(
        in_text.contains("\n\nNever show: painting, readable text.\n\n"),
        "{in_text}"
    );
    assert_eq!(request.negative_text(&BackendLimits::REQUEST), None);
    // A backend with one (Forge) takes them there, and the text leaves them out.
    let negative = BackendLimits {
        maximum_text_characters: Some(4000),
        maximum_negative_characters: Some(4000),
        maximum_pictures: 8,
    };
    assert!(!request.sent_text(&negative).contains("painting"));
    assert_eq!(
        request.negative_text(&negative).as_deref(),
        Some("painting, readable text")
    );
    let nothing = ImageRequest::new("a hall", "hall", None, None).expect("ok");
    assert_eq!(nothing.negative_text(&negative), None);
}

#[test]
fn a_size_is_two_sides_in_pixels_and_steps_are_within_forge_s_bound() {
    let size: PictureSize = "512x512".parse().expect("a size");
    assert_eq!((size.width_px(), size.height_px()), (512, 512));
    assert_eq!(size.to_string(), "512x512");
    for (text, cause) in [
        ("512", PictureSizeError::NotWidthByHeight),
        ("512X512", PictureSizeError::NotWidthByHeight),
        ("512x", PictureSizeError::NotWidthByHeight),
        ("axb", PictureSizeError::NotWidthByHeight),
        ("-5x5", PictureSizeError::NotWidthByHeight),
        ("0x512", PictureSizeError::SideOutOfRange),
        ("512x0", PictureSizeError::SideOutOfRange),
        ("99999x512", PictureSizeError::SideOutOfRange),
    ] {
        assert_eq!(text.parse::<PictureSize>(), Err(cause), "{text}");
    }
    assert_eq!(
        serde_json::from_str::<PictureSize>("\"512x512\"").expect("reads"),
        size
    );
    assert!(serde_json::from_str::<PictureSize>("\"0x1\"").is_err());
    assert_eq!("20".parse::<InferenceSteps>().expect("steps").get(), 20);
    assert!(InferenceSteps::new(1).is_ok() && InferenceSteps::new(100).is_ok());
    for text in ["0", "101", "-1", "2.5", "many", ""] {
        assert!(text.parse::<InferenceSteps>().is_err(), "{text}");
    }
    assert_eq!(
        serde_json::from_str::<InferenceSteps>("20")
            .expect("reads")
            .get(),
        20
    );
    assert!(serde_json::from_str::<InferenceSteps>("101").is_err());
}

#[test]
fn a_size_has_the_shape_of_its_sides_in_lowest_terms() {
    let of = |width, height| Aspect::of_size(PictureSize::new(width, height).unwrap());
    assert_eq!(of(512, 512), Some(shape("1:1")));
    assert_eq!(of(1280, 720), Some(shape("16:9")));
    // Sides that do not reduce to a shape of parts up to 64 are scaled down together.
    assert_eq!(of(1672, 941), Some(shape("64:36")));
    assert_eq!(of(4000, 400), None, "more elongated than a shape may be");
}
