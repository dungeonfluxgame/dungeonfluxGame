//! A closed-choice vision judgement over a few labelled pictures (PLAY-V09, plate segments).
//!
//! The server shows a vision model a handful of labelled pictures (reference portraits and
//! candidate crops of a scene plate) with a fixed instruction and a JSON schema whose answers are
//! closed labels (an `enum` of candidate numbers, an `enum` of facings), so the model can only
//! pick from what the server offered. One bounded `POST /v1/chat/completions` with strict
//! structured output; the adapter never retries; the pictures, the instruction and the answer are
//! never logged or placed in errors. Who a person is and which way they face are never decided
//! by name, word or position matching: this call is the judge.

use std::{fmt, net::SocketAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::native_http::NativeHttp;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const LIVE_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-5.5";
const MAXIMUM_PICTURES: usize = 16;
const MAXIMUM_PICTURE_BYTES: usize = 1024 * 1024;
/// All pictures of one call together; their base64 stays inside the transport's 16 MiB bound.
const MAXIMUM_TOTAL_PICTURE_BYTES: usize = 8 * 1024 * 1024;
const MAXIMUM_INSTRUCTION_CHARACTERS: usize = 6000;
const MAXIMUM_RESPONSE_BYTES: usize = 256 * 1024;
const PHOTO_QUALITY_INSTRUCTION: &str = "Judge this one generated image for photographic quality. Answer only using the supplied labels. frame is single_photograph when it is one coherent camera photograph, grid_or_collage when it is divided into panels or a contact sheet, and cutout_composite when people or objects are pasted/composited together. medium is photograph only for a convincing photographic capture, painting_or_illustration for drawn or painterly art, and cgi_or_render for synthetic rendered imagery. text is visible_text if any readable or invented lettering/signage appears, otherwise none. dress is modern_clothing if any person wears recognizably modern clothes, period if all visible people wear period-appropriate clothes, and no_people if there are no people. For a canonical character only, people_count is exactly_one only when precisely one human person is present; animals do not count as people. Do not infer image intent; judge only visible content.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoJudgeKind {
    ScenePlate,
    CanonicalRoom,
    CanonicalCharacter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoFrame {
    SinglePhotograph,
    GridOrCollage,
    CutoutComposite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoMedium {
    Photograph,
    PaintingOrIllustration,
    CgiOrRender,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoText {
    None,
    VisibleText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoDress {
    Period,
    ModernClothing,
    NoPeople,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoPeopleCount {
    ExactlyOne,
    NotOne,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhotoQualityAnswer {
    pub frame: PhotoFrame,
    pub medium: PhotoMedium,
    pub text: PhotoText,
    pub dress: PhotoDress,
    pub people_count: Option<PhotoPeopleCount>,
}

/// One picture of a judgement: a short plain label the instruction refers to, and a JPEG.
#[derive(Clone, Copy, Debug)]
pub struct JudgePicture<'a> {
    /// 1 to 24 characters of lowercase letters, digits and `-` (`ref-1`, `cand-3`).
    pub label: &'a str,
    pub jpeg: &'a [u8],
}

/// A judgement: the answer, and what the call used, for the caller's spend ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgeAnswer {
    /// JSON that satisfies the schema the call was given.
    pub value: Value,
    /// The provider's own count of input tokens, when it reported one.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Why no judgement came back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlateJudgeError {
    /// Failed local validation; nothing was sent.
    InvalidRequest,
    Unauthorized,
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// The reply was not one JSON value (the schema makes this rare).
    Output,
}

impl PlateJudgeError {
    /// A short stable label for audit lines.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::Provider(_) => "provider_status",
            Self::Transport(NativeHttpRefusal::Deadline) => "deadline",
            Self::Transport(_) => "transport",
            Self::Output => "bad_output",
        }
    }
}

/// Where the adapter sends calls.
pub struct PlateJudgeConfig {
    fixture: Option<SocketAddr>,
}

impl PlateJudgeConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, PlateJudgeError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(PlateJudgeError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

pub struct NativePlateJudge {
    endpoint: String,
    credential: String,
    http: NativeHttp,
}

impl fmt::Debug for NativePlateJudge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativePlateJudge")
            .finish_non_exhaustive()
    }
}

impl NativePlateJudge {
    /// `total_timeout` bounds each call, including the response body.
    pub fn new(
        config: PlateJudgeConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, PlateJudgeError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(PlateJudgeError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: 16 * 1024 * 1024,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 4096,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| PlateJudgeError::InvalidRequest)?;
        Ok(Self {
            endpoint: config.fixture.map_or_else(
                || LIVE_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/chat/completions"),
            ),
            credential: credential.to_owned(),
            http,
        })
    }

    /// Asks `instruction` about `pictures` and returns the model's answer as JSON that satisfies
    /// `schema` (named `schema_name`, strict). One call, never retried.
    pub async fn judge(
        &self,
        instruction: &str,
        pictures: &[JudgePicture<'_>],
        schema_name: &str,
        schema: &Value,
        deadline: Duration,
    ) -> Result<JudgeAnswer, PlateJudgeError> {
        let body = request_body(instruction, pictures, schema_name, schema)?;
        let request = HttpRequest::bearer_native(
            &self.endpoint,
            &self.credential,
            "application/json".to_owned(),
            body,
        );
        let response = self
            .http
            .execute(request, deadline, std::future::pending())
            .await
            .map_err(|failure| PlateJudgeError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(PlateJudgeError::Unauthorized),
            status => return Err(PlateJudgeError::Provider(status)),
        }
        let value = parse_unique_json(
            &response.body,
            TextSchemaLimits {
                maximum_schema_bytes: MAXIMUM_RESPONSE_BYTES,
                maximum_depth: 16,
                maximum_nodes: 4096,
            },
        )
        .map_err(|_| PlateJudgeError::Output)?;
        let content = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or(PlateJudgeError::Output)?;
        let tokens = |name: &str| {
            value
                .get("usage")
                .and_then(|usage| usage.get(name))
                .and_then(Value::as_u64)
        };
        Ok(JudgeAnswer {
            value: serde_json::from_str(content).map_err(|_| PlateJudgeError::Output)?,
            input_tokens: tokens("prompt_tokens"),
            output_tokens: tokens("completion_tokens"),
        })
    }

    /// Judges one generated image with closed labels. Character calls also ask for a people
    /// count. This is one bounded vision request; provider errors are returned without logging.
    pub async fn judge_photo_quality(
        &self,
        image_jpeg: &[u8],
        kind: PhotoJudgeKind,
        deadline: Duration,
    ) -> Result<PhotoQualityAnswer, PlateJudgeError> {
        let character = kind == PhotoJudgeKind::CanonicalCharacter;
        let mut properties = json!({
            "frame": {"type": "string", "enum": ["single_photograph", "grid_or_collage", "cutout_composite"]},
            "medium": {"type": "string", "enum": ["photograph", "painting_or_illustration", "cgi_or_render"]},
            "text": {"type": "string", "enum": ["none", "visible_text"]},
            "dress": {"type": "string", "enum": ["period", "modern_clothing", "no_people"]},
        });
        let mut required = vec!["frame", "medium", "text", "dress"];
        if character {
            properties["people_count"] = json!({
                "type": "string",
                "enum": ["exactly_one", "not_one"],
            });
            required.push("people_count");
        }
        let schema = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        let picture = [JudgePicture {
            label: "candidate-1",
            jpeg: image_jpeg,
        }];
        let answer = self
            .judge(
                PHOTO_QUALITY_INSTRUCTION,
                &picture,
                "photo-quality",
                &schema,
                deadline,
            )
            .await?;
        parse_photo_answer(answer.value, character)
    }
}

fn parse_photo_answer(
    value: Value,
    character: bool,
) -> Result<PhotoQualityAnswer, PlateJudgeError> {
    let object = value.as_object().ok_or(PlateJudgeError::Output)?;
    let expected_fields = if character { 5 } else { 4 };
    if object.len() != expected_fields
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "frame" | "medium" | "text" | "dress" | "people_count"
            ) || (key == "people_count" && !character)
        })
    {
        return Err(PlateJudgeError::Output);
    }
    let label = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .ok_or(PlateJudgeError::Output)
    };
    let frame = match label("frame")? {
        "single_photograph" => PhotoFrame::SinglePhotograph,
        "grid_or_collage" => PhotoFrame::GridOrCollage,
        "cutout_composite" => PhotoFrame::CutoutComposite,
        _ => return Err(PlateJudgeError::Output),
    };
    let medium = match label("medium")? {
        "photograph" => PhotoMedium::Photograph,
        "painting_or_illustration" => PhotoMedium::PaintingOrIllustration,
        "cgi_or_render" => PhotoMedium::CgiOrRender,
        _ => return Err(PlateJudgeError::Output),
    };
    let text = match label("text")? {
        "none" => PhotoText::None,
        "visible_text" => PhotoText::VisibleText,
        _ => return Err(PlateJudgeError::Output),
    };
    let dress = match label("dress")? {
        "period" => PhotoDress::Period,
        "modern_clothing" => PhotoDress::ModernClothing,
        "no_people" => PhotoDress::NoPeople,
        _ => return Err(PlateJudgeError::Output),
    };
    let people_count = if character {
        Some(match label("people_count")? {
            "exactly_one" => PhotoPeopleCount::ExactlyOne,
            "not_one" => PhotoPeopleCount::NotOne,
            _ => return Err(PlateJudgeError::Output),
        })
    } else {
        None
    };
    Ok(PhotoQualityAnswer {
        frame,
        medium,
        text,
        dress,
        people_count,
    })
}

