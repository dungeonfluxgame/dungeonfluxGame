//! Business identity loaded from host-only environment variables.

use std::{env, fmt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusinessConfig {
    pub legal_entity_name: String,
    pub legal_entity_kind: String,
    pub trading_name: String,
    pub support_email: String,
    pub billing_email: String,
    pub privacy_email: String,
    pub security_email: String,
    pub statement_descriptor: String,
    pub seller_published: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusinessConfigError {
    Missing(&'static str),
    Invalid(&'static str),
}

impl fmt::Display for BusinessConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(key) => write!(formatter, "{key} is missing"),
            Self::Invalid(key) => write!(formatter, "{key} is invalid"),
        }
    }
}

impl std::error::Error for BusinessConfigError {}

impl BusinessConfig {
    /// Reads business identity from the named host environment variables.
    pub fn load_from_env() -> Result<Self, BusinessConfigError> {
        Self::from_values(|key| env::var(key).ok())
    }

    fn from_values(
        read: impl Fn(&'static str) -> Option<String>,
    ) -> Result<Self, BusinessConfigError> {
        let legal_entity_name = required(&read, "DF_LEGAL_ENTITY_NAME")?;
        let legal_entity_kind = required(&read, "DF_LEGAL_ENTITY_KIND")?;
        let trading_name = required(&read, "DF_TRADING_NAME")?;
        let support_email = required(&read, "DF_SUPPORT_EMAIL")?;
        let billing_email = required(&read, "DF_BILLING_EMAIL")?;
        let privacy_email = required(&read, "DF_PRIVACY_EMAIL")?;
        let security_email = required(&read, "DF_SECURITY_EMAIL")?;
        let statement_descriptor = required(&read, "DF_STATEMENT_DESCRIPTOR")?;
        let seller_published = match read("DF_SELLER_PUBLISHED").as_deref() {
            Some("true") => true,
            Some("false") => false,
            Some(_) => return Err(BusinessConfigError::Invalid("DF_SELLER_PUBLISHED")),
            None => return Err(BusinessConfigError::Missing("DF_SELLER_PUBLISHED")),
        };

        for (key, value) in [
            ("DF_LEGAL_ENTITY_NAME", &legal_entity_name),
            ("DF_LEGAL_ENTITY_KIND", &legal_entity_kind),
            ("DF_TRADING_NAME", &trading_name),
        ] {
            if value.trim().is_empty() || value.contains(['\r', '\n']) {
                return Err(BusinessConfigError::Invalid(key));
            }
        }
        for (key, value) in [
            ("DF_SUPPORT_EMAIL", &support_email),
            ("DF_BILLING_EMAIL", &billing_email),
            ("DF_PRIVACY_EMAIL", &privacy_email),
            ("DF_SECURITY_EMAIL", &security_email),
        ] {
            if !valid_email(value) {
                return Err(BusinessConfigError::Invalid(key));
            }
        }
        if !(5..=22).contains(&statement_descriptor.len())
            || !statement_descriptor.bytes().all(|byte| {
                byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b' ' || byte == b'-'
            })
        {
            return Err(BusinessConfigError::Invalid("DF_STATEMENT_DESCRIPTOR"));
        }

        Ok(Self {
            legal_entity_name,
            legal_entity_kind,
            trading_name,
            support_email,
            billing_email,
            privacy_email,
            security_email,
            statement_descriptor,
            seller_published,
        })
    }

    /// The single seller identity field exposed by public account configuration.
    pub fn public_seller_identity(&self) -> Option<String> {
        self.seller_published.then(|| {
            format!(
                "{} is operated by {}, {}",
                self.trading_name, self.legal_entity_name, self.legal_entity_kind
            )
        })
    }
}

fn required(
    read: &impl Fn(&'static str) -> Option<String>,
    key: &'static str,
) -> Result<String, BusinessConfigError> {
    read(key)
        .filter(|value| !value.is_empty())
        .ok_or(BusinessConfigError::Missing(key))
}

fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.contains('@')
        && !value.chars().any(char::is_whitespace)
        && !value.contains(['<', '>', '\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::{BusinessConfig, BusinessConfigError};

    fn values(
        published: &str,
        descriptor: &str,
    ) -> std::collections::HashMap<&'static str, String> {
        [
            ("DF_LEGAL_ENTITY_NAME", "EXAMPLE LLC"),
            ("DF_LEGAL_ENTITY_KIND", "limited liability company"),
            ("DF_TRADING_NAME", "DungeonFlux"),
            ("DF_SUPPORT_EMAIL", "support@example.test"),
            ("DF_BILLING_EMAIL", "billing@example.test"),
            ("DF_PRIVACY_EMAIL", "privacy@example.test"),
            ("DF_SECURITY_EMAIL", "security@example.test"),
            ("DF_STATEMENT_DESCRIPTOR", descriptor),
            ("DF_SELLER_PUBLISHED", published),
        ]
        .into_iter()
        .map(|(key, value)| (key, value.to_owned()))
        .collect()
    }

    #[test]
    fn a_published_seller_identity_uses_the_loaded_entity_and_kind() {
        let input = values("true", "DUNGEONFLUX");
        let config = BusinessConfig::from_values(|key| input.get(key).cloned()).expect("valid");

        assert_eq!(
            config.public_seller_identity().as_deref(),
            Some("DungeonFlux is operated by EXAMPLE LLC, limited liability company")
        );
    }

    #[test]
    fn an_unpublished_seller_has_no_public_identity() {
        let input = values("false", "DUNGEONFLUX");
        let config = BusinessConfig::from_values(|key| input.get(key).cloned()).expect("valid");

        assert_eq!(config.public_seller_identity(), None);
    }

    #[test]
    fn a_statement_descriptor_longer_than_twenty_two_characters_is_rejected() {
        let input = values("false", "DUNGEONFLUX-STATEMENT-TOO-LONG");

        assert_eq!(
            BusinessConfig::from_values(|key| input.get(key).cloned()),
            Err(BusinessConfigError::Invalid("DF_STATEMENT_DESCRIPTOR"))
        );
    }
}
