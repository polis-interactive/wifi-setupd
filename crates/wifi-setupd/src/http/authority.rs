use axum::http::{HeaderMap, uri::Authority};
use std::str::FromStr;

/// An explicitly trusted DNS hostname, without a scheme, port, or wildcard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hostname(String);

impl Hostname {
    /// Avahi's default host label comes from the system hostname's first label.
    pub fn from_system_hostname(value: &str) -> Result<Self, &'static str> {
        let hostname: Self = value.parse()?;
        format!("{}.local", hostname.0.split('.').next().unwrap()).parse()
    }
}

impl FromStr for Hostname {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value.strip_suffix('.').unwrap_or(value);
        if name.is_empty()
            || name.len() > 253
            || name.parse::<std::net::IpAddr>().is_ok()
            || !name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err("HTTP hostname must be a DNS name without a scheme, port, or wildcard");
        }
        Ok(Self(name.to_ascii_lowercase()))
    }
}

fn parse(value: &str) -> Option<(String, u16)> {
    if value.ends_with(':')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
    {
        return None;
    }
    let authority: Authority = value.parse().ok()?;
    // Authority::port() returns None for an out-of-range port too. Do not
    // mistake a malformed explicit port for the implicit HTTP port.
    let port = match value.rsplit_once(':') {
        Some((_, port)) if !value.ends_with(']') => port.parse::<u16>().ok()?,
        _ => 80,
    };
    let host = authority.host();
    Some((
        host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase(),
        port,
    ))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

pub(super) struct Authorities {
    primary: (String, u16),
    names: Vec<Hostname>,
}

impl Authorities {
    pub(super) fn new(primary: &str, names: Vec<Hostname>) -> std::io::Result<Self> {
        Ok(Self {
            primary: parse(primary).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid HTTP authority")
            })?,
            names,
        })
    }

    pub(super) fn local_host(&self, headers: &HeaderMap) -> bool {
        single_header(headers, "host")
            .and_then(parse)
            .is_some_and(|(host, port)| {
                port == self.primary.1
                    && (host == self.primary.0 || self.names.iter().any(|name| name.0 == host))
            })
    }

    pub(super) fn redirect_authority(&self) -> String {
        let host = self.names.first().map_or(&self.primary.0, |name| &name.0);
        if self.primary.1 == 80 {
            host.clone()
        } else {
            format!("{host}:{}", self.primary.1)
        }
    }

    pub(super) fn same_origin(&self, headers: &HeaderMap) -> bool {
        if !headers.contains_key("origin") {
            return true;
        }
        let host = single_header(headers, "host").and_then(parse);
        let origin = single_header(headers, "origin")
            .and_then(|origin| origin.strip_prefix("http://"))
            .and_then(parse);
        host.is_some() && origin == host
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn hostname_configuration_rejects_urls_ports_wildcards_and_invalid_labels() {
        for value in ["polis-dev.local", "POLIS-DEV.local.", "setup.example.org"] {
            assert!(value.parse::<Hostname>().is_ok(), "{value}");
        }
        for value in [
            "",
            ".",
            "*.local",
            "http://polis-dev.local",
            "polis-dev.local:80",
            "user@polis-dev.local",
            "polis-dev.local/path",
            "polis-dev..local",
            "-dev.local",
            "dev-.local",
            "dev_name.local",
            "192.168.1.125",
            "[::1]",
            "dévice.local",
            "polis-dev.local\n",
            "polis-dev.local..",
        ] {
            assert!(value.parse::<Hostname>().is_err(), "{value:?}");
        }
        assert!(
            format!("{}.local", "a".repeat(64))
                .parse::<Hostname>()
                .is_err()
        );
    }

    #[test]
    fn host_and_origin_require_the_listener_port_and_same_trusted_name() {
        for port in [80, 8080] {
            let rules = Authorities::new(
                &format!("192.168.1.125:{port}"),
                vec!["polis-dev.local".parse().unwrap()],
            )
            .unwrap();
            for host in [
                format!("POLIS-DEV.LOCAL.:{port}"),
                format!("192.168.1.125:{port}"),
            ] {
                let mut headers = HeaderMap::new();
                headers.insert("host", host.parse().unwrap());
                assert!(rules.local_host(&headers));
                assert!(rules.same_origin(&headers));
                headers.insert("origin", format!("http://{host}").parse().unwrap());
                assert!(rules.same_origin(&headers));
                for origin in [
                    "null",
                    "https://polis-dev.local",
                    "http://evil.local",
                    "http://user@polis-dev.local",
                    "http://polis-dev.local/",
                    "http://polis-dev.local:",
                    "http://polis-dev.local:65536",
                ] {
                    headers.insert("origin", origin.parse().unwrap());
                    assert!(!rules.same_origin(&headers), "{origin}");
                }
                headers.insert("origin", HeaderValue::from_bytes(b"\xff").unwrap());
                assert!(!rules.same_origin(&headers));
                headers.insert("origin", format!("http://{host}").parse().unwrap());
                headers.append("origin", format!("http://{host}").parse().unwrap());
                assert!(!rules.same_origin(&headers));
                headers.append("host", host.parse().unwrap());
                assert!(!rules.local_host(&headers));
            }
            for host in [
                "evil.local",
                "polis-dev.local.evil",
                "polis-dev.local:1234",
                "user@polis-dev.local",
            ] {
                let mut headers = HeaderMap::new();
                headers.insert("host", host.parse().unwrap());
                assert!(!rules.local_host(&headers), "{host}");
            }
            let mut headers = HeaderMap::new();
            headers.insert("host", "polis-dev.local".parse().unwrap());
            assert_eq!(rules.local_host(&headers), port == 80);
            headers.insert("origin", "http://polis-dev.local:80".parse().unwrap());
            assert!(rules.same_origin(&headers));
            headers.insert("host", format!("192.168.1.125:{port}").parse().unwrap());
            headers.insert(
                "origin",
                format!("http://polis-dev.local:{port}").parse().unwrap(),
            );
            assert!(
                !rules.same_origin(&headers),
                "different allowed names are still different origins"
            );
        }
    }
}
