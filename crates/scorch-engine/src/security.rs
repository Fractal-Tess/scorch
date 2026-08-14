use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use tokio::{net::lookup_host, time::timeout};
use url::{Host, Url};

use crate::error::{EngineError, Result};

/// How long a successful resolution stays usable. Kept short so that DNS
/// changes take effect quickly while still collapsing the burst of lookups a
/// single page load triggers (one per proxied subresource connection).
const DNS_CACHE_TTL: Duration = Duration::from_secs(60);
/// Upper bound on cached hosts, so a crawl over many domains cannot grow the
/// map without limit.
const DNS_CACHE_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
struct CachedAddresses {
    addresses: Vec<SocketAddr>,
    expires_at: Instant,
}

type HostKey = (String, u16);

#[derive(Debug, Clone, Default)]
pub struct SecurityPolicy {
    dns_cache: Arc<DashMap<HostKey, CachedAddresses>>,
    /// Per-host locks so that a burst of concurrent requests for the same host
    /// performs one lookup instead of one per caller.
    resolving: Arc<DashMap<HostKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl SecurityPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    fn cached_addresses(&self, host: &str, port: u16) -> Option<Vec<SocketAddr>> {
        let entry = self.dns_cache.get(&(host.to_owned(), port))?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        Some(entry.addresses.clone())
    }

    fn resolve_gate(&self, key: &HostKey) -> Arc<tokio::sync::Mutex<()>> {
        if let Some(existing) = self.resolving.get(key) {
            return Arc::clone(existing.value());
        }
        Arc::clone(
            self.resolving
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .value(),
        )
    }

    fn release_gate(&self, key: &HostKey) {
        // Drop the entry only when this caller holds the last reference, so a
        // waiter that already cloned the Arc still shares the same lock.
        self.resolving
            .remove_if(key, |_, gate| Arc::strong_count(gate) <= 2);
    }

    fn store_addresses(&self, host: &str, port: u16, addresses: &[SocketAddr]) {
        if self.dns_cache.len() >= DNS_CACHE_CAPACITY {
            let now = Instant::now();
            self.dns_cache.retain(|_, entry| entry.expires_at > now);
            if self.dns_cache.len() >= DNS_CACHE_CAPACITY {
                return;
            }
        }
        self.dns_cache.insert(
            (host.to_owned(), port),
            CachedAddresses {
                addresses: addresses.to_vec(),
                expires_at: Instant::now() + DNS_CACHE_TTL,
            },
        );
    }
}

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
        let (host, literal_ip) = normalize_url_host(&mut url)?;
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
        let mut addresses = if let Some(ip) = literal_ip {
            vec![SocketAddr::new(ip, port)]
        } else if let Some(cached) = self.cached_addresses(&host, port) {
            cached
        } else {
            let key = (host.clone(), port);
            let gate = self.resolve_gate(&key);
            let resolved = {
                let _guard = gate.lock().await;
                // Another caller may have resolved this host while we waited.
                match self.cached_addresses(&host, port) {
                    Some(cached) => cached,
                    None => {
                        let dns_started = Instant::now();
                        let resolved =
                            timeout(Duration::from_secs(5), lookup_host((host.as_str(), port)))
                                .await;
                        let mut resolved: Vec<SocketAddr> = resolved
                            .map_err(|_| EngineError::Dns("resolution timed out".into()))?
                            .map_err(|error| EngineError::Dns(error.to_string()))?
                            .take(16)
                            .collect();
                        resolved.sort_unstable();
                        resolved.dedup();
                        tracing::debug!(
                            dns_ms = dns_started.elapsed().as_millis() as u64,
                            %host,
                            addresses = resolved.len(),
                            "resolved host"
                        );
                        // Only successful lookups are cached, and the
                        // public-address check below still runs on every hit.
                        self.store_addresses(&host, port, &resolved);
                        resolved
                    }
                }
            };
            // Runs while `gate` is still held locally, so the strong count
            // reveals whether any other caller is still waiting on this host.
            self.release_gate(&key);
            resolved
        };
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

