//! Synthetic inspection contracts and pixels shared by this private boundary's tests.

use crate::request::{AnalysisRoute, AssetDigest, SourceRef};
use crate::{
    AccountHandle, ApiEndpoint, ImageInspection, InspectionKind, InspectionRequirement, ModelName,
    PrivateImageText,
};
use sha2::{Digest, Sha256};

pub(crate) fn spec(kind: InspectionKind, party: bool) -> ImageInspection {
    ImageInspection {
        route: AnalysisRoute {
            account: AccountHandle::new("analysis-fixture").unwrap(),
            model: ModelName::new("operator/vision:v4").unwrap(),
            endpoint: ApiEndpoint::new("http://127.0.0.1:9/custom%20analysis/v2").unwrap(),
            maximum_output_tokens: 777,
        },
        requirement: InspectionRequirement::BestEffort,
        contract_revision: 1,
        kind,
        expected_heroes: party.then_some(2),
        party_reference: party.then_some(1),
        hand_custody: PrivateImageText::new("hands empty"),
        staging: Vec::new(),
        maximum_corrections: 1,
        allow_visible_text: true,
    }
}

pub(crate) fn png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(pixel));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

pub(crate) fn pin(bytes: &[u8]) -> SourceRef {
    SourceRef {
        asset: df_assets::AssetId::from_bytes(&[7; 16]).unwrap(),
        digest: AssetDigest::from_bytes(Sha256::digest(bytes).into()),
        audience: df_assets::Audience::PublicStatic,
    }
}
