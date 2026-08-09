# Scorch

Scorch is a compact web search, scraping, and crawling API written in Rust. It runs as one service, manages a local headless browser, and keeps bounded work in memory—no database, queue service, or worker deployment required.

## Development

Enter the reproducible development environment:

```sh
devenv shell
```

Run the API:

```sh
cargo run
```

By default it listens on `127.0.0.1:3000`. Configure it with:

- `SCORCH_BIND`
- `SCORCH_BROWSER_PATH`
- `SCORCH_MAX_CONCURRENCY`
- `SCORCH_REQUEST_TIMEOUT_SECS`
- `RUST_LOG`

## Checks

```sh
check
fmt --check
lint
test
```

The initial API and architecture are being designed in `docs/architecture.md`.
