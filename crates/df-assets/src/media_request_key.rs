//! `MediaRequestKey` (planning/asset-storage-research.md 3.4, 6.4): the one dedupe key of the
//! catalog. SHA-256 over the schema label and a fixed sequence of fields, each a big-endian `u32`
//! length followed by its bytes, ending with the full output audience. Length prefixes make the
//! encoding injective: no two different field lists encode to the same bytes, unlike today's
//! newline-joined `df-imagery/1` text.

use crate::audience::Audience;
use sha2::{Digest, Sha256};

/// The schema label hashed first. A change of fields is a new label, so old keys never collide.
pub const MEDIA_REQUEST_SCHEMA: &str = "df-imagery/2";

/// The inputs of one generated picture.
#[derive(Clone, Copy, Debug)]
pub struct MediaRequest<'a> {
    pub kind: &'a str,
    pub model: &'a str,
    pub quality: &'a str,
    pub rating: &'a str,
    pub width_px: u32,
    pub height_px: u32,
    pub is_transparent: bool,
    /// Reference image digests in request order.
    pub references: &'a [&'a str],
    pub prompt: &'a str,
    /// The public scene snapshot a scene plate is evidence for, when the kind has one.
    pub scene_evidence: Option<&'a str>,
    pub audience: &'a Audience,
}

/// SHA-256 of the canonical encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaRequestKey([u8; 32]);

impl MediaRequestKey {
    pub fn derive(request: &MediaRequest<'_>) -> Self {
        let mut encoded = Vec::with_capacity(256 + request.prompt.len());
        field(&mut encoded, MEDIA_REQUEST_SCHEMA.as_bytes());
        field(&mut encoded, request.kind.as_bytes());
        field(&mut encoded, request.model.as_bytes());
        field(&mut encoded, request.quality.as_bytes());
        field(&mut encoded, request.rating.as_bytes());
        field(&mut encoded, &request.width_px.to_be_bytes());
        field(&mut encoded, &request.height_px.to_be_bytes());
        field(&mut encoded, &[u8::from(request.is_transparent)]);
        field(
            &mut encoded,
            &length(request.references.len()).to_be_bytes(),
        );
        for reference in request.references {
            field(&mut encoded, reference.as_bytes());
        }
        field(&mut encoded, request.prompt.as_bytes());
        match request.scene_evidence {
            Some(evidence) => {
                field(&mut encoded, &[1]);
                field(&mut encoded, evidence.as_bytes());
            }
            None => field(&mut encoded, &[0]),
        }
        let mut audience = Vec::with_capacity(64);
        request.audience.encode_into(&mut audience);
        field(&mut encoded, &audience);
        Self(Sha256::digest(&encoded).into())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A length beyond `u32::MAX` cannot occur for bounded prompts (32,000 characters) and reference
/// lists; saturation only keeps the encoding total.
fn length(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&length(bytes.len()).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audience::MemberSet;
    use df_types::{CampaignId, MemberId};

    fn id(byte: u8) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        bytes
    }

    fn shared() -> Audience {
        Audience::CampaignShared(CampaignId::from_bytes(&id(1)).unwrap())
    }

    fn request<'a>(audience: &'a Audience, model: &'a str, quality: &'a str) -> MediaRequest<'a> {
        MediaRequest {
            kind: "scene_plate",
            model,
            quality,
            rating: "mature",
            width_px: 2560,
            height_px: 1440,
            is_transparent: false,
            references: &["aa", "bb"],
            prompt: "a tavern at dusk",
            scene_evidence: None,
            audience,
        }
    }

    #[test]
    fn fields_that_concatenate_to_the_same_text_hash_differently() {
        let audience = shared();
        assert_ne!(
            MediaRequestKey::derive(&request(&audience, "ab", "c")),
            MediaRequestKey::derive(&request(&audience, "a", "bc"))
        );
    }

    #[test]
    fn the_same_request_always_derives_the_same_key() {
        let audience = shared();
        assert_eq!(
            MediaRequestKey::derive(&request(&audience, "m", "q")),
            MediaRequestKey::derive(&request(&audience, "m", "q"))
        );
    }

    #[test]
    fn the_audience_is_part_of_the_key_down_to_the_member_ids() {
        let table = shared();
        let campaign = CampaignId::from_bytes(&id(1)).unwrap();
        let one = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(5)).unwrap()]).unwrap(),
        );
        let other = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(6)).unwrap()]).unwrap(),
        );
        let keys = [
            MediaRequestKey::derive(&request(&table, "m", "q")),
            MediaRequestKey::derive(&request(&one, "m", "q")),
            MediaRequestKey::derive(&request(&other, "m", "q")),
        ];
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
    }

    #[test]
    fn moving_a_reference_into_the_prompt_changes_the_key() {
        let audience = shared();
        let mut moved = request(&audience, "m", "q");
        moved.references = &["aa"];
        let prompt = format!("{}bb", moved.prompt);
        moved.prompt = &prompt;
        assert_ne!(
            MediaRequestKey::derive(&request(&audience, "m", "q")),
            MediaRequestKey::derive(&moved)
        );
    }
}
