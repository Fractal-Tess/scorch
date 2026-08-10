# Browser engine evaluation

Assessment date: 2026-08-09.

## Decision

Scorch uses Obscura as its default browser backend. It links Obscura's Apache-2.0 Rust crates directly into `scorchd`; no Obscura process, CDP server, or sidecar is started. Each render receives an isolated V8 page on bounded blocking work, and all traffic still crosses Scorch's embedded validating proxy.

Chromium remains an optional compatibility backend. The service default and allowlist are controlled by `SCORCH_BROWSER` and `SCORCH_ALLOWED_BROWSERS`; requests may select only an allowed backend. Direct HTTP remains the first engine for static content, so browser work is paid for only when rendering or screenshots are necessary.

## Local measurements

These are directional smoke measurements, not a general browser benchmark. They used Chromium 151, three sequential runs, the same machine and network, and an optimized release build.

| Page | Engine | Median | Range | Extracted HTML |
| --- | --- | ---: | ---: | ---: |
| `example.com` | Direct fetch | 496 ms | 452–554 ms | 559 bytes |
| `example.com` | Chromium | 1,909 ms | 1,359–1,940 ms | 559 bytes |
| JavaScript quote fixture | Direct fetch | 2,325 ms | 523–3,684 ms | 5,808 bytes |
| JavaScript quote fixture | Chromium | 1,265 ms | 1,013–4,394 ms | 8,985 bytes |

The static result supports fetch-first routing. The JavaScript page demonstrates why a browser fallback remains necessary: it returned substantially more rendered HTML at additional latency.

Before integration, the Obscura 0.2.0 Linux stealth binary rendered `example.com` in 66–104 ms after warm filesystem caches and rendered the JavaScript quote fixture in 1,584–1,727 ms. A rough one-page sample observed about 54 MiB peak RSS for Obscura versus about 507 MiB summed process-tree RSS for Chromium. These figures motivated the trial but are not treated as equivalent production benchmarks.

With `scorchd` running, reproduce the API-backed harness with:

```sh
scorch benchmark https://example.com https://quotes.toscrape.com/js/ --runs 3
```

## Alternatives considered

### Gecko anti-detection wrapper

The evaluated option uses a customized Firefox engine behind a Node/Playwright REST server rather than CDP. It is not a drop-in Rust backend and would add a separately versioned service, browser download, telemetry review, and larger supply-chain surface. Its anti-detection and challenge-bypass statements are vendor claims, while its own documentation still describes human CAPTCHA handling and remaining fingerprint inconsistencies.

It is therefore not a default dependency. A CAPTCHA challenge is reported as a remote-site limitation; Scorch does not claim to solve or bypass challenges automatically.

### Lightweight CDP browser

The evaluated lightweight engine has credible vendor-published resource advantages for compatible text workloads, but currently implements only a subset of CDP, omits many Web APIs, and returns placeholder screenshot/PDF artifacts. It is beta, absent from nixpkgs, and AGPL-3.0 licensed. Those constraints conflict with Scorch's screenshot contract, broad web compatibility, packaging, and licensing choices.

It may become an explicit experimental text-only backend after a protocol conformance suite proves every CDP method Scorch needs. Chromium fallback would remain mandatory.

## Benchmark policy

Future comparisons must measure correctness before speed. A candidate must pass navigation, JavaScript, extraction, actions, isolation, redirect, subresource, WebSocket, timeout, cancellation, and SSRF sentinel tests. Resource measurements should use whole-process-tree PSS, cold and warm runs, randomized order, multiple concurrency levels, and at least 30 repetitions. CAPTCHA bypasses must not be tested against third-party sites; owned fixtures should test only challenge detection and safe failure.
