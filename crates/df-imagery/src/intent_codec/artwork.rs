//! Explicit artwork v1 metadata never recovers identity from provider prompt text.
use df_types::{CampaignId, RunId};

use super::{
    CodecError,
    primitives::{Reader, Writer},
};
use crate::{
    ArtworkBinding, ArtworkError, ArtworkSubject, ArtworkSubjectKey, CAMPAIGN_ARTWORK_SPEC_VERSION,
    CampaignArtworkSpec, MAXIMUM_ARTWORK_SUBJECT_BYTES, RunSelectionFence,
};

pub(super) fn encode_spec(
    output: &mut Writer,
    spec: &CampaignArtworkSpec,
) -> Result<(), CodecError> {
    output.u32_be(spec.version())?;
    output.extend(spec.campaign.as_bytes())?;
    output.push(spec.subject.tag())?;
    output.counted(spec.subject.key().as_str().as_bytes())?;
    output.extend(&spec.public_revision)
}

pub(super) fn encode(output: &mut Writer, artwork: &ArtworkBinding) -> Result<(), CodecError> {
    encode_spec(output, &artwork.spec)?;
    output.extend(artwork.selection.run.as_bytes())?;
    output.extend(&artwork.selection.binding)
}

pub(super) fn decode(input: &mut Reader<'_>) -> Result<ArtworkBinding, CodecError> {
    let version = input.u32_be()?;
    if version != CAMPAIGN_ARTWORK_SPEC_VERSION {
        return Err(CodecError::Request(crate::RequestError::InvalidArtwork(
            ArtworkError::UnsupportedVersion { actual: version },
        )));
    }
    let campaign = CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
    let namespace = input.byte()?;
    let key = std::str::from_utf8(input.counted(MAXIMUM_ARTWORK_SUBJECT_BYTES)?)
        .map_err(|_| CodecError::InvalidText)?;
    let key = ArtworkSubjectKey::new(key)
        .map_err(crate::RequestError::InvalidArtwork)
        .map_err(CodecError::Request)?;
    let subject = match namespace {
        0 => ArtworkSubject::Hero(key),
        1 => ArtworkSubject::Npc(key),
        2 => ArtworkSubject::Cast(key),
        3 => ArtworkSubject::Item(key),
        _ => return Err(CodecError::Malformed),
    };
    let public_revision = input
        .take(32)?
        .try_into()
        .map_err(|_| CodecError::Truncated)?;
    let run = RunId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
    let binding = input
        .take(32)?
        .try_into()
        .map_err(|_| CodecError::Truncated)?;
    Ok(ArtworkBinding {
        spec: CampaignArtworkSpec {
            campaign,
            subject,
            public_revision,
        },
        selection: RunSelectionFence { run, binding },
    })
}
