<p align="center">
  <img src="docs/public/brand/scorch-shadow-mark.png" alt="Scorch shadow mark" width="160" height="160">
</p>

<h1 align="center">Scorch</h1>

<p align="center">
  Self-contained web search, scraping, mapping, and crawling in Rust.
</p>

<p align="center">
  <a href="https://github.com/Fractal-Tess/scorch/actions/workflows/release.yml"><img src="https://img.shields.io/github/actions/workflow/status/Fractal-Tess/scorch/release.yml?label=build" alt="Release build status"></a>
  <a href="https://github.com/Fractal-Tess/scorch/actions/workflows/promote-release.yml"><img src="https://img.shields.io/github/actions/workflow/status/Fractal-Tess/scorch/promote-release.yml?label=publish" alt="Release publication status"></a>
  <a href="https://github.com/Fractal-Tess/scorch/releases/latest"><img src="https://img.shields.io/github/v/release/Fractal-Tess/scorch?sort=semver" alt="Latest release"></a>
  <a href="https://skills.sh/Fractal-Tess/scorch"><img src="https://skills.sh/b/Fractal-Tess/scorch" alt="Scorch skill installs"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/Fractal-Tess/scorch" alt="MIT license"></a>
</p>

## What is Scorch?

Scorch is a local-first web retrieval service for people and AI agents. It searches multiple public engines, extracts clean page content, renders JavaScript and screenshots, discovers site URLs, and runs bounded crawls behind one HTTP API. It is designed for private, self-hosted deployments without a database or external browser stack.

Scorch has two executables:

- `scorchd` — HTTP API, metasearch, embedded Obscura rendering, and bounded crawl runtime.
- `scorch` — lightweight HTTP client and MCP stdio adapter.

Static pages use direct HTTP. JavaScript rendering and screenshots use embedded Obscura with stealth transport enabled by default. Scorch needs no database, broker, external browser service, or worker deployment.

## Examples

Start `scorchd`, then search, extract, render, map, or crawl with the lightweight client:

```sh
scorchd

scorch search "rust async runtime" --limit 5
scorch search "rust HTTP clients" --category github
scorch search "времето в София" --country bg --language bg --engine google-cse
scorch search "rust HTTP clients" --engine brave-web
scorch search "времето в София" --country bg --language bg --engine bing,duckduckgo
scorch scrape https://example.com --format markdown,links
scorch scrape https://example.com --render always --format markdown
scorch scrape https://example.com --format screenshot --full-page-screenshot
scorch map https://example.com --limit 100
scorch crawl https://example.com --limit 20 --max-depth 2 --wait
```

