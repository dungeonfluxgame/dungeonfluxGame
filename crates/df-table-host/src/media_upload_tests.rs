//! Reading an add request as it arrives.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::media_ingest::{IngestRefusal, MediaLimits};
use crate::media_upload::{Envelope, EnvelopeParser, EnvelopeProblem};
use crate::wire::MediaAdd;

/// Feeds `request` in pieces of `piece` bytes and returns the decoded file and the envelope.
fn read_in_pieces(
    request: &[u8],
    piece: usize,
    limits: &MediaLimits,
) -> Result<(Vec<u8>, Envelope), IngestRefusal> {
    let mut parser = EnvelopeParser::new(limits);
    let mut file = Vec::new();
    for part in request.chunks(piece) {
        parser.feed(part, &mut file)?;
    }
    Ok((file, parser.finish()?))
}

fn request_for(name: &str, bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&MediaAdd {
        name: name.to_owned(),
        data_base64: STANDARD.encode(bytes),
    })
    .expect("serialises")
}

fn malformed(problem: EnvelopeProblem) -> IngestRefusal {
    IngestRefusal::MalformedRequest(problem)
}

#[test]
fn the_request_the_dev_client_writes_is_read_whatever_the_piece_size() {
    let file: Vec<u8> = (0..=255).cycle().take(1000).collect();
    let request = request_for("hearth.png", &file);
    for piece in [1, 2, 3, 4, 5, 7, 64, 999, request.len()] {
        let (decoded, envelope) =
            read_in_pieces(&request, piece, &MediaLimits::DEFAULT).expect("reads");
        assert_eq!(decoded, file, "piece {piece}");
        assert_eq!(envelope.name, "hearth.png", "piece {piece}");
        assert_eq!(envelope.file_bytes, 1000, "piece {piece}");
    }
}

#[test]
fn every_padding_length_decodes() {
    for length in 0..=7 {
        let file = vec![0xAB_u8; length];
        let (decoded, _) =
            read_in_pieces(&request_for("a.png", &file), 3, &MediaLimits::DEFAULT).expect("reads");
        assert_eq!(decoded, file, "{length}");
    }
}

#[test]
fn the_keys_may_come_in_either_order_with_whitespace_and_an_escaped_name() {
    let request = b" {\n\t\"data_base64\" : \"aGk=\" ,\r\n \"name\":\"\\u0061.png\" }  \n";
    let (decoded, envelope) = read_in_pieces(request, 5, &MediaLimits::DEFAULT).expect("reads");
    assert_eq!(decoded, b"hi");
    assert_eq!(envelope.name, "a.png");
}

