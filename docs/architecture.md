# Architecture

Scorch is a five-crate Rust workspace that produces one executable.

```text
metasearch    <- native concurrent search engines, ranking, cache, circuit breakers
scorch-types  <- shared request and response contracts
      ^
scorch-engine <- network policy, proxy, fetch, browser, extraction, search, map, jobs
      ^
scorch-api    <- Axum transport and HTTP error mapping
      ^
scorch-cli    <- `scorch` executable, HTTP client commands, server mode, MCP mode
```

## Runtime

`scorch serve` owns:

- one Axum server;
- one globally bounded direct-fetch runtime;
- one lazy Chromium process and bounded page semaphore;
- one embedded HTTP/CONNECT safety proxy;
- up to four active crawl tasks;
- an in-memory crawl registry with TTL and retained-byte limits.

There is no durable state. Restarting the process removes every crawl job.

## Scrape pipeline

1. Parse and validate strict request options.
2. Validate URL, DNS answers, port, and redirect policy.
3. Fetch with manual redirects and DNS-pinned sockets.
4. Select fetch output or Chromium based on requested capabilities and content heuristics.
5. Route browser traffic through the embedded validating proxy.
6. Extract readable content, metadata, links, text, and Markdown.
7. Return only requested large formats.

## Discovery and crawl

Mapping is sitemap-first, with bounded nested sitemap traversal and root-link fallback. Crawling seeds from mapping, follows same-origin links breadth-first, applies path filters and robots policy, and runs bounded batches. Page failures are retained beside partial successes.

Each crawl has cancellation, an absolute deadline, a page/depth/concurrency ceiling, a retained-byte ceiling, and an expiry. Status responses are paginated to avoid repeatedly serializing all documents.

## Search

Metasearch is the only search provider and owns routing, concurrent searching, normalization, deduplication, and reciprocal-rank reranking. Bing, Naver, and Wikipedia are internal engine adapters, not request-selectable providers. The server's `SCORCH_SEARCH_ENGINES` policy controls which adapters metasearch may use. It collects additional responses for a short window after the first useful result, so slow engines cannot hold the full request open.

The metasearch runtime has per-engine concurrency limits, a bounded 60-second in-memory response cache, partial-failure behavior, and temporary circuit breakers after repeated engine failures. Challenges, markup drift, and rate limits are failures rather than authoritative empty results. Optional result scraping uses the guarded scrape pipeline.

## MCP

`scorch mcp` invokes the engine directly rather than looping through HTTP. The official Rust MCP SDK provides JSON-RPC framing, generated schemas, cancellation tokens, tool discovery, and stdio lifecycle. Protocol output is isolated on stdout; tracing is always written to stderr.

## Trust boundary

Fetched pages, search results, browser output, sitemaps, and robots files are untrusted data. They never control process configuration or command execution. Direct and browser network paths share one URL/address policy. The browser cannot connect directly: QUIC and non-proxied WebRTC are disabled and Chromium receives an explicit proxy with loopback bypass removed.
