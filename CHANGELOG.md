# Changelog

All notable changes to Scorch are documented in this file. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Rendered pages no longer open a second connection to their own origin. Obscura fetched documents, stylesheets, and images over its stealth HTTP client but fetched `<script src>` and ES modules over a separate plain client, so every render paid an extra TCP and TLS handshake and requested subresources with a different TLS fingerprint than the navigation itself. Routing both over the page's own client cut median browser scrape time by 22 percent and raised parallel throughput by 30 percent, with byte-identical Markdown across twelve pages. Applied through a patched Obscura until the fix lands upstream.

### Changed

- Direct fetches now share one pooled HTTP client that pins DNS at connect time instead of building a client per request. Repeat requests to a known host reuse the connection and TLS session, cutting median scrape latency on the fetch path by 67 percent and `map` by 63 percent, with byte-identical output.
- Rendered pages no longer download stylesheets. Extraction reads the serialized DOM rather than computed style, so the round trips only added latency. Median browser scrape time dropped 31 percent and parallel throughput rose 29 percent, with byte-identical Markdown across twelve pages.

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

[Unreleased]: https://github.com/Fractal-Tess/scorch/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/Fractal-Tess/scorch/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/Fractal-Tess/scorch/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Fractal-Tess/scorch/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Fractal-Tess/scorch/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Fractal-Tess/scorch/compare/v0.1.3...v0.3.0
[0.1.3]: https://github.com/Fractal-Tess/scorch/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Fractal-Tess/scorch/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Fractal-Tess/scorch/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Fractal-Tess/scorch/releases/tag/v0.1.0
