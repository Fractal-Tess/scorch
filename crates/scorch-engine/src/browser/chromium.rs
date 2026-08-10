use std::{env, ops::Deref, path::Path, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chromiumoxide::{
    Page,
    browser::{Browser, BrowserConfig},
    page::ScreenshotParams,
};
use futures_util::StreamExt;
use tokio::{
    sync::Mutex,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tracing::{error, info, warn};

use crate::{
    browser::RenderedPage,
    config::EngineConfig,
    error::{EngineError, Result},
};

struct BrowserRuntime {
    browser: Browser,
    handler: JoinHandle<()>,
    _profile: tempfile::TempDir,
}

pub(super) struct ChromiumBackend {
    config: EngineConfig,
    runtime: Mutex<Option<BrowserRuntime>>,
    proxy_url: String,
}

impl ChromiumBackend {
    pub fn new(config: EngineConfig, proxy_url: String) -> Self {
        Self {
            config,
            runtime: Mutex::new(None),
            proxy_url,
        }
    }

    pub fn available(&self) -> bool {
        executable_exists(&self.config.browser_path)
    }

    pub async fn render(
        &self,
        url: &str,
        request_timeout: Duration,
        wait_for: Duration,
        block_media: bool,
        screenshot: bool,
        full_page: bool,
    ) -> Result<RenderedPage> {
        let page = PageGuard::new(self.new_page().await?);

        if block_media {
            block_media_resources(&page).await?;
        }

        let operation = async {
            page.goto(url)
                .await
                .map_err(|error| EngineError::Browser(error.to_string()))?;
            if !wait_for.is_zero() {
                sleep(wait_for).await;
            }
            let final_url = page
                .url()
                .await
                .map_err(|error| EngineError::Browser(error.to_string()))?
                .unwrap_or_else(|| url.to_owned());
            let html = page
                .content()
                .await
                .map_err(|error| EngineError::Browser(error.to_string()))?;
            if html.len() > self.config.max_response_bytes {
                return Err(EngineError::ResponseTooLarge(
                    self.config.max_response_bytes,
                ));
            }
            let screenshot = if screenshot {
                let params = ScreenshotParams::builder().full_page(full_page).build();
                let bytes = page
                    .screenshot(params)
                    .await
                    .map_err(|error| EngineError::Browser(error.to_string()))?;
                if bytes.len() > self.config.max_response_bytes {
                    return Err(EngineError::ResponseTooLarge(
                        self.config.max_response_bytes,
                    ));
                }
                Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
            } else {
                None
            };
            Ok(RenderedPage {
                html,
                final_url,
                screenshot,
            })
        };

        let result = timeout(request_timeout, operation)
            .await
            .map_err(|_| EngineError::Timeout)?;
        page.finish().await;
        result
    }

    async fn new_page(&self) -> Result<Page> {
        let mut runtime = self.runtime.lock().await;
        if runtime
            .as_ref()
            .is_some_and(|runtime| runtime.handler.is_finished())
        {
            warn!("recycling stopped Chromium runtime");
            runtime.take();
        }
        if runtime.is_none() {
            *runtime = Some(self.launch().await?);
        }
        runtime
            .as_mut()
            .expect("browser initialized")
            .browser
            .new_page("about:blank")
            .await
            .map_err(|error| EngineError::Browser(error.to_string()))
    }

    async fn launch(&self) -> Result<BrowserRuntime> {
        let browser_path = resolve_executable(&self.config.browser_path).ok_or_else(|| {
            EngineError::Browser(format!(
                "executable {} was not found",
                self.config.browser_path.display()
            ))
        })?;
        let profile = tempfile::Builder::new()
            .prefix("scorch-chromium-")
            .tempdir()
            .map_err(|error| {
                EngineError::Browser(format!("failed to create browser profile: {error}"))
            })?;
        info!(
            browser_path = %browser_path.display(),
            proxy = %self.proxy_url,
            "launching Chromium"
        );
        let config = BrowserConfig::builder()
            .chrome_executable(browser_path)
            .user_data_dir(profile.path())
            .new_headless_mode()
            .incognito()
            .disable_cache()
            .respect_https_errors()
            .request_timeout(self.config.request_timeout)
            .args([
                "--disable-background-networking".to_owned(),
                "--disable-component-update".to_owned(),
                "--disable-default-apps".to_owned(),
                "--disable-dev-shm-usage".to_owned(),
                "--disable-features=Translate,MediaRouter,OptimizationHints,AutofillServerCommunication"
                    .to_owned(),
                "--disable-gpu".to_owned(),
                "--disable-quic".to_owned(),
                "--disable-sync".to_owned(),
                "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_owned(),
                "--metrics-recording-only".to_owned(),
                "--no-default-browser-check".to_owned(),
                "--no-first-run".to_owned(),
                "--proxy-bypass-list=<-loopback>".to_owned(),
                format!("--proxy-server={}", self.proxy_url),
            ])
            .build()
            .map_err(EngineError::Browser)?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|error| EngineError::Browser(error.to_string()))?;
        let handler = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(error) = event {
                    error!(%error, "browser handler stopped");
                    break;
                }
            }
            warn!("Chromium event stream ended");
        });
        info!("Chromium is ready");
        Ok(BrowserRuntime {
            browser,
            handler,
            _profile: profile,
        })
    }
}

impl Drop for BrowserRuntime {
    fn drop(&mut self) {
        self.handler.abort();
    }
}

struct PageGuard(Option<Page>);

impl PageGuard {
    fn new(page: Page) -> Self {
        Self(Some(page))
    }

    async fn finish(mut self) {
        if let Some(page) = self.0.take()
            && let Err(error) = page.close().await
        {
            warn!(%error, "failed to close browser page");
        }
    }
}

impl Deref for PageGuard {
    type Target = Page;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("page guard is active")
    }
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        if let Some(page) = self.0.take() {
            tokio::spawn(async move {
                if let Err(error) = page.close().await {
                    warn!(%error, "failed to close cancelled browser page");
                }
            });
        }
    }
}

async fn block_media_resources(page: &Page) -> Result<()> {
    use chromiumoxide::cdp::browser_protocol::network::{BlockPattern, SetBlockedUrLsParams};
    let patterns = [
        "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg", "*.mp4", "*.webm", "*.mp3",
        "*.woff", "*.woff2",
    ]
    .map(|suffix| BlockPattern::new(format!("*://*:*/*{suffix}"), true));
    page.execute(
        SetBlockedUrLsParams::builder()
            .url_patterns(patterns)
            .build(),
    )
    .await
    .map_err(|error| EngineError::Browser(error.to_string()))?;
    Ok(())
}

fn executable_exists(path: &Path) -> bool {
    resolve_executable(path).is_some()
}

fn resolve_executable(path: &Path) -> Option<std::path::PathBuf> {
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_owned());
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|directory| directory.join(path))
            .find(|candidate| candidate.is_file())
    })
}
