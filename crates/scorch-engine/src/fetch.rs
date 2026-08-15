use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{
    StatusCode,
    dns::{Addrs, Name, Resolve, Resolving},
    header::LOCATION,
    redirect::Policy,
};
use tokio::sync::Semaphore;

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
    security::SecurityPolicy,
};

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub final_url: String,
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// Resolves hostnames through [`SecurityPolicy`] so the connector can only ever
/// reach an address the policy accepts. Building the client once and pinning at
/// connect time (instead of rebuilding a client per request with a static
/// address override) keeps the TLS session and connection pool alive across
/// requests, which is most of the cost of a fetch to an already-seen host.
struct PinnedResolver {
    security: SecurityPolicy,
}

impl Resolve for PinnedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let security = self.security.clone();
        Box::pin(async move {
            let addresses = security.resolve_public_ips(name.as_str()).await?;
            // Port zero is a placeholder: the connector overwrites it with the
            // destination port before connecting.
            let addresses: Addrs = Box::new(
                addresses
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addresses)
        })
    }
}

#[derive(Debug, Clone)]
pub struct SafeFetcher {
    security: SecurityPolicy,
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
    config: EngineConfig,
}

impl SafeFetcher {
    pub fn new(security: SecurityPolicy, config: EngineConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .redirect(Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PinnedResolver {
                security: security.clone(),
            }))
            .build()
            .expect("the fetch client has a static configuration");
        Self {
            security,
            client,
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            config,
        }
    }

    pub async fn get(&self, input: &str, timeout: Duration) -> Result<FetchResponse> {
        self.get_with_user_agent(input, timeout, obscura_browser::STEALTH_USER_AGENT)
            .await
    }

    pub async fn get_with_user_agent(
        &self,
        input: &str,
        timeout: Duration,
        user_agent: &str,
    ) -> Result<FetchResponse> {
        let mut current = input.to_owned();
        let deadline = Instant::now() + timeout;
        let _permit = tokio::time::timeout(timeout, self.semaphore.acquire())
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|_| EngineError::Capacity("fetch runtime is shutting down".into()))?;

        for redirect_count in 0..=MAX_REDIRECTS {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(EngineError::Timeout);
            }
            let target = tokio::time::timeout(remaining, self.security.validate(&current))
                .await
                .map_err(|_| EngineError::Timeout)??;
            let response = self
                .client
                .get(target.url.clone())
                .timeout(remaining)
                .header(reqwest::header::USER_AGENT, user_agent)
                .header(
                    "accept",
                    "text/html,application/xhtml+xml,text/plain,application/xml;q=0.9,*/*;q=0.1",
                )
                .header("accept-encoding", "identity")
                .send()
                .await
                .map_err(map_reqwest_error)?;

            if response.status().is_redirection() {
                if redirect_count == MAX_REDIRECTS {
                    return Err(EngineError::Fetch("too many redirects".into()));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .ok_or_else(|| EngineError::Fetch("redirect had no location".into()))?
                    .to_str()
                    .map_err(|_| {
                        EngineError::Fetch("redirect location is not valid text".into())
                    })?;
                current = target
                    .url
                    .join(location)
                    .map_err(|error| EngineError::Fetch(format!("invalid redirect: {error}")))?
                    .to_string();
                continue;
            }

            let status = response.status();
            let final_url = response.url().to_string();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned);
            let headers = selected_headers(response.headers());
            let body = read_limited(response, self.config.max_response_bytes).await?;
            return Ok(FetchResponse {
                final_url,
                status,
                content_type,
                headers,
                body,
            });
        }

        unreachable!("redirect loop always returns")
    }
}

/// The only response headers a caller ever sees. Everything else is withheld,
/// which keeps `set-cookie` and other credential-bearing headers out of scrape
/// responses. The browser path reports the same set from its own navigation.
pub(crate) const REPORTED_HEADERS: [&str; 4] =
    ["content-type", "content-language", "last-modified", "etag"];

fn selected_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    REPORTED_HEADERS
        .into_iter()
        .filter_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

