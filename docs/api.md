# API contract

All request objects reject unknown fields. Errors use:

```json
{"code":"invalid_request","message":"invalid request: ..."}
```

Every response includes an `x-request-id` header.

## Scrape

`POST /v1/scrape`

```json
{
  "url": "https://example.com",
  "options": {
    "formats": ["markdown", "html", "text", "links", "metadata", "screenshot"],
    "render": "auto",
    "browser": "obscura",
    "timeoutMs": 30000,
    "waitForMs": 0,
    "onlyMainContent": true,
    "blockMedia": true,
    "fullPageScreenshot": false
  }
}
```

- `render`: `auto`, `always`, or `never`.
- `browser`: optional `obscura` or `chromium`; omission uses the service default. The selected backend must be allowed by `SCORCH_ALLOWED_BROWSERS`.
- Default format: `markdown`.
- `auto` uses direct fetch unless the page appears JavaScript-dependent or a screenshot is requested.
- Screenshots are PNG data URIs.
- Response `engine` is `fetch`, `obscura`, or `chromium`.
- Timeout range: 100–120,000 ms. Explicit post-load wait is capped at 60,000 ms.
- One remote response is capped at 5 MiB by default.

The response includes requested and final URLs, engine, elapsed time, metadata, requested formats, and warnings.

## Search

`POST /v1/search`

```json
{
  "query": "rust async runtime",
  "limit": 5,
  "country": "us",
  "language": "en",
  "scrapeOptions": null
}
```

Metasearch is the only search provider. Requests cannot select a source engine. The server independently controls the engines metasearch may route to with `SCORCH_SEARCH_ENGINES` or `--search-engines`; allowed values are `bing`, `naver`, and `wikipedia`, and all are enabled by default. Responses report `metasearch` as the provider and identify each result's contributing `sources`.

Metasearch concurrently queries its allowed engines, deduplicates normalized URLs, and uses reciprocal-rank fusion. It returns shortly after the first useful response rather than waiting for every engine, maintains a bounded in-memory cache and failure cooldowns, and reports completed-engine failures in `warnings`. Search remains best-effort because public providers can change markup, rate-limit, or issue challenges. Limits are 1–20 and queries are capped at 512 bytes. Supplying `scrapeOptions` enriches results with bounded page documents while preserving per-result errors.

## Map

`POST /v1/map`

```json
{
  "url": "https://example.com",
  "limit": 100,
  "includeSubdomains": false,
  "includePaths": [],
  "excludePaths": []
}
```

Map limits are 1–1,000. Discovery checks robots-declared sitemaps, the conventional sitemap location, nested sitemap indexes, and root-page links. Results are normalized, deduplicated, path-filtered, and restricted to the requested site. A valid map may contain zero links.

## Crawl

`POST /v1/crawls` returns HTTP 202 and a job summary.

```json
{
  "url": "https://example.com",
  "limit": 20,
  "maxDepth": 2,
  "concurrency": 4,
  "includePaths": [],
  "excludePaths": [],
  "scrapeOptions": {
    "formats": ["markdown"],
    "render": "auto"
  }
}
```

Defaults and configured maxima:

- Limit: 20; maximum 100.
- Discovery depth: 2; maximum 5.
- Concurrency: 4; cannot exceed global concurrency.
- Absolute crawl deadline: 5 minutes.
- Retained data per job: 32 MiB.
- Completed-job TTL: 15 minutes.
- Retained jobs: 128; active crawls: 4.
- `robots.txt` is respected using the `ScorchBot` user agent.

Poll with:

```text
GET /v1/crawls/{id}?cursor=0&pageSize=10
```

Page size is 1–50. Status values are `queued`, `running`, `completed`, `cancelled`, and `failed`. Individual failures remain in `errors` and do not discard successful pages.

`DELETE /v1/crawls/{id}` cancels and removes the job.

## Security behavior

The API accepts only HTTP and HTTPS targets without embedded credentials. It rejects unsafe ports and any hostname with a local or special-purpose DNS answer. DNS answers are pinned to outbound direct connections. Redirects are manual, limited to five, and independently revalidated.

Chromium has no direct network route configured: HTTP, HTTPS, WebSocket, redirect, iframe, worker, and subresource connections traverse an embedded proxy that resolves, validates, and connects to the selected address. QUIC and non-proxied WebRTC UDP are disabled.