#[test]
fn a_request_that_is_not_the_expected_object_is_refused_with_the_reason() {
    for (request, problem) in [
        (&br#"[1]"#[..], EnvelopeProblem::UnexpectedCharacter),
        (&br#"{"name":5}"#[..], EnvelopeProblem::UnexpectedCharacter),
        (
            &br#"{"name":"a.png","data_base64":"aGk=","extra":1}"#[..],
            EnvelopeProblem::UnknownField,
        ),
        (
            &br#"{"name":"a.png","name":"b.png","data_base64":"aGk="}"#[..],
            EnvelopeProblem::DuplicateField,
        ),
        (
            &br#"{"name":"a.png","data_base64":"aGk=","data_base64":"aGk="}"#[..],
            EnvelopeProblem::DuplicateField,
        ),
        (&br#"{"name":"a.png"}"#[..], EnvelopeProblem::MissingField),
        (&br#"{}"#[..], EnvelopeProblem::MissingField),
        (
            &br#"{"name":"a.png","data_base64":"aGk=""#[..],
            EnvelopeProblem::EndedEarly,
        ),
        (&br#""#[..], EnvelopeProblem::EndedEarly),
        (
            &br#"{"name":"a.png","data_base64":"aGk="} x"#[..],
            EnvelopeProblem::UnexpectedCharacter,
        ),
        (
            &br#"{"name":"a.png","data_base64":"aGk=",}"#[..],
            EnvelopeProblem::UnexpectedCharacter,
        ),
    ] {
        let refusal = read_in_pieces(request, 3, &MediaLimits::DEFAULT).expect_err("refused");
        assert_eq!(
            refusal,
            malformed(problem),
            "{}",
            String::from_utf8_lossy(request)
        );
    }
}

#[test]
fn a_name_that_is_not_a_media_name_is_refused_as_soon_as_it_is_read_before_any_data() {
    let mut parser = EnvelopeParser::new(&MediaLimits::DEFAULT);
    let mut file = Vec::new();
    let refusal = parser
        .feed(br#"{"name":"logo.svg","data_base64":"aGk"#, &mut file)
        .expect_err("refused");
    assert_eq!(
        refusal,
        IngestRefusal::NotAMediaName {
            name: "logo.svg".to_owned()
        }
    );
    assert!(file.is_empty());
}

#[test]
fn a_name_far_longer_than_a_name_can_be_is_refused_without_collecting_it() {
    let long = "a".repeat(600);
    let request = format!(r#"{{"name":"{long}.png","data_base64":""}}"#);
    assert_eq!(
        read_in_pieces(request.as_bytes(), 50, &MediaLimits::DEFAULT).expect_err("refused"),
        malformed(EnvelopeProblem::NameTooLong)
    );
}

#[test]
fn data_that_is_not_base64_is_refused_wherever_the_fault_is() {
    for data in [
        "not base64 !!",
        "aGk",
        "aGk=aGk=",
        "aG=k",
        "aGk=x",
        "a\\/b=",
        "====",
    ] {
        let request = format!(r#"{{"name":"a.png","data_base64":"{data}"}}"#);
        assert_eq!(
            read_in_pieces(request.as_bytes(), 4, &MediaLimits::DEFAULT).expect_err(data),
            IngestRefusal::NotBase64,
            "{data}"
        );
    }
}

#[test]
fn data_longer_than_the_limit_is_refused_when_the_limit_is_crossed_and_nothing_past_it_is_written()
{
    let limits = MediaLimits {
        maximum_file_bytes: 30,
        ..MediaLimits::DEFAULT
    };
    let mut parser = EnvelopeParser::new(&limits);
    let mut file = Vec::new();
    let data = STANDARD.encode([7_u8; 90]);
    let request = format!(r#"{{"name":"a.png","data_base64":"{data}"}}"#);
    let mut fed = 0;
    let refusal = loop {
        let end = (fed + 8).min(request.len());
        match parser.feed(&request.as_bytes()[fed..end], &mut file) {
            Ok(()) => fed = end,
            Err(refusal) => break refusal,
        }
    };
    assert_eq!(refusal, IngestRefusal::TooLarge { maximum_bytes: 30 });
    assert!(file.len() <= 30, "{} bytes written", file.len());
    assert!(fed + 40 < request.len(), "read {fed} of {}", request.len());
}

#[test]
fn a_file_of_exactly_the_limit_is_accepted_and_one_byte_more_is_not() {
    let limits = MediaLimits {
        maximum_file_bytes: 30,
        ..MediaLimits::DEFAULT
    };
    assert!(read_in_pieces(&request_for("a.png", &[1; 30]), 9, &limits).is_ok());
    assert_eq!(
        read_in_pieces(&request_for("a.png", &[1; 31]), 9, &limits).expect_err("refused"),
        IngestRefusal::TooLarge { maximum_bytes: 30 }
    );
}

#[test]
fn a_request_longer_than_the_largest_add_is_refused_whatever_it_holds() {
    let limits = MediaLimits {
        maximum_file_bytes: 3,
        ..MediaLimits::DEFAULT
    };
    let filler = " ".repeat(usize::try_from(limits.maximum_upload_body_bytes()).expect("fits"));
    let request = format!(r#"{{"name":"a.png","data_base64":"AAAA"}}{filler}"#);
    assert!(matches!(
        read_in_pieces(request.as_bytes(), 1000, &limits),
        Err(IngestRefusal::TooLarge { .. })
    ));
}

#[test]
fn the_largest_add_is_the_largest_file_as_base64_plus_the_envelope() {
    let limits = MediaLimits::DEFAULT;
    assert_eq!(
        limits.maximum_upload_body_bytes(),
        limits.maximum_file_bytes.div_ceil(3) * 4 + 4096
    );
}
