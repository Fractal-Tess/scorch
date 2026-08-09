# Architecture

Scorch is a four-crate Rust workspace that produces one executable.

```text
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

Search providers are isolated adapters. The default waterfall currently parses public DuckDuckGo HTML and then Bing HTML. Challenges, markup drift, and rate limits are treated as provider failure rather than empty authoritative results. Optional result scraping uses the same guarded scrape pipeline.

## MCP

`scorch mcp` invokes the engine directly rather than looping through HTTP. The official Rust MCP SDK provides JSON-RPC framing, generated schemas, cancellation tokens, tool discovery, and stdio lifecycle. Protocol output is isolated on stdout; tracing is always written to stderr.

## Trust boundary

Fetched pages, search results, browser output, sitemaps, and robots files are untrusted data. They never control process configuration or command execution. Direct and browser network paths share one URL/address policy. The browser cannot connect directly: QUIC and non-proxied WebRTC are disabled and Chromium receives an explicit proxy with loopback bypass removed.
