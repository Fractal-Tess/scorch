# Scorch

Scorch is a self-contained web search, scraping, mapping, and crawling service written in Rust. One `scorch` executable provides an HTTP API, an HTTP client CLI, a bounded in-memory crawl runtime, and an MCP stdio server. It uses direct HTTP for inexpensive static pages and a locally managed headless Chromium process when JavaScript rendering or screenshots are required.

Scorch does not require a database, broker, cache server, browser service, or external worker deployment. Crawl state is intentionally ephemeral and is lost when the process restarts.

## Development

```sh
devenv shell
cargo run -- serve
```

The API listens on `127.0.0.1:3000` by default. Authentication is intentionally not built into this local-first release; do not expose it to an untrusted network without an authenticated TLS gateway.

## Logging

Scorch writes structured operational logs to stderr. The default compact output records startup and shutdown, request IDs, methods, response statuses and latency, engine selection, operation outcomes, browser lifecycle, and crawl lifecycle. Request bodies, search text, and URL query strings are not logged at info level.

Use `RUST_LOG` for filtering and `SCORCH_LOG_FORMAT=json` for newline-delimited JSON:

```sh
RUST_LOG=scorch=debug cargo run -- serve
SCORCH_LOG_FORMAT=json RUST_LOG=scorch=info cargo run -- serve
```

CLI and MCP JSON protocol output remains isolated on stdout.

## CLI

Every web command calls the configured HTTP API:

```sh
cargo run -- scrape https://example.com --format markdown,links
cargo run -- search "rust async runtime" --provider metasearch --limit 5 --scrape
cargo run -- map https://example.com --limit 100
cargo run -- crawl https://example.com --limit 20 --max-depth 2 --wait
```

Set `SCORCH_API_URL` or pass `--api-url` to use another server.

Server and runtime options are available with:

```sh
cargo run -- serve --help
```

### Search providers

Bing is the default and requires no supporting service. Choose a different default when starting the API:

```sh
SCORCH_SEARCH_PROVIDER=metasearch cargo run -- serve
# equivalent: cargo run -- serve --search-provider metasearch
```

Individual CLI and API requests can override it. Available providers are `bing`, `metasearch`, `naver`, `wikipedia`, and `duckduckgo`:

```sh
cargo run -- search "Rust async runtime" --provider bing
cargo run -- search "Rust async runtime" --provider metasearch
```

The native metasearch provider concurrently queries Bing, Naver, and Wikipedia, combines agreement with reciprocal-rank fusion, caches short-lived results, and stops waiting shortly after useful results arrive. It remains inside the single Scorch executable. See `docs/metasearch.md` for design details and engine verification.

### Concurrent search dashboard

With the API running, launch the dependency-free Python terminal dashboard:

```sh
python3 scripts/concurrent_search_demo.py \
  --provider metasearch \
  --requests 8 \
  --concurrency 4
```

Pass custom queries as positional arguments, add `--scrape` to enrich results, or use `--plain` for non-interactive output. Each request receives a distinct `x-request-id` that also appears in Scorch's service logs.

## HTTP API

```sh
curl -sS http://127.0.0.1:3000/v1/scrape \
  -H 'content-type: application/json' \
  -d '{
    "url": "https://example.com",
    "options": {
      "formats": ["markdown", "links"],
      "render": "auto"
    }
  }' | jq
```

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/healthz` | Process liveness |
| `GET` | `/readyz` | Browser readiness and concurrency |
| `POST` | `/v1/scrape` | Fetch or render one page |
| `POST` | `/v1/search` | Search, optionally scraping results |
| `POST` | `/v1/map` | Discover normalized same-site URLs |
| `POST` | `/v1/crawls` | Start an in-memory crawl |
| `GET` | `/v1/crawls/{id}` | Read paginated status and results |
| `DELETE` | `/v1/crawls/{id}` | Cancel and remove a crawl |

See `docs/api.md` for request contracts and limits.

## MCP

Run the stdio server directly:

```sh
cargo run -- mcp
```

A client configuration can point at a release build:

```json
{
  "mcpServers": {
    "scorch": {
      "command": "/absolute/path/to/scorch",
      "args": ["mcp"],
      "env": {
        "SCORCH_BROWSER_PATH": "/absolute/path/to/chromium"
      }
    }
  }
}
```

The tools are `scorch_search`, `scorch_scrape`, `scorch_map`, `scorch_crawl_start`, `scorch_crawl_status`, and `scorch_crawl_cancel`. MCP protocol traffic is written only to stdout; diagnostics go to stderr.

## Browser and security

Chromium traffic is forced through an embedded validating HTTP/CONNECT proxy. Direct fetches and browser connections reject local, private, link-local, reserved, and unsafe targets, repin DNS addresses, and revalidate redirects. Response sizes, redirects, browser pages, crawl depth, crawl count, retained bytes, request bodies, and job lifetimes are bounded.

The browser choice and measured tradeoffs are documented in `docs/browser-evaluation.md`.

## Build profiles

The production profile uses thin LTO, one code-generation unit, symbol stripping, and overflow checks while retaining unwind behavior for panic isolation:

```sh
cargo build --release
```

For production-equivalent profiling with debug symbols:

```sh
cargo build --profile release-debug
```

## Checks

```sh
check
fmt --check
lint
test
```

## License

Licensed under the MIT License. Copyright © 2026 Fractal-Tess.