The default endpoint is `http://127.0.0.1:33000`. Override it with `SCORCH_API_URL` or `--api-url`. The server allows 19 live-validated, credential-free engines by default: bing, brave-web, crates-io, crossref, docker-hub, duckduckgo, github, google-cse, hacker-news, hugging-face, mwmbl, npm, nvd, openalex, open-library, pubmed, wikidata, wikipedia, yahoo. Ordinary requests still use only DuckDuckGo unless they select another allowed subset. Use `--category github` for GitHub-restricted discovery, select `--engine brave-web` for Brave's public website results, select `--engine google-cse` for Google-derived Programmable Search results, or add Bing with `--engine bing,duckduckgo`. The `brave-web` integration scrapes Brave's public HTML and is best-effort; `brave` remains the separate official credential-backed API adapter. The official Google JSON API adapter is also credential-backed; [Google's Custom Search JSON API](https://developers.google.com/custom-search/v1/overview) is closed to new customers and scheduled for retirement on January 1, 2027. Google CSE uses Blackle's public Programmable Search Engine and may change independently of Scorch.

## Self-hosted footprint and browser-scrape benchmark

We compared Scorch `0.5.0` with Firecrawl `2.11.0` at commit [`ef12eb36`](https://github.com/firecrawl/firecrawl/tree/ef12eb36b2f3382838dfe0a0c1a5add3d5df7fe5). Firecrawl used its pinned, unmodified full Docker Compose configuration, which starts six long-running containers plus a completed one-shot FoundationDB initializer. Scorch ran as one systemd service with maximum concurrency four.

| Measured result | Firecrawl | Scorch |
| --- | ---: | ---: |
| Long-running deployment units | 6 containers | **1 service** |
| Sequential scrape latency, median (12 requests) | **0.859 s** | 1.822 s |
| Parallel trial duration, median (8 requests, concurrency 4) | **2.296 s** | 7.644 s |
| Successful scrape throughput at concurrency 4 | **3.48 req/s** | 1.05 req/s |
| Successful parallel requests | 24 / 24 | 24 / 24 |
| Warm-idle measured cgroup memory | 2,931 MiB | **118 MiB** |
| Observed peak measured cgroup memory | 3,107 MiB | **236 MiB** |
| Observed OS processes, warm / peak | 49 / 57 | **1 / 1** |
| Bundled state and queue services started | PostgreSQL, Redis, RabbitMQ, FoundationDB | **None** |

This is a small end-to-end browser-rendered Markdown scrape and deployment-footprint microbenchmark, not a feature-parity, extraction-quality, crawl-speed, startup-time, maximum-throughput, or CPU-efficiency comparison. It shows the tradeoff in the tested configurations: Firecrawl processed this workload faster, while Scorch used about 25× less warm-idle memory, about 13× less observed peak memory, and one process.

Four public pages were used: Example Domain, Scrape This Site, Books to Scrape, and Quotes to Scrape's JavaScript page. Sequential latency is the pooled median of 12 balanced requests—three per URL. Parallel throughput is derived from the median of three balanced eight-request trials—two requests per URL per trial—at client concurrency four. Product and URL order were alternated. Both APIs had browser rendering forced with a 1 ms post-load wait and 30-second timeout; Firecrawl caching was disabled. Success required a successful API response, a 2xx/3xx page status, non-empty Markdown containing page-specific expected content, and, for Scorch, confirmation that Obscura rendered the page.

Memory was sampled every 50 ms from cgroup v2 as `memory.current - inactive_file`, matching Docker's Linux working-set convention. Firecrawl values are simultaneous sums across its six running container cgroups; Scorch uses the `scorchd` service cgroup. Process counts come from recursive `cgroup.procs` and exclude threads. Docker/containerd daemons and shims, systemd, the benchmark client, build time, and the completed one-shot initializer are excluded. Results were collected on August 13, 2026, on a 16-thread Ryzen 7 5825U host with 30.7 GiB RAM, Docker 29.6.2, and Compose 5.4.0. Public-network conditions and the small page sample make the timing figures host- and run-specific.

## Install

### Installer

Install the latest release into `~/.local/bin` with either curl or wget:

```sh
curl -fsSL https://github.com/Fractal-Tess/scorch/releases/latest/download/install.sh | sh
```

```sh
wget -qO- https://github.com/Fractal-Tess/scorch/releases/latest/download/install.sh | sh
```

The installer detects the Linux architecture and verifies the release checksum. Pin a version or choose another directory with:

```sh
curl -fsSL https://raw.githubusercontent.com/Fractal-Tess/scorch/v0.5.0/install.sh \
  | sh -s -- --version 0.5.0 --install-dir ~/.local/bin
```

### Release archive

Download the archive from [GitHub Releases](https://github.com/Fractal-Tess/scorch/releases) for the current Linux architecture and verify its published checksum:

```sh
VERSION=0.5.0
TARGET="$(uname -m)-unknown-linux-gnu"
ARCHIVE="scorch-v${VERSION}-${TARGET}.tar.xz"
BASE="https://github.com/Fractal-Tess/scorch/releases/download/v${VERSION}"

curl -fLO "${BASE}/${ARCHIVE}"
curl -fLO "${BASE}/${ARCHIVE}.sha256"
sha256sum --check "${ARCHIVE}.sha256"
tar -xJf "${ARCHIVE}"
sudo install -Dm755 \
  "scorch-v${VERSION}-${TARGET}/bin/scorch" \
  "scorch-v${VERSION}-${TARGET}/bin/scorchd" \
  /usr/local/bin/
```

Release archives support `x86_64-linux` and `aarch64-linux`.

### Nix

Nix downloads the same fixed-hash CI binaries rather than compiling the Rust workspace:

```sh
nix run github:Fractal-Tess/scorch/v0.5.0#scorch -- --help
nix run github:Fractal-Tess/scorch/v0.5.0#scorchd -- --help
```

Use `github:Fractal-Tess/scorch/v0.5.0` as a flake input. The flake exports packages, an overlay, NixOS and Home Manager modules, checks, and the Agent Skill. See the [Nix guide](docs/src/pages/nix.astro) for a complete declarative configuration.

## Python and JavaScript clients

Typed, zero-runtime-dependency HTTP clients live under [`clients/`](clients/). They currently install from a repository checkout and are not yet published to PyPI or npm.

```sh
python -m pip install ./clients/python
npm install ./clients/javascript
```

```python
from scorch_client import ScorchClient

results = ScorchClient().search("Rust", categories=["github"])
```

```js
import { ScorchClient } from "@fractal-tess/scorch";

const results = await new ScorchClient().search("Rust", {
  categories: ["github"],
});
```

Both clients cover health, readiness, search, scrape, map, crawl lifecycle, structured errors, authenticated gateway headers, timeouts, cancellation where supported, and bounded response reads. See the [client guide](docs/src/pages/clients.astro) for details.

## MCP and agent setup

Start `scorchd` first. Each agent spawns `scorch mcp` as a local stdio child process, discovers its six tools, and forwards tool calls to the daemon at `http://127.0.0.1:33000` by default. The `scorch` executable must be available on the agent's `PATH`.

### [Claude Code](https://code.claude.com/docs/en/mcp)

```sh
claude mcp add --scope user scorch -- scorch mcp
```

### [Codex](https://developers.openai.com/codex/mcp/)

```sh
codex mcp add scorch -- scorch mcp
```

### [OpenCode](https://opencode.ai/docs/mcp-servers/)

Merge this into `~/.config/opencode/opencode.json`:

```json
{
  "mcp": {
    "scorch": {
      "type": "local",
      "command": ["scorch", "mcp"],
      "enabled": true
    }
  }
}
```

### [Gemini CLI](https://geminicli.com/docs/tools/mcp-server/)

```sh
gemini mcp add --scope user scorch scorch mcp
```

### Other MCP clients

Clients using the common `mcpServers` format can launch Scorch with:

```json
{
  "mcpServers": {
    "scorch": {
      "command": "scorch",
      "args": ["mcp"]
    }
  }
}
```

Scorch exposes six tools: `scorch_search`, `scorch_scrape`, `scorch_map`, `scorch_crawl_start`, `scorch_crawl_status`, and `scorch_crawl_cancel`. Some clients prefix tool names with the server name; OpenCode therefore displays names such as `scorch_scorch_search`. Protocol output stays isolated on stdout. Check connections with `claude mcp list`, `codex mcp list`, `opencode mcp list`, or `gemini mcp list`.

## Agent Skill

The standard skill lives at [`.agents/skills/scorch/SKILL.md`](.agents/skills/scorch/SKILL.md). Install it with the open Agent Skills CLI:

```sh
npx skills add Fractal-Tess/scorch --skill scorch
```

Alternatively, build its installable Nix package:

```sh
nix build github:Fractal-Tess/scorch/v0.5.0#skill
```

## Development

The included `.envrc` loads the flake development shell:

```sh
direnv allow
cargo run -p scorch-server --
```

Without direnv, use `nix develop`.

Run the core checks with:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo deny check advisories licenses sources
```

Documentation uses Astro and Bun:

```sh
cd docs
bun install --frozen-lockfile
bun run build
```

Validate the application clients with:

```sh
cd clients/python
python -m pip install --editable . -r requirements-dev.txt
ruff format --check . && ruff check .
basedpyright src
python -m unittest discover -s tests -v

cd ../javascript
npm ci
npm run check
npm test
```

## Security

Direct fetches and Obscura traffic share SSRF, DNS, redirect, unsafe-port, response-size, concurrency, and deadline controls. Crawl state is bounded, ephemeral, and lost when `scorchd` restarts. Authentication and TLS belong at the deployment gateway; do not expose Scorch to an untrusted network without them.

## Documentation

- [Getting started](docs/src/pages/getting-started.astro)
- [HTTP API](docs/src/pages/api.astro)
- [Python and JavaScript clients](docs/src/pages/clients.astro)
- [Architecture](docs/src/pages/architecture.astro)
- [Configuration](docs/src/pages/configuration.astro)
- [Nix](docs/src/pages/nix.astro)

## License

Scorch is MIT licensed © 2026 Fractal-Tess. Dependency licenses and attributions are recorded in [THIRD_PARTY_LICENSES.html](THIRD_PARTY_LICENSES.html).
