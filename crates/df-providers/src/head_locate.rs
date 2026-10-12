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

    /// The head of the person `description` names in the JPEG `picture`; one call, never retried.
    pub async fn locate(
        &self,
        picture: &[u8],
        description: &str,
        deadline: Duration,
    ) -> Result<HeadBox, HeadLocateError> {
        let body = request_body(picture, description)?;
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
        parse_head_box(content)
    }
}

/// The request body: the picture as a data URI and the fixed instruction.
fn request_body(picture: &[u8], description: &str) -> Result<Vec<u8>, HeadLocateError> {
    if picture.is_empty()
        || picture.len() > MAXIMUM_PICTURE_BYTES
        || description.trim().is_empty()
        || description.chars().count() > MAXIMUM_DESCRIPTION_CHARACTERS
    {
        return Err(HeadLocateError::InvalidRequest);
    }
    let instruction = format!(
        "In this picture, find the head of this one person: {description}. Reply with only JSON \
         {{\"left\":0.0,\"top\":0.0,\"right\":0.0,\"bottom\":0.0}}: the box of that person's \
         head (hair to chin), each number a fraction of the picture width (left, right) or \
         height (top, bottom)."
    );
    let uri = format!("data:image/jpeg;base64,{}", STANDARD.encode(picture));
    serde_json::to_vec(&json!({
        "model": MODEL,
        "max_completion_tokens": 1500,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": instruction},
                {"type": "image_url", "image_url": {"url": uri, "detail": "low"}},
            ],
        }],
    }))
    .map_err(|_| HeadLocateError::InvalidRequest)
}

/// One head box from the model's text: JSON, possibly inside a code fence, whose numbers are
/// fractions of the picture and whose size is that of a head.
fn parse_head_box(content: &str) -> Result<HeadBox, HeadLocateError> {
    let start = content.find('{').ok_or(HeadLocateError::Output)?;
    let end = content.rfind('}').ok_or(HeadLocateError::Output)?;
    let value: Value =
        serde_json::from_str(content.get(start..=end).ok_or(HeadLocateError::Output)?)
            .map_err(|_| HeadLocateError::Output)?;
    let side = |name: &str| -> Result<f32, HeadLocateError> {
        let number = value
            .get(name)
            .and_then(Value::as_f64)
            .ok_or(HeadLocateError::Output)?;
        (0.0..=1.0)
            .contains(&number)
            .then_some(number as f32)
            .ok_or(HeadLocateError::Output)
    };
    let found = HeadBox {
        left: side("left")?,
        top: side("top")?,
        right: side("right")?,
        bottom: side("bottom")?,
    };
    let (width, height) = (found.right - found.left, found.bottom - found.top);
    if width <= 0.0 || height <= 0.0 || width > MAXIMUM_HEAD_WIDTH || height > MAXIMUM_HEAD_HEIGHT {
        return Err(HeadLocateError::Output);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_head_box_is_read_from_plain_or_fenced_json() {
        let plain = parse_head_box(r#"{"left":0.328,"top":0.25,"right":0.396,"bottom":0.413}"#);
        assert_eq!(
            plain,
            Ok(HeadBox {
                left: 0.328,
                top: 0.25,
                right: 0.396,
                bottom: 0.413
            })
        );
        let fenced = parse_head_box(
            "```json\n{\"left\":0.1,\"top\":0.2,\"right\":0.2,\"bottom\":0.35}\n```",
        );
        assert!(fenced.is_ok());
    }

    #[test]
    fn answers_that_are_not_one_plausible_head_are_refused() {
        for bad in [
            "",
            "no head here",
            r#"{"left":0.5,"top":0.2,"right":0.4,"bottom":0.3}"#,
            r#"{"left":0.1,"top":0.1,"right":0.9,"bottom":0.4}"#,
            r#"{"left":0.1,"top":0.1,"right":0.2,"bottom":0.9}"#,
            r#"{"left":-0.1,"top":0.1,"right":0.2,"bottom":0.3}"#,
            r#"{"left":0.1,"top":0.1,"right":1.4,"bottom":0.3}"#,
            r#"{"left":"a","top":0.1,"right":0.2,"bottom":0.3}"#,
            r#"{"left":0.1,"top":0.1,"right":0.2}"#,
        ] {
            assert_eq!(parse_head_box(bad), Err(HeadLocateError::Output), "{bad}");
        }
    }

    #[test]
    fn the_request_carries_the_picture_and_the_fixed_instruction_and_refuses_bad_input() {
        let body = request_body(b"\xff\xd8jpeg", "an innkeeper in a work apron").unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["model"], MODEL);
        let content = &value["messages"][0]["content"];
        assert!(
            content[0]["text"]
                .as_str()
                .unwrap()
                .contains("an innkeeper in a work apron")
        );
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert_eq!(request_body(b"", "x"), Err(HeadLocateError::InvalidRequest));
        assert_eq!(
            request_body(b"jpeg", "  "),
            Err(HeadLocateError::InvalidRequest)
        );
        assert_eq!(
            request_body(b"jpeg", &"x".repeat(301)),
            Err(HeadLocateError::InvalidRequest)
        );
    }
}
