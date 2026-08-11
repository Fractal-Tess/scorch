# Changelog

All notable changes to Scorch are documented in this file. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added request-level metasearch engine subsets bounded by the server allowlist.
- Added completed-engine diagnostics to search responses.

### Changed

- Made Bing and DuckDuckGo the default search engines while keeping Naver and Wikipedia available on explicit requests.
- Improved Agent Skill guidance for locale-sensitive search, compact results, and structured JSON APIs.

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

[Unreleased]: https://github.com/Fractal-Tess/scorch/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/Fractal-Tess/scorch/compare/v0.1.3...v0.3.0
[0.1.3]: https://github.com/Fractal-Tess/scorch/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Fractal-Tess/scorch/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Fractal-Tess/scorch/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Fractal-Tess/scorch/releases/tag/v0.1.0
