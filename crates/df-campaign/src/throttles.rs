//! The service's throttle layers and the keys they hash addresses with. Every layer is its own
//! fail-closed `KeyedWindows` map, so flooding one layer cannot lock out another. Pure apart from
//! the mutexes: the time is supplied.
//!
//! Order matters for the guess gate: the address keys (loose first) and the account key are
//! counted first, and the process-wide ceiling only for a request that passed them, so a flood of
//! refused requests cannot use up the ceiling for everyone else.

use std::{
    net::IpAddr,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use df_types::AccountId;

use crate::{
    config::Tunables,
    keyed_throttle::{KeyedWindows, Throttle},
    secrets::{IP_KEY_TAG, keyed_digest, unhex},
    throttle::ThrottlePolicy,
};

/// The two secrets of the service. They are configuration, stable across restarts and shared by
/// every replica: room-code digests and address hashes are stored in the database, so a key that
/// changed per process would break every stored code and multiply the pending caps by the
/// replica count.
#[derive(Clone)]
pub struct ServiceKeys {
    ip_key: [u8; 32],
    code_key: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeysRefusal {
    /// A key is not 64 lowercase hexadecimal digits.
    Malformed,
    /// A key is all zeroes, or the two keys are equal.
    Weak,
}

impl ServiceKeys {
    /// The configured keys, each 64 lowercase hexadecimal digits (32 random bytes). Equal or
    /// all-zero keys are refused.
    pub fn from_hex(ip_key: &str, code_key: &str) -> Result<Self, KeysRefusal> {
        let parse = |text: &str| -> Result<[u8; 32], KeysRefusal> {
            unhex(text)
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or(KeysRefusal::Malformed)
        };
        let (ip_key, code_key) = (parse(ip_key)?, parse(code_key)?);
        if ip_key == [0; 32] || code_key == [0; 32] || ip_key == code_key {
            return Err(KeysRefusal::Weak);
        }
        Ok(Self { ip_key, code_key })
    }

    /// Random keys for a unit test that builds one service. Compiled for this crate's own tests
    /// only: a feature another crate turns on must not put a per-process key generator in a build
    /// that could run two replicas. Anything that needs a second instance uses `from_hex`.
    #[cfg(test)]
    pub fn generate() -> Result<Self, crate::secrets::RandomUnavailable> {
        Ok(Self {
            ip_key: crate::secrets::random_bytes::<32>()?,
            code_key: crate::secrets::random_bytes::<32>()?,
        })
    }

    pub fn code_key(&self) -> &[u8; 32] {
        &self.code_key
    }

    pub fn ip_key(&self) -> &[u8; 32] {
        &self.ip_key
    }
}

/// The tight and loose keys of one address: an IPv6 source is keyed by its /64 and by its /48,
/// an IPv4 source by the address and by its /24, so one allocation cannot fill the tight map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpKeys {
    pub tight: [u8; 32],
    pub loose: [u8; 32],
}

pub fn ip_keys(key: &[u8; 32], ip: IpAddr) -> IpKeys {
    let ip = ip.to_canonical();
    let (family, tight, loose): (u8, Vec<u8>, Vec<u8>) = match ip {
        IpAddr::V4(address) => {
            let octets = address.octets();
            (4, octets.to_vec(), octets[..3].to_vec())
        }
        IpAddr::V6(address) => {
            let octets = address.octets();
            (6, octets[..8].to_vec(), octets[..6].to_vec())
        }
    };
    let keyed = |scope: u8, bytes: &[u8]| {
        let mut data = vec![family, scope];
        data.extend_from_slice(bytes);
        keyed_digest(key, IP_KEY_TAG, &data)
    };
    IpKeys {
        tight: keyed(0, &tight),
        loose: keyed(1, &loose),
    }
}

/// Why a throttle layer refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateRefusal {
    Throttled,
    /// The layer's map is full of live windows and the key is new.
    Saturated,
}

