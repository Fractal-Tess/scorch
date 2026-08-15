# Changelog

All notable changes to Scorch are documented in this file. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0] - 2026-08-15

### Fixed

- Fixed searches returning nothing by default. DuckDuckGo is the default engine, and it answered every Scorch request with its bot challenge instead of results: the request carried only a `User-Agent`, and DuckDuckGo requires `Accept-Encoding` together with `Accept` or `Accept-Language` before it will serve results. The challenge was then parsed as zero hits, which is indistinguishable from a query that genuinely matches nothing, so the engine kept a clean health record, the circuit breaker never tripped, no error reached the caller, and the empty answer was cached for the rest of its TTL. Scorch now sends browser-like headers and treats the challenge as a rate-limit failure. `Accept-Encoding: identity` is deliberate, since the HTTP client is built without decompression support and a compressed body would parse to zero hits in the same silent way. No other engine was affected.
- Rendered pages no longer open a second connection to their own origin. Obscura fetched documents, stylesheets, and images over its stealth HTTP client but fetched `<script src>` and ES modules over a separate plain client, so every render paid an extra TCP and TLS handshake and requested subresources with a different TLS fingerprint than the navigation itself. Routing both over the page's own client cut median browser scrape time by 22 percent and raised parallel throughput by 30 percent, with byte-identical Markdown across twelve pages. Applied through a patched Obscura until the fix lands upstream.

### Changed

- Scrapes now retain eligible extracted results in a bounded in-memory cache for up to five minutes. The foreground request extracts and returns only its requested formats; an eight-job, 16 MiB background queue then prepares the remaining formats without retaining raw HTML in the result cache. Entries are capped at 8 MiB each, 256 entries, and a 64 MiB weighted process budget; redirects, final-URL changes, cookies, private or `no-store` responses, unsafe `Vary`, and stale origin freshness bypass retention. `maxAgeMs: 0` forces a refresh and `storeInCache: false` bypasses reads and writes. On the README's four-page workload through the always-stealth browser path, twenty-four warm sequential hits had a 0.724 ms median and six warm eight-request trials had a 6.36 ms median (1,257 req/s), versus the README's cache-disabled 638 ms and 1.438 s figures. Warming all formats added about 472 KiB RSS over a no-cache control after the same four cold pages; most of both processes' roughly 48 MiB RSS was browser state.
- Every scrape now executes JavaScript through embedded Obscura with its Chrome-like stealth transport statically enabled. Across three cache-disabled collections of the README workload, stealth had a 662 ms pooled sequential median and 2.067 s parallel-trial median versus 676 ms and 2.200 s without stealth, so the ordinary transport provided no speed advantage. This policy changes fingerprints but does not rotate the egress IP.
- Long-lived browser slots now cache explicitly public, fresh scripts and ES modules in Obscura's stealth transport, so repeat pages on the same origin execute retained bytes instead of downloading shared bundles again. Five warm React documentation renders had a 251 ms median versus 382 ms without the cache, a 34 percent reduction, and their serialized HTML was byte-identical. Credentialed requests, arbitrary extra headers, redirects, `Set-Cookie`, `no-cache`, `no-store`, private responses, `Vary: Referer`, and objects over 4 MiB bypass retention. Cache bodies share a 128 MiB process budget divided across slots, with at most 128 entries per slot.
- Extraction now runs off Tokio's async worker threads and performs Readability, text, link, Markdown, and HTML work only when the requested formats need it. Browser HTML moves into extraction without another full copy, text normalization no longer builds intermediate vectors, and `elapsedMs` now includes extraction rather than stopping before it. Debug logs expose validation, browser-queue, and extraction time separately.
- The NixOS module no longer pins `maxConcurrency` to four. Its nullable default now leaves the flag unset so `scorchd` uses its CPU-derived 4–16 worker default; deployments with a fixed memory or origin-load budget can retain an explicit value.
- An engine that reports a rate limit is now set aside for an hour rather than thirty seconds, and on the first refusal rather than after two. Services that gate on bot detection block an address rather than a session, and a request sent inside that block extends it, so the short retry that suits a timeout turned a single refusal into a standing one. Every other failure keeps the thirty-second cooldown and the two-failure threshold.
- DuckDuckGo requests now take the shape a browser produces. The no-JS endpoint is reached by submitting its form, so Scorch sends a POST with the form's own fields and carries the region and safe-search preferences in cookies, the `Sec-Fetch-*` headers and the referrer a form submission sets, and a User-Agent that matches a browser that exists — the previous one claimed to be Chrome but omitted the `(KHTML, like Gecko)` token and the full version. Scorch also recognizes DuckDuckGo's second challenge markup, `form#challenge-form`, in addition to the modal it already detected. These changes were derived from SearXNG's DuckDuckGo engine, which has tracked this endpoint's bot detection for far longer.
- Browser renders now run on a fixed set of long-lived render slots instead of a browser context and Tokio runtime built per render. A connection pool belongs to the runtime that drives it, so discarding the runtime after every render meant every render paid a fresh TCP and TLS handshake to the origin. Repeat renders of the same page dropped from 179 ms to 89 for example.com, 858 to 174 for `httpbin.org/html`, 1248 to 447 for `books.toscrape.com`, and 1654 to 533 for Hacker News; at concurrency eight, throughput rose 53 percent and median latency fell from 805 ms to 610. Extracted Markdown was identical. Renders sharing a slot now share its connections and TLS sessions, so isolation is weaker than before: each render still gets its own page and an empty cookie jar, while only explicitly public anonymous scripts can enter the bounded response cache; documents and credentialed responses cannot cross between renders. Idle memory is unchanged and peak memory rose from 160 MB to 203 MB, since a slot holds its context for the life of the process.
- Browser scraping no longer downloads the page twice. The old forced path ran a direct fetch alongside the browser and discarded its body, keeping only the status and headers that navigation already records. A scrape now makes one request to the origin and never requests the same page under two different TLS fingerprints milliseconds apart.
- Updated the embedded Obscura to current upstream. The notable fixes for Scorch are containment of duplicate module graphs and the heap exhaustion they caused, a retry for GET requests dropped by a connection reset, and cookie deletion on expired updates. Median scrape time, parallel throughput, and extracted Markdown were unchanged across twelve pages.
- URL mapping fetches now share one pooled HTTP client that pins DNS at connect time instead of building a client per request. Repeat requests to a known host reuse the connection and TLS session, cutting median `map` latency by 63 percent, with byte-identical output.
- Rendered pages no longer download stylesheets. Extraction reads the serialized DOM rather than computed style, so the round trips only added latency. Median browser scrape time dropped 31 percent and parallel throughput rose 29 percent, with byte-identical Markdown across twelve pages.

