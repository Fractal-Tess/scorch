mod chromium;
mod obscura;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use scorch_types::BrowserBackend;
use tokio::{sync::Semaphore, time::timeout};

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
    proxy::SafeProxy,
    security::SecurityPolicy,
};

use self::{chromium::ChromiumBackend, obscura::ObscuraBackend};

pub struct BrowserManager {
    config: EngineConfig,
    semaphore: Arc<Semaphore>,
    chromium: ChromiumBackend,
    obscura: ObscuraBackend,
    security: SecurityPolicy,
    _proxy: Arc<SafeProxy>,
}

#[derive(Debug)]
pub struct RenderedPage {
    pub html: String,
    pub final_url: String,
    pub screenshot: Option<String>,
}

impl BrowserManager {
    pub async fn new(config: EngineConfig, security: SecurityPolicy) -> Result<Self> {
        validate_policy(&config)?;
        let proxy = Arc::new(SafeProxy::start(security.clone()).await.map_err(|error| {
            EngineError::Browser(format!("failed to start safe proxy: {error}"))
        })?);
        let proxy_url = proxy.url().to_string();
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            chromium: ChromiumBackend::new(config.clone(), proxy_url.clone()),
            obscura: ObscuraBackend::new(config.clone(), proxy_url),
            config,
            security,
            _proxy: proxy,
        })
    }

    pub fn available(&self) -> bool {
        self.backend_available(self.config.browser)
    }

    pub fn backend_available(&self, browser: BrowserBackend) -> bool {
        match browser {
            BrowserBackend::Obscura => true,
            BrowserBackend::Chromium => self.chromium.available(),
        }
    }

    pub fn resolve(&self, requested: Option<BrowserBackend>) -> Result<BrowserBackend> {
        let browser = requested.unwrap_or(self.config.browser);
        if !self.config.allowed_browsers.contains(&browser) {
            return Err(EngineError::InvalidRequest(format!(
                "browser {} is forbidden by server policy",
                browser.as_str()
            )));
        }
        if !self.backend_available(browser) {
            return Err(EngineError::Browser(format!(
                "browser {} is unavailable",
                browser.as_str()
            )));
        }
        Ok(browser)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn render(
        &self,
        browser: BrowserBackend,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
        screenshot: bool,
        full_page: bool,
    ) -> Result<RenderedPage> {
        let started = Instant::now();
        timeout(request_timeout, self.security.validate(url))
            .await
            .map_err(|_| EngineError::Timeout)??;
        let remaining = request_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(EngineError::Timeout);
        }
        let permit = timeout(remaining, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|_| EngineError::Browser("browser semaphore closed".into()))?;
        let remaining = request_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(EngineError::Timeout);
        }
        match browser {
            BrowserBackend::Obscura => {
                self.obscura
                    .render(
                        permit,
                        url,
                        remaining,
                        wait_for,
                        block_media,
                        screenshot,
                        full_page,
                    )
                    .await
            }
            BrowserBackend::Chromium => {
                let result = self
                    .chromium
                    .render(url, remaining, wait_for, block_media, screenshot, full_page)
                    .await;
                drop(permit);
                result
            }
        }
    }
}

fn validate_policy(config: &EngineConfig) -> Result<()> {
    if config.allowed_browsers.is_empty() {
        return Err(EngineError::InvalidRequest(
            "at least one browser must be allowed".into(),
        ));
    }
    if !config.allowed_browsers.contains(&config.browser) {
        return Err(EngineError::InvalidRequest(format!(
            "default browser {} is not in the allowed browser list",
            config.browser.as_str()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use scorch_types::BrowserBackend;

    use super::validate_policy;
    use crate::EngineConfig;

    #[test]
    fn policy_requires_the_default_browser_to_be_allowed() {
        let config = EngineConfig {
            browser: BrowserBackend::Chromium,
            allowed_browsers: vec![BrowserBackend::Obscura],
            ..Default::default()
        };
        assert!(validate_policy(&config).is_err());
    }

    #[test]
    fn default_policy_enables_only_obscura() {
        let config = EngineConfig::default();
        assert_eq!(config.browser, BrowserBackend::Obscura);
        assert_eq!(config.allowed_browsers, vec![BrowserBackend::Obscura]);
        assert!(config.obscura_stealth);
        assert!(validate_policy(&config).is_ok());
    }
}
