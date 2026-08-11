# Search engine compatibility

> Status inventory for clean-room Scorch integrations inspired by publicly observable search protocols. A checked item is implemented and tested. An unchecked item is not shipped; see the reason code and notes below. SearXNG source code is AGPL and is not copied.

## Engine adapter inventory

| Status | SearXNG adapter | Scorch name | Decision |
|---|---|---|---|
| [ ] | `mankier` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `packagist` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `1337x` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `360search` | `—` | Under evaluation [E] |
| [ ] | `360search_videos` | `—` | Needs a richer media/map result model [M] |
| [ ] | `500px` | `—` | Needs a richer media/map result model [M] |
| [ ] | `9gag` | `—` | Needs a richer media/map result model [M] |
| [ ] | `acfun` | `—` | Needs a richer media/map result model [M] |
| [ ] | `adobe_stock` | `—` | Needs a richer media/map result model [M] |
| [ ] | `ahmia` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `alpinelinux` | `—` | Under evaluation [E] |
| [ ] | `annas_archive` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `ansa` | `—` | Under evaluation [E] |
| [ ] | `apkmirror` | `—` | Under evaluation [E] |
| [ ] | `apple_app_store` | `—` | Under evaluation [E] |
| [ ] | `apple_maps` | `—` | Needs a richer media/map result model [M] |
| [ ] | `archlinux` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `artic` | `—` | Needs a richer media/map result model [M] |
| [ ] | `artstation` | `—` | Needs a richer media/map result model [M] |
| [ ] | `arxiv` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `astrophysics_data_system` | `—` | Credentials or paid service [C] |
| [ ] | `azure` | `—` | Credentials or paid service [C] |
| [ ] | `baidu` | `—` | Under evaluation [E] |
| [ ] | `bandcamp` | `—` | Needs a richer media/map result model [M] |
| [ ] | `base` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `bilibili` | `—` | Needs a richer media/map result model [M] |
| [x] | `bing` | `bing` | Implemented |
| [ ] | `bing_images` | `—` | Needs a richer media/map result model [M] |
| [ ] | `bing_news` | `—` | Under evaluation [E] |
| [ ] | `bing_videos` | `—` | Needs a richer media/map result model [M] |
| [ ] | `bitchute` | `—` | Needs a richer media/map result model [M] |
| [ ] | `boardreader` | `—` | Under evaluation [E] |
| [ ] | `bpb` | `—` | Under evaluation [E] |
| [x] | `brave` | `brave` | Implemented |
| [ ] | `braveapi` | `—` | Credentials or paid service [C] |
| [ ] | `bt4g` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `btdigg` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `cachy_os` | `—` | Under evaluation [E] |
| [ ] | `cara` | `—` | Needs a richer media/map result model [M] |
| [ ] | `ccc_media` | `—` | Needs a richer media/map result model [M] |
| [ ] | `chatnoir` | `—` | Under evaluation [E] |
| [ ] | `chefkoch` | `—` | Under evaluation [E] |
| [ ] | `chinaso` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `cloudflareai` | `—` | Credentials or paid service [C] |
| [ ] | `command` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `core` | `—` | Credentials or paid service [C] |
| [x] | `crates` | `crates-io` | Implemented and live-tested |
| [x] | `crossref` | `crossref` | Implemented and live-tested |
| [ ] | `currency_convert` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `dailymotion` | `—` | Needs a richer media/map result model [M] |
| [ ] | `deepl` | `—` | Credentials or paid service [C] |
| [ ] | `deezer` | `—` | Needs a richer media/map result model [M] |
| [ ] | `demo_offline` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `demo_online` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `destatis` | `—` | Under evaluation [E] |
| [ ] | `deviantart` | `—` | Needs a richer media/map result model [M] |
| [ ] | `devicons` | `—` | Needs a richer media/map result model [M] |
| [ ] | `dictzone` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `digbt` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `discourse` | `—` | Requires external or operator-selected configuration [X] |
| [x] | `docker_hub` | `docker-hub` | Implemented and live-tested |
| [ ] | `dogpile` | `—` | Under evaluation [E] |
| [ ] | `doku` | `—` | Requires external or operator-selected configuration [X] |
| [x] | `duckduckgo` | `duckduckgo` | Implemented |
| [ ] | `duckduckgo_definitions` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `duckduckgo_extra` | `—` | Under evaluation [E] |
| [ ] | `duckduckgo_weather` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `duckduckgo_web` | `—` | Under evaluation [E] |
| [ ] | `duden` | `—` | Under evaluation [E] |
| [ ] | `dummy` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `dummy-offline` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `ebay` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `elasticsearch` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `emojipedia` | `—` | Under evaluation [E] |
| [ ] | `exaapi` | `—` | Credentials or paid service [C] |
| [ ] | `fdroid` | `—` | Under evaluation [E] |
| [ ] | `findfiles` | `—` | Under evaluation [E] |
| [ ] | `findthatmeme` | `—` | Needs a richer media/map result model [M] |
| [ ] | `fireball` | `—` | Under evaluation [E] |
| [ ] | `flaticon` | `—` | Needs a richer media/map result model [M] |
| [ ] | `flickr` | `—` | Credentials or paid service [C] |
| [ ] | `flickr_noapi` | `—` | Needs a richer media/map result model [M] |
| [ ] | `freesound` | `—` | Credentials or paid service [C] |
| [ ] | `frinkiac` | `—` | Needs a richer media/map result model [M] |
| [ ] | `fyyd` | `—` | Needs a richer media/map result model [M] |
| [ ] | `geizhals` | `—` | Under evaluation [E] |
| [ ] | `genius` | `—` | Under evaluation [E] |
| [ ] | `giphy` | `—` | Needs a richer media/map result model [M] |
| [ ] | `gitea` | `—` | Requires external or operator-selected configuration [X] |
| [x] | `github` | `github` | Implemented and live-tested |
| [ ] | `github_code` | `—` | Credentials or paid service [C] |
| [ ] | `gitlab` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `gmx` | `—` | Under evaluation [E] |
| [ ] | `goodreads` | `—` | Under evaluation [E] |
| [x] | `google` | `google` | Implemented |
| [x] | `google_cse` | `google-cse` | Implemented |
| [ ] | `google_images` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `google_news` | `—` | Under evaluation [E] |
| [ ] | `google_play` | `—` | Needs a richer media/map result model [M] |
| [ ] | `google_scholar` | `—` | Under evaluation [E] |
| [ ] | `google_videos` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `grokipedia` | `—` | Inactive or unavailable upstream [I] |
| [x] | `hackernews` | `hacker-news` | End-to-end live-tested; one transient timeout followed by three consecutive successful binary-level searches |
| [ ] | `heexy` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `hex` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [x] | `huggingface` | `hugging-face` | Implemented and live-tested |
| [ ] | `il_post` | `—` | Under evaluation [E] |
| [ ] | `imdb` | `—` | Under evaluation [E] |
| [ ] | `imgur` | `—` | Needs a richer media/map result model [M] |
| [ ] | `ina` | `—` | Needs a richer media/map result model [M] |
| [ ] | `invidious` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `ipernity` | `—` | Needs a richer media/map result model [M] |
| [ ] | `iqiyi` | `—` | Needs a richer media/map result model [M] |
| [ ] | `iseek` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `jina` | `—` | Credentials or paid service [C] |
| [ ] | `jisho` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `json_engine` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `kagi` | `—` | Credentials or paid service [C] |
| [ ] | `keenable` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `kickass` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `lemmy` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `lib_rs` | `—` | Under evaluation [E] |
| [ ] | `libretranslate` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `lingva` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `loc` | `—` | Needs a richer media/map result model [M] |
| [ ] | `lucide` | `—` | Needs a richer media/map result model [M] |
| [ ] | `luxxle` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `magnific` | `—` | Needs a richer media/map result model [M] |
| [ ] | `marginalia` | `—` | Credentials or paid service [C] |
| [ ] | `mariadb_server` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `mastodon` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `material_icons` | `—` | Needs a richer media/map result model [M] |
| [ ] | `mediathekviewweb` | `—` | Needs a richer media/map result model [M] |
| [ ] | `mediawiki` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `meilisearch` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `metacpan` | `—` | Under evaluation [E] |
| [ ] | `microsoft_learn` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `mixcloud` | `—` | Needs a richer media/map result model [M] |
| [ ] | `mojeek` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `mongodb` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `moviepilot` | `—` | Under evaluation [E] |
| [ ] | `mozhi` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `mrs` | `—` | Requires external or operator-selected configuration [X] |
| [x] | `mwmbl` | `mwmbl` | Implemented and live-tested |
| [ ] | `mysql_server` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `naver` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `neocities` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `neosearch` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `niconico` | `—` | Needs a richer media/map result model [M] |
| [x] | `npm` | `npm` | Implemented and live-tested |
| [x] | `nvd` | `nvd` | Implemented and live-tested |
| [ ] | `nyaa` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `odysee` | `—` | Needs a richer media/map result model [M] |
| [ ] | `ollama` | `—` | Under evaluation [E] |
| [ ] | `open_meteo` | `—` | Not an ordinary titled-URL search operation [M] |
| [x] | `openalex` | `openalex` | Implemented and live-tested |
| [ ] | `openclipart` | `—` | Inactive or unavailable upstream [I] |
| [x] | `openlibrary` | `open-library` | Implemented and live-tested |
| [ ] | `opensemantic` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `openstreetmap` | `—` | Needs a richer media/map result model [M] |
| [ ] | `openverse` | `—` | Needs a richer media/map result model [M] |
| [ ] | `pdbe` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `peertube` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `pexels` | `—` | Needs a richer media/map result model [M] |
| [ ] | `photon` | `—` | Needs a richer media/map result model [M] |
| [ ] | `picjumbo` | `—` | Needs a richer media/map result model [M] |
| [ ] | `pinterest` | `—` | Needs a richer media/map result model [M] |
| [ ] | `piped` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `piratebay` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `pixabay` | `—` | Needs a richer media/map result model [M] |
| [ ] | `pixiv` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `pkg_go_dev` | `—` | Under evaluation [E] |
| [ ] | `podchaser` | `—` | Under evaluation [E] |
| [ ] | `postgresql` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `privacywall` | `—` | Under evaluation [E] |
| [ ] | `public_domain_image_archive` | `—` | Needs a richer media/map result model [M] |
| [x] | `pubmed` | `pubmed` | Implemented and live-tested |
| [ ] | `pypi` | `—` | Public search currently returns a JavaScript client challenge [B] |
| [ ] | `quark` | `—` | Under evaluation [E] |
| [ ] | `qwant` | `—` | Under evaluation [E] |
| [ ] | `radio_browser` | `—` | Needs a richer media/map result model [M] |
| [ ] | `recoll` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `repology` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `resulthunter` | `—` | Under evaluation [E] |
| [ ] | `reuters` | `—` | Under evaluation [E] |
| [ ] | `rottentomatoes` | `—` | Under evaluation [E] |
| [ ] | `rumble` | `—` | Needs a richer media/map result model [M] |
| [ ] | `s1search` | `—` | Under evaluation [E] |
| [ ] | `scanr_structures` | `—` | Under evaluation [E] |
| [ ] | `searchzee` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `searx_engine` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `seekninja` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `selfhst` | `—` | Needs a richer media/map result model [M] |
| [ ] | `semantic_scholar` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `senscritique` | `—` | Under evaluation [E] |
| [ ] | `sepiasearch` | `—` | Needs a richer media/map result model [M] |
| [ ] | `seznam` | `—` | Under evaluation [E] |
| [ ] | `shopify_stock` | `—` | Needs a richer media/map result model [M] |
| [ ] | `sogou` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `sogou_images` | `—` | Needs a richer media/map result model [M] |
| [ ] | `sogou_videos` | `—` | Needs a richer media/map result model [M] |
| [ ] | `sogou_wechat` | `—` | Under evaluation [E] |
| [ ] | `solidtorrents` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `solr` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `soundcloud` | `—` | Needs a richer media/map result model [M] |
| [ ] | `sourcehut` | `—` | Under evaluation [E] |
| [ ] | `spotify` | `—` | Credentials or paid service [C] |
| [ ] | `springer` | `—` | Credentials or paid service [C] |
| [ ] | `sqlite` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `stackexchange` | `—` | Removed from the English-focused binary allowlist as redundant with general web and GitHub search [X] |
| [ ] | `startpage` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `startpagina` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `steam` | `—` | Removed from the English-focused binary allowlist as redundant, non-English, or too specialized [X] |
| [ ] | `stocksnap` | `—` | Needs a richer media/map result model [M] |
| [ ] | `swisscows` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `swisscows_news` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `tagesschau` | `—` | Under evaluation [E] |
| [ ] | `tiger` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `tineye` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `tokyotoshokan` | `—` | Excluded for abuse, legal, or security risk [R] |
| [ ] | `tonline` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `tootfinder` | `—` | Under evaluation [E] |
| [ ] | `torznab` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `translated` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `tubearchivist` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `tusksearch` | `—` | Under evaluation [E] |
| [ ] | `unsplash` | `—` | Needs a richer media/map result model [M] |
| [ ] | `uxwing` | `—` | Needs a richer media/map result model [M] |
| [ ] | `valkey_server` | `—` | Requires external or operator-selected configuration [X] |
| [ ] | `vimeo` | `—` | Needs a richer media/map result model [M] |
| [ ] | `voidlinux` | `—` | Under evaluation [E] |
| [ ] | `vuhuv` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `wallhaven` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `wikicommons` | `—` | Needs a richer media/map result model [M] |
| [x] | `wikidata` | `wikidata` | Implemented and live-tested |
| [x] | `wikipedia` | `wikipedia` | Implemented |
| [ ] | `wolframalpha_api` | `—` | Credentials or paid service [C] |
| [ ] | `wolframalpha_noapi` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `wordnik` | `—` | API key required [C] |
| [ ] | `wttr` | `—` | Not an ordinary titled-URL search operation [M] |
| [ ] | `www1x` | `—` | Needs a richer media/map result model [M] |
| [ ] | `xpath` | `—` | Framework/test adapter; not an upstream search source [N] |
| [ ] | `yacy` | `—` | Requires external or operator-selected configuration [X] |
| [x] | `yahoo` | `yahoo` | Implemented and end-to-end live-tested through `scorchd` + `scorch search` |
| [ ] | `yahoo_news` | `—` | Inactive or unavailable upstream [I] |
| [ ] | `yandex` | `—` | Under evaluation [E] |
| [ ] | `yandex_music` | `—` | Needs a richer media/map result model [M] |
| [ ] | `yep` | `—` | Blocked by current live probe or persistent challenge [B] |
| [ ] | `youtube_api` | `—` | Credentials or paid service [C] |
| [ ] | `youtube_noapi` | `—` | Needs a richer media/map result model [M] |
| [ ] | `zlibrary` | `—` | Excluded for abuse, legal, or security risk [R] |