fn normalize_url_host(url: &mut Url) -> Result<(String, Option<IpAddr>)> {
    match url
        .host()
        .ok_or_else(|| EngineError::InvalidRequest("URL has no host".into()))?
    {
        Host::Domain(domain) => {
            let host = domain.trim_end_matches('.').to_ascii_lowercase();
            url.set_host(Some(&host))
                .map_err(|_| EngineError::InvalidRequest("URL has an invalid host".into()))?;
            Ok((host, None))
        }
        Host::Ipv4(address) => {
            let address = IpAddr::V4(address);
            url.set_ip_host(address)
                .map_err(|_| EngineError::InvalidRequest("URL has an invalid host".into()))?;
            Ok((address.to_string(), Some(address)))
        }
        Host::Ipv6(address) => {
            let address = IpAddr::V6(address);
            url.set_ip_host(address)
                .map_err(|_| EngineError::InvalidRequest("URL has an invalid host".into()))?;
            Ok((address.to_string(), Some(address)))
        }
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
                || segments[..6] == [0x0064, 0xff9b, 0, 0, 0, 0]
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
        let error = SecurityPolicy::new()
            .validate("http://127.0.0.1:8080/private")
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::UnsafeUrl(_)));
    }

    #[tokio::test]
    async fn rejects_credentials() {
        let error = SecurityPolicy::new()
            .validate("https://user:pass@example.com")
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::UnsafeUrl(_)));
    }

    #[test]
    fn canonicalizes_trailing_dot_hosts() {
        let mut url = Url::parse("https://EXAMPLE.com./path").unwrap();
        let (host, ip) = normalize_url_host(&mut url).unwrap();
        assert_eq!(host, "example.com");
        assert!(ip.is_none());
        assert_eq!(url.as_str(), "https://example.com/path");

        let mut ipv6 = Url::parse("https://[2606:4700:4700::1111]/").unwrap();
        let (host, ip) = normalize_url_host(&mut ipv6).unwrap();
        assert_eq!(host, "2606:4700:4700::1111");
        assert_eq!(ip, Some("2606:4700:4700::1111".parse().unwrap()));
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
            "64:ff9b::7f00:1",
            "64:ff9b:1::7f00:1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn caches_resolved_addresses_until_they_expire() {
        let policy = SecurityPolicy::new();
        let address: SocketAddr = "93.184.216.34:443".parse().unwrap();
        policy.store_addresses("example.com", 443, &[address]);

        assert_eq!(
            policy.cached_addresses("example.com", 443),
            Some(vec![address])
        );
        // The port is part of the key, so a different port must miss.
        assert_eq!(policy.cached_addresses("example.com", 80), None);
        assert_eq!(policy.cached_addresses("other.example", 443), None);

        policy
            .dns_cache
            .get_mut(&("example.com".into(), 443))
            .unwrap()
            .expires_at = Instant::now() - Duration::from_secs(1);
        assert_eq!(policy.cached_addresses("example.com", 443), None);
    }

    #[test]
    fn dns_cache_stays_bounded() {
        let policy = SecurityPolicy::new();
        let address: SocketAddr = "93.184.216.34:443".parse().unwrap();
        for index in 0..(DNS_CACHE_CAPACITY + 50) {
            policy.store_addresses(&format!("host{index}.example"), 443, &[address]);
        }
        assert!(policy.dns_cache.len() <= DNS_CACHE_CAPACITY);
    }

    #[tokio::test]
    async fn concurrent_resolutions_share_one_gate() {
        let policy = SecurityPolicy::new();
        let key = ("example.com".to_owned(), 443);
        let first = policy.resolve_gate(&key);
        let second = policy.resolve_gate(&key);
        assert!(Arc::ptr_eq(&first, &second));

        // While another caller still holds a reference the gate is retained.
        policy.release_gate(&key);
        assert!(policy.resolving.contains_key(&key));

        drop(second);
        policy.release_gate(&key);
        assert!(!policy.resolving.contains_key(&key));
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
