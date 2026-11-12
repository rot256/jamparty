//! Strict parsing of the `Host` header.
//!
//! A host is exactly `<domain>` (the front door) or exactly one label of
//! 16 lowercase base32 characters followed by `.<domain>` (a session host),
//! with the configured port if there is one, and nothing else.
//!
//! Case: hosts are not normalised. Browsers always send lowercase hosts, and
//! seeds are lowercase, so any uppercase character makes the host
//! [`HostKind::Unknown`]. One spelling per origin keeps the expected
//! `Origin` and the cookie scope unambiguous.

use std::fmt;

use crate::core::ids::Seed;

/// Scheme of the public URLs (HTTPS in production; HTTP for
/// `*.localhost` in tests, which browsers treat as a secure context).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
}

impl Scheme {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

/// What a request's `Host` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKind {
    FrontDoor,
    Session(Seed),
    Unknown,
}

/// The public address of the server: scheme, domain and optional port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostConfig {
    scheme: Scheme,
    domain: String,
    port: Option<u16>,
    /// `:<port>` or empty.
    port_suffix: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DomainError(String);

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid domain {:?}: lowercase DNS labels of a-z, 0-9 and '-', no trailing dot",
            self.0
        )
    }
}

impl std::error::Error for DomainError {}

fn is_valid_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

impl HostConfig {
    /// Validates `domain`: lowercase DNS labels, at most 253 characters, no
    /// trailing dot.
    pub(crate) fn new(
        scheme: Scheme,
        domain: &str,
        port: Option<u16>,
    ) -> Result<Self, DomainError> {
        let valid = domain.len() <= 253 && domain.split('.').all(is_valid_label);
        if !valid {
            return Err(DomainError(domain.to_owned()));
        }
        Ok(Self {
            scheme,
            domain: domain.to_owned(),
            port,
            port_suffix: port.map(|port| format!(":{port}")).unwrap_or_default(),
        })
    }

    #[must_use]
    pub(crate) fn domain(&self) -> &str {
        &self.domain
    }

    #[must_use]
    pub(crate) fn port(&self) -> Option<u16> {
        self.port
    }

    /// Classifies a `Host` header value.
    #[must_use]
    pub(crate) fn classify(&self, host: &str) -> HostKind {
        let Some(name) = host.strip_suffix(self.port_suffix.as_str()) else {
            return HostKind::Unknown;
        };
        if name == self.domain {
            return HostKind::FrontDoor;
        }
        let seed = name
            .strip_suffix(self.domain.as_str())
            .and_then(|rest| rest.strip_suffix('.'))
            .and_then(|label| label.parse::<Seed>().ok());
        match seed {
            Some(seed) => HostKind::Session(seed),
            None => HostKind::Unknown,
        }
    }

