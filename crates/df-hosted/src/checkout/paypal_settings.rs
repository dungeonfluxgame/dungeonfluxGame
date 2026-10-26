//! PayPal credentials stay native and configuration failures identify names only.

use crate::config::{ConfigError, Profile};

pub(crate) struct Settings {
    pub(super) client_id: String,
    pub(super) client_secret: String,
    pub(super) webhook_id: Option<String>,
    pub(super) is_test_mode: bool,
    #[cfg(test)]
    pub(super) endpoint: Option<String>,
}

impl Settings {
    pub(crate) fn parse(
        get: &impl Fn(&str) -> Option<String>,
        profile: Profile,
    ) -> Result<Option<Self>, ConfigError> {
        let present = |name| get(name).filter(|value| !value.trim().is_empty());
        let live = match present("DF_PAYPAL_LIVE").as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            _ => return Err(ConfigError::Invalid("DF_PAYPAL_LIVE")),
        };
        let environment = present("DF_PAYPAL_ENV");
        let client_id = present("DF_PAYPAL_CLIENT_ID");
        let client_secret = present("DF_PAYPAL_CLIENT_SECRET");
        let webhook_id = present("DF_PAYPAL_WEBHOOK_ID");
        if environment.is_none() && client_id.is_none() && client_secret.is_none() {
            return if webhook_id.is_none() {
                Ok(None)
            } else {
                Err(ConfigError::Missing(
                    "DF_PAYPAL_ENV/DF_PAYPAL_CLIENT_ID/DF_PAYPAL_CLIENT_SECRET",
                ))
            };
        }
        let is_test_mode = match environment.as_deref() {
            Some("sandbox") => true,
            Some("live") if profile == Profile::Development || live => false,
            Some("live") => return Err(ConfigError::Invalid("DF_PAYPAL_LIVE")),
            Some(_) => return Err(ConfigError::Invalid("DF_PAYPAL_ENV")),
            None => return Err(ConfigError::Missing("DF_PAYPAL_ENV")),
        };
        Ok(Some(Self {
            client_id: client_id.ok_or(ConfigError::Missing("DF_PAYPAL_CLIENT_ID"))?,
            client_secret: client_secret.ok_or(ConfigError::Missing("DF_PAYPAL_CLIENT_SECRET"))?,
            webhook_id,
            is_test_mode,
            #[cfg(test)]
            endpoint: None,
        }))
    }
}
