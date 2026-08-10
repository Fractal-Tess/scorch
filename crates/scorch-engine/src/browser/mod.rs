mod obscura;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{sync::Semaphore, time::timeout};

use crate::{
    config::EngineConfig,
    error::{EngineError, Result},
    proxy::SafeProxy,
    security::SecurityPolicy,
};

use self::obscura::ObscuraBackend;

pub struct BrowserManager {
    semaphore: Arc<Semaphore>,
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
        let proxy = Arc::new(SafeProxy::start(security.clone()).await.map_err(|error| {
            EngineError::Browser(format!("failed to start safe proxy: {error}"))
        })?);
        let proxy_url = proxy.url().to_string();
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            obscura: ObscuraBackend::new(config, proxy_url),
            security,
            _proxy: proxy,
        })
    }

    pub fn available(&self) -> bool {
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn render(
        &self,
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
}

#[cfg(test)]
mod tests {
    use crate::EngineConfig;

    #[test]
    fn obscura_stealth_is_enabled_by_default() {
        assert!(EngineConfig::default().obscura_stealth);
    }
}
