//! The client's address behind trusted proxies. Per-address limits are only as good as the
//! address: a client must not be able to choose it, and a request whose address is unknown must
//! be refused rather than counted in a bucket shared by every such request.
//!
//! The peer address of the connection comes from the server (`ConnectInfo`; composition serves
//! with `into_make_service_with_connect_info`). If the peer is not one of the configured proxy
//! networks, `X-Forwarded-For` is ignored entirely: anyone can send it. If the peer is a trusted
//! proxy, the client is the RIGHTMOST hop that is not itself a trusted proxy: a proxy appends the
//! address it received from, so everything left of the first untrusted hop is whatever the client
//! (or an earlier, untrusted hop) chose to claim.

use std::net::IpAddr;

/// An IP network (`10.0.0.0/8`, `fd00::/8`, or a single address).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Network {
    address: IpAddr,
    prefix: u8,
}

/// Why a proxy network was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkRefusal;

impl Network {
    fn parse(text: &str) -> Result<Self, NetworkRefusal> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address.trim().parse().map_err(|_| NetworkRefusal)?;
        let address = canonical(address);
        let widest = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => widest,
            Some(prefix) => prefix
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= widest)
                .ok_or(NetworkRefusal)?,
        };
        Ok(Self { address, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(network) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(network) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// An IPv4 address that arrived on a dual-stack socket as `::ffff:a.b.c.d` is that IPv4 address.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// The networks whose connections are trusted proxies. The default is none: the peer address is
/// the client.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrustedProxies(Vec<Network>);

impl TrustedProxies {
    pub fn none() -> Self {
        Self::default()
    }

    /// Parses configured networks, e.g. `["10.0.0.0/8", "fd00::/8"]`.
    pub fn parse(networks: &[&str]) -> Result<Self, NetworkRefusal> {
        networks
            .iter()
            .map(|network| Network::parse(network))
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    fn contains(&self, ip: IpAddr) -> bool {
        self.0.iter().any(|network| network.contains(ip))
    }
}

/// Why no client address could be determined; the request is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientIpRefusal {
    /// The server passed no connection information.
    NoPeer,
    /// A trusted proxy sent no forwarding header, or one that is not a list of addresses.
    MalformedForwarding,
}

impl ClientIpRefusal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoPeer => "no_peer",
            Self::MalformedForwarding => "malformed_forwarding",
        }
    }
}

