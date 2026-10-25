//! Public hostnames as configuration (coordinator launch decision 32). Production is one origin
//! (`https://dungeonfluxdnd.com`), staging another; transactional mail goes out from a `mail.`
//! subdomain and replies go to the support mailbox. None of these is a constant in code: the
//! email From and Reply-To, every link base in an email template, the identity provider's
//! redirect allowlist and the payment processor's return and cancel URLs all derive from one
//! `HostConfig`, parsed from `dungeonflux.hosts/1` JSON owned by deployment.

use serde_json::Value;

use crate::{email::LinkStyle, records::EmailAddress};

pub const HOSTS_SCHEMA: &str = "dungeonflux.hosts/1";

/// Why a configured origin was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginRefusal {
    /// Not `https://host[:port]`, nor plain http on a loopback host.
    NotSecure,
    /// Has a path, query, fragment, credentials or trailing slash.
    NotAnOrigin,
}

/// The one canonical external origin links and form checks use (`bootstrap-origin-policy.md`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicOrigin(String);

impl PublicOrigin {
    /// Accepts `https://host[:port]`, or `http://` on `127.0.0.1`, `[::1]` or `localhost` for
    /// loopback development. Never derived from a request.
    pub fn parse(text: &str) -> Result<Self, OriginRefusal> {
        let (scheme, rest) = text.split_once("://").ok_or(OriginRefusal::NotAnOrigin)?;
        if rest.is_empty() || rest.contains(['/', '?', '#', '@']) {
            return Err(OriginRefusal::NotAnOrigin);
        }
        let (host, port) = match rest.strip_prefix('[') {
            // `[::1]` or `[::1]:8099`.
            Some(bracketed) => {
                let (inside, after) = bracketed
                    .split_once(']')
                    .ok_or(OriginRefusal::NotAnOrigin)?;
                if inside.parse::<std::net::Ipv6Addr>().is_err() {
                    return Err(OriginRefusal::NotAnOrigin);
                }
                let port = match after {
                    "" => None,
                    after => Some(after.strip_prefix(':').ok_or(OriginRefusal::NotAnOrigin)?),
                };
                (format!("[{inside}]"), port)
            }
            None => match rest.split_once(':') {
                Some((host, port)) => (host.to_owned(), Some(port)),
                None => (rest.to_owned(), None),
            },
        };
        if let Some(port) = port
            && !(!port.is_empty()
                && port.len() <= 5
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0))
        {
            return Err(OriginRefusal::NotAnOrigin);
        }
        if !host_is_well_formed(&host) {
            return Err(OriginRefusal::NotAnOrigin);
        }
        match scheme {
            "https" => Ok(Self(text.to_owned())),
            "http" if matches!(host.as_str(), "127.0.0.1" | "[::1]" | "localhost") => {
                Ok(Self(text.to_owned()))
            }
            _ => Err(OriginRefusal::NotSecure),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A DNS name (letters, digits and hyphens in non-empty labels, no leading or trailing hyphen),
/// an IPv4 address or a bracketed IPv6 address. Anything else (spaces, `%`, `\`, `@`, `_`,
/// non-ASCII) is not a host we would put in a link or compare an `Origin` with.
fn host_is_well_formed(host: &str) -> bool {
    if host.starts_with('[') {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// Why a hosts file was refused. The field names the first bad value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostsRefusal {
    NotJson,
    WrongSchema,
    Missing(&'static str),
    BadOrigin(OriginRefusal),
    BadAddress(&'static str),
    BadName,
    BadStyle,
}

/// The address and name mail is sent from, and where replies go.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailIdentity {
    from_name: String,
    from: EmailAddress,
    reply_to: EmailAddress,
}

impl MailIdentity {
    /// A display name without quotes, angle brackets or control characters (header injection),
    /// and two well-formed addresses.
    pub fn new(from_name: &str, from: &str, reply_to: &str) -> Result<Self, HostsRefusal> {
        let name = from_name.trim();
        if name.is_empty()
            || name.chars().count() > 60
            || name
                .chars()
                .any(|character| character.is_control() || matches!(character, '"' | '<' | '>'))
        {
            return Err(HostsRefusal::BadName);
        }
        Ok(Self {
            from_name: name.to_owned(),
            from: EmailAddress::parse(from).map_err(|_| HostsRefusal::BadAddress("mail.from"))?,
            reply_to: EmailAddress::parse(reply_to)
                .map_err(|_| HostsRefusal::BadAddress("mail.reply_to"))?,
        })
    }

    /// The `From` header value: `DungeonFlux <accounts@mail.example>`.
    pub fn from_header(&self) -> String {
        format!("{} <{}>", self.from_name, self.from.display())
    }

    pub fn reply_to_header(&self) -> String {
        self.reply_to.display().to_owned()
    }
}

/// A lowercase hexadecimal identifier safe to put in a URL: the only kind of value the payment
/// return and cancel URLs accept, so no query text can be smuggled into them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HexId(String);

impl HexId {
    /// The hexadecimal text of an identifier's bytes.
    pub fn of(bytes: &[u8]) -> Self {
        Self(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// Accepts 2 to 64 lowercase hexadecimal characters, an even count.
    pub fn parse(text: &str) -> Option<Self> {
        let valid = (2..=64).contains(&text.len())
            && text.len().is_multiple_of(2)
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Every public URL the account service and the payment processor need, from one origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostConfig {
    origin: PublicOrigin,
    mail: MailIdentity,
    style: LinkStyle,
}

impl HostConfig {
    pub fn new(origin: PublicOrigin, mail: MailIdentity, style: LinkStyle) -> Self {
        Self {
            origin,
            mail,
            style,
        }
    }

    /// Local development and tests: a loopback origin, mail from a `.test` domain, and the
    /// server-rendered pages' links.
    pub fn local(origin: &str) -> Result<Self, HostsRefusal> {
        Ok(Self::new(
            PublicOrigin::parse(origin).map_err(HostsRefusal::BadOrigin)?,
            MailIdentity::new(
                "DungeonFlux",
                "accounts@mail.example.test",
                "support@example.test",
            )?,
            LinkStyle::Classic,
        ))
    }

    /// Parses the deployment's hosts file.
    pub fn parse(text: &str) -> Result<Self, HostsRefusal> {
        let document: Value = serde_json::from_str(text).map_err(|_| HostsRefusal::NotJson)?;
        if document.get("schema").and_then(Value::as_str) != Some(HOSTS_SCHEMA) {
            return Err(HostsRefusal::WrongSchema);
        }
        let text_at = |section: &str, key: &'static str| -> Result<&str, HostsRefusal> {
            let node = if section.is_empty() {
                Some(&document)
            } else {
                document.get(section)
            };
            node.and_then(|node| node.get(key))
                .and_then(Value::as_str)
                .ok_or(HostsRefusal::Missing(key))
        };
        let origin =
            PublicOrigin::parse(text_at("", "origin")?).map_err(HostsRefusal::BadOrigin)?;
        let mail = MailIdentity::new(
            text_at("mail", "from_name")?,
            text_at("mail", "from_address")?,
            text_at("mail", "reply_to")?,
        )?;
        let style = match document
            .get("links")
            .and_then(|links| links.get("style"))
            .and_then(Value::as_str)
        {
            None | Some("app") => LinkStyle::App,
            Some("classic") => LinkStyle::Classic,
            Some(_) => return Err(HostsRefusal::BadStyle),
        };
        Ok(Self::new(origin, mail, style))
    }

    pub fn origin(&self) -> &PublicOrigin {
        &self.origin
    }

    pub fn mail(&self) -> &MailIdentity {
        &self.mail
    }

    pub fn style(&self) -> LinkStyle {
        self.style
    }

    /// The redirect URLs to register at the identity provider (WorkOS dashboard redirects): the
    /// password-reset page and the email-verification page the emails point at, and the
    /// sign-out landing page. A provider call that carries any URL is checked against this list.
    pub fn identity_redirect_allowlist(&self) -> Vec<String> {
        let origin = self.origin.as_str();
        match self.style {
            LinkStyle::App => vec![
                format!("{origin}/app/reset-password"),
                format!("{origin}/app/verify-email"),
                format!("{origin}/app/signed-out"),
            ],
            LinkStyle::Classic => vec![
                format!("{origin}/password-reset/confirm"),
                format!("{origin}/verify-email"),
                format!("{origin}/login"),
            ],
        }
    }

    /// Where the processor returns a person after paying. `checkout` is our own checkout id as
    /// hexadecimal text; the return page only reads state, it never credits (plan PAY-01).
    pub fn payment_return_url(&self, checkout: &HexId) -> String {
        format!(
            "{}/app/credit/return?checkout={}",
            self.origin.as_str(),
            checkout.as_str()
        )
    }

    /// Where the processor sends a person who backs out of paying.
    pub fn payment_cancel_url(&self, checkout: &HexId) -> String {
        format!(
            "{}/app/credit/return?checkout={}&canceled=1",
            self.origin.as_str(),
            checkout.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{
        "schema": "dungeonflux.hosts/1",
        "origin": "https://staging.dungeonfluxdnd.com",
        "mail": {"from_name": "DungeonFlux", "from_address": "accounts@mail.dungeonfluxdnd.com",
                 "reply_to": "support@dungeonfluxdnd.com"}
    }"#;

    #[test]
    fn every_public_url_comes_from_the_file() {
        let hosts = HostConfig::parse(FILE).unwrap();
        assert_eq!(
            hosts.origin().as_str(),
            "https://staging.dungeonfluxdnd.com"
        );
        assert_eq!(
            hosts.mail().from_header(),
            "DungeonFlux <accounts@mail.dungeonfluxdnd.com>"
        );
        assert_eq!(hosts.mail().reply_to_header(), "support@dungeonfluxdnd.com");
        assert_eq!(
            hosts.identity_redirect_allowlist(),
            [
                "https://staging.dungeonfluxdnd.com/app/reset-password",
                "https://staging.dungeonfluxdnd.com/app/verify-email",
                "https://staging.dungeonfluxdnd.com/app/signed-out",
            ]
        );
        assert_eq!(
            hosts.payment_return_url(&HexId::parse("ab12").unwrap()),
            "https://staging.dungeonfluxdnd.com/app/credit/return?checkout=ab12"
        );
        assert_eq!(
            hosts.payment_cancel_url(&HexId::parse("ab12").unwrap()),
            "https://staging.dungeonfluxdnd.com/app/credit/return?checkout=ab12&canceled=1"
        );
    }

    #[test]
    fn a_different_file_changes_every_url() {
        let production = FILE.replace("staging.dungeonfluxdnd.com", "dungeonfluxdnd.com");
        let hosts = HostConfig::parse(&production).unwrap();
        assert!(
            hosts
                .identity_redirect_allowlist()
                .iter()
                .all(|url| url.starts_with("https://dungeonfluxdnd.com/"))
        );
        assert!(
            hosts
                .payment_cancel_url(&HexId::parse("c1").unwrap())
                .starts_with("https://dungeonfluxdnd.com/")
        );
    }

    #[test]
    fn plain_http_elsewhere_and_non_origins_are_refused() {
        assert_eq!(
            PublicOrigin::parse("http://192.168.1.20:8099"),
            Err(OriginRefusal::NotSecure)
        );
        for origin in [
            "https://play.example.test/",
            "https://play.example.test/account",
            "https://user@play.example.test",
            "play.example.test",
        ] {
            assert_eq!(
                PublicOrigin::parse(origin),
                Err(OriginRefusal::NotAnOrigin),
                "{origin}"
            );
        }
        assert!(PublicOrigin::parse("http://localhost:8099").is_ok());
    }

    #[test]
    fn bad_files_are_refused_by_the_first_bad_value() {
        assert_eq!(HostConfig::parse("{nope"), Err(HostsRefusal::NotJson));
        assert_eq!(
            HostConfig::parse(r#"{"schema": "other/1"}"#),
            Err(HostsRefusal::WrongSchema)
        );
        assert_eq!(
            HostConfig::parse(&FILE.replace("https://staging", "http://staging")),
            Err(HostsRefusal::BadOrigin(OriginRefusal::NotSecure))
        );
        assert_eq!(
            HostConfig::parse(&FILE.replace("support@dungeonfluxdnd.com", "support")),
            Err(HostsRefusal::BadAddress("mail.reply_to"))
        );
        assert_eq!(
            HostConfig::parse(&FILE.replace("\"DungeonFlux\"", "\"Dungeon<Flux>\"")),
            Err(HostsRefusal::BadName)
        );
        assert_eq!(
            HostConfig::parse(
                &FILE.replace("\"origin\"", "\"links\": {\"style\": \"x\"}, \"origin\"")
            ),
            Err(HostsRefusal::BadStyle)
        );
    }

    #[test]
    fn a_host_must_be_a_plain_dns_name_address_or_bracketed_ipv6() {
        for origin in [
            "https://play.example.test:8443",
            "https://a-b.example.test",
            "http://[::1]:8099",
            "http://127.0.0.1",
        ] {
            assert!(PublicOrigin::parse(origin).is_ok(), "{origin}");
        }
        for origin in [
            "https://play example.test",
            "https://play.example.test:0",
            "https://play.example.test:99999",
            "https://play.example.test:8o",
            "https://play_x.example.test",
            "https://-play.example.test",
            "https://play..example.test",
            "https://pl%61y.example.test",
            "https://play.example.test\\evil.test",
            "https://[not-an-address]",
            "https://",
        ] {
            assert!(PublicOrigin::parse(origin).is_err(), "{origin}");
        }
    }

    #[test]
    fn only_lowercase_hexadecimal_goes_into_a_payment_url() {
        assert!(HexId::parse("ab12").is_some());
        for bad in ["", "a", "AB12", "ab12&x=1", "ab 12", "zz", &"a".repeat(66)] {
            assert!(HexId::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(HexId::of(&[0xab, 0x01]).as_str(), "ab01");
    }

    #[test]
    fn a_display_name_cannot_inject_a_header() {
        assert_eq!(
            MailIdentity::new("Dungeon\r\nBcc: x", "a@mail.test.example", "b@test.example"),
            Err(HostsRefusal::BadName)
        );
    }
}
