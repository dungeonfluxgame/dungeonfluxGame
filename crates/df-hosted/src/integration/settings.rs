//! Validated integration settings. No Debug implementation can reveal a pepper.
use crate::config::ConfigError;
use df_customer::integration::Peppers;
use df_portal::integration_api::RatePolicy;
pub(crate) struct Settings {
    pub(crate) peppers: Peppers,
    pub(crate) rate: RatePolicy,
}
pub(crate) fn share_tombstone_days(get:&impl Fn(&str)->Option<String>)->Result<u16,ConfigError>{
    let days=number(get,"DF_SHARE_TOMBSTONE_DAYS",Some(30))?;
    if !(1..=365).contains(&days){return Err(ConfigError::Invalid("DF_SHARE_TOMBSTONE_DAYS"));}
    u16::try_from(days).map_err(|_|ConfigError::Invalid("DF_SHARE_TOMBSTONE_DAYS"))
}
pub(crate) fn share_public_policy(get:&impl Fn(&str)->Option<String>)->Result<df_portal::share::PublicSharePolicy,ConfigError>{
    df_portal::share::PublicSharePolicy::new(number(get,"DF_SHARE_REQUESTS_PER_MINUTE",Some(600))?,number(get,"DF_SHARE_PREFIX_REQUESTS_PER_MINUTE",Some(60))?)
        .map_err(|_|ConfigError::Invalid("DF_SHARE_REQUESTS_PER_MINUTE/DF_SHARE_PREFIX_REQUESTS_PER_MINUTE"))
}
fn key(get: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<[u8; 32], ConfigError> {
    let text = get(name).ok_or(ConfigError::Missing(name))?;
    df_campaign::secrets::unhex(&text)
        .and_then(|bytes| bytes.try_into().ok())
        .filter(|key| key != &[0; 32])
        .ok_or(ConfigError::Invalid(name))
}
fn number(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: Option<u32>,
) -> Result<u32, ConfigError> {
    match get(name) {
        Some(value) => value.parse().map_err(|_| ConfigError::Invalid(name)),
        None => default.ok_or(ConfigError::Missing(name)),
    }
}
impl Settings {
    pub(crate) fn parse(
        get: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ConfigError> {
        if get("DF_PAT_PEPPER_KEY").is_none() && get("DF_PAT_PEPPER_ID").is_none() {
            for name in [
                "DF_PAT_RETIRING_PEPPER_KEY",
                "DF_PAT_RETIRING_PEPPER_ID",
                "DF_PAT_RETIRING_UNTIL_MS",
                "DF_PAT_REQUESTS_PER_MINUTE",
                "DF_PAT_ACCOUNT_REQUESTS_PER_MINUTE",
                "DF_PAT_CONCURRENCY",
            ] {
                if get(name).is_some() {
                    return Err(ConfigError::Missing("DF_PAT_PEPPER_KEY/DF_PAT_PEPPER_ID"));
                }
            }
            return Ok(None);
        }
        let id = number(get, "DF_PAT_PEPPER_ID", None)?;
        let active = key(get, "DF_PAT_PEPPER_KEY")?;
        let retiring = if [
            "DF_PAT_RETIRING_PEPPER_ID",
            "DF_PAT_RETIRING_PEPPER_KEY",
            "DF_PAT_RETIRING_UNTIL_MS",
        ]
        .iter()
        .any(|name| get(name).is_some())
        {
            let old = number(get, "DF_PAT_RETIRING_PEPPER_ID", None)?;
            let old_key = key(get, "DF_PAT_RETIRING_PEPPER_KEY")?;
            let until = get("DF_PAT_RETIRING_UNTIL_MS")
                .ok_or(ConfigError::Missing("DF_PAT_RETIRING_UNTIL_MS"))?
                .parse::<u64>()
                .map_err(|_| ConfigError::Invalid("DF_PAT_RETIRING_UNTIL_MS"))?;
            Some((old, old_key, until))
        } else {
            None
        };
        let peppers = Peppers::new(id, active, retiring)
            .map_err(|_| ConfigError::Invalid("DF_PAT_PEPPER_ID/DF_PAT_PEPPER_KEY"))?;
        let rate = RatePolicy::new(
            number(get, "DF_PAT_REQUESTS_PER_MINUTE", Some(120))?,
            number(get, "DF_PAT_ACCOUNT_REQUESTS_PER_MINUTE", Some(600))?,
            usize::try_from(number(get, "DF_PAT_CONCURRENCY", Some(10))?)
                .map_err(|_| ConfigError::Invalid("DF_PAT_CONCURRENCY"))?,
        )
        .map_err(|_| {
            ConfigError::Invalid(
                "DF_PAT_REQUESTS_PER_MINUTE/DF_PAT_ACCOUNT_REQUESTS_PER_MINUTE/DF_PAT_CONCURRENCY",
            )
        })?;
        Ok(Some(Self { peppers, rate }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn integration_settings_require_paired_nonzero_peppers_and_an_explicit_retirement_window() {
        let mut values = BTreeMap::new();
        assert!(
            Settings::parse(&|name| values.get(name).cloned())
                .unwrap()
                .is_none()
        );
        values.insert("DF_PAT_PEPPER_KEY", "00".repeat(32));
        assert!(Settings::parse(&|name| values.get(name).cloned()).is_err());
        values.insert("DF_PAT_PEPPER_ID", "1".into());
        values.insert("DF_PAT_PEPPER_KEY", "47".repeat(32));
        assert!(
            Settings::parse(&|name| values.get(name).cloned())
                .unwrap()
                .is_some()
        );
        values.insert("DF_PAT_RETIRING_PEPPER_ID", "2".into());
        values.insert("DF_PAT_RETIRING_PEPPER_KEY", "51".repeat(32));
        assert!(Settings::parse(&|name| values.get(name).cloned()).is_err());
        values.insert("DF_PAT_RETIRING_UNTIL_MS", "1800000000000".into());
        assert!(
            Settings::parse(&|name| values.get(name).cloned())
                .unwrap()
                .is_some()
        );
        values.insert("DF_PAT_CONCURRENCY", "0".into());
        assert!(Settings::parse(&|name| values.get(name).cloned()).is_err());
    }
}
