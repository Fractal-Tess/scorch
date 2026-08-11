# Scorch API clients

Official source clients for the Scorch HTTP API:

- [`python`](python/) — Python 3.11+, synchronous, typed, and dependency-free.
- [`javascript`](javascript/) — ESM for Node.js 22+ and modern Fetch API runtimes, with TypeScript declarations and no runtime dependencies.

Both clients cover health, readiness, search, scrape, map, crawl start/status/cancel, structured errors, custom gateway headers, request timeouts, and bounded response reads. They default to `http://127.0.0.1:33000` and honor `SCORCH_API_URL` under their supported server runtimes.

These packages currently install from the repository and are not yet published to PyPI or npm.
