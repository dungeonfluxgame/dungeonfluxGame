//! Direct Google composition; password identities remain in their existing provider adapter.

use df_customer::{
    AccountStore, CustomerService, EmailSender, GoogleStore, HostConfig, OAuthSecret,
};
use df_google::{Config, ConfigError, Google};

pub(super) fn configure<S: AccountStore + GoogleStore + Clone, M: EmailSender>(
    service: CustomerService<S, M>,
    accounts: S,
    hosts: &HostConfig,
) -> Result<CustomerService<S, M>, ConfigError> {
    let client_id = credential("DF_GOOGLE_CLIENT_ID")?;
    let client_secret = credential("DF_GOOGLE_CLIENT_SECRET")?;
    match client_id.zip(client_secret) {
        Some((client_id, client_secret)) => {
            let provider = Google::new(Config::new(
                client_id,
                OAuthSecret::new(client_secret),
                hosts,
            )?)?;
            Ok(service.with_google(provider, accounts))
        }
        None => Ok(service.with_google_store(accounts)),
    }
}

fn credential(name: &str) -> Result<Option<String>, ConfigError> {
    std::env::var_os(name)
        .map(|value| value.into_string().map_err(|_| ConfigError::Invalid))
        .transpose()
        .map(|value| value.filter(|text| !text.is_empty()))
}