pub(crate) fn response_cache_ttl(
    status: u16,
    redirected: bool,
    has_set_cookie: bool,
    cache_control: Option<&str>,
    age: Option<&str>,
    expires: Option<&str>,
    vary: Option<&str>,
) -> Option<Duration> {
    if !(200..300).contains(&status)
        || redirected
        || has_set_cookie
        || vary.is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|name| !name.eq_ignore_ascii_case("accept-encoding"))
        })
        || cache_control.is_some_and(cache_control_forbids_reuse)
    {
        return None;
    }

    let max_age = cache_control.and_then(cache_control_max_age);
    let freshness = if let Some(seconds) = max_age {
        let age = age.and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
        Duration::from_secs(seconds.saturating_sub(age))
    } else if let Some(expires) = expires {
        httpdate::parse_http_date(expires)
            .ok()?
            .duration_since(std::time::SystemTime::now())
            .ok()?
    } else {
        crate::cache::CACHE_TTL
    };
    (!freshness.is_zero()).then(|| freshness.min(crate::cache::CACHE_TTL))
}

pub(crate) fn cache_control_forbids_reuse(value: &str) -> bool {
    value
        .split(',')
        .map(cache_directive)
        .any(|(name, argument)| {
            name.eq_ignore_ascii_case("no-store")
                || name.eq_ignore_ascii_case("no-cache")
                || name.eq_ignore_ascii_case("private")
                || ((name.eq_ignore_ascii_case("max-age") || name.eq_ignore_ascii_case("s-maxage"))
                    && argument.and_then(|value| value.parse::<u64>().ok()) == Some(0))
        })
}

pub(crate) fn cache_control_is_private(value: &str) -> bool {
    value.split(',').map(cache_directive).any(|(name, _)| {
        name.eq_ignore_ascii_case("no-store") || name.eq_ignore_ascii_case("private")
    })
}

fn cache_control_max_age(value: &str) -> Option<u64> {
    let directives = value.split(',').map(cache_directive).collect::<Vec<_>>();
    directives
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("s-maxage"))
        .or_else(|| {
            directives
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("max-age"))
        })
        .and_then(|(_, argument)| argument.as_ref()?.parse().ok())
}

fn cache_directive(directive: &str) -> (&str, Option<&str>) {
    directive
        .split_once('=')
        .map_or((directive.trim(), None), |(name, argument)| {
            (name.trim(), Some(argument.trim().trim_matches('"')))
        })
}

async fn read_limited(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(EngineError::ResponseTooLarge(limit));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_reqwest_error)?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(EngineError::ResponseTooLarge(limit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn map_reqwest_error(error: reqwest::Error) -> EngineError {
    if error.is_timeout() {
        EngineError::Timeout
    } else {
        EngineError::Fetch(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_cache_respects_freshness_and_privacy_directives() {
        let ttl = |status, redirected, has_set_cookie, cache_control, age, vary| {
            response_cache_ttl(
                status,
                redirected,
                has_set_cookie,
                cache_control,
                age,
                None,
                vary,
            )
        };
        assert_eq!(
            ttl(200, false, false, None, None, None),
            Some(crate::cache::CACHE_TTL)
        );
        assert_eq!(
            ttl(
                200,
                false,
                false,
                Some("public, max-age=60"),
                Some("50"),
                None
            ),
            Some(Duration::from_secs(10))
        );
        assert!(ttl(200, true, false, Some("max-age=60"), None, None).is_none());
        assert!(ttl(200, false, false, Some("private, max-age=60"), None, None).is_none());
        assert!(ttl(200, false, false, Some("no-store"), None, None).is_none());
        assert!(ttl(200, false, false, Some("no-cache"), None, None).is_none());
        assert!(ttl(200, false, false, Some("max-age=0"), None, None).is_none());
        assert!(ttl(200, false, true, None, None, None).is_none());
        assert!(ttl(200, false, false, None, None, Some("referer")).is_none());
        assert!(ttl(403, false, false, None, None, None).is_none());
    }
}
