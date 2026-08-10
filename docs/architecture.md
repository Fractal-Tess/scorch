# Architecture

Scorch is a six-crate Rust workspace that produces two executables with a strict process boundary.

```text
metasearch     <- native concurrent search engines, routing, ranking, cache, breakers
scorch-types   <- shared HTTP request and response contracts
       ^
scorch-engine  <- network policy, proxy, fetch, browser, extraction, search, map, jobs
       ^
scorch-api     <- Axum transport and HTTP error mapping
       ^
scorch-server  <- `scorchd`: service configuration and lifecycle

scorch-cli     <- `scorch`: lightweight HTTP client, API benchmark, API-backed MCP adapter
```

`scorch` does not link `scorch-engine`, `scorch-api`, Obscura, Chromium, or metasearch. Every operation it exposes crosses the HTTP API boundary. Only `scorchd` owns runtime and engine configuration.

## Runtime

`scorchd` owns:

- one Axum server;
- one globally bounded direct-fetch runtime;
- an in-process Obscura renderer with bounded isolated work;
- an optional lazy Chromium compatibility backend;
- one embedded HTTP/CONNECT safety proxy shared by both backends;
- up to four active crawl tasks;
- an in-memory crawl registry with TTL and retained-byte limits.

There is no durable state. Restarting the process removes every crawl job.

## Scrape pipeline

1. Parse and validate strict request options.
2. Validate URL, DNS answers, port, and redirect policy.
3. Fetch with manual redirects and DNS-pinned sockets.
4. Select fetch output or an allowed browser backend based on requested capabilities and content heuristics.
5. Route Obscura or Chromium traffic through the embedded validating proxy.
6. Extract readable content, metadata, links, text, and Markdown.
7. Return only requested large formats.

## Discovery and crawl

Mapping is sitemap-first, with bounded nested sitemap traversal and root-link fallback. Crawling seeds from mapping, follows same-origin links breadth-first, applies path filters and robots policy, and runs bounded batches. Page failures are retained beside partial successes.

Each crawl has cancellation, an absolute deadline, a page/depth/concurrency ceiling, a retained-byte ceiling, and an expiry. Status responses are paginated to avoid repeatedly serializing all documents.

## Search

Metasearch is the only search provider and owns routing, concurrent searching, normalization, deduplication, and reciprocal-rank reranking. Bing, Naver, and Wikipedia are internal engine adapters, not request-selectable providers. The server's `SCORCH_SEARCH_ENGINES` policy controls which adapters metasearch may use. It collects additional responses for a short window after the first useful result, so slow engines cannot hold the full request open.

The metasearch runtime has per-engine concurrency limits, a bounded 60-second in-memory response cache, partial-failure behavior, and temporary circuit breakers after repeated engine failures. Challenges, markup drift, and rate limits are failures rather than authoritative empty results. Optional result scraping uses the guarded scrape pipeline.

## MCP

`scorch mcp` is an HTTP-backed adapter: every MCP tool calls the configured `SCORCH_API_URL`, and cancellation drops the corresponding HTTP request future. It never constructs or invokes the engine directly. The official Rust MCP SDK provides JSON-RPC framing, generated schemas, cancellation tokens, tool discovery, and stdio lifecycle. Protocol output is isolated on stdout.

## Trust boundary

Fetched pages, search results, browser output, sitemaps, and robots files are untrusted data. They never control process configuration or command execution. Direct and browser network paths share one URL/address policy. Obscura receives only the embedded proxy endpoint and keeps its own private-address guard enabled. Optional Chromium receives an explicit proxy with loopback bypass removed while QUIC and non-proxied WebRTC are disabled.
