//! Where is the speaker's head in a generated scene plate (PLAY-V09)?
//!
//! The image model composes a plate but does not place the speaker where it was asked (it moved
//! the speaker by up to a third of the frame), and the display must put the speech bubble beside
//! the head and never over it. So the finished plate is shown, downscaled, to a vision model once,
//! and the box of one named person's head comes back as fractions of the picture. One bounded
//! `POST /v1/chat/completions`; the adapter never retries; the picture, the description and the
//! answer are never logged or placed in errors. The description is a fixed phrase of the server's
//! closed vocabulary, never player text.

use std::{fmt, net::SocketAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::native_http::NativeHttp;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const LIVE_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-5.5";
const MAXIMUM_PICTURE_BYTES: usize = 1024 * 1024;
const MAXIMUM_DESCRIPTION_CHARACTERS: usize = 300;
const MAXIMUM_TARGETS: usize = 4;
const MAXIMUM_RESPONSE_BYTES: usize = 256 * 1024;
/// A head is smaller than this share of the picture; a larger box is not a head.
const MAXIMUM_HEAD_WIDTH: f32 = 0.35;
const MAXIMUM_HEAD_HEIGHT: f32 = 0.5;

/// A head's box as fractions of the picture: `left < right` and `top < bottom`, each in 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// One person to find: a short plain id (`speaker`, `hero-1`) and a fixed description.
#[derive(Clone, Copy, Debug)]
pub struct LocateTarget<'a> {
    pub id: &'a str,
    pub description: &'a str,
}

/// A head found in the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct LocatedHead {
    pub id: String,
    pub head: HeadBox,
}

/// Why no head box came back. The caller keeps the layout it predicted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeadLocateError {
    /// The request failed local validation and nothing was sent.
    InvalidRequest,
    Unauthorized,
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// The answer was not one plausible head box.
    Output,
}

/// Where the adapter sends calls.
pub struct HeadLocateConfig {
    fixture: Option<SocketAddr>,
}

impl HeadLocateConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, HeadLocateError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(HeadLocateError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

pub struct NativeHeadLocator {
    endpoint: String,
    credential: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeHeadLocator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeHeadLocator")
            .finish_non_exhaustive()
    }
}

impl NativeHeadLocator {
    /// `total_timeout` bounds each call, including the response body.
    pub fn new(
        config: HeadLocateConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, HeadLocateError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(HeadLocateError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: 2 * MAXIMUM_PICTURE_BYTES,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 4096,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| HeadLocateError::InvalidRequest)?;
        Ok(Self {
            endpoint: config.fixture.map_or_else(
                || LIVE_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/chat/completions"),
            ),
            credential: credential.to_owned(),
            http,
        })
    }