    /// `scheme://<domain>[:port]`, the front door's origin.
    #[must_use]
    pub(crate) fn front_door_origin(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme.as_str(),
            self.domain,
            self.port_suffix
        )
    }

    /// `scheme://<seed>.<domain>[:port]`, a session host's origin.
    #[must_use]
    pub(crate) fn session_origin(&self, seed: &Seed) -> String {
        format!(
            "{}://{seed}.{}{}",
            self.scheme.as_str(),
            self.domain,
            self.port_suffix
        )
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        host::{HostConfig, HostKind, Scheme},
        ids::Seed,
    };

    const SEED: &str = "k3x7q2mfa7wd5brt";

    fn prod() -> HostConfig {
        HostConfig::new(Scheme::Https, "jam.example", None).unwrap()
    }

    fn local() -> HostConfig {
        HostConfig::new(Scheme::Http, "jam.localhost", Some(8080)).unwrap()
    }

    fn session() -> HostKind {
        HostKind::Session(SEED.parse().unwrap())
    }

    #[test]
    fn front_door_and_session_hosts() {
        let hosts = prod();
        assert_eq!(hosts.classify("jam.example"), HostKind::FrontDoor);
        assert_eq!(hosts.classify(&format!("{SEED}.jam.example")), session());
        assert_eq!(hosts.front_door_origin(), "https://jam.example");
        assert_eq!(
            hosts.session_origin(&SEED.parse::<Seed>().unwrap()),
            format!("https://{SEED}.jam.example")
        );
    }

    #[test]
    fn everything_else_is_unknown() {
        let hosts = prod();
        for bad in [
            "",
            "jam.example.",
            "JAM.example",
            "jam.example:443",
            "jam.example:",
            "example",
            "xjam.example",
            ".jam.example",
            "k3x7q2mfa7wd5brt.jam.example.",
            "K3X9Q2MFA7WD5BRT.jam.example",
            "k3x7q2mfa7wd5brt.JAM.EXAMPLE",
            "k3x7q2mfa7wd5br.jam.example",
            "k3x7q2mfa7wd5brtt.jam.example",
            "a.k3x7q2mfa7wd5brt.jam.example",
            "k3x7q2mfa7wd5brt.a.jam.example",
            "k3x7q2mfa7wd5brtjam.example",
            "k3x7q2mfa7wd5brt..jam.example",
            "k3x7q2mfa7wd5brt.jam.example:443",
            "k3x7q2mfa7wd5br1.jam.example",
            "user@jam.example",
            "[::1]",
            "127.0.0.1",
            "jam.example.evil",
        ] {
            assert_eq!(hosts.classify(bad), HostKind::Unknown, "{bad:?}");
        }
    }

    #[test]
    fn configured_port_must_match_exactly() {
        let hosts = local();
        assert_eq!(hosts.classify("jam.localhost:8080"), HostKind::FrontDoor);
        assert_eq!(
            hosts.classify(&format!("{SEED}.jam.localhost:8080")),
            session()
        );
        assert_eq!(hosts.front_door_origin(), "http://jam.localhost:8080");
        for bad in [
            "jam.localhost",
            "jam.localhost:80",
            "jam.localhost:08080",
            "jam.localhost:8080:8080",
            "jam.localhost.:8080",
            "jam.localhost:8080.",
            &format!("{SEED}.jam.localhost"),
            &format!("{SEED}.jam.localhost:8081"),
        ] {
            assert_eq!(hosts.classify(bad), HostKind::Unknown, "{bad:?}");
        }
    }

    #[test]
    fn domains_are_validated() {
        assert!(HostConfig::new(Scheme::Https, "localhost", None).is_ok());
        for bad in [
            "",
            "Jam.example",
            "jam.example.",
            ".jam",
            "jam..example",
            "-a.b",
            "a_b.c",
        ] {
            assert!(
                HostConfig::new(Scheme::Https, bad, None).is_err(),
                "{bad:?}"
            );
        }
    }

    fn well_formed(host: &str) -> Option<Seed> {
        let label = host.strip_suffix(".jam.example")?;
        let valid = label.len() == 16
            && label
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'));
        valid.then(|| label.parse().ok()).flatten()
    }

    proptest! {
        #[test]
        fn fuzzed_hosts_are_session_hosts_only_when_well_formed(
            host in "[a-zA-Z0-9.:\\-\\[\\]@ ]{0,40}(\\.jam\\.example)?(:[0-9]{1,5})?\\.?",
        ) {
            match prod().classify(&host) {
                HostKind::Session(seed) => prop_assert_eq!(Some(seed), well_formed(&host)),
                HostKind::FrontDoor => prop_assert_eq!(host.as_str(), "jam.example"),
                HostKind::Unknown => prop_assert!(well_formed(&host).is_none() && host != "jam.example"),
            }
        }

        #[test]
        fn mutated_session_hosts_are_classified_by_their_form(
            label in "[a-z2-7]{16}",
            position in 0usize..30,
            replacement in any::<char>(),
        ) {
            let host = format!("{label}.jam.example");
            let mutated: String = host
                .chars()
                .enumerate()
                .map(|(index, c)| if index == position { replacement } else { c })
                .collect();
            let expected = well_formed(&mutated).map_or(
                if mutated == "jam.example" { HostKind::FrontDoor } else { HostKind::Unknown },
                HostKind::Session,
            );
            prop_assert_eq!(prod().classify(&mutated), expected);
        }

        #[test]
        fn arbitrary_strings_never_panic_or_misclassify(host in "\\PC{0,64}") {
            if let HostKind::Session(seed) = prod().classify(&host) {
                prop_assert_eq!(host, format!("{seed}.jam.example"));
            }
        }
    }
}
