# Changelog

All notable changes to Scorch are documented in this file. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/Fractal-Tess/scorch/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/Fractal-Tess/scorch/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Fractal-Tess/scorch/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Fractal-Tess/scorch/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Fractal-Tess/scorch/releases/tag/v0.1.0
