use std::net::{IpAddr, SocketAddr};

use tokio::{net::lookup_host, time::timeout};
use url::Url;

use crate::error::{EngineError, Result};

#[derive(Debug, Clone, Default)]
pub struct SecurityPolicy;

#[derive(Debug, Clone)]
pub struct ValidatedTarget {
    pub url: Url,
    pub host: String,
    pub addresses: Vec<SocketAddr>,
}

impl SecurityPolicy {
    pub async fn validate(&self, input: &str) -> Result<ValidatedTarget> {
        if input.len() > 8 * 1024 {
            return Err(EngineError::InvalidRequest(
                "URL exceeds the 8192 byte limit".into(),
            ));
        }
        let mut url = Url::parse(input)
            .map_err(|error| EngineError::InvalidRequest(format!("invalid URL: {error}")))?;

        if !matches!(url.scheme(), "http" | "https") {
            return Err(EngineError::UnsafeUrl(
                "only HTTP and HTTPS URLs are supported".into(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(EngineError::UnsafeUrl(
                "credentials in target URLs are not allowed".into(),
            ));
        }

        url.set_fragment(None);
        let host = url
            .host_str()
            .ok_or_else(|| EngineError::InvalidRequest("URL has no host".into()))?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
            return Err(EngineError::UnsafeUrl(format!(
                "host {host} is local or private"
            )));
        }

        let port = url
            .port_or_known_default()
            .ok_or_else(|| EngineError::InvalidRequest("URL has no usable port".into()))?;
        if is_restricted_port(port) {
            return Err(EngineError::UnsafeUrl(format!(
                "port {port} is not allowed for web requests"
            )));
        }
        let mut addresses: Vec<_> = timeout(
            std::time::Duration::from_secs(5),
            lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| EngineError::Dns("resolution timed out".into()))?
        .map_err(|error| EngineError::Dns(error.to_string()))?
        .take(16)
        .collect();
        addresses.sort_unstable();
        addresses.dedup();

        if addresses.is_empty() {
            return Err(EngineError::Dns(format!("{host} resolved to no addresses")));
        }
        if let Some(address) = addresses.iter().find(|address| !is_public_ip(address.ip())) {
            return Err(EngineError::UnsafeUrl(format!(
                "{host} resolves to non-public address {}",
                address.ip()
            )));
        }

        Ok(ValidatedTarget {
            url,
            host,
            addresses,
        })
    }
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (18..=19).contains(&b)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let first = segments[0];
            let documentation =
                (first == 0x2001 && segments[1] == 0x0db8) || first & 0xfff0 == 0x3ff0;
            let protocol_assignments = first == 0x2001 && segments[1] < 0x0200;
            let discarded = (first == 0x0100 && segments[1..4] == [0, 0, 0])
                || (first == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
                || first == 0x2002
                || first == 0x5f00;
            let unique_local = first & 0xfe00 == 0xfc00;
            let link_or_site_local = first & 0xffc0 == 0xfe80 || first & 0xffc0 == 0xfec0;
            let multicast = first & 0xff00 == 0xff00;
            let ipv4_embedded =
                segments[..6] == [0, 0, 0, 0, 0, 0] || segments[..6] == [0, 0, 0, 0, 0, 0xffff];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || documentation
                || protocol_assignments
                || discarded
                || unique_local
                || link_or_site_local
                || multicast
                || ipv4_embedded)
        }
    }
}

fn is_restricted_port(port: u16) -> bool {
    matches!(
        port,
        1 | 7 | 9 | 11 | 13 | 15 | 17 | 19 | 20 | 21 | 22 | 23 | 25 | 37 | 42 | 43
            | 53 | 69 | 77 | 79 | 87 | 95 | 101 | 102 | 103 | 104 | 109 | 110 | 111 | 113
            | 115 | 117 | 119 | 123 | 135 | 137 | 139 | 143 | 161 | 179 | 389 | 427 | 465
            | 512..=515 | 526 | 530..=532 | 540 | 548 | 554 | 556 | 563 | 587 | 601 | 636
            | 989 | 990 | 993 | 995 | 1719 | 1720 | 1723 | 2049 | 3659 | 4045 | 5060
            | 5061 | 6000 | 6379 | 6665..=6669 | 6697 | 10080
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_loopback() {
        let error = SecurityPolicy
            .validate("http://127.0.0.1:8080/private")
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::UnsafeUrl(_)));
    }

    #[tokio::test]
    async fn rejects_credentials() {
        let error = SecurityPolicy
            .validate("https://user:pass@example.com")
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::UnsafeUrl(_)));
    }

    #[test]
    fn rejects_special_purpose_addresses() {
        for address in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn rejects_non_web_ports() {
        assert!(is_restricted_port(22));
        assert!(is_restricted_port(25));
        assert!(is_restricted_port(6379));
        assert!(!is_restricted_port(80));
        assert!(!is_restricted_port(443));
        assert!(!is_restricted_port(33000));
    }
}
