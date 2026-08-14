//! Does reusing the browser's connection pool across renders remove the
//! handshake every render currently pays?
//!
//! Scorch builds a fresh `BrowserContext` and `Page` per render, so each render
//! opens a new connection to the origin. Measured against the pooled fetch path
//! this costs 132 ms on a nearby scriptless page and 633 ms on a distant one,
//! which is the whole gap between the two paths when no scripts are involved.
//!
//! Reusing the context alone cannot help: `Page::new` builds its own
//! `StealthHttpClient`, and with stealth on that client fetches everything. So
//! this measures three navigations to the same URL:
//!
//!   cold    fresh context and page, as scorch does today
//!   page    fresh page on the same context, so only the plain client is shared
//!   pooled  fresh page that also inherits the first page's stealth client
//!
//! Measured results, direct rather than through scorch's proxy, so absolute
//! numbers are lower than production but the comparison holds.
//!
//! Sharing the stealth client *within a single runtime* is worth a lot:
//! example.com 63 -> 28 ms, httpbin.org/html 525 -> 140 ms, books.toscrape.com
//! 860 -> 580 ms. Sharing only the context is worth nothing, because
//! `Page::new` mints its own stealth client and that is the client stealth
//! pages fetch over.
//!
//! Sharing it *across* runtimes, which is what scorch would be doing today
//! since it builds a runtime per render, does not work: the pool is driven by
//! the runtime that opened its connections, so once that runtime is dropped the
//! reused client hands out dead connections. This probe measured a 22.8 s
//! navigation and an outright failure on httpbin that way. Reusing connections
//! therefore requires a runtime that outlives individual renders, not just a
//! shared client.
//!
//! Run with: cargo run --release -p scorch-engine --example pool_probe -- <url>

use std::{sync::Arc, time::Instant};

use obscura_browser::{BrowserContext, Page};

const BLOCKED: [&str; 13] = [
    "*.css", "*.css?*", "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg", "*.mp4", "*.webm",
    "*.mp3", "*.woff", "*.woff2",
];

fn new_page(context: &Arc<BrowserContext>, name: &str) -> Page {
    let mut page = Page::new(format!("probe-{name}"), Arc::clone(context));
    page.set_viewport((1280.0, 720.0));
    page.set_navigation_timeout(std::time::Duration::from_secs(30));
    page.set_blocked_urls(BLOCKED.iter().map(|p| (*p).to_owned()).collect());
    page
}

async fn navigate(page: &mut Page, url: &str) -> u128 {
    let started = Instant::now();
    match page.navigate(url).await {
        Ok(()) => started.elapsed().as_millis(),
        Err(error) => {
            eprintln!("navigation failed: {error}");
            0
        }
    }
}

/// Each render gets its own current-thread runtime, exactly as scorch does via
/// `spawn_blocking`. A connection pool is driven by the runtime that created it,
/// so reuse that works inside one runtime may still collapse when the runtime
/// the connections were opened on is dropped. That is the case this has to
/// reproduce, not the easy single-runtime one.
fn on_fresh_runtime<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_owned());
    let rounds: usize = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);

    // One warm-up render so process-wide lazy initialisation is not charged to
    // the first measured navigation.
    on_fresh_runtime(async {
        let context = Arc::new(BrowserContext::with_options(
            "probe-warm".to_owned(),
            None,
            true,
        ));
        let mut page = new_page(&context, "warm");
        navigate(&mut page, &url).await;
    });

    // The shared context and stealth client outlive the runtime each render
    // runs on, which is the arrangement a slot pool would use.
    let shared_context = Arc::new(BrowserContext::with_options(
        "probe-shared".to_owned(),
        None,
        true,
    ));
    let shared_stealth = on_fresh_runtime(async {
        let mut page = new_page(&shared_context, "mint");
        navigate(&mut page, &url).await;
        page.stealth_client.clone()
    });

    println!("\n{url}\n{:>8} {:>8}", "cold", "pooled");
    let mut totals = [0u128; 2];
    for round in 0..rounds {
        // cold: a fresh context and page on a fresh runtime, as scorch does
        let cold = on_fresh_runtime(async {
            let context = Arc::new(BrowserContext::with_options(
                format!("probe-cold-{round}"),
                None,
                true,
            ));
            let mut page = new_page(&context, "cold");
            navigate(&mut page, &url).await
        });

        // pooled: fresh runtime and page, but the context and stealth client
        // are the long-lived shared ones
        let pooled = on_fresh_runtime(async {
            let mut page = new_page(&shared_context, "pooled");
            page.stealth_client = shared_stealth.clone();
            shared_context.cookie_jar.clear();
            navigate(&mut page, &url).await
        });

        println!("{cold:>7}m {pooled:>7}m");
        totals[0] += cold;
        totals[1] += pooled;
    }
    let mean = |total: u128| total as f64 / rounds as f64;
    println!(
        "\nmean  cold {:.0}ms   pooled {:.0}ms",
        mean(totals[0]),
        mean(totals[1])
    );
}