### Removed

- Removed the HTTP `render` option, CLI `--render` flag, JavaScript and Python `RenderMode` types, daemon `--obscura-stealth` flag, `SCORCH_OBSCURA_STEALTH` environment variable, and NixOS `services.scorchd.obscuraStealth` option. Requests no longer control execution or transport policy.

## [0.6.0] - 2026-08-14

### Fixed

- Fixed a startup race that segfaulted the process when two renders entered Obscura's lazy process-wide initialization at once, by rendering one throwaway page before serving traffic.
- Fixed an abort that let a single ordinary page terminate the service. Certain page layouts tripped an assertion in the bundled `taffy` layout engine, and release builds abort on panic. Removing the `render` feature drops `taffy` from the dependency graph, so those pages now scrape normally.

### Changed

- Dropped Obscura's `render` feature. Rendering no longer prefetches images and web fonts for rasterization, cutting median render time by 29 percent and up to 65 percent on image-heavy pages. Extracted Markdown was byte-identical across twelve pages, including JavaScript-driven ones.

### Removed

- Removed screenshot capture. Scorch returns text content only, so the `screenshot` format, the `fullPageScreenshot` option, and the `screenshot` response field are gone from the HTTP API, CLI, and clients.

## [0.5.1] - 2026-08-13

### Added

- Added standalone, checksummed `scorch` CLI and `scorchd` daemon assets for each supported Linux architecture.
- Added a minimal multi-platform `scratch` container image that starts `scorchd` automatically and also contains the `scorch` client.

### Changed

- Optimized release binaries with level-three optimization, fat LTO, one codegen unit, stripped symbols, and abort-on-panic behavior.

## [0.5.0] - 2026-08-12

### Added

- Added typed, zero-runtime-dependency Python and JavaScript clients for the complete Scorch HTTP API.
- Added a GitHub search category across HTTP, CLI, MCP, Python, and JavaScript.
- Added credential-free Brave Web, Google CSE, Yahoo, Mwmbl, GitHub, Hacker News, Hugging Face, crates.io, npm, Docker Hub, Crossref, OpenAlex, Open Library, PubMed, NVD, and Wikidata search adapters.
- Added `search-engines.md`, a clean-room compatibility inventory for the evaluated SearXNG engine surface.

### Changed

- Made DuckDuckGo the sole implicit search engine.
- Narrowed the selectable credential-free binary surface to 19 useful English-focused engines after every retained engine returned source-attributed results through built `scorchd` and `scorch search` binaries.
- Updated the HTTP API, CLI, MCP schemas, Python and JavaScript clients, Nix modules, Agent Skill, and documentation for the final engine set.

### Removed

- Removed redundant, non-English, or overly specialized engines from the selectable binary surface, including Naver, Stack Overflow, Microsoft Learn, Steam, Jisho, PDBe, Arch Linux, GitLab, Hex, Packagist, and ManKier.

## [0.4.0] - 2026-08-11

### Added

- Added request-level metasearch engine subsets bounded by the server allowlist.
- Added completed-engine diagnostics to search responses.

### Changed

