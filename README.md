# Scorch

Scorch is a self-contained web search, scraping, mapping, and crawling service written in Rust. `scorchd` runs the HTTP API, browser, metasearch, and bounded in-memory crawl runtime. The separate `scorch` executable is a lightweight HTTP client for convenient API, benchmark, and MCP access. It uses direct HTTP for inexpensive static pages and embeds Obscura for JavaScript rendering and screenshots. Chromium remains an optional operator-enabled compatibility backend.

Scorch does not require a database, broker, cache server, browser service, or external worker deployment. Crawl state is intentionally ephemeral and is lost when the process restarts.

## Install with Nix

Run either executable directly from the flake. Versioned Linux binaries are built by GitHub Actions and fetched by Nix, so installation does not compile the Rust workspace locally:

```sh
nix run github:Fractal-Tess/scorch/v0.1.2#scorch -- --help
nix run github:Fractal-Tess/scorch/v0.1.2#scorchd -- --help
```

Or enable the client and service declaratively on NixOS:

```nix
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.scorch.url = "github:Fractal-Tess/scorch/v0.1.2";

  outputs = { nixpkgs, scorch, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        scorch.nixosModules.default
        {
          programs.scorch = {
            enable = true;
            apiUrl = "http://127.0.0.1:33000";
          };
          services.scorchd.enable = true;
        }
      ];
    };
  };
}
```

The flake also exports an optional `scorchd-with-chromium` package, an overlay, a Home Manager module, checks, and the Scorch Agent Skill. See the [Nix guide](docs/src/pages/nix.astro) for service policy and secret-file configuration.

## Agent Skill

The repository includes a standard Agent Skill at `.agents/skills/scorch/SKILL.md`. It is discovered automatically while working in this repository. The flake packages it for installation into another agent environment:

```sh
nix build github:Fractal-Tess/scorch/v0.1.2#skill
mkdir -p ~/.agents/skills
rm -rf ~/.agents/skills/scorch
cp -R result/share/agent-skills/scorch ~/.agents/skills/scorch
```

See the [Agent Skill guide](docs/src/pages/skill.astro) for installation and behavior.

## Development

```sh
devenv shell
cargo run -p scorch-server --
```

The API listens on `127.0.0.1:33000` by default. Authentication is intentionally not built into this local-first release; do not expose it to an untrusted network without an authenticated TLS gateway.

## Logging

Scorch writes structured operational logs to stderr. The default compact output records startup and shutdown, request IDs, methods, response statuses and latency, engine selection, operation outcomes, browser lifecycle, and crawl lifecycle. Request bodies, search text, and URL query strings are not logged at info level.

Use `RUST_LOG` for filtering and `SCORCH_LOG_FORMAT=json` for newline-delimited JSON:

```sh
RUST_LOG=scorch=debug cargo run -p scorch-server --
SCORCH_LOG_FORMAT=json RUST_LOG=scorch=info cargo run -p scorch-server --
```

CLI and MCP JSON protocol output remains isolated on stdout.

## CLI

Every web command calls the configured HTTP API:

```sh
cargo run -p scorch-cli -- scrape https://example.com --format markdown,links
cargo run -p scorch-cli -- search "rust async runtime" --limit 5 --scrape
cargo run -p scorch-cli -- map https://example.com --limit 100
cargo run -p scorch-cli -- crawl https://example.com --limit 20 --max-depth 2 --wait
```

Set `SCORCH_API_URL` or pass `--api-url` to use another server.

`scorch` contains no engine, browser, or API-server dependencies. Server and runtime options belong exclusively to `scorchd`:

```sh
cargo run -p scorch-server -- --help
```

Obscura is the default and only allowed browser. Operators can enable Chromium, or make it the default, without letting requests escape that policy:

```sh
SCORCH_ALLOWED_BROWSERS=obscura,chromium scorchd
SCORCH_BROWSER=chromium SCORCH_ALLOWED_BROWSERS=chromium scorchd
```

A scrape request may select an allowed backend with `options.browser`; omitted selection uses `SCORCH_BROWSER`. The service rejects a request that selects a backend not listed in `SCORCH_ALLOWED_BROWSERS`.