/// The client's address. `forwarded_for` holds the value of every `X-Forwarded-For` header.
pub fn client_ip(
    peer: Option<IpAddr>,
    forwarded_for: &[&str],
    proxies: &TrustedProxies,
) -> Result<IpAddr, ClientIpRefusal> {
    let peer = canonical(peer.ok_or(ClientIpRefusal::NoPeer)?);
    if !proxies.contains(peer) {
        return Ok(peer);
    }
    let mut hops = Vec::new();
    for value in forwarded_for {
        for hop in value.split(',') {
            let hop = hop.trim().trim_start_matches('[').trim_end_matches(']');
            let ip: IpAddr = hop
                .parse()
                .map_err(|_| ClientIpRefusal::MalformedForwarding)?;
            hops.push(canonical(ip));
        }
    }
    // A trusted proxy appends what it saw; with nothing, it is not forwarding.
    let leftmost = *hops.first().ok_or(ClientIpRefusal::MalformedForwarding)?;
    Ok(hops
        .iter()
        .rev()
        .find(|hop| !proxies.contains(**hop))
        .copied()
        .unwrap_or(leftmost))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn proxies() -> TrustedProxies {
        TrustedProxies::parse(&["10.0.0.0/8", "192.168.5.9", "fd00::/8"]).unwrap()
    }

    #[test]
    fn without_a_peer_the_request_is_refused_not_shared() {
        assert_eq!(
            client_ip(None, &["203.0.113.7"], &proxies()),
            Err(ClientIpRefusal::NoPeer)
        );
        assert_eq!(
            client_ip(None, &[], &TrustedProxies::none()),
            Err(ClientIpRefusal::NoPeer)
        );
    }

    #[test]
    fn a_peer_that_is_not_a_proxy_is_the_client_whatever_the_header_says() {
        assert_eq!(
            client_ip(Some(ip("203.0.113.7")), &["198.51.100.1"], &proxies()),
            Ok(ip("203.0.113.7"))
        );
        assert_eq!(
            client_ip(
                Some(ip("203.0.113.7")),
                &["10.0.0.1"],
                &TrustedProxies::none()
            ),
            Ok(ip("203.0.113.7"))
        );
    }

    #[test]
    fn a_trusted_proxy_gives_the_rightmost_untrusted_hop() {
        // The client claimed to be 1.2.3.4; the first proxy saw 203.0.113.7 and appended it;
        // a second proxy (10.1.1.1) appended the first.
        assert_eq!(
            client_ip(
                Some(ip("10.9.9.9")),
                &["1.2.3.4, 203.0.113.7, 10.1.1.1"],
                &proxies()
            ),
            Ok(ip("203.0.113.7"))
        );
        // Several header lines are one list, in order.
        assert_eq!(
            client_ip(
                Some(ip("10.9.9.9")),
                &["1.2.3.4", "203.0.113.7, 10.1.1.1"],
                &proxies()
            ),
            Ok(ip("203.0.113.7"))
        );
        assert_eq!(
            client_ip(Some(ip("192.168.5.9")), &["198.51.100.2"], &proxies()),
            Ok(ip("198.51.100.2"))
        );
    }

    #[test]
    fn a_spoofed_leftmost_entry_never_wins_while_an_untrusted_hop_stands_to_its_right() {
        assert_eq!(
            client_ip(
                Some(ip("10.0.0.5")),
                &["10.0.0.99, 203.0.113.7"],
                &proxies()
            ),
            Ok(ip("203.0.113.7"))
        );
    }

    #[test]
    fn when_every_hop_is_a_proxy_the_leftmost_is_the_client() {
        assert_eq!(
            client_ip(Some(ip("10.0.0.5")), &["10.2.2.2, 10.3.3.3"], &proxies()),
            Ok(ip("10.2.2.2"))
        );
    }

    #[test]
    fn a_trusted_proxy_without_a_usable_header_is_refused() {
        for forwarded in [
            &[][..],
            &[""][..],
            &["not an address"][..],
            &["1.2.3.4:80"][..],
        ] {
            assert_eq!(
                client_ip(Some(ip("10.0.0.5")), forwarded, &proxies()),
                Err(ClientIpRefusal::MalformedForwarding),
                "{forwarded:?}"
            );
        }
    }

    #[test]
    fn ipv6_and_mapped_ipv4_are_compared_canonically() {
        assert_eq!(
            client_ip(Some(ip("::ffff:10.0.0.5")), &["[2001:db8::1]"], &proxies()),
            Ok(ip("2001:db8::1"))
        );
        assert_eq!(
            client_ip(Some(ip("::ffff:203.0.113.7")), &["1.2.3.4"], &proxies()),
            Ok(ip("203.0.113.7"))
        );
        assert_eq!(
            client_ip(Some(ip("fd00::5")), &["2001:db8::9"], &proxies()),
            Ok(ip("2001:db8::9"))
        );
    }

    #[test]
    fn networks_have_exact_boundaries_and_bad_ones_are_refused() {
        let proxies = TrustedProxies::parse(&["10.1.0.0/16", "0.0.0.0/0"]);
        assert!(proxies.is_ok());
        let narrow = TrustedProxies::parse(&["10.1.0.0/16"]).unwrap();
        assert!(narrow.contains(ip("10.1.255.255")));
        assert!(!narrow.contains(ip("10.2.0.0")));
        let host = TrustedProxies::parse(&["10.1.2.3"]).unwrap();
        assert!(host.contains(ip("10.1.2.3")) && !host.contains(ip("10.1.2.4")));
        let everything = TrustedProxies::parse(&["0.0.0.0/0"]).unwrap();
        assert!(everything.contains(ip("8.8.8.8")));
        for bad in [
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0.0/-1",
            "banana",
            "10.0.0.0/",
            "",
        ] {
            assert_eq!(TrustedProxies::parse(&[bad]), Err(NetworkRefusal), "{bad}");
        }
    }
}