- Kept DuckDuckGo as the sole implicit search engine while allowing request-level engine subsets.
- Improved Agent Skill guidance for locale-sensitive search, compact results, and structured JSON APIs.
- Updated pinned core GitHub Actions to their current major releases.

## [0.3.0] - 2026-08-11

### Added

- Added checksum-verifying release installation, documented MCP setup for major coding agents, and generated third-party license notices.
- Added dependency license, source, and RustSec policy checks with Cargo Deny.
- Added source manifests and exact source-revision validation to immutable release promotion.

### Changed

- Made embedded Obscura the sole browser renderer while keeping stealth transport enabled by default.
- Simplified readiness output to report Obscura directly without backend allowlist or executable-path fields.
- Raised the minimum Rust version from 1.88 to 1.97 and refreshed direct dependencies to their latest stable releases.
- Replaced the GPL-licensed HTML-to-Markdown dependency with Apache-2.0-licensed `htmd`.
- Made crawl admission atomic, cancellation prompt, terminal TTLs accurate, and progress totals consistent.
- Enforced absolute scrape, map, search-queue, and crawl deadlines and bounded CLI API responses.
- Hardened release preparation with formatting, checking, strict Clippy, tests, dependency policy, version validation, and third-party notices.
- Added a dedicated 1200×630 Open Graph splash image, large-card social metadata, structured data, and image loading optimizations to the documentation site.

### Security

- Rejected NAT64-embedded private targets and canonicalized trailing-dot hosts before DNS pinning.
- Restricted sitemap discovery and crawl seeding to the requested origin.
- Extended bounded browser proxy streams without truncating slow page loads.

### Removed

- Removed the Chromium backend, `chromiumoxide`, and all Chromium runtime code and development dependencies.
- Removed scrape `options.browser`, CLI `--browser`, and the `SCORCH_BROWSER`, `SCORCH_ALLOWED_BROWSERS`, and `SCORCH_BROWSER_PATH` server settings.
- Removed the NixOS `browser`, `allowedBrowsers`, and `browserPath` options and the `scorchd-with-chromium` flake package.
- Removed the CLI benchmark command, repository benchmark harnesses, Devenv configuration, vendored dependency copies, and obsolete non-Astro documentation.

## [0.1.3] - 2026-08-10

### Changed

- Renamed the operational probe endpoints from `/healthz` and `/readyz` to `/health` and `/ready`.

## [0.1.2] - 2026-08-10

### Changed

- Moved the default Scorch API listener and client endpoint from port 3000 to port 33000 to avoid common development-server conflicts.

## [0.1.1] - 2026-08-10

### Changed

- Nix packages now install versioned, CI-built GitHub release binaries instead of compiling the Rust workspace locally.
- Added reproducible Linux release archives for the `scorch` client and `scorchd` service on x86_64 and AArch64.

## [0.1.0] - 2026-08-10

### Added

- `scorchd`, a self-contained HTTP service for web search, scraping, mapping, crawling, browser rendering, screenshots, and bounded ephemeral crawl jobs.
- `scorch`, a lightweight HTTP-only client with CLI commands, benchmarking, and an MCP stdio adapter.
- Clean-room metasearch across configurable source engines with concurrent queries, URL deduplication, caching, circuit breaking, and reranking.
- Embedded Obscura rendering with stealth enabled by default and optional Chromium compatibility.
- Service-level and request-level browser policy, including strict backend allowlisting.
- Fetch-first page extraction with Markdown, HTML, text, links, metadata, and screenshot formats.
- Shared URL, DNS, redirect, port, response-size, concurrency, and SSRF controls across direct and browser traffic.
- Structured compact or JSON operational logs with request IDs and credential-aware redaction.
- Static Astro documentation, a generated Scorch visual system, and an NGINX production container.
- Nix flake packages for `scorch`, `scorchd`, and the Scorch Agent Skill; runnable flake apps; an overlay; a development shell; and NixOS and Home Manager modules.
- A repository-local Agent Skill for safe search, extraction, mapping, crawling, benchmarking, and MCP usage through Scorch.

### Security

- Browser traffic is forced through the same validating HTTP/CONNECT boundary as direct fetches.
- Requests reject non-HTTP(S), loopback, private, link-local, reserved, and unsafe targets, and revalidate DNS answers and redirects.
- Embedded Obscura requests use fresh context, page, cookie, cache, and JavaScript state to prevent caller data leakage.
- Render requests enforce one caller-visible deadline across validation, queueing, navigation, execution, serialization, and screenshots.

[Unreleased]: https://github.com/Fractal-Tess/scorch/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/Fractal-Tess/scorch/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/Fractal-Tess/scorch/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/Fractal-Tess/scorch/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Fractal-Tess/scorch/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Fractal-Tess/scorch/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Fractal-Tess/scorch/compare/v0.1.3...v0.3.0
[0.1.3]: https://github.com/Fractal-Tess/scorch/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Fractal-Tess/scorch/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Fractal-Tess/scorch/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Fractal-Tess/scorch/releases/tag/v0.1.0