Obscura uses its stealth transport by default. For higher throughput where transport-level browser fingerprinting is not required, select the standard transport with `SCORCH_OBSCURA_STEALTH=false` or `scorchd --obscura-stealth false`.

### Metasearch engines

Metasearch is Scorch's only search provider. It owns engine routing, concurrent searching, result merging, and reranking. Configure the engines it is allowed to use independently when starting the API:

```sh
SCORCH_SEARCH_ENGINES=bing,wikipedia cargo run -p scorch-server --
# installed binary: scorchd --search-engines bing,wikipedia
```

Allowed engines are `bing`, `brave`, `duckduckgo`, `google`, `naver`, and `wikipedia`. Bing, DuckDuckGo, Naver, and Wikipedia are enabled by default; Brave and Google require credentials. Engine selection is a server policy and cannot be overridden by individual search requests.

The native metasearch provider combines agreement with reciprocal-rank fusion, caches short-lived results, and stops waiting shortly after useful results arrive. It remains inside `scorchd`. See `docs/metasearch.md` for design details and engine verification.

### Concurrent search dashboard

With the API running, launch the dependency-free Python terminal dashboard:

```sh
python3 scripts/concurrent_search_demo.py \
  --requests 8 \
  --concurrency 4
```

Pass custom queries as positional arguments, add `--scrape` to enrich results, or use `--plain` for non-interactive output. Each request receives a distinct `x-request-id` that also appears in Scorch's service logs.

## HTTP API

```sh
curl -sS http://127.0.0.1:33000/v1/scrape \
  -H 'content-type: application/json' \
  -d '{
    "url": "https://example.com",
    "options": {
      "formats": ["markdown", "links"],
      "render": "auto",
      "browser": "obscura"
    }
  }' | jq
```

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/healthz` | Process liveness |
| `GET` | `/readyz` | Browser readiness and concurrency |
| `POST` | `/v1/scrape` | Fetch or render one page |
| `POST` | `/v1/search` | Search, optionally scraping results |
| `POST` | `/v1/map` | Discover normalized same-site URLs |
| `POST` | `/v1/crawls` | Start an in-memory crawl |
| `GET` | `/v1/crawls/{id}` | Read paginated status and results |
| `DELETE` | `/v1/crawls/{id}` | Cancel and remove a crawl |

See `docs/api.md` for request contracts and limits.

## MCP

Run the API-backed stdio adapter directly:

```sh
cargo run -p scorch-cli -- mcp
```

The adapter performs no web or browser work itself; every tool call is forwarded to `SCORCH_API_URL`.

A client configuration can point at a release build:

```json
{
  "mcpServers": {
    "scorch": {
      "command": "/absolute/path/to/scorch",
      "args": ["mcp"],
      "env": {
        "SCORCH_API_URL": "http://127.0.0.1:33000"
      }
    }
  }
}
```

The tools are `scorch_search`, `scorch_scrape`, `scorch_map`, `scorch_crawl_start`, `scorch_crawl_status`, and `scorch_crawl_cancel`. MCP protocol traffic is written only to stdout; diagnostics go to stderr.

## Browser and security

Obscura runs as an in-process Rust library rather than a sidecar or executable. Obscura and optional Chromium traffic are forced through an embedded validating HTTP/CONNECT proxy. Direct fetches and browser connections reject local, private, link-local, reserved, and unsafe targets, repin DNS addresses, and revalidate redirects. Response sizes, redirects, browser work, crawl depth, crawl count, retained bytes, request bodies, and job lifetimes are bounded.

The browser choice and measured tradeoffs are documented in `docs/browser-evaluation.md`.

## Build profiles

The production profile uses thin LTO, one code-generation unit, symbol stripping, and overflow checks while retaining unwind behavior for panic isolation:

```sh
cargo build --release --workspace
# target/release/scorch and target/release/scorchd
```

For production-equivalent profiling with debug symbols:

```sh
cargo build --profile release-debug --workspace
```

## Checks

```sh
check
fmt --check
lint
test
```

## License

Licensed under the MIT License. Copyright © 2026 Fractal-Tess.