fn plain_label(label: &str) -> bool {
    (1..=24).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn request_body(
    instruction: &str,
    pictures: &[JudgePicture<'_>],
    schema_name: &str,
    schema: &Value,
) -> Result<Vec<u8>, PlateJudgeError> {
    if instruction.trim().is_empty()
        || instruction.chars().count() > MAXIMUM_INSTRUCTION_CHARACTERS
        || pictures.is_empty()
        || pictures.len() > MAXIMUM_PICTURES
        || pictures.iter().any(|picture| {
            !plain_label(picture.label)
                || picture.jpeg.is_empty()
                || picture.jpeg.len() > MAXIMUM_PICTURE_BYTES
        })
        || pictures
            .iter()
            .map(|picture| picture.jpeg.len())
            .sum::<usize>()
            > MAXIMUM_TOTAL_PICTURE_BYTES
        || !plain_label(schema_name)
        || !schema.is_object()
    {
        return Err(PlateJudgeError::InvalidRequest);
    }
    let mut content = vec![json!({"type": "text", "text": instruction})];
    for picture in pictures {
        content.push(json!({"type": "text", "text": format!("Picture {}:", picture.label)}));
        content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:image/jpeg;base64,{}", STANDARD.encode(picture.jpeg)),
                "detail": "low",
            },
        }));
    }
    serde_json::to_vec(&json!({
        "model": MODEL,
        "max_completion_tokens": 3000,
        "messages": [{"role": "user", "content": content}],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": schema_name, "strict": true, "schema": schema},
        },
    }))
    .map_err(|_| PlateJudgeError::InvalidRequest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {"facing": {"type": "string", "enum": ["left", "right"]}},
            "required": ["facing"],
            "additionalProperties": false,
        })
    }

    #[test]
    fn the_request_labels_every_picture_and_carries_the_closed_schema() {
        let pictures = [
            JudgePicture {
                label: "ref-1",
                jpeg: b"\xff\xd8a",
            },
            JudgePicture {
                label: "cand-1",
                jpeg: b"\xff\xd8b",
            },
        ];
        let body = request_body("Pick one.", &pictures, "choice", &schema()).unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["model"], MODEL);
        let content = value["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 5);
        assert_eq!(content[1]["text"], "Picture ref-1:");
        assert_eq!(content[3]["text"], "Picture cand-1:");
        assert!(
            content[4]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        let format = &value["response_format"]["json_schema"];
        assert_eq!(format["strict"], true);
        assert_eq!(format["schema"]["properties"]["facing"]["enum"][1], "right");
    }

    #[test]
    fn bad_input_is_refused_before_anything_is_sent() {
        let one = [JudgePicture {
            label: "cand-1",
            jpeg: b"jpeg",
        }];
        let bad_label = [JudgePicture {
            label: "Bad Label",
            jpeg: b"jpeg",
        }];
        let empty_picture = [JudgePicture {
            label: "cand-1",
            jpeg: b"",
        }];
        for (instruction, pictures, name, schema) in [
            ("", &one[..], "choice", schema()),
            ("x", &[][..], "choice", schema()),
            ("x", &bad_label[..], "choice", schema()),
            ("x", &empty_picture[..], "choice", schema()),
            ("x", &one[..], "Bad Name", schema()),
            ("x", &one[..], "choice", json!("not an object")),
        ] {
            assert_eq!(
                request_body(instruction, pictures, name, &schema),
                Err(PlateJudgeError::InvalidRequest)
            );
        }
        let long = "x".repeat(MAXIMUM_INSTRUCTION_CHARACTERS + 1);
        assert_eq!(
            request_body(&long, &one, "choice", &schema()),
            Err(PlateJudgeError::InvalidRequest)
        );
        let big = vec![0u8; MAXIMUM_PICTURE_BYTES];
        let heavy: Vec<JudgePicture<'_>> = (0..9)
            .map(|_| JudgePicture {
                label: "cand-1",
                jpeg: &big,
            })
            .collect();
        assert_eq!(
            request_body("x", &heavy, "choice", &schema()),
            Err(PlateJudgeError::InvalidRequest)
        );
        let many: Vec<JudgePicture<'_>> = (0..=MAXIMUM_PICTURES)
            .map(|_| JudgePicture {
                label: "cand-1",
                jpeg: b"jpeg",
            })
            .collect();
        assert_eq!(
            request_body("x", &many, "choice", &schema()),
            Err(PlateJudgeError::InvalidRequest)
        );
    }
}
