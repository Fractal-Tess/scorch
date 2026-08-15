mod obscura;

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    sync::{OnceCell, Semaphore},
    time::timeout,
};

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

#[derive(Clone, Debug)]
pub struct RenderedPage {
    pub html: Arc<str>,
    pub final_url: String,
    /// Status and headers of the main document, taken from the browser's own
    /// navigation record.
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub cache_ttl: Option<Duration>,
    pub cache_observed_at: Option<Instant>,
}

impl BrowserManager {
    pub async fn new(config: EngineConfig, security: SecurityPolicy) -> Result<Self> {
        let proxy = Arc::new(SafeProxy::start(security.clone()).await.map_err(|error| {
            EngineError::Browser(format!("failed to start safe proxy: {error}"))
        })?);
        let proxy_url = proxy.url().to_string();
        let manager = Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            obscura: ObscuraBackend::new(config, proxy_url),
            security,
            _proxy: proxy,
        };
        manager.warm_up().await;
        Ok(manager)
    }

    /// Render one throwaway page before serving traffic.
    ///
    /// The browser engine initialises process-wide state (V8 platform, snapshot,
    /// font and style caches) lazily on the first render. Two renders entering
    /// that cold path at once segfault the process, which is fatal for every
    /// in-flight request because the engine runs in-process. Doing one render
    /// here forces the initialisation to completion while nothing else can be
    /// running, so later concurrent renders only ever hit the warm path.
    ///
    /// The state is per process, not per manager, so this runs exactly once and
    /// any manager built while it is still running waits for it to finish.
    ///
    /// Failures are ignored on purpose: the goal is to run the lazy setup once,
    /// and a warm-up that cannot even load `about:blank` must not stop start-up.
    async fn warm_up(&self) {
        static WARMED: OnceCell<()> = OnceCell::const_new();
        WARMED.get_or_init(|| self.warm_up_once()).await;
    }

    async fn warm_up_once(&self) {
        let Ok(permit) = Arc::clone(&self.semaphore).acquire_owned().await else {
            return;
        };
        let started = Instant::now();
        let result = self
            .obscura
            .render(
                permit,
                "about:blank",
                Duration::from_secs(30),
                Duration::from_millis(1),
                true,
            )
            .await;
        match result {
            Ok(_) => tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "browser warm-up render finished"
            ),
            Err(error) => tracing::debug!(
                %error,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "browser warm-up render failed; continuing"
            ),
        }
    }

    pub fn available(&self) -> bool {
        true
    }

    pub async fn render(
        &self,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
    ) -> Result<RenderedPage> {
        let started = Instant::now();
        timeout(request_timeout, self.security.validate(url))
            .await
            .map_err(|_| EngineError::Timeout)??;
        let validation_elapsed = started.elapsed();
        let remaining = request_timeout.saturating_sub(validation_elapsed);
        if remaining.is_zero() {
            return Err(EngineError::Timeout);
        }
        let queue_started = Instant::now();
        let permit = timeout(remaining, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(|_| EngineError::Browser("browser semaphore closed".into()))?;
        let queue_elapsed = queue_started.elapsed();
        tracing::debug!(
            browser = "obscura",
            validation_ms = validation_elapsed.as_millis(),
            queue_ms = queue_elapsed.as_millis(),
            "browser admission phases completed"
        );
        let remaining = request_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(EngineError::Timeout);
        }
        self.obscura
            .render(permit, url, remaining, wait_for, block_media)
            .await
    }
}