fn outcome(throttle: Throttle) -> Result<(), GateRefusal> {
    match throttle {
        Throttle::Allowed => Ok(()),
        Throttle::Throttled { .. } => Err(GateRefusal::Throttled),
        Throttle::Saturated => Err(GateRefusal::Saturated),
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned throttle map is still a valid map: the panic that poisoned it left whole
    // windows, so the guard is used as is.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One address layer: the loose key is checked first, so a request it refuses never inserts a
/// tight key.
pub(crate) struct AddressLayer {
    tight: Mutex<KeyedWindows<[u8; 32]>>,
    loose: Mutex<KeyedWindows<[u8; 32]>>,
}

impl AddressLayer {
    pub(crate) fn new(
        limit: u32,
        multiplier: u32,
        window: Duration,
        tight_keys: usize,
        loose_keys: usize,
    ) -> Self {
        Self {
            tight: Mutex::new(KeyedWindows::new(
                ThrottlePolicy { limit, window },
                tight_keys,
            )),
            loose: Mutex::new(KeyedWindows::new(
                ThrottlePolicy {
                    limit: limit.saturating_mul(multiplier),
                    window,
                },
                loose_keys,
            )),
        }
    }

    pub(crate) fn attempt(&self, keys: &IpKeys, now_ms: u64) -> Result<(), GateRefusal> {
        outcome(lock(&self.loose).attempt(keys.loose, now_ms))?;
        outcome(lock(&self.tight).attempt(keys.tight, now_ms))
    }

    fn saturated(&self) -> u64 {
        lock(&self.tight).saturated_refusals() + lock(&self.loose).saturated_refusals()
    }
}

/// A per-key limit: one window per key, fail closed at capacity.
pub(crate) struct KeyLimit<K>(Mutex<KeyedWindows<K>>);

impl<K: Eq + std::hash::Hash> KeyLimit<K> {
    pub(crate) fn new(limit: u32, window: Duration, maximum_keys: usize) -> Self {
        Self(Mutex::new(KeyedWindows::new(
            ThrottlePolicy { limit, window },
            maximum_keys,
        )))
    }

    pub(crate) fn attempt(&self, key: K, now_ms: u64) -> Result<(), GateRefusal> {
        outcome(lock(&self.0).attempt(key, now_ms))
    }

    fn saturated(&self) -> u64 {
        lock(&self.0).saturated_refusals()
    }
}

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);

/// Every layer of the service.
pub(crate) struct Throttles {
    code_ip: AddressLayer,
    code_account: KeyLimit<AccountId>,
    code_process: KeyLimit<()>,
    preview_ip: AddressLayer,
    accept_account: KeyLimit<AccountId>,
    pub(crate) invitation_create: KeyLimit<AccountId>,
    pub(crate) display_approval: KeyLimit<AccountId>,
    pub(crate) member_changes: KeyLimit<AccountId>,
    pub(crate) draft_saves: KeyLimit<AccountId>,
    pub(crate) lobby_changes: KeyLimit<AccountId>,
    pub(crate) lobby_host: KeyLimit<AccountId>,
    challenge_ip: AddressLayer,
    status_ip: AddressLayer,
    ticket_ip: AddressLayer,
    pub(crate) redeem_pairing: KeyLimit<[u8; 32]>,
    pub(crate) renew_ticket: KeyLimit<[u8; 32]>,
}

impl Throttles {
    pub(crate) fn new(tunables: &Tunables) -> Self {
        let multiplier = tunables.throttle_loose_multiplier;
        let tight = tunables.throttle_maximum_keys_tight;
        let loose = tunables.throttle_maximum_keys_loose;
        let accounts = tunables.throttle_maximum_keys_account;
        let minutes = |count: u32| Duration::from_secs(u64::from(count) * 60);
        Self {
            code_ip: AddressLayer::new(
                tunables.lookup_limit_per_ip,
                multiplier,
                minutes(tunables.lookup_window_ip_minutes),
                tight,
                loose,
            ),
            code_account: KeyLimit::new(
                tunables.lookup_limit_per_account,
                minutes(tunables.lookup_window_account_minutes),
                accounts,
            ),
            code_process: KeyLimit::new(tunables.lookup_limit_process_per_minute, MINUTE, 1),
            preview_ip: AddressLayer::new(
                tunables.preview_limit_per_ip_per_minute,
                multiplier,
                MINUTE,
                tight,
                loose,
            ),
            accept_account: KeyLimit::new(
                tunables.accept_limit_per_account_per_hour,
                HOUR,
                accounts,
            ),
            invitation_create: KeyLimit::new(
                tunables.invitation_create_limit_per_account_per_hour,
                HOUR,
                accounts,
            ),
            display_approval: KeyLimit::new(
                tunables.display_approval_limit_per_account,
                minutes(15),
                accounts,
            ),
            member_changes: KeyLimit::new(
                tunables.member_change_limit_per_account_per_minute,
                MINUTE,
                accounts,
            ),
            draft_saves: KeyLimit::new(
                tunables.draft_save_limit_per_account_per_hour,
                HOUR,
                accounts,
            ),
            lobby_changes: KeyLimit::new(
                tunables.lobby_change_limit_per_account_per_hour,
                HOUR,
                accounts,
            ),
            lobby_host: KeyLimit::new(
                tunables.lobby_host_limit_per_account_per_hour,
                HOUR,
                accounts,
            ),
            challenge_ip: AddressLayer::new(
                tunables.display_challenge_limit_per_ip_per_minute,
                multiplier,
                MINUTE,
                tight,
                loose,
            ),
            status_ip: AddressLayer::new(
                tunables.display_status_limit_per_ip_per_minute,
                multiplier,
                MINUTE,
                tight,
                loose,
            ),
            ticket_ip: AddressLayer::new(
                tunables.display_ticket_limit_per_ip_per_minute,
                multiplier,
                MINUTE,
                tight,
                loose,
            ),
            // Keyed by a pairing or a ticket the store has confirmed, so they are bounded by what
            // exists and sized with the other id-keyed windows.
            redeem_pairing: KeyLimit::new(
                tunables.display_redeem_limit_per_pairing_per_hour,
                HOUR,
                accounts,
            ),
            renew_ticket: KeyLimit::new(
                tunables.display_renew_limit_per_ticket_per_hour,
                HOUR,
                accounts,
            ),
        }
    }

    /// The gate a lookup of a code passes (room codes, display codes): the address keys and the
    /// account first, the process ceiling only for a request that passed them.
    pub(crate) fn code_gate(
        &self,
        keys: &IpKeys,
        account: Option<AccountId>,
        now_ms: u64,
    ) -> Result<(), GateRefusal> {
        self.code_ip.attempt(keys, now_ms)?;
        if let Some(account) = account {
            self.code_account.attempt(account, now_ms)?;
        }
        self.code_process.attempt((), now_ms)
    }

    /// Invitation links have their own layers and no process ceiling: previews by address keys
    /// (30 per minute), accepts per account (10 per hour). A flood of junk tokens therefore cannot
    /// stop other people's links from working.
    pub(crate) fn preview_gate(&self, keys: &IpKeys, now_ms: u64) -> Result<(), GateRefusal> {
        self.preview_ip.attempt(keys, now_ms)
    }

    pub(crate) fn accept_gate(&self, account: AccountId, now_ms: u64) -> Result<(), GateRefusal> {
        self.accept_account.attempt(account, now_ms)
    }

    pub(crate) fn challenge_gate(&self, keys: &IpKeys, now_ms: u64) -> Result<(), GateRefusal> {
        self.challenge_ip.attempt(keys, now_ms)
    }

    pub(crate) fn status_gate(&self, keys: &IpKeys, now_ms: u64) -> Result<(), GateRefusal> {
        self.status_ip.attempt(keys, now_ms)
    }

    /// The address layer in front of redeem and renew. Their per-pairing and per-ticket windows are
    /// keyed by cookies the caller chooses, so nothing may reach them before this passes, and they
    /// are counted only for a pairing or ticket the store has confirmed.
    pub(crate) fn ticket_gate(&self, keys: &IpKeys, now_ms: u64) -> Result<(), GateRefusal> {
        self.ticket_ip.attempt(keys, now_ms)
    }

    /// Refusals because a map was full, per layer, for the dev view.
    pub(crate) fn saturation_report(&self) -> Vec<(&'static str, u64)> {
        vec![
            ("code_ip", self.code_ip.saturated()),
            ("code_account", self.code_account.saturated()),
            ("preview_ip", self.preview_ip.saturated()),
            ("accept_account", self.accept_account.saturated()),
            ("invitation_create", self.invitation_create.saturated()),
            ("display_approval", self.display_approval.saturated()),
            ("member_changes", self.member_changes.saturated()),
            ("draft_saves", self.draft_saves.saturated()),
            ("lobby_changes", self.lobby_changes.saturated()),
            ("lobby_host", self.lobby_host.saturated()),
            ("challenge_ip", self.challenge_ip.saturated()),
            ("status_ip", self.status_ip.saturated()),
            ("ticket_ip", self.ticket_ip.saturated()),
            ("redeem_pairing", self.redeem_pairing.saturated()),
            ("renew_ticket", self.renew_ticket.saturated()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [3; 32];

    #[test]
    fn configured_keys_are_64_hex_digits_nonzero_and_distinct() {
        let a = "a1".repeat(32);
        let b = "b2".repeat(32);
        assert!(ServiceKeys::from_hex(&a, &b).is_ok());
        assert_eq!(
            ServiceKeys::from_hex(&a, "short").err(),
            Some(KeysRefusal::Malformed)
        );
        assert_eq!(ServiceKeys::from_hex(&a, &a).err(), Some(KeysRefusal::Weak));
        assert_eq!(
            ServiceKeys::from_hex(&"00".repeat(32), &b).err(),
            Some(KeysRefusal::Weak)
        );
        assert_eq!(
            ServiceKeys::from_hex(&"A1".repeat(32), &b).err(),
            Some(KeysRefusal::Malformed),
            "lowercase only"
        );
    }

    #[test]
    fn generated_keys_pass_the_configured_key_rules_and_differ_each_time() {
        let first = ServiceKeys::generate().unwrap();
        let second = ServiceKeys::generate().unwrap();
        assert_ne!(first.ip_key(), first.code_key());
        assert_ne!(first.ip_key(), second.ip_key());
        assert_ne!(first.ip_key(), &[0; 32]);
    }

    #[test]
    fn an_ipv6_source_is_keyed_by_its_64_tightly_and_its_48_loosely() {
        let a: IpAddr = "2001:db8:1:1::1".parse().unwrap();
        let same_64: IpAddr = "2001:db8:1:1:ffff::2".parse().unwrap();
        let same_48: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let other_48: IpAddr = "2001:db8:2:1::1".parse().unwrap();
        assert_eq!(ip_keys(&KEY, a).tight, ip_keys(&KEY, same_64).tight);
        assert_ne!(ip_keys(&KEY, a).tight, ip_keys(&KEY, same_48).tight);
        assert_eq!(ip_keys(&KEY, a).loose, ip_keys(&KEY, same_48).loose);
        assert_ne!(ip_keys(&KEY, a).loose, ip_keys(&KEY, other_48).loose);
    }

    #[test]
    fn an_ipv4_source_is_keyed_by_its_address_tightly_and_its_24_loosely() {
        let a: IpAddr = "203.0.113.7".parse().unwrap();
        let neighbour: IpAddr = "203.0.113.8".parse().unwrap();
        let far: IpAddr = "203.0.114.7".parse().unwrap();
        assert_ne!(ip_keys(&KEY, a).tight, ip_keys(&KEY, neighbour).tight);
        assert_eq!(ip_keys(&KEY, a).loose, ip_keys(&KEY, neighbour).loose);
        assert_ne!(ip_keys(&KEY, a).loose, ip_keys(&KEY, far).loose);
    }

    #[test]
    fn a_mapped_ipv4_address_keys_like_the_plain_one_and_the_keys_depend_on_the_secret() {
        let plain: IpAddr = "198.51.100.9".parse().unwrap();
        let mapped: IpAddr = "::ffff:198.51.100.9".parse().unwrap();
        assert_eq!(ip_keys(&KEY, plain), ip_keys(&KEY, mapped));
        assert_ne!(ip_keys(&KEY, plain).tight, ip_keys(&[4; 32], plain).tight);
    }

    #[test]
    fn a_request_the_loose_key_refuses_never_inserts_a_tight_key() {
        let layer = AddressLayer::new(1, 1, Duration::from_secs(60), 10, 10);
        let first: IpAddr = "203.0.113.1".parse().unwrap();
        let second: IpAddr = "203.0.113.2".parse().unwrap();
        assert_eq!(layer.attempt(&ip_keys(&KEY, first), 0), Ok(()));
        assert_eq!(
            layer.attempt(&ip_keys(&KEY, second), 1),
            Err(GateRefusal::Throttled)
        );
        assert_eq!(lock(&layer.tight).len(), 1);
    }

    #[test]
    fn the_loose_limit_is_ten_times_the_tight_limit_by_default() {
        let tunables = Tunables::default();
        assert_eq!(tunables.throttle_loose_multiplier, 10);
        let layer = AddressLayer::new(
            tunables.lookup_limit_per_ip,
            tunables.throttle_loose_multiplier,
            Duration::from_secs(60),
            100,
            100,
        );
        let mut allowed = 0;
        for host in 1..=200_u8 {
            let ip: IpAddr = format!("203.0.113.{host}").parse().unwrap();
            if layer.attempt(&ip_keys(&KEY, ip), 0).is_ok() {
                allowed += 1;
            }
        }
        assert_eq!(allowed, 100);
    }

    #[test]
    fn a_full_loose_map_refuses_every_new_loose_key_and_leaves_existing_keys_working() {
        let layer = AddressLayer::new(5, 10, Duration::from_secs(60), 100, 2);
        let one: IpAddr = "10.1.0.1".parse().unwrap();
        let two: IpAddr = "10.2.0.1".parse().unwrap();
        let three: IpAddr = "10.3.0.1".parse().unwrap();
        assert_eq!(layer.attempt(&ip_keys(&KEY, one), 0), Ok(()));
        assert_eq!(layer.attempt(&ip_keys(&KEY, two), 0), Ok(()));
        assert_eq!(
            layer.attempt(&ip_keys(&KEY, three), 0),
            Err(GateRefusal::Saturated)
        );
        assert_eq!(layer.attempt(&ip_keys(&KEY, one), 0), Ok(()));
    }

    #[test]
    fn the_guess_windows_are_the_designs_fifteen_minutes_and_an_hour() {
        let tunables = Tunables::default();
        assert_eq!(tunables.lookup_window_ip_minutes, 15);
        assert_eq!(tunables.lookup_window_account_minutes, 60);
        let throttles = Throttles::new(&tunables);
        let ip: IpAddr = "198.51.100.5".parse().unwrap();
        let keys = ip_keys(&KEY, ip);
        for _ in 0..tunables.lookup_limit_per_ip {
            assert_eq!(throttles.code_gate(&keys, None, 0), Ok(()));
        }
        // Still refused fourteen minutes later; allowed again once the fifteenth has passed.
        assert!(throttles.code_gate(&keys, None, 14 * 60_000).is_err());
        assert!(throttles.code_gate(&keys, None, 15 * 60_000).is_ok());
    }

    #[test]
    fn a_refused_request_does_not_use_the_process_ceiling() {
        let tunables = Tunables {
            lookup_limit_per_ip: 1,
            lookup_limit_process_per_minute: 5,
            ..Tunables::default()
        };
        let throttles = Throttles::new(&tunables);
        let noisy = ip_keys(&KEY, "203.0.113.50".parse().unwrap());
        assert_eq!(throttles.code_gate(&noisy, None, 0), Ok(()));
        for _ in 0..100 {
            assert!(throttles.code_gate(&noisy, None, 1).is_err());
        }
        // One slot was used by the one request that passed; the other four are still there.
        for host in 1..=4_u8 {
            let other = ip_keys(&KEY, format!("192.0.2.{host}").parse().unwrap());
            assert_eq!(throttles.code_gate(&other, None, 2), Ok(()), "host {host}");
        }
    }

    #[test]
    fn invitation_links_use_their_own_layers_and_not_the_process_ceiling() {
        let tunables = Tunables {
            lookup_limit_process_per_minute: 1,
            ..Tunables::default()
        };
        let throttles = Throttles::new(&tunables);
        let keys = ip_keys(&KEY, "198.51.100.9".parse().unwrap());
        // The ceiling for codes is spent, and previews still pass (30 per minute per address).
        assert_eq!(throttles.code_gate(&keys, None, 0), Ok(()));
        assert!(throttles.code_gate(&keys, None, 1).is_err());
        for _ in 0..30 {
            assert_eq!(throttles.preview_gate(&keys, 2), Ok(()));
        }
        assert!(throttles.preview_gate(&keys, 3).is_err());
        let account = AccountId::from_bytes(&[5; 16]).unwrap();
        for _ in 0..10 {
            assert_eq!(throttles.accept_gate(account, 4), Ok(()));
        }
        assert!(throttles.accept_gate(account, 5).is_err());
    }
}