    /// The heads of `targets` in the JPEG `picture`; `reference` is the face of the first target
    /// (a JPEG), which helps tell the right person from the many in the picture. One call, never
    /// retried. Anyone the model leaves out is simply absent from the answer.
    pub async fn locate(
        &self,
        picture: &[u8],
        reference: Option<&[u8]>,
        targets: &[LocateTarget<'_>],
        deadline: Duration,
    ) -> Result<Vec<LocatedHead>, HeadLocateError> {
        let body = request_body(picture, reference, targets)?;
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
            .map_err(|failure| HeadLocateError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(HeadLocateError::Unauthorized),
            status => return Err(HeadLocateError::Provider(status)),
        }
        let value = parse_unique_json(
            &response.body,
            TextSchemaLimits {
                maximum_schema_bytes: MAXIMUM_RESPONSE_BYTES,
                maximum_depth: 16,
                maximum_nodes: 4096,
            },
        )
        .map_err(|_| HeadLocateError::Output)?;
        let content = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or(HeadLocateError::Output)?;
        parse_heads(content, targets)
    }
}

/// The request body: the pictures as data URIs and the fixed instruction.
fn request_body(
    picture: &[u8],
    reference: Option<&[u8]>,
    targets: &[LocateTarget<'_>],
) -> Result<Vec<u8>, HeadLocateError> {
    let plain_id = |id: &str| {
        (1..=16).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if picture.is_empty()
        || picture.len() > MAXIMUM_PICTURE_BYTES
        || reference.is_some_and(|face| face.is_empty() || face.len() > MAXIMUM_PICTURE_BYTES)
        || targets.is_empty()
        || targets.len() > MAXIMUM_TARGETS
        || targets.iter().any(|target| {
            !plain_id(target.id)
                || target.description.trim().is_empty()
                || target.description.chars().count() > MAXIMUM_DESCRIPTION_CHARACTERS
        })
    {
        return Err(HeadLocateError::InvalidRequest);
    }
    let people: Vec<String> = targets
        .iter()
        .map(|target| format!("{}: {}", target.id, target.description))
        .collect();
    let second = if reference.is_some() {
        format!(" The second picture is the face of {}.", targets[0].id)
    } else {
        String::new()
    };
    let instruction = format!(
        "The first picture is a film still.{second} Find the head of each of these people in the \
         still: {}. Reply with only JSON {{\"heads\":[{{\"id\":\"name\",\"left\":0.0,\"top\":0.0,\
         \"right\":0.0,\"bottom\":0.0}}]}}: the box of each head (hair to chin; for a person seen \
         from behind, the back of the head), each number a fraction of the still's width (left, \
         right) or height (top, bottom). Leave out anyone who is not in the still.",
        people.join("; ")
    );
    let uri = |bytes: &[u8]| format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes));
    let mut content = vec![
        json!({"type": "text", "text": instruction}),
        json!({"type": "image_url", "image_url": {"url": uri(picture), "detail": "low"}}),
    ];
    if let Some(face) = reference {
        content
            .push(json!({"type": "image_url", "image_url": {"url": uri(face), "detail": "low"}}));
    }
    serde_json::to_vec(&json!({
        "model": MODEL,
        "max_completion_tokens": 2000,
        "messages": [{"role": "user", "content": content}],
    }))
    .map_err(|_| HeadLocateError::InvalidRequest)
}

/// One head box from a JSON object: numbers are fractions of the picture and the size is that of
/// a head.
fn parse_box(value: &Value) -> Option<HeadBox> {
    let side = |name: &str| -> Option<f32> {
        let number = value.get(name).and_then(Value::as_f64)?;
        (0.0..=1.0).contains(&number).then_some(number as f32)
    };
    let found = HeadBox {
        left: side("left")?,
        top: side("top")?,
        right: side("right")?,
        bottom: side("bottom")?,
    };
    let (width, height) = (found.right - found.left, found.bottom - found.top);
    (width > 0.0 && height > 0.0 && width <= MAXIMUM_HEAD_WIDTH && height <= MAXIMUM_HEAD_HEIGHT)
        .then_some(found)
}

/// The heads in the model's text: JSON, possibly inside a code fence. Entries for people who were
/// not asked for, repeated or implausible are dropped; at least one must remain.
fn parse_heads(
    content: &str,
    targets: &[LocateTarget<'_>],
) -> Result<Vec<LocatedHead>, HeadLocateError> {
    let start = content.find('{').ok_or(HeadLocateError::Output)?;
    let end = content.rfind('}').ok_or(HeadLocateError::Output)?;
    let value: Value =
        serde_json::from_str(content.get(start..=end).ok_or(HeadLocateError::Output)?)
            .map_err(|_| HeadLocateError::Output)?;
    let mut found: Vec<LocatedHead> = Vec::new();
    for entry in value
        .get("heads")
        .and_then(Value::as_array)
        .ok_or(HeadLocateError::Output)?
    {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if targets.iter().any(|target| target.id == id)
            && !found.iter().any(|head| head.id == id)
            && let Some(head) = parse_box(entry)
        {
            found.push(LocatedHead {
                id: id.to_owned(),
                head,
            });
        }
    }
    if found.is_empty() {
        return Err(HeadLocateError::Output);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGETS: [LocateTarget<'static>; 2] = [
        LocateTarget {
            id: "speaker",
            description: "an innkeeper in a work apron",
        },
        LocateTarget {
            id: "hero-1",
            description: "the red-bearded dwarf fighter seen from behind",
        },
    ];

    #[test]
    fn heads_are_read_from_plain_or_fenced_json_and_unasked_or_repeated_entries_are_dropped() {
        let plain = parse_heads(
            r#"{"heads":[{"id":"speaker","left":0.328,"top":0.25,"right":0.396,"bottom":0.413},
               {"id":"hero-1","left":0.7,"top":0.2,"right":0.8,"bottom":0.4},
               {"id":"speaker","left":0.1,"top":0.1,"right":0.2,"bottom":0.2},
               {"id":"stranger","left":0.1,"top":0.1,"right":0.2,"bottom":0.2}]}"#,
            &TARGETS,
        )
        .unwrap();
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0].id, "speaker");
        assert_eq!(plain[0].head.left, 0.328);
        assert_eq!(plain[1].id, "hero-1");
        let fenced = parse_heads(
            "```json\n{\"heads\":[{\"id\":\"speaker\",\"left\":0.1,\"top\":0.2,\"right\":0.2,\"bottom\":0.35}]}\n```",
            &TARGETS,
        );
        assert_eq!(fenced.unwrap().len(), 1);
    }

    #[test]
    fn answers_with_no_plausible_head_are_refused() {
        for bad in [
            "",
            "no head here",
            r#"{"heads":[]}"#,
            r#"{"heads":[{"id":"speaker","left":0.5,"top":0.2,"right":0.4,"bottom":0.3}]}"#,
            r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.9,"bottom":0.4}]}"#,
            r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.2,"bottom":0.9}]}"#,
            r#"{"heads":[{"id":"speaker","left":-0.1,"top":0.1,"right":0.2,"bottom":0.3}]}"#,
            r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":1.4,"bottom":0.3}]}"#,
            r#"{"heads":[{"id":"speaker","left":"a","top":0.1,"right":0.2,"bottom":0.3}]}"#,
            r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.2}]}"#,
            r#"{"left":0.1}"#,
        ] {
            assert_eq!(
                parse_heads(bad, &TARGETS),
                Err(HeadLocateError::Output),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_request_carries_the_pictures_and_the_fixed_instruction_and_refuses_bad_input() {
        let body = request_body(b"\xff\xd8jpeg", Some(b"\xff\xd8face"), &TARGETS).unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["model"], MODEL);
        let content = &value["messages"][0]["content"];
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("speaker: an innkeeper in a work apron"));
        assert!(text.contains("hero-1: the red-bearded dwarf fighter"));
        assert!(text.contains("The second picture is the face of speaker"));
        for picture in [&content[1], &content[2]] {
            assert!(
                picture["image_url"]["url"]
                    .as_str()
                    .unwrap()
                    .starts_with("data:image/jpeg;base64,")
            );
        }
        let without = request_body(b"jpeg", None, &TARGETS[..1]).unwrap();
        let value: Value = serde_json::from_slice(&without).unwrap();
        assert_eq!(value["messages"][0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(
            request_body(b"", None, &TARGETS),
            Err(HeadLocateError::InvalidRequest)
        );
        assert_eq!(
            request_body(b"jpeg", None, &[]),
            Err(HeadLocateError::InvalidRequest)
        );
        let long = "x".repeat(301);
        for (id, description) in [("speaker", long.as_str()), ("Bad Id", "x")] {
            assert_eq!(
                request_body(b"jpeg", None, &[LocateTarget { id, description }]),
                Err(HeadLocateError::InvalidRequest)
            );
        }
    }
}
