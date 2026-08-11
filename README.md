<p align="center">
  <img src="docs/public/brand/scorch-mark.png" alt="Scorch flame mark" width="160" height="160">
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
scorch scrape https://example.com --format markdown,links
scorch scrape https://example.com --render always --format markdown
scorch scrape https://example.com --format screenshot --full-page-screenshot
scorch map https://example.com --limit 100
scorch crawl https://example.com --limit 20 --max-depth 2 --wait
```

The default endpoint is `http://127.0.0.1:33000`. Override it with `SCORCH_API_URL` or `--api-url`. Metasearch combines Bing, DuckDuckGo, Naver, and Wikipedia by default; Brave and Google are optional credential-backed engines.

## Smaller than self-hosted Firecrawl

Firecrawl's [official self-hosted stack](https://github.com/firecrawl/firecrawl/blob/e72fe3acac88651c31fc2ac8398926d7fa2fcdd3/docker-compose.yaml) runs six long-lived services. Scorch runs one daemon with no database, cache, or message broker.

| | Self-hosted Firecrawl | Scorch |
| --- | ---: | ---: |
| Long-running services | 6 | **1** |
| First render | 0.661 s | **0.192 s** |
| Warm renderer memory | 407.9 MiB | **37.3 MiB** |
| Peak renderer processes | 14 | **1** |
| Parallel render speed | 3.03 req/s | 1.99 req/s |
| Database, cache, and broker | PostgreSQL, Redis, RabbitMQ | **None** |

Scorch is lighter because Obscura runs inside `scorchd`, static pages skip the browser, and crawl jobs stay in bounded memory. That means less startup and coordination before a crawl begins.

The Scorch figures were confirmed on an optimized `0.2.0` release-candidate build using the median of three trials, four public pages, and eight parallel renders at concurrency four; all requests succeeded. The Firecrawl column uses the same host's Chromium baseline because Firecrawl uses Playwright with Chromium and publishes no equivalent reproducible self-hosted benchmark. This is not an end-to-end crawl-speed claim. See the [architecture guide](docs/src/pages/architecture.astro) for details.

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
curl -fsSL https://raw.githubusercontent.com/Fractal-Tess/scorch/v0.3.0/install.sh \
  | sh -s -- --version 0.3.0 --install-dir ~/.local/bin
```

### Release archive

Download the archive from [GitHub Releases](https://github.com/Fractal-Tess/scorch/releases) for the current Linux architecture and verify its published checksum:

```sh
VERSION=0.3.0
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
nix run github:Fractal-Tess/scorch/v0.3.0#scorch -- --help
nix run github:Fractal-Tess/scorch/v0.3.0#scorchd -- --help
```

Use `github:Fractal-Tess/scorch/v0.3.0` as a flake input. The flake exports packages, an overlay, NixOS and Home Manager modules, checks, and the Agent Skill. See the [Nix guide](docs/src/pages/nix.astro) for a complete declarative configuration.

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
nix build github:Fractal-Tess/scorch/v0.3.0#skill
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

## Security

Direct fetches and Obscura traffic share SSRF, DNS, redirect, unsafe-port, response-size, concurrency, and deadline controls. Crawl state is bounded, ephemeral, and lost when `scorchd` restarts. Authentication and TLS belong at the deployment gateway; do not expose Scorch to an untrusted network without them.

## Documentation

- [Getting started](docs/src/pages/getting-started.astro)
- [HTTP API](docs/src/pages/api.astro)
- [Architecture](docs/src/pages/architecture.astro)
- [Configuration](docs/src/pages/configuration.astro)
- [Nix](docs/src/pages/nix.astro)

## License

Scorch is MIT licensed © 2026 Fractal-Tess. Dependency licenses and attributions are recorded in [THIRD_PARTY_LICENSES.html](THIRD_PARTY_LICENSES.html).
