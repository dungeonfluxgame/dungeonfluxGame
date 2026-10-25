//! Secret-bearing processor settings are validated without retaining values in errors.

use crate::config::ConfigError;

pub(crate) struct Settings {
    pub(super) secret_key: String,
    pub(super) webhook_secret: String,
    pub(super) is_test_mode: bool,
    #[cfg(test)]
    pub(super) endpoint: Option<String>,
}

impl Settings {
    pub(crate) fn parse(
        get: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ConfigError> {
        let present = |name| get(name).filter(|value| !value.trim().is_empty());
        let live = match present("DF_STRIPE_LIVE").as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            _ => return Err(ConfigError::Invalid("DF_STRIPE_LIVE")),
        };
        match (
            present("DF_STRIPE_SECRET_KEY"),
            present("DF_STRIPE_WEBHOOK_SECRET"),
        ) {
            (None, None) => Ok(None),
            (Some(secret_key), Some(webhook_secret)) => {
                let is_test_mode = secret_key.starts_with("sk_test_");
                if !is_test_mode && !(live && secret_key.starts_with("sk_live_")) {
                    return Err(ConfigError::Invalid("DF_STRIPE_SECRET_KEY"));
                }
                if !webhook_secret.starts_with("whsec_") {
                    return Err(ConfigError::Invalid("DF_STRIPE_WEBHOOK_SECRET"));
                }
                Ok(Some(Self {
                    secret_key,
                    webhook_secret,
                    is_test_mode,
                    #[cfg(test)]
                    endpoint: None,
                }))
            }
            _ => Err(ConfigError::Missing(
                "DF_STRIPE_SECRET_KEY/DF_STRIPE_WEBHOOK_SECRET",
            )),
        }
    }
}
