# Architecture proposal

## Goals

- One Rust server executable that owns browser lifecycle and worker tasks.
- No database, message broker, cache server, or separate browser service.
- Bounded CPU, memory, browser pages, crawl depth, response size, and request time.
- Fetch static pages directly; render with Chromium only when JavaScript or actions require it.
- Reject private, loopback, link-local, and otherwise unsafe network targets, including redirects and browser subrequests.
- Return partial crawl results when individual pages fail.

## Proposed modules

- `api`: HTTP routes, validation, response envelopes, and error mapping.
- `fetch`: guarded HTTP client and redirect policy.
- `browser`: one managed Chromium process with isolated contexts and a page semaphore.
- `extract`: HTML cleanup, metadata, links, images, text, and Markdown conversion.
- `search`: provider adapters, beginning with search-page parsing and optional result scraping.
- `crawl`: sitemap-first discovery plus bounded same-origin breadth-first traversal.
- `jobs`: in-memory async job registry, cancellation tokens, expiry, and bounded result retention.
- `security`: URL normalization, DNS/IP checks, request interception, and size/time limits.

## Proposed first API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/healthz` | Process liveness. |
| `GET` | `/readyz` | Browser and worker readiness. |
| `POST` | `/v1/search` | Search the web; optionally scrape each result. |
| `POST` | `/v1/scrape` | Fetch or render one URL and return requested formats. |
| `POST` | `/v1/map` | Discover normalized same-site URLs without scraping every page. |
| `POST` | `/v1/crawls` | Start a bounded in-memory crawl job. |
| `GET` | `/v1/crawls/{id}` | Read crawl status and available results. |
| `DELETE` | `/v1/crawls/{id}` | Cancel and remove a crawl job. |

## Later, if needed

- Batch scrape jobs using the same in-memory job runtime.
- Short-lived browser interaction sessions.
- Screenshots and PDF extraction.
- Authentication and per-key rate limits for untrusted deployments.

Structured AI extraction and autonomous research are intentionally outside the initial core because they require a model provider and weaken the no-external-services guarantee.
