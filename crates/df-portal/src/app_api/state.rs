//! What every handler shares: the account service, the configured origin and the ports.

use std::sync::Arc;

use df_customer::{AccountStore, Authenticated, CustomerService, EmailSender, ServiceError};

use super::{guard::GuardState, ports::Ports};
use crate::PortalOrigin;

pub struct AppApiState<S, M, P> {
    pub(super) service: CustomerService<S, M>,
    pub(super) origin: Arc<PortalOrigin>,
    pub(super) ports: Arc<P>,
}

impl<S, M, P> Clone for AppApiState<S, M, P> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            origin: Arc::clone(&self.origin),
            ports: Arc::clone(&self.ports),
        }
    }
}

impl<S, M, P> AppApiState<S, M, P> {
    pub fn new(service: CustomerService<S, M>, origin: PortalOrigin, ports: Arc<P>) -> Self {
        Self {
            service,
            origin: Arc::new(origin),
            ports,
        }
    }
}

impl<S: AccountStore, M: EmailSender, P: Ports> GuardState for AppApiState<S, M, P> {
    fn origin(&self) -> &str {
        self.origin.as_str()
    }

    async fn authenticate(
        &self,
        session_token: &str,
    ) -> Result<Option<Authenticated>, ServiceError> {
        self.service.authenticate(session_token).await
    }
}
