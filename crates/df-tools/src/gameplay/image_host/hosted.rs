//! Trusted hosted placement inputs accompany the same image runtime resources.

use std::io;

use df_finance::ports::FenceRequest;
use df_persistence::imagery_identity::HostedRunGrant;

use super::{LocalHostSettings, failure};

/// Database roles may differ; canonical, gameplay, imagery and finance state share one database.
#[derive(Clone)]
pub(in crate::gameplay) struct HostedHostSettings {
    pub(in crate::gameplay) grant: HostedRunGrant,
    pub(in crate::gameplay) fence: FenceRequest,
    pub(in crate::gameplay) finance_database: tokio_postgres::Config,
    pub(in crate::gameplay) member_database: tokio_postgres::Config,
    pub(in crate::gameplay) authority_database: tokio_postgres::Config,
    pub(in crate::gameplay) dispatcher: [u8; 16],
    pub(in crate::gameplay) quote_lifetime_ms: u64,
}

impl HostedHostSettings {
    pub(in crate::gameplay) fn validate(
        &self,
        resources: &LocalHostSettings,
    ) -> Result<(), io::Error> {
        let run = &self.grant.binding;
        let gameplay = &resources.run_grant;
        if run.gameplay_session != gameplay.session
            || run.run != gameplay.run
            || run.fence != gameplay.fence
            || self.grant.bootstrap_credential != gameplay.bootstrap_credential
            || self.fence.tenant.as_bytes() != &run.tenant
            || self.fence.campaign.as_bytes() != &run.campaign
            || self.dispatcher == [0; 16]
            || self.quote_lifetime_ms == 0
            || self.quote_lifetime_ms > i64::MAX as u64
        {
            return Err(failure(
                "hosted image placement differs from captured authority",
            ));
        }
        for database in [
            &self.finance_database,
            &self.member_database,
            &self.authority_database,
        ] {
            if database.get_dbname() != resources.worker_database.get_dbname()
                || database.get_hosts() != resources.worker_database.get_hosts()
                || database.get_ports() != resources.worker_database.get_ports()
            {
                return Err(failure("hosted images require one canonical database"));
            }
        }
        Ok(())
    }
}
