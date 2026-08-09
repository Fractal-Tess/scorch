# Native metasearch

Scorch's `metasearch` crate is an original MIT-licensed implementation. It keeps multi-engine search inside the Scorch executable and does not require a Python service, Redis, or another deployment.

## Runtime behavior

A request starts every enabled engine concurrently. The runtime accepts the first non-empty response, keeps a short collection window open for additional agreement, then cancels unfinished requests. This prevents one blocked provider from determining total latency.

Collected URLs are normalized by removing fragments and common tracking parameters. Duplicate URLs are merged and ranked with weighted reciprocal-rank fusion. The response identifies contributing sources for each result.

The runtime also provides:

- per-engine concurrency limits;
- a six-second absolute deadline;
- a 900 ms collection window after the first useful response;
- a bounded 60-second in-memory cache;
- partial results when individual engines fail;
- temporary 30-second circuit breakers after repeated failures;
- response-body and query-size limits.

## Engine verification

Each engine is implemented and tested separately before being added to the aggregate. Live tests are ignored during the normal suite because they contact public services and can be affected by rate limits.

| Engine | Included in aggregate | Verification |
| --- | --- | --- |
| Bing | Yes, weight 1.0 | Parser fixture and successful live search |
| Naver | Yes, weight 0.85 | Parser fixture and successful live search |
| Wikipedia | Yes, weight 0.55 | JSON fixture, input validation, and successful live search |
| DuckDuckGo | No; explicit legacy provider only | Parser fixture; currently unreachable from the development network |

Other evaluated HTML endpoints returned JavaScript-only shells, consent redirects, CAPTCHA pages, temporary errors, or explicit automated-query blocks. They are not included merely to increase the engine count. Credential-backed APIs can be added later as separately configured engines.

On the development network, one six-request dashboard run using client concurrency six and server concurrency four completed successfully in 2.50 seconds wall time, with 1.39 seconds median latency and 2.40 requests/second. These are directional network measurements, not universal performance guarantees.

## Testing

Run deterministic tests:

```sh
cargo test -p metasearch
```

Run each public-service smoke test deliberately:

```sh
cargo test -p metasearch engines::bing::tests::live_search_returns_public_results -- --ignored --exact
cargo test -p metasearch engines::naver::tests::live_search_returns_public_results -- --ignored --exact
cargo test -p metasearch engines::wikipedia::tests::live_search_returns_public_results -- --ignored --exact
cargo test -p metasearch aggregate::tests::live_metasearch_returns_without_waiting_for_every_engine -- --ignored --exact
```