## Decision codes

- **[E] Under evaluation:** protocol, current availability, licensing boundary, and result-model fit still need a live clean-room test.
- **[C] Credentials or paid service:** requires an API key, subscription, OAuth, or another secret and is outside the credential-free target.
- **[X] External configuration:** requires an operator-selected instance, local database/index, command, URL template, proxy, or sidecar.
- **[M] Result-model mismatch:** primarily returns images, video, audio, files, translations, calculations, weather, maps, or other structured answers that `SearchHit` cannot faithfully represent yet.
- **[B] Blocked or challenged:** a live request from the Scorch deployment environment returned CAPTCHA, anti-bot, Cloudflare, or persistent rate limiting.
- **[I] Inactive or unavailable:** upstream is retired, inactive in current SearXNG, or its public endpoint no longer works.
- **[R] Risk exclusion:** source is intentionally excluded for security, abuse, copyright, or distribution-risk reasons.
- **[N] Not an engine:** SearXNG framework, test, or generic adapter rather than a concrete zero-configuration source.

## Implemented engines

- `bing`: credential-free HTML web search; live-tested.
- `crates-io` and `npm`: credential-free Rust and JavaScript/TypeScript package registries; end-to-end live-tested and retained.
- `hugging-face`: credential-free public model search; end-to-end live-tested and retained.
- `nvd`: credential-free CVE search through NVD; end-to-end live-tested and retained.
- `mwmbl`: credential-free independent web index API; end-to-end live-tested and retained.
- Arch Linux, GitLab, Hex, Packagist, ManKier, Jisho, PDBe, Microsoft Learn, Stack Overflow, Steam, and Naver were successfully evaluated but intentionally removed from the final English-focused binary surface.
- `crossref`: credential-free scholarly metadata search; live-tested.
- `docker-hub`: credential-free container repository search; live-tested.
- `openalex`: credential-free scholarly works search; live-tested.
- `pubmed`: credential-free biomedical citation search using NCBI E-utilities; live-tested.
- `hacker-news`: credential-free Hacker News Algolia search; live-tested.
- `github`: credential-free GitHub repository search; live-tested; anonymous quotas are strict.
- `wikidata`: credential-free entity search; live-tested.
- `open-library`: credential-free book catalog search; live-tested.
- `brave`: official paid Brave Search API; retained for existing credentialed deployments, not part of the free target.
- `brave-web`: clean-room public Brave HTML integration; live-tested; CAPTCHA responses are classified as rate limits.
- `duckduckgo`: credential-free HTML web search; challenge-prone but supported.
- `google`: official credential-backed Custom Search JSON API; retained for existing deployments, not part of the free target.
- `google-cse`: credential-free public Programmable Search Element integration; live-tested and request-explicit.
- `wikipedia`: credential-free MediaWiki API search; live-tested.

## Current live-test blockers

- `mojeek` **[B]**: `https://www.mojeek.com/search` returned an ALTCHA page requiring JavaScript on the current server IP.
- `yep` **[B]**: `https://api.yep.com/search` returned HTTP 403 from Cloudflare on the current server IP.
- `arxiv` **[B]**: `https://export.arxiv.org/api/query` returned HTTP 429 during the clean-room probe.
- `semantic_scholar` **[B]**: the unauthenticated Graph API returned HTTP 429 during the clean-room probe.

- PyPI public search returned a JavaScript client-challenge page during live testing.
- Wordnik returned HTTP 401 without an API key during live testing.

## Maintenance rules

1. Verify every candidate against its public endpoint from the Scorch environment.
2. Implement from public protocol observations and documentation, not SearXNG implementation code.
3. Require bounded response streaming, absolute request timeouts, strict parsing, URL sanitization, and rate-limit classification.
4. Keep unstable frontend scrapers request-explicit; do not expand the implicit DuckDuckGo default.
5. Mark an adapter implemented only after unit tests and an ignored live integration test pass.
6. Record failed live probes here and move on rather than shipping an engine that only exists nominally.
