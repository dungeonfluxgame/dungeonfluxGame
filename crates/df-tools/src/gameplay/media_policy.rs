//! Cost-sensitive media defaults: explicit video opt-in and deterministic image routing.
use std::{env::VarError, io};

pub(super) fn enabled(input: Result<String, VarError>, name: &str) -> Result<bool, io::Error> {
    match input.as_deref() {
        Err(VarError::NotPresent) | Ok("" | "off") => Ok(false),
        Ok("on") => Ok(true),
        _ => Err(io::Error::other(format!("{name} invalid"))),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageProvider {
    OpenAi,
    Flux,
}

pub(super) fn image_provider(
    input: Result<String, VarError>,
    has_fal: bool,
) -> Result<ImageProvider, io::Error> {
    match input.as_deref() {
        Err(VarError::NotPresent) | Ok("" | "openai") => Ok(ImageProvider::OpenAi),
        Ok("flux") if has_fal => Ok(ImageProvider::Flux),
        Ok("flux") => Err(io::Error::other(
            "DF_GAMEPLAY_IMAGERY_PROVIDER=flux needs FAL_KEY",
        )),
        _ => Err(io::Error::other("DF_GAMEPLAY_IMAGERY_PROVIDER refused")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_defaults_to_openai_regardless_of_fal_and_explicit_flux_needs_its_key() {
        for has_fal in [false, true] {
            assert_eq!(
                image_provider(Err(VarError::NotPresent), has_fal).unwrap(),
                ImageProvider::OpenAi
            );
            for choice in ["", "openai"] {
                assert_eq!(
                    image_provider(Ok(choice.to_owned()), has_fal).unwrap(),
                    ImageProvider::OpenAi
                );
            }
        }
        assert_eq!(
            image_provider(Ok("flux".to_owned()), true).unwrap(),
            ImageProvider::Flux
        );
        assert!(image_provider(Ok("flux".to_owned()), false).is_err());
        for choice in ["OpenAI", "openai ", "private-secret"] {
            let error = image_provider(Ok(choice.to_owned()), true).unwrap_err();
            assert!(!error.to_string().contains(choice));
        }
        assert!(image_provider(Err(VarError::NotUnicode("private-secret".into())), true).is_err());
    }

    #[test]
    fn credentials_cannot_enable_video_without_an_explicit_switch() {
        for name in [
            "DF_GAMEPLAY_HOME_VIDEO",
            "DF_GAMEPLAY_SCENE_VIDEO",
            "DF_GAMEPLAY_CHARACTER_VIDEO",
        ] {
            assert!(!enabled(Err(VarError::NotPresent), name).unwrap());
            for value in ["", "off"] {
                assert!(!enabled(Ok(value.to_owned()), name).unwrap());
            }
            assert!(enabled(Ok("on".to_owned()), name).unwrap());
        }
    }
    #[test]
    fn invalid_video_flags_are_refused_without_echoing_values() {
        for value in ["ON", "on ", "true", "private-secret"] {
            let error = enabled(Ok(value.to_owned()), "DF_GAMEPLAY_SCENE_VIDEO").unwrap_err();
            assert!(!error.to_string().contains(value));
        }
        assert!(
            enabled(
                Err(VarError::NotUnicode("private-secret".into())),
                "DF_GAMEPLAY_HOME_VIDEO"
            )
            .is_err()
        );
    }
}
