use std::time::Duration;

use df_providers::{
    ImageBackground, ImageEdit, ImageEditConfig, ImageEditError, ImageEditModel, ImageFormat,
    ImageModeration, ImageQuality, ImageUsage, NativeImageEditProvider, ReferenceImage,
};

fn edit<'a>(references: &'a [ReferenceImage<'a>]) -> ImageEdit<'a> {
    ImageEdit {
        model: ImageEditModel::Gpt25Flare,
        prompt: "Keep the same person.",
        references,
        width: 1024,
        height: 1536,
        quality: ImageQuality::Medium,
        background: ImageBackground::Transparent,
        output: ImageFormat::Png,
        moderation: ImageModeration::Low,
    }
}

fn reference() -> ReferenceImage<'static> {
    ReferenceImage {
        format: ImageFormat::Webp,
        bytes: b"RIFFfake",
    }
}

#[test]
fn a_well_formed_edit_passes_local_validation() {
    let references = [reference()];
    assert_eq!(edit(&references).validate(), Ok(()));
    // A transparent result may be a PNG or a WebP; only a JPEG cannot carry alpha.
    let webp = ImageEdit {
        output: ImageFormat::Webp,
        ..edit(&references)
    };
    assert_eq!(webp.validate(), Ok(()));
}

#[test]
fn malformed_edits_are_refused_before_anything_is_sent() {
    let references = [reference()];
    let good = || edit(&references);
    let mut cases: Vec<ImageEdit<'_>> = Vec::new();
    for width in [1000, 4000, 16] {
        cases.push(ImageEdit { width, ..good() });
    }
    cases.push(ImageEdit {
        height: 100,
        width: 1024,
        ..good()
    });
    cases.push(ImageEdit {
        prompt: "   ",
        ..good()
    });
    cases.push(ImageEdit {
        output: ImageFormat::Jpeg,
        ..good()
    });
    for case in &cases {
        assert_eq!(case.validate(), Err(ImageEditError::InvalidRequest));
    }
    // No reference at all is a text-only generation, never an edit of a blank carrier.
    assert_eq!(
        ImageEdit {
            references: &[],
            ..good()
        }
        .validate(),
        Ok(())
    );
    let long = "x".repeat(df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS + 1);
    assert_eq!(
        ImageEdit {
            prompt: &long,
            ..good()
        }
        .validate(),
        Err(ImageEditError::InvalidRequest)
    );
    let empty = [ReferenceImage {
        format: ImageFormat::Png,
        bytes: b"",
    }];
    assert_eq!(
        good_with(&empty).validate(),
        Err(ImageEditError::InvalidRequest)
    );
}

fn good_with<'a>(references: &'a [ReferenceImage<'a>]) -> ImageEdit<'a> {
    edit(references)
}

#[test]
fn the_documented_image_edit_prompt_budget_accepts_merchant_assembly_size() {
    let references = [reference()];
    let merchant_scene_prompt = "x".repeat(4574);
    assert_eq!(
        ImageEdit {
            prompt: &merchant_scene_prompt,
            ..edit(&references)
        }
        .validate(),
        Ok(())
    );
    let over_budget = "x".repeat(df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS + 1);
    assert_eq!(
        ImageEdit {
            prompt: &over_budget,
            ..edit(&references)
        }
        .validate(),
        Err(ImageEditError::InvalidRequest)
    );
}

#[test]
fn a_credential_with_whitespace_or_nothing_is_not_accepted_and_no_remote_address_is_a_fixture() {
    for credential in ["", "has space", "line\nbreak"] {
        assert!(
            NativeImageEditProvider::new(
                ImageEditConfig::live(),
                credential,
                Duration::from_secs(5)
            )
            .is_err(),
            "{credential:?}"
        );
    }
    assert!(ImageEditConfig::loopback_fixture("8.8.8.8:443".parse().unwrap()).is_err());
    assert!(ImageEditConfig::loopback_fixture("127.0.0.1:0".parse().unwrap()).is_err());
    assert!(ImageEditConfig::loopback_fixture("127.0.0.1:8080".parse().unwrap()).is_ok());
}

#[test]
fn usage_is_priced_from_the_published_token_rates() {
    let usage = ImageUsage {
        text_input_tokens: 83,
        image_input_tokens: 864,
        output_tokens: 343,
    };
    assert_eq!(usage.micro_dollars(), 83 * 5 + 864 * 8 + 343 * 30);
    assert_eq!(ImageUsage::default().micro_dollars(), 0);
}

#[test]
fn only_the_documented_models_are_accepted_by_id() {
    assert_eq!(
        ImageEditModel::from_model_id("gpt-image-2.5-flare"),
        Some(ImageEditModel::Gpt25Flare)
    );
    assert_eq!(
        ImageEditModel::from_model_id("gpt-image-2.5-sunburst"),
        Some(ImageEditModel::Gpt25Sunburst)
    );
    assert_eq!(ImageEditModel::from_model_id("dall-e-3"), None);
    assert_eq!(ImageEditModel::from_model_id(""), None);
}
