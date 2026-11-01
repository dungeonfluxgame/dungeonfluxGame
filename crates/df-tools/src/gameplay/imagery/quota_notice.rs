//! Display-only aggregation of live host quota registries; no admission or retry authority.
//!
//! One registration is owned by each host. Dead registrations are pruned at registration/read,
//! and Drop removes its own entry. The collection is bounded by actual simultaneous host owners.

use df_imagery::runtime::QuotaRegistry;
use std::sync::{Arc, Mutex, Weak};

struct RegisteredHost {
    registry: Weak<QuotaRegistry>,
    lifetime: Weak<()>,
}
static HOSTS: Mutex<Vec<RegisteredHost>> = Mutex::new(Vec::new());

/// Owned by one host Shared; worker/config rebuilds retain the same registration.
pub(in crate::gameplay) struct QuotaNoticeRegistration {
    lifetime: Arc<()>,
}

/// Registers only a weak read-only view; the token never retains a host or its quota state.
pub(in crate::gameplay) fn register_quota_notice(
    registry: &Arc<QuotaRegistry>,
) -> QuotaNoticeRegistration {
    let lifetime = Arc::new(());
    let mut hosts = super::locked(&HOSTS);
    hosts.retain(|host| host.lifetime.strong_count() > 0 && host.registry.strong_count() > 0);
    hosts.push(RegisteredHost {
        registry: Arc::downgrade(registry),
        lifetime: Arc::downgrade(&lifetime),
    });
    QuotaNoticeRegistration { lifetime }
}
impl Drop for QuotaNoticeRegistration {
    fn drop(&mut self) {
        let lifetime = Arc::downgrade(&self.lifetime);
        super::locked(&HOSTS).retain(|host| {
            !Weak::ptr_eq(&host.lifetime, &lifetime)
                && host.lifetime.strong_count() > 0
                && host.registry.strong_count() > 0
        });
    }
}

pub(super) fn operational_notice() -> Option<&'static str> {
    let mut open = false;
    let mut unavailable = false;
    super::locked(&HOSTS).retain(|host| {
        if host.lifetime.strong_count() == 0 {
            return false;
        }
        let Some(registry) = host.registry.upgrade() else {
            return false;
        };
        match registry.summary() {
            Ok(summary) => open |= summary.open_accounts > 0 || summary.probing_accounts > 0,
            Err(_) => unavailable = true,
        }
        true
    });
    if open {
        Some("An image provider has reported exhausted quota")
    } else if unavailable {
        Some("Image provider status unavailable")
    } else {
        None
    }
}
